use super::*;

impl BrowserShell {
    pub fn animation_frame_opportunity(&mut self, at: std::time::Instant) -> bool {
        if self.event_loop.has_requested_animation_frame() {
            self.event_loop
                .run_animation_frame_at_now(self.runtime.context_mut());
            self.event_loop.step(self.runtime.context_mut());
            for message in self.event_loop.take_task_errors() {
                tracing::warn!(%message, "browser chrome animation frame failed");
            }
        }

        let active_id = self.priv_state.borrow().active_tab_id;
        let Some(page) = self.pages.get_mut(&active_id) else {
            return true;
        };

        if let Some(page_context) = page.runtime_context.as_ref() {
            if let Ok(mut context) = page_context.try_borrow_mut() {
                if context.event_loop.has_requested_animation_frame() {
                    let PageRuntimeContext {
                        ref mut event_loop,
                        ref mut runtime,
                        ..
                    } = *context;
                    event_loop.run_animation_frame_at_now(runtime.context_mut());
                    event_loop.step(runtime.context_mut());
                    for message in event_loop.take_task_errors() {
                        tracing::warn!(%message, "page animation frame failed");
                    }
                }
            }
        }

        let Some(worker) = page.page_worker.as_ref() else {
            return true;
        };
        if page.worker_animation_frame_ready_to_paint {
            if self.pending_page_layouts.contains_key(&active_id)
                || self.deferred_worker_snapshots.contains_key(&active_id)
            {
                return false;
            }
            page.worker_animation_frame_ready_to_paint = false;
            return true;
        }
        if page.worker_animation_frame_in_flight {
            return false;
        }
        if page.worker_animation_frame_requested {
            let sent = worker.borrow().animation_frame_opportunity(at);
            if sent {
                page.worker_animation_frame_requested = false;
                page.worker_animation_frame_in_flight = true;
            }
            return false;
        }
        true
    }

    pub fn has_pending_animation_frame_opportunity(&self) -> bool {
        if self.event_loop.has_requested_animation_frame() {
            return true;
        }
        let active_id = self.priv_state.borrow().active_tab_id;
        let Some(page) = self.pages.get(&active_id) else {
            return false;
        };
        if page.worker_animation_frame_ready_to_paint {
            return !self.pending_page_layouts.contains_key(&active_id)
                && !self.deferred_worker_snapshots.contains_key(&active_id);
        }
        if page.worker_animation_frame_requested && !page.worker_animation_frame_in_flight {
            return true;
        }
        page.runtime_context.as_ref().is_some_and(|context| {
            context
                .try_borrow()
                .is_ok_and(|context| context.event_loop.has_requested_animation_frame())
        })
    }

    pub fn deliver_pending_animation_frames(&mut self, at: std::time::Instant) -> usize {
        let mut count = 0;
        if self.event_loop.has_requested_animation_frame() {
            count += self
                .event_loop
                .run_animation_frame_at_now(self.runtime.context_mut());
        }
        for page in self.pages.values_mut() {
            if let Some(page_context) = page.runtime_context.as_ref() {
                if let Ok(mut context) = page_context.try_borrow_mut() {
                    if context.event_loop.has_requested_animation_frame() {
                        let PageRuntimeContext {
                            ref mut event_loop,
                            ref mut runtime,
                            ..
                        } = *context;
                        count += event_loop.run_animation_frame_at_now(runtime.context_mut());
                        event_loop.step(runtime.context_mut());
                        for message in event_loop.take_task_errors() {
                            tracing::warn!(%message, "page animation frame failed");
                        }
                    }
                }
            }
            if page.worker_animation_frame_requested && !page.worker_animation_frame_in_flight {
                if let Some(ref worker) = page.page_worker {
                    if worker.borrow().animation_frame_opportunity(at) {
                        page.worker_animation_frame_requested = false;
                        page.worker_animation_frame_in_flight = true;
                        count += 1;
                    }
                }
            }
        }
        count
    }

    pub fn step_event_loop(&mut self) -> usize {
        let mut steps = self.poll_page_workers();
        steps += self.poll_page_layout_completions();
        steps += self.poll_image_completions();
        steps += self.tick_media_engines(std::time::Instant::now());
        self.schedule_page_images();
        self.sync_page_layout_inputs();
        steps += self.event_loop.step(self.runtime.context_mut());
        for message in self.event_loop.take_task_errors() {
            tracing::warn!(%message, "browser chrome task failed");
        }
        steps += self.deliver_pending_animation_frames(std::time::Instant::now());
        let active_id = self.priv_state.borrow().active_tab_id;
        if let Some(page) = self.pages.get_mut(&active_id) {
            let page_ctx = page.runtime_context.clone();
            if let Some(page_ctx) = page_ctx {
                if let Ok(mut ctx) = page_ctx.try_borrow_mut() {
                    let PageRuntimeContext {
                        ref mut event_loop,
                        ref mut runtime,
                        ..
                    } = *ctx;
                    steps += event_loop.step(runtime.context_mut());
                    for record in event_loop.take_resource_loads() {
                        retain_resource_record(page, record);
                    }
                    if event_loop.take_resource_loads_truncated() {
                        page.resource_trace_truncated = true;
                    }
                    for message in event_loop.take_task_errors() {
                        let base = page.status_message.split(" · JS: ").next().unwrap_or("");
                        let brief: String = message.chars().take(240).collect();
                        page.status_message = format!("{base} · JS: {brief}");
                        steps += 1;
                    }
                }
            }
        }
        steps
    }

    pub(super) fn shell_theme(&self) -> String {
        match self.customization.theme_mode() {
            ThemeMode::Dark => "dark".to_string(),
            ThemeMode::Light => "light".to_string(),
            ThemeMode::Custom => "dark".to_string(),
        }
    }

    pub fn build_page_from_html(&self, url: &str, html: &str) -> PageContent {
        self.build_page_from_html_with_stylesheets(url, html, HashMap::new())
    }

    pub(super) fn build_page_from_html_with_stylesheets(
        &self,
        url: &str,
        html: &str,
        external_stylesheets: HashMap<String, String>,
    ) -> PageContent {
        self.build_page_from_document(url, html, external_stylesheets, parse_html(html), None)
    }

    pub(super) fn build_page_from_document(
        &self,
        url: &str,
        html: &str,
        external_stylesheets: HashMap<String, String>,
        mut doc: Document,
        prepared_layout: Option<LayoutTree>,
    ) -> PageContent {
        let mut title = "New Tab".to_string();

        let theme_str = self.shell_theme();
        if let Some(html_id) = doc.document_element() {
            if let Some(NodeData::Element(elem)) = doc.get_mut(html_id) {
                if url.contains("youtube.com") || url.contains("youtu.be") {
                    if theme_str == "dark" {
                        elem.set_attribute("dark", "");
                    } else {
                        elem.remove_attribute("dark");
                    }
                }
            }
        }

        for node_id in doc.descendants(doc.root()) {
            if let Some(NodeData::Element(elem)) = doc.get(node_id) {
                if &*elem.name.local == "title" {
                    let mut title_str = String::new();
                    for child_id in doc.children(node_id) {
                        if let Some(NodeData::Text(t)) = doc.get(child_id) {
                            title_str.push_str(t);
                        }
                    }
                    if !title_str.trim().is_empty() {
                        title = title_str.trim().to_string();
                    }
                    break;
                }
            }
        }

        let surf_w = self.web_surface.bounds.width.max(1.0);
        let surf_h = self.web_surface.bounds.height.max(1.0);
        let layout_tree = prepared_layout.unwrap_or_else(|| {
            compute_page_layout(
                &doc,
                url,
                &external_stylesheets,
                surf_w,
                surf_h,
                &self.font_database,
            )
        });

        let display_list =
            DisplayList::build_from_layout_with_dom(&layout_tree, Some(&doc), &self.font_database);

        let final_title = if !title.is_empty() && title != "New Tab" {
            title
        } else {
            url.to_string()
        };
        let rendered_dom_revision = doc.revision();
        let node_count = doc.descendants(doc.root()).count();

        PageContent {
            url: url.to_string(),
            title: final_title,
            html: html.to_string(),
            external_stylesheets,
            document: doc,
            layout_tree,
            layout_viewport: (surf_w, surf_h),
            display_list,
            images: HashMap::new(),
            image_attempts: std::collections::HashSet::new(),
            dynamic_script_attempts: std::collections::HashSet::new(),
            module_source_cache: HashMap::new(),
            dynamic_script_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(
                self.navigation_resource_policy.concurrent_resources.max(1),
            )),
            loaded_source_bytes: 0,
            image_generation: next_page_display_revision(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            is_loading: false,
            status_message: "Internal page".to_string(),
            in_page_input: String::new(),
            input_selection: TextSelection::default(),
            in_page_focused: false,
            focused_input: None,
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            runtime_context: None,
            page_worker: None,
            resource_trace: Vec::new(),
            resource_trace_truncated: false,
            rendered_dom_revision,
            display_revision: next_page_display_revision(),
            node_count,
            scheduled_images_revision: 0,
            scheduled_images_scroll_y: 0.0,
            hovered_node: None,
            hovered_chain: Vec::new(),
            worker_layout_sync: WorkerLayoutSync::default(),
            worker_animation_frame_requested: false,
            worker_animation_frame_in_flight: false,
            worker_animation_frame_ready_to_paint: false,
            pending_history_traversal: false,
        }
    }

    pub(super) fn normalize_navigation_target(&self, input: &str) -> String {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return "about:blank".to_string();
        }
        if trimmed.starts_with("rix://") || trimmed.starts_with("about:") {
            return trimmed.to_string();
        }
        if trimmed.starts_with("rix:") {
            let fixed = format!("rix://{}", trimmed.trim_start_matches("rix:").trim_start_matches('/'));
            return fixed;
        }
        if trimmed.contains("://") {
            return RixUrl::parse(trimmed)
                .map(|url| url.to_string())
                .unwrap_or_else(|_| trimmed.to_string());
        }
        if trimmed.starts_with('/')
            || trimmed.starts_with("./")
            || trimmed.starts_with("../")
            || trimmed.starts_with('?')
            || trimmed.starts_with('#')
        {
            let active_id = self.active_tab_id();
            if let Some(base) = self
                .pages
                .get(&active_id)
                .map(|page| document_base_url(&page.document, &page.url))
                .and_then(|base| RixUrl::parse(&base).ok())
            {
                if let Ok(url) = base.join(trimmed) {
                    return url.to_string();
                }
            }
        }

        let looks_like_host = !trimmed.chars().any(char::is_whitespace)
            && (trimmed.contains('.')
                || trimmed.starts_with("localhost")
                || trimmed.starts_with("127.")
                || trimmed.starts_with('['));
        if looks_like_host {
            return RixUrl::from_user_input(trimmed)
                .map(|url| url.to_string())
                .unwrap_or_else(|_| trimmed.to_string());
        }

        let query = url::form_urlencoded::byte_serialize(trimmed.as_bytes()).collect::<String>();
        self.search_engine_template.replace("{query}", &query)
    }

    pub async fn navigate_active_tab(&mut self, target_url: &str) {
        let active_id = self.priv_state.borrow().active_tab_id;
        self.navigate_tab(active_id, target_url).await;
    }

    pub async fn navigate_tab_without_history(&mut self, tab_id: u32, target_url: &str) {
        let (back, forward) = self
            .pages
            .get(&tab_id)
            .map(|p| (p.back_stack.clone(), p.forward_stack.clone()))
            .unwrap_or_default();
        self.navigate_tab(tab_id, target_url).await;
        if let Some(page) = self.pages.get_mut(&tab_id) {
            page.back_stack = back;
            page.forward_stack = forward;
        }
    }

    pub fn navigate_tab<'a>(
        &'a mut self,
        tab_id: u32,
        target_url: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>> {
        Box::pin(async move {
            let request = self.prepare_navigation(tab_id, target_url, None);
            let bounds = self.web_surface.bounds;
            let theme = self.shell_theme();
            let scheme = if theme == "dark" {
                rix_css::ColorScheme::Dark
            } else {
                rix_css::ColorScheme::Light
            };
            rix_css::set_preferred_color_scheme(scheme);
            self.http_client.set_preferred_color_scheme(&theme);
            if let Some(storage) = self.storage.as_ref().or_else(|| self.http_client.storage()) {
                if target_url.contains("youtube.com") {
                    let pref_val = if theme == "dark" { "f6=400" } else { "f6=8" };
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
            }
            let content = Box::pin(load_navigation_content(
                self.http_client.clone(),
                &request.url,
                bounds.width,
                bounds.height,
                self.font_database.clone(),
                self.navigation_resource_policy,
                &theme,
            ))
            .await;
            self.finish_navigation(request, content);
        })
    }

    pub(super) fn prepare_navigation(
        &mut self,
        tab_id: u32,
        target_url: &str,
        history_override: Option<(Vec<String>, Vec<String>)>,
    ) -> NavigationRequest {
        if let Some(tasks) = self.image_tasks.remove(&tab_id) {
            for task in tasks {
                task.abort();
            }
        }
        self.pending_images.retain(|(tab, _, _)| *tab != tab_id);
        if let Some(tasks) = self.dynamic_script_tasks.remove(&tab_id) {
            for task in tasks {
                task.abort();
            }
        }
        self.pending_dynamic_scripts
            .retain(|(tab, _, _)| *tab != tab_id);
        self.worker_defaults.retain(|(tab, _), _| *tab != tab_id);
        self.worker_keys.retain(|(tab, _), _| *tab != tab_id);
        self.media_engines.retain(|(tab, _), engine| {
            if *tab == tab_id {
                engine.shutdown();
                false
            } else {
                true
            }
        });
        self.pending_page_layouts.remove(&tab_id);
        self.deferred_worker_snapshots.remove(&tab_id);
        if let Some(task) = self.navigation_tasks.remove(&tab_id) {
            task.abort();
        }
        let url = self.normalize_navigation_target(target_url);
        let (previous_url, previous_back, previous_forward) = self
            .pages
            .get(&tab_id)
            .map(|page| {
                (
                    page.url.clone(),
                    page.back_stack.clone(),
                    page.forward_stack.clone(),
                )
            })
            .unwrap_or_default();
        if self.priv_state.borrow().active_tab_id == tab_id {
            self.omnibox_text = url.clone();
            self.omnibox_selection = TextSelection::collapsed(self.omnibox_text.chars().count());
            self.set_chrome_omnibox_value(&url);
        }
        let is_internal = url.starts_with("rix://") || url == "about:blank";
        let default_title = if is_internal {
            if url.contains("bookmarks") {
                "Bookmarks".to_string()
            } else if url.contains("history") {
                "History".to_string()
            } else if url.contains("downloads") {
                "Downloads".to_string()
            } else if url.contains("shortcuts") {
                "Shortcuts".to_string()
            } else if url.contains("settings") {
                "Settings".to_string()
            } else {
                "New Tab".to_string()
            }
        } else {
            "Loading…".to_string()
        };
        let loading_html = if is_internal {
            format!("<!doctype html><html><head><title>{default_title}</title></head><body></body></html>")
        } else {
            loading_page_html(&url, &self.shell_theme())
        };
        let mut loading_page = self.build_page_from_html(&url, &loading_html);
        loading_page.title.clone_from(&default_title);
        loading_page.is_loading = !is_internal;
        if !is_internal {
            loading_page.status_message = "Requesting page…".to_string();
        }
        if let Some(tab) = self
            .priv_state
            .borrow_mut()
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        {
            tab.url = url.clone();
            tab.title.clone_from(&loading_page.title);
        }
        self.pages.insert(tab_id, loading_page);
        let id = NEXT_NAVIGATION_ID.fetch_add(1, Ordering::Relaxed);
        self.pending_navigations.insert(tab_id, id);
        self.notify_chrome_tabs_changed();
        NavigationRequest {
            id,
            tab_id,
            url,
            previous_url,
            previous_back,
            previous_forward,
            history_override,
        }
    }

    pub(super) fn commit_navigation_document(
        &mut self,
        request: &NavigationRequest,
        content: NavigationContent,
    ) {
        if self.pending_navigations.get(&request.tab_id) != Some(&request.id) {
            return;
        }
        let bounds = self.web_surface.bounds;
        let layout =
            if content.layout_viewport == Some((bounds.width.to_bits(), bounds.height.to_bits())) {
                content.prepared_layout
            } else {
                None
            };
        let mut page = self.build_page_from_document(
            &content.final_url,
            &content.html,
            content.stylesheets,
            content.document,
            layout,
        );
        page.is_loading = true;
        page.status_message = format!("{} · loading resources", content.status_message);
        page.resource_trace = content.resource_trace;
        page.back_stack.clone_from(&request.previous_back);
        page.forward_stack.clone_from(&request.previous_forward);
        if let Some(tab) = self
            .priv_state
            .borrow_mut()
            .tabs
            .iter_mut()
            .find(|tab| tab.id == request.tab_id)
        {
            tab.title.clone_from(&page.title);
        }
        self.pages.insert(request.tab_id, page);
        self.notify_chrome_tabs_changed();
    }

    pub(super) fn finish_navigation(
        &mut self,
        mut request: NavigationRequest,
        content: NavigationContent,
    ) {
        if self.pending_navigations.get(&request.tab_id) != Some(&request.id) {
            return;
        }
        self.pending_navigations.remove(&request.tab_id);
        self.navigation_tasks.remove(&request.tab_id);
        let NavigationContent {
            html,
            document,
            prepared_layout,
            layout_viewport,
            final_url,
            status_message,
            stylesheets,
            scripts,
            failed_resources,
            navigation_clock,
            resource_trace,
        } = content;
        request.url = final_url;
        let current_bounds = self.web_surface.bounds;
        let prepared_layout = match layout_viewport {
            Some((width, height))
                if width == current_bounds.width.to_bits()
                    && height == current_bounds.height.to_bits() =>
            {
                prepared_layout
            }
            _ => None,
        };
        let mut final_page = self.build_page_from_document(
            &request.url,
            &html,
            stylesheets,
            document,
            prepared_layout,
        );
        final_page.resource_trace_truncated = resource_trace.len() > MAX_PAGE_RESOURCE_TRACE;
        final_page.resource_trace = resource_trace
            .into_iter()
            .take(MAX_PAGE_RESOURCE_TRACE)
            .collect();
        let mut retained_external_scripts = std::collections::HashSet::new();
        let parser_script_bytes = scripts.iter().fold(0usize, |total, script| {
            let count = match script.kind {
                crate::page_worker::PageScriptKind::InlineClassic
                | crate::page_worker::PageScriptKind::InlineModule
                | crate::page_worker::PageScriptKind::DynamicClassic
                | crate::page_worker::PageScriptKind::DiagnosticProbe => script.source.len(),
                crate::page_worker::PageScriptKind::ExternalClassic
                    if retained_external_scripts.insert((false, script.source_url.as_str())) =>
                {
                    script.source.len()
                }
                crate::page_worker::PageScriptKind::ExternalClassic => 0,
                crate::page_worker::PageScriptKind::ExternalModule
                    if retained_external_scripts.insert((true, script.source_url.as_str())) =>
                {
                    script.source.len()
                }
                crate::page_worker::PageScriptKind::ExternalModule => 0,
            };
            total.saturating_add(count)
        });
        final_page.loaded_source_bytes = final_page
            .external_stylesheets
            .values()
            .map(String::len)
            .fold(parser_script_bytes, usize::saturating_add);
        final_page.is_loading = false;
        final_page.status_message = if failed_resources == 0 {
            status_message
        } else {
            format!(
                "{} · {} resource(s) failed",
                status_message, failed_resources
            )
        };

        let page_title = final_page.title.clone();
        if let Some(tab) = self
            .priv_state
            .borrow_mut()
            .tabs
            .iter_mut()
            .find(|tab| tab.id == request.tab_id)
        {
            tab.url = request.url.clone();
            tab.title = page_title;
        }
        if self.priv_state.borrow().active_tab_id == request.tab_id {
            self.omnibox_text = request.url.clone();
            self.omnibox_selection = TextSelection::collapsed(self.omnibox_text.chars().count());
            self.set_chrome_omnibox_value(&self.omnibox_text.clone());
        }
        self.notify_chrome_tabs_changed();

        if let Some(command) = &self.page_worker_command {
            let parser_source_resource_count = final_page
                .resource_trace
                .iter()
                .filter(|record| {
                    matches!(
                        record.kind,
                        ResourceLoadKind::Script | ResourceLoadKind::Stylesheet
                    )
                })
                .count();
            let dynamic_script_request_limit = self
                .navigation_resource_policy
                .max_resources
                .saturating_sub(parser_source_resource_count);
            let navigation = navigation_clock.map(|(timing, started)| {
                crate::page_worker::NavigationPerformance {
                    timing,
                    elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                }
            });
            let page_visible = self.active_tab_id() == request.tab_id;
            let cookies = self
                .storage
                .as_ref()
                .or_else(|| self.http_client.storage())
                .and_then(|storage| storage.get_all_cookies().ok())
                .unwrap_or_default();
            let origin = RixUrl::parse(&request.url).map(|u| u.origin()).unwrap_or_default();
            let local_storage = self
                .storage
                .as_ref()
                .or_else(|| self.http_client.storage())
                .and_then(|storage| storage.get_all_local_storage(&origin).ok())
                .unwrap_or_default();
            match crate::page_worker::PageWorker::spawn_with_storage(
                command,
                &request.url,
                &html,
                Some(&final_page.document),
                scripts,
                navigation,
                crate::page_layout_host::LayoutInputs {
                    stylesheets: final_page.external_stylesheets.clone(),
                    viewport: final_page.layout_viewport,
                    scroll: (final_page.scroll_x, final_page.scroll_y),
                    image_sizes: HashMap::new(),
                },
                dynamic_script_request_limit,
                page_visible,
                cookies,
                local_storage,
                Some(self.shell_theme()),
            ) {
                Ok(worker) => {
                    final_page.worker_layout_sync = WorkerLayoutSync {
                        viewport: Some(final_page.layout_viewport),
                        scroll_x: Some(final_page.scroll_x),
                        scroll_y: Some(final_page.scroll_y),
                        images: Default::default(),
                    };
                    final_page.page_worker = Some(Rc::new(RefCell::new(worker)));
                }
                Err(error) => {
                    final_page.status_message = format!(
                        "{} · page runtime failed: {error}",
                        final_page.status_message
                    )
                }
            }
        } else {
            let parsed_origin = RixUrl::parse(&request.url).ok();
            let mut page_config = JsRuntimeConfig::default();
            page_config.origin = parsed_origin.as_ref().map(|url| url.as_str().to_string());
            let mut page_runtime = JsRuntime::new(page_config);
            let page_web_state = Rc::new(RefCell::new(WebApiState::new(parsed_origin)));
            let _ = register_web_apis(page_runtime.context_mut(), page_web_state.clone());
            if let Some((timing, started)) = navigation_clock {
                let _ = rix_js::web_api::set_navigation_performance(
                    page_runtime.context_mut(),
                    timing,
                    started.elapsed().as_secs_f64() * 1000.0,
                );
            }
            let page_dom_state = Rc::new(RefCell::new(DomBridgeState::new(
                final_page.document.clone(),
            )));
            let script_elements = {
                let document = page_dom_state.borrow().document.clone();
                let document = document.borrow();
                document
                    .descendants(document.root())
                    .filter(|node| {
                        matches!(document.get(*node), Some(rix_dom::NodeData::Element(element))
                            if &*element.name.local == "script")
                    })
                    .collect::<Vec<_>>()
            };
            let _ = register_dom_bridge(page_runtime.context_mut(), page_dom_state.clone());
            let root = page_dom_state.borrow().document.borrow().root();
            let _ = rix_js::dom_bridge::set_document_ready_state(
                page_runtime.context_mut(),
                root,
                "loading",
            );
            rix_js::web_api::mark_navigation_milestone(
                page_runtime.context_mut(),
                rix_js::web_api::NavigationMilestone::DomLoading,
            );
            let mut script_failures = 0;
            for script in scripts {
                page_dom_state.borrow_mut().current_script =
                    script_elements.get(script.element_index).copied();
                let result = page_runtime.evaluate_with_timeout(&script.source, 100);
                page_dom_state.borrow_mut().current_script = None;
                if let Err(error) = result {
                    script_failures += 1;
                    tracing::warn!(url = %request.url, error = %error, "Page script did not execute");
                }
            }
            let _ = rix_js::dom_bridge::set_document_ready_state(
                page_runtime.context_mut(),
                root,
                "interactive",
            );
            rix_js::web_api::mark_navigation_milestone(
                page_runtime.context_mut(),
                rix_js::web_api::NavigationMilestone::DomInteractive,
            );
            rix_js::web_api::mark_navigation_milestone(
                page_runtime.context_mut(),
                rix_js::web_api::NavigationMilestone::DomContentLoadedEventStart,
            );
            let dom_content_loaded_props = [
                ("bubbles", boa_engine::JsValue::new(true)),
                ("cancelable", boa_engine::JsValue::new(false)),
            ];
            if let Err(error) = dispatch_dom_event_to_js(
                root,
                "DOMContentLoaded",
                Some(&dom_content_loaded_props),
                page_runtime.context_mut(),
            ) {
                tracing::warn!(%error, "DOMContentLoaded listener failed");
            }
            rix_js::web_api::mark_navigation_milestone(
                page_runtime.context_mut(),
                rix_js::web_api::NavigationMilestone::DomContentLoadedEventEnd,
            );
            let _ = rix_js::dom_bridge::set_document_ready_state(
                page_runtime.context_mut(),
                root,
                "complete",
            );
            rix_js::web_api::mark_navigation_milestone(
                page_runtime.context_mut(),
                rix_js::web_api::NavigationMilestone::DomComplete,
            );
            rix_js::web_api::mark_navigation_milestone(
                page_runtime.context_mut(),
                rix_js::web_api::NavigationMilestone::LoadEventStart,
            );
            let _ = page_runtime.evaluate("window.dispatchEvent(new Event('load'));");
            rix_js::web_api::mark_navigation_milestone(
                page_runtime.context_mut(),
                rix_js::web_api::NavigationMilestone::LoadEventEnd,
            );
            if script_failures > 0 {
                final_page.status_message = format!(
                    "{} · {} script(s) failed",
                    final_page.status_message, script_failures,
                );
            }
            let (resource_records, trace_truncated) = {
                let mut page_web_state = page_web_state.borrow_mut();
                (
                    page_web_state.take_resource_loads(),
                    page_web_state.take_resource_loads_truncated(),
                )
            };
            for record in resource_records {
                retain_resource_record(&mut final_page, record);
            }
            final_page.resource_trace_truncated |= trace_truncated;
            let page_event_loop = EventLoop::new(page_web_state.clone());
            final_page.runtime_context = Some(Rc::new(RefCell::new(PageRuntimeContext {
                runtime: page_runtime,
                web_state: page_web_state,
                dom_state: page_dom_state,
                event_loop: page_event_loop,
            })));
        }
        self.pages.insert(request.tab_id, final_page);

        if let Some(page) = self.pages.get_mut(&request.tab_id) {
            if let Some((back, forward)) = request.history_override {
                page.back_stack = back;
                page.forward_stack = forward;
            } else if !request.previous_url.is_empty() && request.previous_url != request.url {
                let mut back = request.previous_back;
                back.push(request.previous_url);
                if back.len() > 50 {
                    back.remove(0);
                }
                page.back_stack = back;
                page.forward_stack.clear();
            } else {
                page.back_stack = request.previous_back;
                page.forward_stack = request.previous_forward;
            }
        }
        self.notify_chrome_tabs_changed();
        self.schedule_page_images();
    }

    pub fn start_navigation(&mut self, tab_id: u32, target_url: &str) {
        self.start_navigation_with_history(tab_id, target_url, None);
    }

    pub fn start_navigation_active_tab(&mut self, target_url: &str) {
        let tab_id = self.active_tab_id();
        self.start_navigation(tab_id, target_url);
    }

    pub fn start_navigation_without_history(&mut self, tab_id: u32, target_url: &str) {
        let history = self
            .pages
            .get(&tab_id)
            .map(|page| (page.back_stack.clone(), page.forward_stack.clone()))
            .unwrap_or_default();
        self.start_navigation_with_history(tab_id, target_url, Some(history));
    }

    pub(super) fn start_navigation_with_history(
        &mut self,
        tab_id: u32,
        target_url: &str,
        history_override: Option<(Vec<String>, Vec<String>)>,
    ) {
        let request = self.prepare_navigation(tab_id, target_url, history_override);
        if request.url == "rix://welcome" {
            self.start_navigation_with_history(tab_id, "https://www.google.com", request.history_override);
            return;
        }
        if request.url == "about:blank" {
            let html = "<!doctype html><html><head><title></title></head><body></body></html>".to_string();
            let final_url = request.url.clone();
            self.finish_navigation(
                request,
                simple_navigation_content(&final_url, html, "Internal page"),
            );
            return;
        }
        let theme = self.shell_theme();
        let scheme = if theme == "dark" {
            rix_css::ColorScheme::Dark
        } else {
            rix_css::ColorScheme::Light
        };
        rix_css::set_preferred_color_scheme(scheme);
        self.http_client.set_preferred_color_scheme(&theme);
        if let Some(storage) = self.storage.as_ref().or_else(|| self.http_client.storage()) {
            if request.url.contains("youtube.com") {
                let pref_val = if theme == "dark" { "f6=400" } else { "f6=8" };
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
                let _ = storage.set_cookie(rix_storage::models::Cookie {
                    name: "SOCS".to_string(),
                    value: "CAESEwgDEgk2OTU3NDk5MTAaAmVuIAEaBgiA_K-0Bg".to_string(),
                    domain: ".youtube.com".to_string(),
                    path: "/".to_string(),
                    expires_at: Some(u64::MAX / 2),
                    secure: true,
                    http_only: false,
                    same_site: rix_storage::models::SameSitePolicy::Lax,
                });
                let _ = storage.set_cookie(rix_storage::models::Cookie {
                    name: "CONSENT".to_string(),
                    value: "PENDING+999".to_string(),
                    domain: ".youtube.com".to_string(),
                    path: "/".to_string(),
                    expires_at: Some(u64::MAX / 2),
                    secure: true,
                    http_only: false,
                    same_site: rix_storage::models::SameSitePolicy::Lax,
                });
            }
        }
        let Ok(target) = RixUrl::parse(&request.url) else {
            let url = request.url.clone();
            let html = error_page_html(
                &url,
                "This address isn't a valid URL. Check the spelling and try again.",
                &theme,
            );
            let content = simple_navigation_content(&url, html, "Invalid URL");
            self.finish_navigation(request, content);
            return;
        };
        if target.is_internal()
            || request.url.starts_with("rix://")
            || request.url == "about:blank"
        {
            let url = request.url.clone();
            let default_title = if url.contains("bookmarks") {
                "Bookmarks"
            } else if url.contains("history") {
                "History"
            } else if url.contains("downloads") {
                "Downloads"
            } else if url.contains("shortcuts") {
                "Shortcuts"
            } else if url.contains("settings") {
                "Settings"
            } else {
                "New Tab"
            };
            let html = format!(
                "<!doctype html><html><head><title>{default_title}</title></head><body></body></html>"
            );
            let content = simple_navigation_content(&url, html, "Internal page");
            self.finish_navigation(request, content);
            return;
        }
        if !matches!(
            target.scheme(),
            rix_core::url::RixScheme::Http | rix_core::url::RixScheme::Https
        ) {
            let url = request.url.clone();
            let error = format!(
                "RixBrowser can't open {} addresses. Only http and https are supported.",
                target.scheme()
            );
            let html = error_page_html(&url, &error, &theme);
            let content = simple_navigation_content(&url, html, "Unsupported URL");
            self.finish_navigation(request, content);
            return;
        }
        let client = self.http_client.clone();
        let fonts = self.font_database.clone();
        let sender = self.navigation_tx.clone();
        let resource_policy = self.navigation_resource_policy;
        let viewport_width = self.web_surface.bounds.width;
        let viewport_height = self.web_surface.bounds.height;
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let task = handle.spawn(async move {
                    let (document_tx, mut document_rx) = tokio::sync::mpsc::unbounded_channel();
                    let loading = load_navigation_content_progressive(
                        client, &request.url, viewport_width, viewport_height, fonts, Some(document_tx), resource_policy,
                        &theme,
                    );
                    tokio::pin!(loading);
                    loop {
                        tokio::select! {
                            content = &mut loading => {
                                while let Ok(content) = document_rx.try_recv() {
                                    let _ = sender.send(NavigationCompletion { request: request.clone(), content, is_final: false });
                                }
                                let _ = sender.send(NavigationCompletion { request: request.clone(), content, is_final: true });
                                break;
                            }
                            Some(content) = document_rx.recv() => {
                                let _ = sender.send(NavigationCompletion { request: request.clone(), content, is_final: false });
                            }
                        }
                    }
                });
                self.navigation_tasks.insert(tab_id, task);
            }
            Err(error) => {
                let url = request.url.clone();
                let html = error_page_html(
                    &url,
                    &format!("No async runtime available: {error}"),
                    &theme,
                );
                let content = simple_navigation_content(&url, html, "No runtime");
                self.finish_navigation(request, content);
            }
        }
    }

    pub fn poll_navigation_completions(&mut self) -> usize {
        let mut completed = 0;
        while let Ok(completion) = self.navigation_rx.try_recv() {
            if self.pending_navigations.get(&completion.request.tab_id)
                == Some(&completion.request.id)
            {
                if completion.is_final {
                    self.finish_navigation(completion.request, completion.content);
                } else {
                    self.commit_navigation_document(&completion.request, completion.content);
                }
                completed += 1;
            }
        }
        if completed > 0 {
            self.needs_redraw = true;
        }
        completed + self.poll_page_layout_completions() + self.poll_image_completions()
    }

    pub fn has_pending_navigations(&self) -> bool {
        !self.pending_navigations.is_empty()
    }

    fn start_go_back_reload(&mut self, tab_id: u32) -> bool {
        let (target, back, forward) = match self.pages.get(&tab_id) {
            Some(page) => {
                let mut back = page.back_stack.clone();
                let Some(target) = back.pop() else {
                    return false;
                };
                let mut forward = page.forward_stack.clone();
                forward.push(page.url.clone());
                (target, back, forward)
            }
            None => return false,
        };
        self.start_navigation_with_history(tab_id, &target, Some((back, forward)));
        true
    }

    fn start_go_forward_reload(&mut self, tab_id: u32) -> bool {
        let (target, back, forward) = match self.pages.get(&tab_id) {
            Some(page) => {
                let mut forward = page.forward_stack.clone();
                let Some(target) = forward.pop() else {
                    return false;
                };
                let mut back = page.back_stack.clone();
                back.push(page.url.clone());
                (target, back, forward)
            }
            None => return false,
        };
        self.start_navigation_with_history(tab_id, &target, Some((back, forward)));
        true
    }

    pub fn start_go_back(&mut self, tab_id: u32) -> bool {
        let already_pending = self
            .pages
            .get(&tab_id)
            .map(|page| page.pending_history_traversal)
            .unwrap_or(false);
        if already_pending {
            if let Some(page) = self.pages.get_mut(&tab_id) {
                page.pending_history_traversal = false;
            }
            return self.start_go_back_reload(tab_id);
        }
        let has_back = self
            .pages
            .get(&tab_id)
            .map(|page| !page.back_stack.is_empty())
            .unwrap_or(false);
        if !has_back {
            return false;
        }
        let worker = self
            .pages
            .get(&tab_id)
            .and_then(|page| page.page_worker.clone());
        if let Some(worker) = worker {
            if let Some(page) = self.pages.get_mut(&tab_id) {
                page.pending_history_traversal = true;
            }
            worker.borrow().traverse_history(-1);
            return true;
        }
        self.start_go_back_reload(tab_id)
    }

    pub fn start_go_forward(&mut self, tab_id: u32) -> bool {
        let already_pending = self
            .pages
            .get(&tab_id)
            .map(|page| page.pending_history_traversal)
            .unwrap_or(false);
        if already_pending {
            if let Some(page) = self.pages.get_mut(&tab_id) {
                page.pending_history_traversal = false;
            }
            return self.start_go_forward_reload(tab_id);
        }
        let has_forward = self
            .pages
            .get(&tab_id)
            .map(|page| !page.forward_stack.is_empty())
            .unwrap_or(false);
        if !has_forward {
            return false;
        }
        let worker = self
            .pages
            .get(&tab_id)
            .and_then(|page| page.page_worker.clone());
        if let Some(worker) = worker {
            if let Some(page) = self.pages.get_mut(&tab_id) {
                page.pending_history_traversal = true;
            }
            worker.borrow().traverse_history(1);
            return true;
        }
        self.start_go_forward_reload(tab_id)
    }

    pub fn start_reload_active_tab(&mut self) {
        let tab_id = self.active_tab_id();
        self.start_reload(tab_id);
    }

    pub fn start_reload(&mut self, tab_id: u32) {
        let url = self
            .pages
            .get(&tab_id)
            .map(|page| page.url.clone())
            .unwrap_or_else(|| self.omnibox_text.clone());
        self.start_navigation_without_history(tab_id, &url);
    }

    pub fn start_new_tab(&mut self, url: &str) -> u32 {
        let previous_tab_id = self.active_tab_id();
        let tab_id = self.priv_state.borrow_mut().create_tab(url.to_string());
        self.transition_page_visibility(previous_tab_id, tab_id);
        self.start_navigation(tab_id, url);
        self.switch_tab(tab_id);
        tab_id
    }

    pub async fn create_new_tab(&mut self, url: &str) -> u32 {
        let previous_tab_id = self.active_tab_id();
        let new_id = self.priv_state.borrow_mut().create_tab(url.to_string());
        self.transition_page_visibility(previous_tab_id, new_id);
        self.navigate_tab(new_id, url).await;
        self.switch_tab(new_id);
        new_id
    }

    pub fn close_tab(&mut self, tab_id: u32) {
        let previous_tab_id = self.active_tab_id();
        self.priv_state.borrow_mut().close_tab(tab_id);
        let active_tab_id = self.active_tab_id();
        self.transition_page_visibility(previous_tab_id, active_tab_id);
        if let Some(tasks) = self.image_tasks.remove(&tab_id) {
            for task in tasks {
                task.abort();
            }
        }
        self.pending_images.retain(|(tab, _, _)| *tab != tab_id);
        if let Some(tasks) = self.dynamic_script_tasks.remove(&tab_id) {
            for task in tasks {
                task.abort();
            }
        }
        self.pending_dynamic_scripts
            .retain(|(tab, _, _)| *tab != tab_id);
        self.worker_defaults.retain(|(tab, _), _| *tab != tab_id);
        self.worker_keys.retain(|(tab, _), _| *tab != tab_id);
        self.media_engines.retain(|(tab, _), engine| {
            if *tab == tab_id {
                engine.shutdown();
                false
            } else {
                true
            }
        });
        self.pending_navigations.remove(&tab_id);
        self.pending_page_layouts.remove(&tab_id);
        self.deferred_worker_snapshots.remove(&tab_id);
        if let Some(task) = self.navigation_tasks.remove(&tab_id) {
            task.abort();
        }
        self.pages.remove(&tab_id);
        let active_id = self.priv_state.borrow().active_tab_id;
        let active_url = self.pages.get(&active_id).map(|page| page.url.clone());
        if let Some(url) = active_url {
            self.omnibox_text = url.clone();
            self.omnibox_selection = TextSelection::collapsed(self.omnibox_text.chars().count());
            self.set_chrome_omnibox_value(&self.omnibox_text.clone());
        }
        self.notify_chrome_tabs_changed();
    }

    pub fn switch_tab(&mut self, tab_id: u32) {
        let previous_tab_id = self.active_tab_id();
        self.priv_state.borrow_mut().switch_tab(tab_id);
        let active_tab_id = self.active_tab_id();
        self.transition_page_visibility(previous_tab_id, active_tab_id);
        let active_url = self.pages.get(&tab_id).map(|page| page.url.clone());
        if let Some(url) = active_url {
            self.omnibox_text = url.clone();
            self.omnibox_selection = TextSelection::collapsed(self.omnibox_text.chars().count());
            self.set_chrome_omnibox_value(&self.omnibox_text.clone());
        }
        self.notify_chrome_tabs_changed();
    }

    fn transition_page_visibility(&self, previous_tab_id: u32, active_tab_id: u32) {
        if previous_tab_id == active_tab_id {
            return;
        }
        if let Some(worker) = self
            .pages
            .get(&previous_tab_id)
            .and_then(|page| page.page_worker.as_ref())
        {
            worker.borrow().set_page_visibility(false);
        }
        if let Some(worker) = self
            .pages
            .get(&active_tab_id)
            .and_then(|page| page.page_worker.as_ref())
        {
            worker.borrow().set_page_visibility(true);
        }
    }

    pub fn scroll_active_tab(&mut self, delta_y: f32) {
        let active_id = self.priv_state.borrow().active_tab_id;
        if let Some(page) = self.pages.get_mut(&active_id) {
            page.scroll_y = (page.scroll_y + delta_y).max(0.0);
        }
    }

    pub(super) fn apply_page_history_change(
        &mut self,
        tab_id: u32,
        url: &str,
        kind: rix_js::web_api::HistoryChangeKind,
        _index: u32,
    ) -> bool {
        use rix_js::web_api::HistoryChangeKind;
        let normalized = self.normalize_navigation_target(url);
        let previous = match self.pages.get_mut(&tab_id) {
            Some(page) => {
                let previous = page.url.clone();
                page.url = normalized.clone();
                match kind {
                    HistoryChangeKind::Push => {
                        if !previous.is_empty() && previous != normalized {
                            page.back_stack.push(previous.clone());
                            if page.back_stack.len() > 50 {
                                page.back_stack.remove(0);
                            }
                        }
                        page.forward_stack.clear();
                    }
                    HistoryChangeKind::Replace | HistoryChangeKind::Traverse => {}
                }
                previous
            }
            None => return false,
        };
        if previous == normalized {
            return false;
        }
        if self.priv_state.borrow().active_tab_id == tab_id {
            if !self.omnibox_focused {
                self.omnibox_text = normalized.clone();
                self.omnibox_selection =
                    TextSelection::collapsed(self.omnibox_text.chars().count());
                self.set_chrome_omnibox_value(&normalized);
            }
        }
        if let Some(tab) = self
            .priv_state
            .borrow_mut()
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        {
            tab.url = normalized.clone();
        }
        self.notify_chrome_tabs_changed();
        true
    }

    pub fn can_go_back(&self, tab_id: u32) -> bool {
        self.pages
            .get(&tab_id)
            .map(|p| !p.back_stack.is_empty())
            .unwrap_or(false)
    }

    pub fn can_go_forward(&self, tab_id: u32) -> bool {
        self.pages
            .get(&tab_id)
            .map(|p| !p.forward_stack.is_empty())
            .unwrap_or(false)
    }

    pub async fn go_back(&mut self, tab_id: u32) -> bool {
        let (target, back, forward) = match self.pages.get(&tab_id) {
            Some(page) => {
                let mut back = page.back_stack.clone();
                let Some(target) = back.pop() else {
                    return false;
                };
                let mut forward = page.forward_stack.clone();
                forward.push(page.url.clone());
                (target, back, forward)
            }
            None => return false,
        };
        self.navigate_tab_without_history(tab_id, &target).await;
        if let Some(page) = self.pages.get_mut(&tab_id) {
            page.back_stack = back;
            page.forward_stack = forward;
        }
        true
    }

    pub async fn go_forward(&mut self, tab_id: u32) -> bool {
        let (target, back, forward) = match self.pages.get(&tab_id) {
            Some(page) => {
                let mut forward = page.forward_stack.clone();
                let Some(target) = forward.pop() else {
                    return false;
                };
                let mut back = page.back_stack.clone();
                back.push(page.url.clone());
                (target, back, forward)
            }
            None => return false,
        };
        self.navigate_tab_without_history(tab_id, &target).await;
        if let Some(page) = self.pages.get_mut(&tab_id) {
            page.back_stack = back;
            page.forward_stack = forward;
        }
        true
    }

    pub async fn reload_active_tab(&mut self) {
        let active_id = self.priv_state.borrow().active_tab_id;
        let current_url = self.omnibox_text.clone();
        let (back, forward) = self
            .pages
            .get(&active_id)
            .map(|p| (p.back_stack.clone(), p.forward_stack.clone()))
            .unwrap_or_default();
        self.navigate_tab_without_history(active_id, &current_url)
            .await;
        if let Some(page) = self.pages.get_mut(&active_id) {
            page.back_stack = back;
            page.forward_stack = forward;
        }
    }
}
