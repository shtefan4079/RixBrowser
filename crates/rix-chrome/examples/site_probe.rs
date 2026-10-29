#![recursion_limit = "256"]

use rix_chrome::customization::{CustomizationConfig, CustomizationManager};
use rix_chrome::shell::BrowserShell;
use rix_render::display_list::DrawCommand;
use rix_render::VelloRenderer;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn quiet_period_elapsed(
    quiet_since: &mut Option<Instant>,
    is_quiet: bool,
    required_period: Duration,
    now: Instant,
) -> bool {
    if !is_quiet {
        *quiet_since = None;
        return false;
    }
    let started = quiet_since.get_or_insert(now);
    now.saturating_duration_since(*started) >= required_period
}

fn network_and_layout_are_quiet(
    has_pending_page_layouts: bool,
    worker_state: Option<(bool, usize, bool, bool)>,
) -> bool {
    !has_pending_page_layouts
        && worker_state.map_or(true, |(ready, _pending_network, running, busy)| {
            ready && running && !busy
        })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--page-runtime-worker") {
        return rix_chrome::page_worker::run_stdio_worker().map_err(Into::into);
    }
    let log_level = match std::env::var("RIX_LOG") {
        Ok(value) => value.parse().unwrap_or(tracing::Level::WARN),
        Err(_) => tracing::Level::WARN,
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(log_level)
        .try_init();
    let mut args = std::env::args().skip(1);
    let url = args.next().unwrap_or_else(|| "https://example.org/".into());
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("rix-site-probe"));
    let input_selector = args.next();
    let settle_quiet_period = match std::env::var("RIX_SITE_PROBE_SETTLE_MS") {
        Ok(value) => Duration::from_millis(value.parse()?),
        Err(std::env::VarError::NotPresent) => Duration::from_millis(300),
        Err(error) => return Err(error.into()),
    };
    std::fs::create_dir_all(&output)?;
    let css = tempfile::NamedTempFile::new()?;
    let window_width = std::env::var("RIX_WINDOW_WIDTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1280.0);
    let window_height = std::env::var("RIX_WINDOW_HEIGHT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(900.0);
    let mut shell = BrowserShell::new_with_customization(
        window_width,
        window_height,
        CustomizationManager::new(CustomizationConfig {
            custom_css_path: css.path().to_path_buf(),
            ..Default::default()
        }),
    )?;
    let profile_path = match std::env::var("RIX_PROFILE_PATH") {
        Ok(path) => PathBuf::from(path),
        Err(_) => output.join("profile.redb"),
    };
    if let Ok(storage) = rix_storage::StorageEngine::open_persistent(&profile_path) {
        shell.set_storage(storage);
    }
    let worker_arguments = match std::env::var("RIX_PAGE_PROBE_SCRIPT") {
        Ok(path) => vec!["--rix-page-probe-script".into(), path],
        Err(std::env::VarError::NotPresent) => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    shell.page_worker_command = Some(if std::env::var_os("RIX_WORKER_GDB").is_some() {
        let gdb_log = std::env::temp_dir().join("rix_gdb_bt.log");
        let gdb_log_cmd = format!("set logging file {}", gdb_log.display());
        let mut arguments: Vec<String> = [
            "--batch",
            "--quiet",
            "-ex",
            "set pagination off",
            "-ex",
            "set print thread-events off",
            "-ex",
            "set confirm off",
            "-ex",
            &gdb_log_cmd,
            "-ex",
            "set logging redirect on",
            "-ex",
            "set logging enabled on",
            "-ex",
            "run",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        for _ in 0..7 {
            arguments.push("-ex".into());
            arguments.push("thread apply all bt 20".into());
            arguments.push("-ex".into());
            arguments.push("continue".into());
        }
        arguments.push("-ex".into());
        arguments.push("kill".into());
        arguments.push("--args".into());
        arguments.push(std::env::current_exe()?.display().to_string());
        arguments.extend(worker_arguments.iter().cloned());
        rix_chrome::page_worker::WorkerCommand {
            executable: PathBuf::from("gdb"),
            arguments,
        }
    } else {
        rix_chrome::page_worker::WorkerCommand {
            executable: std::env::current_exe()?,
            arguments: worker_arguments,
        }
    });
    let start = Instant::now();
    eprintln!("Navigation start {url}");
    shell.start_navigation_active_tab(&url);
    let mut first_paint = None;
    while shell.has_pending_navigations() {
        shell.poll_navigation_completions();
        shell.step_event_loop();
        if shell.has_pending_animation_frame_opportunity() {
            shell.animation_frame_opportunity(Instant::now());
        }
        if first_paint.is_none()
            && shell
                .pages
                .get(&shell.active_tab_id())
                .is_some_and(|page| !page.html.contains("Connecting to host…"))
        {
            first_paint = Some(start.elapsed());
            eprintln!("First document {:.3}s", start.elapsed().as_secs_f64());
        }
        shell.render_frame();
        if start.elapsed() > Duration::from_secs(60) {
            return Err("Navigation exceeded probe deadline".into());
        }
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
    eprintln!("Navigation commit {:.3}s", start.elapsed().as_secs_f64());
    let page = shell.pages.get(&shell.active_tab_id()).unwrap();
    eprintln!(
        "URL {} status {} DOM nodes {} layout boxes {} HTML bytes {}",
        page.url,
        page.status_message,
        page.document.descendants(page.document.root()).count(),
        page.layout_tree.len(),
        page.html.len()
    );
    let settle_budget = Duration::from_secs(
        std::env::var("RIX_PROBE_SETTLE_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(45),
    );
    let mut settle = Instant::now();
    let mut quiet_since = None;
    let mut settled = false;
    let mut worker_stopped = false;
    loop {
        shell.step_event_loop();
        shell.poll_navigation_completions();
        if shell.has_pending_navigations() {
            settle = Instant::now();
            quiet_since = None;
        }
        if shell.has_pending_animation_frame_opportunity() {
            shell.animation_frame_opportunity(Instant::now());
        }
        shell.render_frame();
        let worker_state = shell
            .pages
            .get(&shell.active_tab_id())
            .and_then(|page| page.page_worker.as_ref())
            .map(|worker| {
                let worker = worker.borrow();
                (
                    worker.ready,
                    worker.pending_network_requests(),
                    worker.is_running(),
                    worker.is_busy(),
                )
            });
        let network_and_layout_quiet =
            network_and_layout_are_quiet(shell.has_pending_page_layouts(), worker_state);
        if quiet_period_elapsed(
            &mut quiet_since,
            network_and_layout_quiet,
            settle_quiet_period,
            Instant::now(),
        ) {
            settled = true;
            break;
        }
        if worker_state.is_some_and(|(_, _, running, _)| !running) {
            worker_stopped = true;
            break;
        }
        if settle.elapsed() > settle_budget {
            break;
        }
        if start.elapsed() > Duration::from_secs(300) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
    eprintln!(
        "Network/layout quiet window {} after {:.3}s (including animation frames and rendering pipeline)",
        if settled {
            "reached"
        } else if worker_stopped {
            "unavailable: page worker stopped"
        } else {
            "timed out"
        },
        start.elapsed().as_secs_f64()
    );
    let page = shell.pages.get(&shell.active_tab_id()).unwrap();
    let pending_network_requests = page
        .page_worker
        .as_ref()
        .map_or(0, |worker| worker.borrow().pending_network_requests());
    let resource_failures = page
        .resource_trace
        .iter()
        .filter(|record| record.error.is_some())
        .count();
    eprintln!(
        "Resource trace {} records, {} failures{}",
        page.resource_trace.len(),
        resource_failures,
        if page.resource_trace_truncated {
            " (truncated)"
        } else {
            ""
        }
    );
    std::fs::write(
        output.join("resource_trace.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "records": &page.resource_trace,
            "truncated": page.resource_trace_truncated,
            "settled": settled,
            "quietCondition": "network-and-layout",
            "workerStopped": worker_stopped,
            "pendingNetworkRequests": pending_network_requests,
        }))?,
    )?;
    let mut step_error: Option<String> = None;
    if let Ok(selector) = std::env::var("RIX_PROBE_CLICK_SELECTOR") {
        if !selector.is_empty() {
            if let Err(error) = click_selector(&mut shell, &selector).await {
                eprintln!("Click step failed: {error}");
                step_error = Some(error.to_string());
            }
        }
    }
    if let Some(selector) = input_selector {
        if let Err(error) = exercise_input(&mut shell, &selector).await {
            eprintln!("Input step failed: {error}");
            if step_error.is_none() {
                step_error = Some(error.to_string());
            }
        }
    }
    if let Some(context) = shell
        .pages
        .get(&shell.active_tab_id())
        .and_then(|page| page.runtime_context.as_ref())
    {
        for message in context.borrow().runtime.console_messages() {
            eprintln!("Console {:?}: {}", message.level, message.message);
        }
    }
    let worker_console = shell
        .pages
        .get(&shell.active_tab_id())
        .and_then(|page| page.page_worker.clone())
        .map(|worker| worker.borrow_mut().take_console_messages())
        .unwrap_or_default();
    for message in &worker_console {
        eprintln!("Page console {}: {}", message.level, message.message);
    }
    std::fs::write(
        output.join("page_console.json"),
        serde_json::to_vec_pretty(&worker_console)?,
    )?;
    let worker_script_timings = shell
        .pages
        .get(&shell.active_tab_id())
        .and_then(|page| page.page_worker.clone())
        .map(|worker| worker.borrow_mut().take_script_timings())
        .unwrap_or_default();
    let mut slowest_scripts = worker_script_timings.clone();
    slowest_scripts.sort_by(|left, right| {
        (right.evaluation_ms + right.promise_jobs_ms)
            .total_cmp(&(left.evaluation_ms + left.promise_jobs_ms))
    });
    for timing in slowest_scripts.iter().take(10) {
        eprintln!(
            "Page script {}: {} bytes, parse {:.1}ms, compile {:.1}ms, execute {:.1}ms, total {:.1}ms, Promise jobs {:.1}ms",
            timing.index,
            timing.source_bytes,
            timing.parse_ms,
            timing.compile_ms,
            timing.execute_ms,
            timing.evaluation_ms,
            timing.promise_jobs_ms
        );
    }
    std::fs::write(
        output.join("page_script_timings.json"),
        serde_json::to_vec_pretty(&worker_script_timings)?,
    )?;
    let frame = Instant::now();
    let page = shell.pages.get(&shell.active_tab_id()).unwrap();
    eprintln!(
        "Settled status {} images {} layout boxes {}",
        page.status_message,
        page.images.len(),
        page.layout_tree.len()
    );
    let html = if page.html.is_empty() {
        page.document.to_html_string(page.document.root())
    } else {
        page.html.clone()
    };
    std::fs::write(output.join("document.html"), &html)?;
    let live_html = page.document.to_html_string(page.document.root());
    std::fs::write(output.join("live_document.html"), &live_html)?;
    let mut all_css = Vec::new();
    for (_, sheets) in page.document.all_adopted_stylesheets() {
        for s in sheets {
            all_css.push(s.clone());
        }
    }
    for (url, css) in &page.external_stylesheets {
        all_css.push(format!("/* URL: {} */\n{}", url, css));
    }
    let _ = std::fs::write(output.join("all_stylesheets.css"), all_css.join("\n\n"));
    let layout: Vec<_> = page.layout_tree.iter().map(|item| {
        let rect = item.dimensions.content;
        let (tag, id, class, attrs) = item
            .dom_node
            .and_then(|node| page.document.get(node))
            .and_then(|node| match node {
                rix_dom::NodeData::Element(element) => Some((
                    Some(element.name.local.to_string()),
                    element.get_attribute("id").map(str::to_string),
                    element.get_attribute("class").map(str::to_string),
                    element
                        .attributes
                        .iter()
                        .map(|(k, v)| (k.local.to_string(), v.clone()))
                        .collect::<Vec<_>>(),
                )),
                _ => None,
            })
            .unwrap_or((None, None, None, Vec::new()));
        let (box_type, text) = match &item.box_type {
            rix_layout::tree::AnonymousBoxType::None => ("element", None),
            rix_layout::tree::AnonymousBoxType::AnonymousBlock => ("anonymous-block", None),
            rix_layout::tree::AnonymousBoxType::AnonymousInline => ("anonymous-inline", None),
            rix_layout::tree::AnonymousBoxType::Text(text) => {
                ("text", Some(text.chars().take(256).collect::<String>()))
            }
            rix_layout::tree::AnonymousBoxType::PseudoElement(pseudo) => (
                match pseudo {
                    rix_dom::query::PseudoElement::Before => "pseudo-before",
                    rix_dom::query::PseudoElement::After => "pseudo-after",
                    rix_dom::query::PseudoElement::Placeholder => "pseudo-placeholder",
                },
                None,
            ),
        };
        let text_layout = item.text_layout.as_ref().map(|layout| {
            serde_json::json!({
                "lineCount": layout.lines.len(),
                "glyphCount": layout.lines.iter().map(|line| line.glyphs.len()).sum::<usize>(),
                "width": layout.width,
                "height": layout.height,
            })
        });
        serde_json::json!({
            "box_id": item.id.0,
            "parent": item.parent.map(|p| p.0),
            "children": item.children.iter().map(|c| c.0).collect::<Vec<_>>(),
            "tag":tag, "id_attr":id, "class":class, "attrs":attrs, "boxType":box_type, "text":text, "textLayout":text_layout,
            "rect":[rect.x,rect.y,rect.width,rect.height], "natural":item.intrinsic_size,
            "padding": [item.dimensions.padding.top, item.dimensions.padding.right, item.dimensions.padding.bottom, item.dimensions.padding.left],
            "border": [item.dimensions.border.top, item.dimensions.border.right, item.dimensions.border.bottom, item.dimensions.border.left],
            "style": {
                "border_radius": item.style.border_radius,
                "corner_radii": item.style.corner_radii(),
                "box_sizing": item.style.box_sizing,
                "padding_top": item.style.padding_top,
                "padding_right": item.style.padding_right,
                "padding_bottom": item.style.padding_bottom,
                "padding_left": item.style.padding_left,
                "width": item.style.width,
                "height": item.style.height,
                "width_percent": item.style.width_percent,
                "height_percent": item.style.height_percent,
                "display": format!("{:?}", item.style.display),
                "flex_grow": item.style.flex_grow,
                "flex_shrink": item.style.flex_shrink,
                "position": item.style.position,
                "color": [item.style.color.r, item.style.color.g, item.style.color.b, item.style.color.a],
                "background_color": [
                    item.style.background_color.r,
                    item.style.background_color.g,
                    item.style.background_color.b,
                    item.style.background_color.a,
                ],
                "visibility": item.style.visibility,
                "opacity": item.style.opacity,
                "z_index": item.style.z_index,
                "font_size": item.style.font_size,
                "font_family": item.style.font_family.chars().take(256).collect::<String>(),
                "overflow_x": item.style.overflow_x,
                "overflow_y": item.style.overflow_y,
                "align_items": item.style.align_items,
                "align_self": item.style.align_self,
                "justify_content": item.style.justify_content,
                "top": item.style.top,
                "bottom": item.style.bottom,
                "left": item.style.left,
                "right": item.style.right,
            },
        })
    }).collect();
    std::fs::write(
        output.join("layout.json"),
        serde_json::to_vec_pretty(&layout)?,
    )?;
    let display_list = &shell
        .pages
        .get(&shell.active_tab_id())
        .unwrap()
        .display_list;
    let mut command_counts = std::collections::BTreeMap::<&'static str, usize>::new();
    let mut text_samples = Vec::new();
    let mut glyph_count = 0usize;
    for item in &display_list.items {
        let kind = match &item.command {
            DrawCommand::DrawRect { .. } => "rect",
            DrawCommand::DrawLinearGradient { .. } => "linear-gradient",
            DrawCommand::DrawBorder { .. } => "border",
            DrawCommand::DrawText { text, .. } => {
                if text_samples.len() < 16 {
                    text_samples.push(text.chars().take(120).collect::<String>());
                }
                "text"
            }
            DrawCommand::DrawGlyphs { glyphs, .. } => {
                glyph_count += glyphs.len();
                "glyphs"
            }
            DrawCommand::DrawPath { .. } => "path",
            DrawCommand::DrawImage { .. } => "image",
            DrawCommand::DrawImageResource { .. } => "image-resource",
            DrawCommand::DrawShadow { .. } => "shadow",
            DrawCommand::PushClip { .. } => "push-clip",
            DrawCommand::PopClip => "pop-clip",
            DrawCommand::PushOpacity { .. } => "push-opacity",
            DrawCommand::PopOpacity => "pop-opacity",
        };
        *command_counts.entry(kind).or_default() += 1;
    }
    std::fs::write(
        output.join("paint_summary.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "displayListItems": display_list.len(),
            "commandCounts": command_counts,
            "glyphCount": glyph_count,
            "textSamples": text_samples,
        }))?,
    )?;
    let (scene, _) = shell.render_frame();
    eprintln!("Scene {:.3}s", frame.elapsed().as_secs_f64());
    let render_width = window_width.round() as u32;
    let render_height = window_height.round() as u32;
    VelloRenderer::render_scene_to_png(&scene, render_width, render_height, &output.join("page.png")).await?;
    let bounds = shell.web_surface.viewport_rect();
    let (x, y) = (
        bounds.x.max(0.0).round() as u32,
        bounds.y.max(0.0).round() as u32,
    );
    if std::env::var_os("RIX_PROBE_DUMP_DISPLAY_LIST").is_some() {
        let page = shell.pages.get(&shell.active_tab_id()).unwrap();
        let items: Vec<_> = page
            .display_list
            .items
            .iter()
            .map(|item| {
                let mut entry = serde_json::json!({"z": item.z_index});
                match &item.command {
                    DrawCommand::DrawRect { rect, color, border_radius, corner_radii } => {
                        entry["kind"] = "rect".into();
                        entry["rect"] = serde_json::json!([rect.x, rect.y, rect.width, rect.height]);
                        entry["color"] =
                            serde_json::json!([color.r, color.g, color.b, color.a]);
                        entry["border_radius"] = (*border_radius).into();
                        entry["corner_radii"] = serde_json::json!(corner_radii);
                    }
                    DrawCommand::DrawGlyphs { glyphs, x, y } => {
                        entry["kind"] = "glyphs".into();
                        entry["count"] = glyphs.len().into();
                        entry["origin"] = serde_json::json!([x, y]);
                        if let Some(first) = glyphs.first() {
                            entry["first"] = serde_json::json!([first.x, first.y]);
                        }
                    }
                    DrawCommand::DrawPath { d, x, y, fill, .. } => {
                        entry["kind"] = "path".into();
                        entry["origin"] = serde_json::json!([x, y]);
                        entry["d"] = d.chars().take(40).collect::<String>().into();
                        if let Some(fill) = fill {
                            entry["fill"] =
                                serde_json::json!([fill.r, fill.g, fill.b, fill.a]);
                        }
                    }
                    DrawCommand::DrawBorder { rect, color, width, border_radius, corner_radii, widths } => {
                        entry["kind"] = "border".into();
                        entry["rect"] = serde_json::json!([rect.x, rect.y, rect.width, rect.height]);
                        entry["color"] =
                            serde_json::json!([color.r, color.g, color.b, color.a]);
                        entry["width"] = (*width).into();
                        entry["widths"] = serde_json::json!(widths);
                        entry["border_radius"] = (*border_radius).into();
                        entry["corner_radii"] = serde_json::json!(corner_radii);
                    }
                    DrawCommand::PushClip { rect, corner_radii } => {
                        entry["kind"] = "push-clip".into();
                        entry["rect"] = serde_json::json!([rect.x, rect.y, rect.width, rect.height]);
                        entry["corner_radii"] = serde_json::json!(corner_radii);
                    }
                    DrawCommand::PushOpacity { opacity } => {
                        entry["kind"] = "push-opacity".into();
                        entry["opacity"] = (*opacity).into();
                    }
                    other => {
                        entry["kind"] = format!("{other:?}")
                            .split([' ', '('])
                            .next()
                            .unwrap_or("other")
                            .into();
                    }
                }
                entry
            })
            .collect();
        std::fs::write(
            output.join("display_list.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "scroll": [page.scroll_x, page.scroll_y],
                "surface": [bounds.x, bounds.y, bounds.width, bounds.height],
                "displayRevision": page.display_revision,
                "items": items,
            }))?,
        )?;
        let translated = page.display_list.translate(bounds.x, bounds.y);
        let layer_scene = rix_render::VelloSceneBuilder::build_scene(&translated);
        VelloRenderer::render_scene_to_png(
            &layer_scene,
            render_width,
            render_height,
            &output.join("page_layer_only.png"),
        )
        .await?;
    }
    let full = image::open(output.join("page.png"))?.to_rgba8();
    let width = (bounds.width.round() as u32).min(full.width().saturating_sub(x));
    let height = (bounds.height.round() as u32).min(full.height().saturating_sub(y));
    if width > 0 && height > 0 {
        image::imageops::crop_imm(&full, x, y, width, height)
            .to_image()
            .save(output.join("viewport.png"))?;
        std::fs::write(
            output.join("viewport.json"),
            serde_json::to_vec(
                &serde_json::json!({"x": x, "y": y, "width": width, "height": height}),
            )?,
        )?;
        eprintln!("Viewport {width}x{height} at {x},{y}");
    }
    eprintln!(
        "Total {:.3}s screenshot {}",
        start.elapsed().as_secs_f64(),
        output.join("page.png").display()
    );
    if let Some(error) = step_error {
        return Err(error.into());
    }
    Ok(())
}

async fn pump(shell: &mut BrowserShell) {
    tokio::time::sleep(Duration::from_millis(16)).await;
    shell.step_event_loop();
    shell.poll_navigation_completions();
    shell.render_frame();
}

async fn wait_for_quiet(shell: &mut BrowserShell, required: Duration, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    let mut quiet_since = None;
    while Instant::now() < deadline {
        shell.step_event_loop();
        shell.poll_navigation_completions();
        if shell.has_pending_animation_frame_opportunity() {
            shell.animation_frame_opportunity(Instant::now());
        }
        shell.render_frame();
        let worker_state = shell
            .pages
            .get(&shell.active_tab_id())
            .and_then(|page| page.page_worker.as_ref())
            .map(|worker| {
                let worker = worker.borrow();
                (
                    worker.ready,
                    worker.pending_network_requests(),
                    worker.is_running(),
                    worker.is_busy(),
                )
            });
        let quiet = !shell.has_pending_navigations()
            && network_and_layout_are_quiet(shell.has_pending_page_layouts(), worker_state);
        if quiet_period_elapsed(&mut quiet_since, quiet, required, Instant::now()) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
    false
}

async fn wait_for_value(
    shell: &mut BrowserShell,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pump(shell).await;
        if shell
            .pages
            .get(&shell.active_tab_id())
            .is_some_and(|page| page.in_page_input == expected)
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("Native input did not become {expected:?}").into());
        }
    }
}

async fn wait_for_painted_value(
    shell: &mut BrowserShell,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let displayed = shell.pages.get(&shell.active_tab_id()).is_some_and(|page| {
            page.focused_input.is_some_and(|focused| {
                page.layout_tree.iter().any(|item| {
                    item.dom_node == Some(focused)
                        && matches!(&item.box_type, rix_layout::AnonymousBoxType::Text(text) if text == expected)
                })
            })
        });
        if displayed {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("Native input value {expected:?} did not reach layout").into());
        }
        pump(shell).await;
    }
}

async fn click_selector(
    shell: &mut BrowserShell,
    selector: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (x, y) = {
        let page = shell
            .pages
            .get(&shell.active_tab_id())
            .ok_or("Missing page")?;
        let node = page
            .document
            .query_selector(page.document.root(), selector)
            .ok_or("Click selector did not match")?;
        let rect = page
            .layout_tree
            .iter()
            .find(|item| {
                item.dom_node == Some(node)
                    && !matches!(item.box_type, rix_layout::AnonymousBoxType::Text(_))
            })
            .ok_or("Click target has no layout box")?
            .dimensions
            .content;
        (
            shell.web_surface.bounds.x + rect.x + rect.width / 2.0,
            shell.web_surface.bounds.y + rect.y + rect.height / 2.0 - page.scroll_y,
        )
    };
    shell
        .handle_input(rix_ipc::protocol::IpcInputEvent::MouseDown { x, y, button: 0 })
        .await?;
    shell
        .handle_input(rix_ipc::protocol::IpcInputEvent::MouseUp { x, y, button: 0 })
        .await?;
    let settle_ms = std::env::var("RIX_PROBE_CLICK_SETTLE_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3000u64);
    let deadline = Instant::now() + Duration::from_millis(settle_ms);
    while Instant::now() < deadline {
        pump(shell).await;
    }
    let still_matches = shell.pages.get(&shell.active_tab_id()).is_some_and(|page| {
        page.document
            .query_selector(page.document.root(), selector)
            .is_some()
    });
    eprintln!(
        "Click step: {selector:?} clicked at ({x:.0},{y:.0}), still_matches={still_matches}"
    );
    Ok(())
}

async fn exercise_input(
    shell: &mut BrowserShell,
    selector: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let page = shell
        .pages
        .get(&shell.active_tab_id())
        .ok_or("Missing page")?;
    let node = page
        .document
        .query_selector(page.document.root(), selector)
        .ok_or("Input selector did not match")?;
    let rect = page
        .layout_tree
        .iter()
        .find(|item| {
            item.dom_node == Some(node)
                && !matches!(item.box_type, rix_layout::AnonymousBoxType::Text(_))
        })
        .ok_or("Input has no layout box")?
        .dimensions
        .content;
    let x = shell.web_surface.bounds.x + rect.x + rect.width / 2.0;
    let y = shell.web_surface.bounds.y + rect.y + rect.height / 2.0 - page.scroll_y;
    shell
        .handle_input(rix_ipc::protocol::IpcInputEvent::MouseDown { x, y, button: 0 })
        .await?;
    if !shell
        .pages
        .get(&shell.active_tab_id())
        .is_some_and(|page| page.focused_input == Some(node))
    {
        let page = shell.pages.get(&shell.active_tab_id()).unwrap();
        eprintln!(
            "Selected field rect {rect:?}, focus {:?}",
            page.focused_input
        );
        let local_x = x - shell.web_surface.bounds.x;
        let local_y = y - shell.web_surface.bounds.y;
        if let Some(hit) = rix_layout::hit_test::hit_test_tree_with_scroll(
            &page.layout_tree,
            Some(&page.document),
            local_x,
            local_y,
            page.scroll_x,
            page.scroll_y,
        ) {
            for node in std::iter::once(hit).chain(page.document.ancestors(hit)) {
                eprintln!("Hit path {:?}", page.document.get(node));
            }
            let boxes = page.layout_tree.boxes();
            let describe = |item: &rix_layout::LayoutBoxData| {
                let name = item
                    .dom_node
                    .and_then(|dom| page.document.get(dom))
                    .and_then(|data| match data {
                        rix_dom::NodeData::Element(element) => Some((
                            element.name.local.to_string(),
                            element.get_attribute("class").unwrap_or_default().to_string(),
                        )),
                        _ => None,
                    });
                format!(
                    "{name:?} {:?} z={} vis={} pe={} ov={}/{}",
                    item.dimensions.content,
                    item.style.z_index,
                    item.style.visibility,
                    item.style.pointer_events,
                    item.style.overflow_x,
                    item.style.overflow_y
                )
            };
            if let Some(index) = boxes.iter().position(|item| item.dom_node == Some(hit)) {
                let item = &boxes[index];
                eprintln!("Hit box children ({}):", item.children.len());
                for child in &item.children {
                    if let Some(child_item) = boxes.get(child.0) {
                        eprintln!("  child {} {}", describe(child_item), {
                            let inside = child_item.dimensions.border_box().contains(local_x, local_y);
                            format!("contains_point={inside}")
                        });
                    }
                }
            }
            if let Some(body_node) = page.document.query_selector(page.document.root(), "body") {
                if let Some(index) = boxes.iter().position(|item| item.dom_node == Some(body_node)) {
                    let item = &boxes[index];
                    eprintln!("Body box children ({}):", item.children.len());
                    for child in &item.children {
                        if let Some(child_item) = boxes.get(child.0) {
                            eprintln!(
                                "  child#{:?} {:?} {} {}",
                                child.0,
                                child_item.box_type,
                                describe(child_item),
                                format!(
                                    "contains_point={}",
                                    child_item.dimensions.border_box().contains(local_x, local_y)
                                )
                            );
                        }
                    }
                }
            }
        }
        for item in page
            .layout_tree
            .iter()
            .filter(|item| item.dimensions.content.contains(local_x, local_y))
        {
            if let Some(rix_dom::NodeData::Element(element)) =
                item.dom_node.and_then(|node| page.document.get(node))
            {
                eprintln!(
                    "Overlapping {} {:?} {:?} {:?}",
                    element.name.local,
                    element.get_attribute("class"),
                    item.dimensions.content,
                    item.style.z_index
                );
            }
        }
        let boxes = page.layout_tree.boxes();
        if let Some(index) = boxes.iter().position(|item| {
            item.dom_node == Some(node)
                && !matches!(item.box_type, rix_layout::AnonymousBoxType::Text(_))
        }) {
            let mut current = Some(rix_layout::tree::LayoutBoxId(index));
            while let Some(id) = current {
                let Some(item) = boxes.get(id.0) else { break };
                let name = item
                    .dom_node
                    .and_then(|dom| page.document.get(dom))
                    .and_then(|data| match data {
                        rix_dom::NodeData::Element(element) => Some((
                            element.name.local.to_string(),
                            element.get_attribute("class").unwrap_or_default().to_string(),
                        )),
                        _ => None,
                    });
                eprintln!(
                    "Box chain {name:?} content={:?} padding={:?} overflow={}/{} visibility={} position={} fragments={} replaced={}",
                    item.dimensions.content,
                    item.dimensions.padding_box(),
                    item.style.overflow_x,
                    item.style.overflow_y,
                    item.style.visibility,
                    item.style.position,
                    item.inline_fragments.len(),
                    item.is_replaced,
                );
                current = item.parent;
            }
        }
        return Err("Click did not focus the selected field".into());
    }
    shell
        .handle_native_key("a", "KeyA", true, false, false, false)
        .await;
    for _ in 0..3 {
        pump(shell).await;
    }
    let sample = std::env::var("RIX_PROBE_INPUT_TEXT")
        .unwrap_or_else(|_| "keyboard probe second".to_string());
    let expected_value: String = {
        let trimmed = sample.trim_end();
        match trimmed.rfind(char::is_whitespace) {
            Some(index) => trimmed[..=index].to_string(),
            None => String::new(),
        }
    };
    let mut expected = String::new();
    let typing_start = Instant::now();
    let mut max_dispatch = Duration::ZERO;
    let mut max_acknowledgement = Duration::ZERO;
    for character in sample.chars() {
        let key_start = Instant::now();
        shell
            .handle_native_key(&character.to_string(), "", false, false, false, false)
            .await;
        max_dispatch = max_dispatch.max(key_start.elapsed());
        expected.push(character);
        wait_for_value(shell, &expected).await?;
        max_acknowledgement = max_acknowledgement.max(key_start.elapsed());
    }
    shell
        .handle_native_key("Backspace", "Backspace", true, false, false, false)
        .await;
    wait_for_value(shell, &expected_value).await?;
    let paint_start = Instant::now();
    wait_for_painted_value(shell, &expected_value).await?;
    eprintln!(
        "Native input timings: {} keys {:.3}s, max dispatch {:.1}ms, max acknowledged value {:.1}ms, final layout wait {:.1}ms",
        sample.chars().count() + 1,
        typing_start.elapsed().as_secs_f64(),
        max_dispatch.as_secs_f64() * 1000.0,
        max_acknowledgement.as_secs_f64() * 1000.0,
        paint_start.elapsed().as_secs_f64() * 1000.0
    );
    let page = shell.pages.get(&shell.active_tab_id()).unwrap();
    let focused = page.focused_input.ok_or("Input lost focus")?;
    let displayed = page.layout_tree.iter().any(|item| item.dom_node == Some(focused) && matches!(&item.box_type, rix_layout::AnonymousBoxType::Text(text) if text == &expected_value));
    if !displayed {
        return Err("Current input value is missing from the rendered layout".into());
    }
    eprintln!("Native input verified: click, typing, Ctrl+A and Ctrl+Backspace; current value is present in layout");
    if std::env::var_os("RIX_PROBE_PRESS_ENTER").is_some() {
        let before = shell
            .pages
            .get(&shell.active_tab_id())
            .map(|page| page.url.clone())
            .unwrap_or_default();
        shell
            .handle_native_key("Enter", "Enter", false, false, false, false)
            .await;
        let quiet = wait_for_quiet(shell, Duration::from_millis(3000), Duration::from_secs(45)).await;
        let page = shell.pages.get(&shell.active_tab_id()).unwrap();
        let count = |selector: &str| {
            page.document
                .query_selector_all(page.document.root(), selector)
                .len()
        };
        eprintln!(
            "Enter step: quiet={quiet} url_before={before} url_after={} nodes={} item_sections={} video_renderers={}",
            page.url,
            page.document.descendants(page.document.root()).count(),
            count("ytd-item-section-renderer"),
            count("ytd-video-renderer")
        );
    }
    Ok(())
}
