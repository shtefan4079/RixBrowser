use super::*;

impl BrowserShell {
    pub fn recompute_layout(&mut self) -> Result<(), ShellError> {
        self.compositor.set_viewport(self.width, self.height);
        let previous_surface_bounds = self.web_surface.bounds;
        let override_sheet = self.customization.load_override_stylesheet();
        let custom_author_index;
        let author_index = if override_sheet.rules.is_empty() {
            &self.chrome_author_index
        } else {
            let mut author_sheet = self.chrome_stylesheet.clone();
            author_sheet.rules.extend(override_sheet.rules);
            custom_author_index = RuleIndex::from_stylesheet(&author_sheet);
            &custom_author_index
        };
        let ctx = LayoutContext {
            viewport_width: self.width,
            viewport_height: self.height,
            root_font_size: 14.0,
        };

        let doc = self.document.borrow();
        let mut styles = HashMap::new();
        compute_tree_styles(
            &doc,
            doc.root(),
            None,
            &self.chrome_ua_index,
            author_index,
            &ctx,
            &mut styles,
        );

        let mut layout_tree = LayoutTree::from_dom(&doc, doc.root(), &styles);
        let fonts = ChromeFontResources::capture(&self.font_database);
        let reused = self.chrome_font_resources == fonts
            && self.chrome_layout_viewport == (self.width, self.height)
            && self
                .layout_tree
                .as_ref()
                .is_some_and(|previous| layout_tree.reuse_geometry_from(previous));
        if !reused {
            self.chrome_layout_engine.compute_layout_with_fonts(
                &mut layout_tree,
                self.width,
                self.height,
                &self.font_database,
            )?;
        }
        let mut web_surface = self.web_surface.clone();
        web_surface
            .update_from_layout(&layout_tree, &doc)
            .map_err(|error| ShellError::Layout(error.to_string()))?;
        if web_surface.bounds.width <= 0.0 || web_surface.bounds.height <= 0.0 {
            return Err(ShellError::Layout(
                "Chrome CSS produced an empty web-surface".into(),
            ));
        }

        self.web_surface = web_surface;

        let (scroll_node, scroll_y) = if self.is_active_internal() {
            let active_id = self.active_tab_id();
            let y = self.pages.get(&active_id).map(|p| p.scroll_y).unwrap_or(0.0);
            let node = doc.query_selector(doc.root(), "#internal-viewport");
            (node, y)
        } else {
            (None, 0.0)
        };
        self.internal_scroll_y = scroll_y;

        let display_list = DisplayList::build_from_layout_with_dom_and_scroll(
            &layout_tree,
            Some(&doc),
            &self.font_database,
            scroll_node,
            0.0,
            scroll_y,
        );
        let mut base_list = DisplayList::new();
        let mut overlay_list = DisplayList::new();
        for item in display_list.items {
            if item.z_index >= 1000 {
                overlay_list.items.push(item);
            } else {
                base_list.items.push(item);
            }
        }
        self.compositor
            .set_layer_display_list(self.chrome_layer_id, base_list);
        self.compositor
            .set_layer_display_list(self.overlay_layer_id, overlay_list);
        self.needs_redraw = true;

        self.layout_tree = Some(layout_tree);
        self.chrome_dom_revision = doc.revision();
        self.chrome_style_revision = doc.style_revision();
        self.chrome_layout_viewport = (self.width, self.height);
        self.chrome_font_resources = fonts;
        drop(doc);
        let bounds = self.web_surface.bounds;
        if previous_surface_bounds.x != bounds.x
            || previous_surface_bounds.y != bounds.y
            || previous_surface_bounds.width != bounds.width
            || previous_surface_bounds.height != bounds.height
        {
            self.reflow_pages_for_surface();
        }

        Ok(())
    }

    pub(super) fn reflow_pages_for_surface(&mut self) {
        let pages: Vec<_> = self
            .pages
            .iter()
            .map(|(tab_id, page)| (*tab_id, page.clone()))
            .collect();

        for (tab_id, mut page) in pages {
            let viewport = (
                self.web_surface.bounds.width.max(1.0),
                self.web_surface.bounds.height.max(1.0),
            );
            if let Some(worker) = &page.page_worker {
                worker
                    .borrow()
                    .layout_change(crate::page_worker::LayoutChange::Viewport {
                        width: viewport.0,
                        height: viewport.1,
                    });
                page.worker_layout_sync.viewport = Some(viewport);
            }
            page.layout_tree = compute_page_layout(
                &page.document,
                &page.url,
                &page.external_stylesheets,
                viewport.0,
                viewport.1,
                &self.font_database,
            );
            page.layout_viewport = viewport;
            page.display_list = DisplayList::build_from_layout_with_dom(
                &page.layout_tree,
                Some(&page.document),
                &self.font_database,
            );
            page.display_revision = next_page_display_revision();
            if !page.images.is_empty() {
                bind_page_image_resources(&mut page);
            }
            self.pages.insert(tab_id, page);
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) -> Result<(), ShellError> {
        self.width = width;
        self.height = height;
        self.recompute_layout()
    }

    pub fn apply_customization_to_chrome(&mut self) -> Result<(), ShellError> {
        let theme_str = self.shell_theme();
        let scheme = match theme_str.as_str() {
            "light" => rix_css::ColorScheme::Light,
            _ => rix_css::ColorScheme::Dark,
        };
        rix_css::set_preferred_color_scheme(scheme);
        rix_css::clear_media_query_cache();
        self.http_client.set_preferred_color_scheme(&theme_str);
        rix_js::web_api::set_shared_network_client(self.http_client.clone());
        if let Some(storage) = self.storage.as_ref().or_else(|| self.http_client.storage()) {
            let pref_val = if theme_str == "dark" { "f6=400" } else { "f6=8" };
            let _ = storage.set_cookie(rix_storage::models::Cookie {
                name: "PREF".to_string(),
                value: pref_val.to_string(),
                domain: ".youtube.com".to_string(),
                path: "/".to_string(),
                expires_at: Some(u64::MAX / 2),
                secure: true,
                http_only: false,
                same_site: rix_storage::models::SameSitePolicy::Lax,
            });
        }
        let theme = match self.customization.theme_mode() {
            ThemeMode::Dark => Some("dark"),
            ThemeMode::Light => Some("light"),
            ThemeMode::Custom => None,
        };
        let vertical = self.customization.tab_orientation() == TabOrientation::Vertical;
        let document = self.document.clone();
        {
            let mut document = document.borrow_mut();
            let root = document.root();
            if let Some(theme_node) = document.query_selector(root, "html") {
                if let Some(NodeData::Element(element)) = document.get_mut(theme_node) {
                    if let Some(theme) = theme {
                        element.set_attribute("data-theme", theme);
                    } else {
                        element.remove_attribute("data-theme");
                    }
                }
            }
            if let Some(shell_node) = document.query_selector(root, "#browser-shell") {
                if let Some(NodeData::Element(element)) = document.get_mut(shell_node) {
                    let mut classes: Vec<String> = element
                        .get_attribute("class")
                        .unwrap_or_default()
                        .split_whitespace()
                        .map(str::to_string)
                        .filter(|class| class != "vertical-tabs")
                        .collect();
                    if vertical {
                        classes.push("vertical-tabs".to_string());
                    }
                    element.set_attribute("class", &classes.join(" "));
                }
            }
        }
        for page in self.pages.values_mut() {
            if let Some(html_id) = page.document.document_element() {
                if let Some(NodeData::Element(elem)) = page.document.get_mut(html_id) {
                    if page.url.contains("youtube.com") || page.url.contains("youtu.be") {
                        if theme_str == "dark" {
                            elem.set_attribute("dark", "");
                        } else {
                            elem.remove_attribute("dark");
                        }
                    }
                }
            }
            page.document.bump_style_revision();
            page.document.bump_revision();
            if let Some(worker) = &page.page_worker {
                worker.borrow().set_theme(&theme_str);
            }
            let viewport = page.layout_viewport;
            page.layout_tree = compute_page_layout(
                &page.document,
                &page.url,
                &page.external_stylesheets,
                viewport.0,
                viewport.1,
                &self.font_database,
            );
            page.display_list = DisplayList::build_from_layout_with_dom(
                &page.layout_tree,
                Some(&page.document),
                &self.font_database,
            );
            page.display_revision = next_page_display_revision();
        }
        let _ = self.runtime.evaluate("if (typeof updateChromeToggleLabels === 'function') updateChromeToggleLabels();");
        self.needs_redraw = true;
        self.recompute_layout()
    }

    pub fn render_frame(&mut self) -> (vello::Scene, FrameStats) {
        let frame_started = std::time::Instant::now();
        let result = self.render_frame_inner();
        if std::env::var_os("RIX_WORKER_TRACE").is_some() {
            let elapsed = frame_started.elapsed();
            if elapsed.as_millis() >= 30 {
                eprintln!(
                    "[rix-shell] render {}ms pid={}",
                    elapsed.as_millis(),
                    std::process::id()
                );
            }
        }
        result
    }

    fn render_frame_inner(&mut self) -> (vello::Scene, FrameStats) {
        self.poll_page_layout_completions();
        self.poll_image_completions();
        self.poll_page_workers();
        let keyboard_focus = {
            let doc = self.document.borrow();
            doc.focused_node()
                .is_some_and(|node| doc.is_focus_visible(node))
        };
        self.synchronize_chrome_input_state(keyboard_focus);
        let (chrome_revision, style_revision) = {
            let doc = self.document.borrow();
            (doc.revision(), doc.style_revision())
        };
        let internal_scroll = if self.is_active_internal() {
            let active_id = self.active_tab_id();
            self.pages.get(&active_id).map(|p| p.scroll_y).unwrap_or(0.0)
        } else {
            0.0
        };
        if style_revision != self.chrome_style_revision
            || self.chrome_font_resources != ChromeFontResources::capture(&self.font_database)
            || chrome_revision != self.chrome_dom_revision
            || (internal_scroll - self.internal_scroll_y).abs() > 0.1
        {
            let _ = self.recompute_layout();
        }

        self.refresh_active_page_render_data();
        self.schedule_page_images();
        self.sync_page_layout_inputs();

        let active_tab_id = self.priv_state.borrow().active_tab_id;
        let surface_bounds = self.web_surface.bounds;
        let is_internal = self.is_active_internal();

        if is_internal {
            let current_scroll = self.pages.get(&active_tab_id).map(|p| p.scroll_y).unwrap_or(0.0);
            if (current_scroll - self.internal_scroll_y).abs() > 0.1 {
                let _ = self.recompute_layout();
            }
        }
        let page_layer_key = self.pages.get(&active_tab_id).map(|page| {
            (
                active_tab_id,
                page.display_revision,
                is_internal,
                surface_bounds.x.to_bits(),
                surface_bounds.y.to_bits(),
                surface_bounds.width.to_bits(),
                surface_bounds.height.to_bits(),
                page.scroll_x.to_bits(),
                page.scroll_y.to_bits(),
            )
        });
        if self.surface_layer_key != page_layer_key {
            let page_layer = if self.is_active_internal() {
                DisplayList::new()
            } else {
                self.pages
                    .get(&active_tab_id)
                    .map(|page| {
                        let mut canvas = DisplayList::new();
                        let background = ["html", "body"]
                            .into_iter()
                            .find_map(|tag| {
                                let node = page.document.query_selector(page.document.root(), tag)?;
                                let color = page
                                    .layout_tree
                                    .iter()
                                    .find(|item| item.dom_node == Some(node))?
                                    .style
                                    .background_color;
                                (color.a > 0.0).then_some(color)
                            })
                            .unwrap_or_else(|| {
                                if rix_css::preferred_color_scheme() == rix_css::ColorScheme::Dark {
                                    rix_css::color::CssColor::from_rgba_u8(15, 15, 15, 255)
                                } else {
                                    rix_css::color::CssColor::WHITE
                                }
                            });
                        canvas.push(
                            rix_render::display_list::DrawCommand::DrawRect {
                                rect: surface_bounds,
                                color: background,
                                border_radius: 0.0,
                                corner_radii: [0.0; 4],
                            },
                            i32::MIN,
                        );
                        let translated = translate_page_display_list(
                            &page.display_list,
                            surface_bounds,
                            page.scroll_x,
                            page.scroll_y,
                        );
                        canvas.items.extend(translated.items);
                        canvas
                    })
                    .unwrap_or_else(|| self.web_surface.build_surface_display_list())
            };
            self.compositor
                .set_layer_display_list(self.surface_layer_id, page_layer);
            self.surface_layer_key = page_layer_key;
        }
        if let Some(page) = self.pages.get_mut(&active_tab_id) {
            page.worker_animation_frame_ready_to_paint = false;
        }
        self.compositor.render_frame_with_fonts(&self.font_database)
    }

    pub(super) fn refresh_active_page_render_data(&mut self) {
        let active_tab_id = self.active_tab_id();
        let page_revision = self.pages.get(&active_tab_id).and_then(|page| {
            page.runtime_context.as_ref().and_then(|context| {
                let context = context.try_borrow().ok()?;
                let dom_state = context.dom_state.try_borrow().ok()?;
                let document = dom_state.document.try_borrow().ok()?;
                Some(document.revision())
            })
        });
        let Some(page_revision) = page_revision else {
            return;
        };
        if self
            .pages
            .get(&active_tab_id)
            .is_some_and(|page| page.rendered_dom_revision == page_revision)
            || self.pending_page_layouts.contains_key(&active_tab_id)
        {
            return;
        }

        let snapshot = self.pages.get(&active_tab_id).and_then(|page| {
            page.runtime_context.as_ref().and_then(|context| {
                let context = context.try_borrow().ok()?;
                let dom_state = context.dom_state.try_borrow().ok()?;
                let document = dom_state.document.try_borrow().ok()?;
                Some(document.clone())
            })
        });
        let Some(snapshot) = snapshot else {
            return;
        };
        let (url, page_title, external_stylesheets) = {
            let Some(page) = self.pages.get(&active_tab_id) else {
                return;
            };
            (
                page.url.clone(),
                page.title.clone(),
                page.external_stylesheets.clone(),
            )
        };

        let new_title = {
            let root = snapshot.root();
            let title = snapshot.document_title(root).trim().to_string();
            if title.is_empty() {
                url.clone()
            } else {
                title
            }
        };
        if new_title != page_title {
            if let Some(page_mut) = self.pages.get_mut(&active_tab_id) {
                page_mut.title = new_title.clone();
            }
            let tab_id = active_tab_id;
            if let Some(tab) = self
                .priv_state
                .borrow_mut()
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
            {
                tab.title = new_title;
            }
            self.notify_chrome_tabs_changed();
        }
        let bounds = self.web_surface.bounds;
        let sender = self.page_layout_tx.clone();
        let fonts = self.font_database.clone();
        let request_id = next_page_display_revision();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                self.pending_page_layouts.insert(active_tab_id, request_id);
                let token = self.layout_tokens.entry(active_tab_id).or_default().clone();
                token.store(request_id, std::sync::atomic::Ordering::Release);
                let token_clone = token.clone();
                handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        if token_clone.load(std::sync::atomic::Ordering::Acquire) != request_id {
                            return PageLayoutCompletion {
                                tab_id: active_tab_id,
                                revision: page_revision,
                                request_id,
                                html: String::new(),
                                document: snapshot,
                                layout: LayoutTree::new(),
                                layout_viewport: (bounds.width, bounds.height),
                                worker_snapshot: None,
                            };
                        }
                        let layout = compute_page_layout(
                            &snapshot,
                            &url,
                            &external_stylesheets,
                            bounds.width,
                            bounds.height,
                            &fonts,
                        );
                        let updated_html = snapshot.to_html_string(snapshot.root());
                        PageLayoutCompletion {
                            tab_id: active_tab_id,
                            revision: page_revision,
                            request_id,
                            html: updated_html,
                            document: snapshot,
                            layout,
                            layout_viewport: (bounds.width, bounds.height),
                            worker_snapshot: None,
                        }
                    })
                    .await;
                    let result = match result {
                        Ok(completion) => PageLayoutResult::Ready(completion),
                        Err(_) => PageLayoutResult::Failed {
                            tab_id: active_tab_id,
                            request_id,
                        },
                    };
                    let _ = sender.send(result);
                });
            }
            Err(_) => {
                let layout = compute_page_layout(
                    &snapshot,
                    &url,
                    &external_stylesheets,
                    bounds.width,
                    bounds.height,
                    &fonts,
                );
                let updated_html = snapshot.to_html_string(snapshot.root());
                self.apply_page_layout(PageLayoutCompletion {
                    tab_id: active_tab_id,
                    revision: page_revision,
                    request_id,
                    html: updated_html,
                    document: snapshot,
                    layout,
                    layout_viewport: (bounds.width, bounds.height),
                    worker_snapshot: None,
                });
            }
        }
    }

    pub(super) fn apply_page_layout(&mut self, completion: PageLayoutCompletion) -> bool {
        let live_revision = self.pages.get(&completion.tab_id).and_then(|page| {
            page.runtime_context.as_ref().and_then(|context| {
                let context = context.try_borrow().ok()?;
                let dom_state = context.dom_state.try_borrow().ok()?;
                let document = dom_state.document.try_borrow().ok()?;
                Some(document.revision())
            })
        });
        let isolated = self
            .pages
            .get(&completion.tab_id)
            .is_some_and(|page| page.page_worker.is_some());
        if !isolated && live_revision != Some(completion.revision) {
            return false;
        }
        {
            let page_url = self
                .pages
                .get(&completion.tab_id)
                .map(|page| page.url.clone())
                .unwrap_or_default();
            let old_title = self
                .pages
                .get(&completion.tab_id)
                .map(|page| page.title.clone())
                .unwrap_or_default();
            let root = completion.document.root();
            let title = completion.document.document_title(root).trim().to_string();
            let new_title = if title.is_empty() {
                page_url
            } else {
                title
            };
            if new_title != old_title {
                if let Some(page) = self.pages.get_mut(&completion.tab_id) {
                    page.title = new_title.clone();
                }
                if let Some(tab) = self
                    .priv_state
                    .borrow_mut()
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == completion.tab_id)
                {
                    tab.title = new_title;
                }
                self.notify_chrome_tabs_changed();
            }
        }
        let font_database = &self.font_database;
        let Some(page) = self.pages.get_mut(&completion.tab_id) else {
            return false;
        };
        if completion.worker_snapshot.as_ref().is_some_and(|snapshot| {
            page.page_worker.as_ref().is_some_and(|worker| {
                snapshot.state.focus.revision < worker.borrow().latest_revision()
            })
        }) {
            return false;
        }
        let previous_token = page.page_worker.as_ref().and_then(|worker| {
            page.focused_input
                .and_then(|node| worker.borrow().node_token(&page.document, node))
        });
        let mut snapshot_focus = None;
        let snapshot_node_count = completion.worker_snapshot.as_ref().map(|s| s.state.nodes.len());
        if let Some(snapshot) = completion.worker_snapshot {
            if let Some(worker) = &page.page_worker {
                let mut worker = worker.borrow_mut();
                worker.install_snapshot(&snapshot.state, snapshot.nodes);
                let focus = snapshot.state.focus;
                if focus.epoch == worker.focus_epoch() {
                    page.focused_input = focus
                        .target
                        .and_then(|token| worker.resolve_token(&completion.document, token));
                    snapshot_focus = Some(focus);
                } else {
                    page.focused_input = previous_token
                        .and_then(|token| worker.resolve_token(&completion.document, token));
                }
            }
        } else if isolated {
            page.focused_input = None;
        }
        page.in_page_focused = page.focused_input.is_some();
        if !completion.html.is_empty() {
            page.html = completion.html;
        }
        page.node_count = snapshot_node_count.unwrap_or_else(|| {
            completion.document.descendants(completion.document.root()).count()
        });
        page.document = completion.document;
        if let Some(node) = page.focused_input {
            if let Some(value) = control_value(&page.document, node) {
                page.in_page_input = value;
            }
            if let Some(focus) = snapshot_focus {
                page.input_selection = focus.selection.unwrap_or_else(|| {
                    TextSelection::collapsed(page.in_page_input.chars().count())
                });
            }
            page.input_selection
                .clamp(page.in_page_input.chars().count());
            page.document.set_focused(Some(node), true);
            page.document.set_text_selection(Some((
                node,
                page.input_selection.anchor,
                page.input_selection.cursor,
            )));
        }
        page.rendered_dom_revision = completion.revision;
        let display_started = std::time::Instant::now();
        page.display_list = DisplayList::build_from_layout_with_dom(
            &completion.layout,
            Some(&page.document),
            font_database,
        );
        if std::env::var_os("RIX_WORKER_TRACE").is_some() {
            eprintln!(
                "[rix-shell] display-list {}ms boxes={} pid={}",
                display_started.elapsed().as_millis(),
                completion.layout.len(),
                std::process::id()
            );
        }
        page.layout_tree = completion.layout;
        page.layout_viewport = completion.layout_viewport;
        if !page.images.is_empty() {
            bind_page_image_resources(page);
        }
        page.display_revision = next_page_display_revision();
        self.needs_redraw = true;
        true
    }

    pub(super) fn poll_page_layout_completions(&mut self) -> usize {
        let mut completed = 0;
        while let Ok(result) = self.page_layout_rx.try_recv() {
            let (tab_id, request_id) = match &result {
                PageLayoutResult::Ready(result) => (result.tab_id, result.request_id),
                PageLayoutResult::Failed { tab_id, request_id } => (*tab_id, *request_id),
            };
            if self.pending_page_layouts.get(&tab_id) != Some(&request_id) {
                continue;
            }
            self.pending_page_layouts.remove(&tab_id);
            if let PageLayoutResult::Ready(completion) = result {
                if let Some(viewport) = self
                    .pages
                    .get(&tab_id)
                    .map(|page| page.layout_viewport)
                    .filter(|viewport| *viewport != completion.layout_viewport)
                {
                    self.reschedule_page_layout(completion, viewport);
                } else {
                    self.apply_page_layout(completion);
                    completed += 1;
                }
            } else if let PageLayoutResult::Failed { .. } = result {
                if let Some(page) = self.pages.get_mut(&tab_id) {
                    page.status_message =
                        format!("{} · layout update failed", page.status_message);
                }
                completed += 1;
            }
            if let Some(deferred) = self.deferred_worker_snapshots.remove(&tab_id) {
                if self
                    .pages
                    .get(&tab_id)
                    .is_some_and(|page| page.image_generation == deferred.generation)
                {
                    self.queue_worker_snapshot_layout(tab_id, deferred.html, deferred.state);
                }
            }
        }
        completed
    }

    pub(super) fn reschedule_page_layout(&self, mut completion: PageLayoutCompletion, viewport: (f32, f32)) {
        let Some(page) = self.pages.get(&completion.tab_id) else {
            return;
        };
        let url = page.url.clone();
        let sheets = page.external_stylesheets.clone();
        let fonts = self.font_database.clone();
        let sender = self.page_layout_tx.clone();
        let tab_id = completion.tab_id;
        let request_id = completion.request_id;
        let compute = move || {
            completion.layout = compute_page_layout(
                &completion.document,
                &url,
                &sheets,
                viewport.0,
                viewport.1,
                &fonts,
            );
            completion.layout_viewport = viewport;
            completion
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(compute).await;
                let _ = sender.send(match result {
                    Ok(completion) => PageLayoutResult::Ready(completion),
                    Err(_) => PageLayoutResult::Failed { tab_id, request_id },
                });
            });
        } else {
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(compute));
                let _ = sender.send(match result {
                    Ok(completion) => PageLayoutResult::Ready(completion),
                    Err(_) => PageLayoutResult::Failed { tab_id, request_id },
                });
            });
        }
    }

    pub fn open_media_engine(
        &mut self,
        tab_id: u32,
        session_id: rix_ipc::media::MediaSessionId,
        generation_id: u64,
    ) -> &mut rix_media::PlaybackEngine {
        self.media_engines
            .entry((tab_id, session_id))
            .or_insert_with(|| rix_media::PlaybackEngine::new(session_id, generation_id))
    }

    pub fn close_media_engine(&mut self, tab_id: u32, session_id: rix_ipc::media::MediaSessionId) {
        if let Some(mut engine) = self.media_engines.remove(&(tab_id, session_id)) {
            engine.shutdown();
        }
    }

    pub fn tick_media_engines(&mut self, now: std::time::Instant) -> usize {
        let active_tab_id = self.priv_state.borrow().active_tab_id;
        let mut repainted = 0;

        for ((tab_id, _session_id), engine) in self.media_engines.iter_mut() {
            if *tab_id != active_tab_id {
                continue;
            }
            let output = engine.tick(now);
            if let Some(video_bitmap) = output.active_video_frame {
                tracing::debug!(
                    session = _session_id.0,
                    timestamp = video_bitmap.timestamp_seconds,
                    width = video_bitmap.width,
                    height = video_bitmap.height,
                    "media engine produced a video frame"
                );
                if let Some(page) = self.pages.get_mut(tab_id) {
                    if let Some(vid_node) = page.document.query_selector(page.document.root(), "video") {
                        if page.document.set_video_frame(vid_node, Some(video_bitmap)) {
                            page.display_list = DisplayList::build_from_layout_with_dom(
                                &page.layout_tree,
                                Some(&page.document),
                                &self.font_database,
                            );
                            page.display_revision = next_page_display_revision();
                            repainted += 1;
                        }
                    }
                }
            }
        }
        repainted
    }
}
