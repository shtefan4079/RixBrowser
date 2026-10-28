use super::*;

impl BrowserShell {
    pub(super) fn dispatch_page_click(&mut self, x: f32, y: f32) -> Option<ClickAction> {
        let active_id = self.active_tab_id();
        let (target, action, focused_input, initial_value, page_runtime) = {
            let page = self.pages.get(&active_id)?;
            let base_url = document_base_url(&page.document, &page.url);
            let target = rix_layout::hit_test::hit_test_tree_with_scroll(
                &page.layout_tree,
                Some(&page.document),
                x - self.web_surface.bounds.x,
                y - self.web_surface.bounds.y,
                page.scroll_x,
                page.scroll_y,
            );
            let target = target?;

            let mut action = None;
            let mut focused_input = None;
            let mut current = Some(target);
            while let Some(node_id) = current {
                if let Some(NodeData::Element(element)) = page.document.get(node_id) {
                    if element.get_attribute("disabled").is_some()
                        && matches!(
                            &*element.name.local,
                            "input" | "textarea" | "select" | "button"
                        )
                    {
                        break;
                    }
                    if &*element.name.local == "a" {
                        if let Some(href) = element.get_attribute("href") {
                            if let Some(url) = resolve_page_link(&base_url, href) {
                                action = Some(ClickAction::NavigateUrl(url));
                            }
                            break;
                        }
                    } else if &*element.name.local == "input" {
                        let input_type = element.get_attribute("type").unwrap_or("text");
                        if matches!(input_type, "submit" | "image") {
                            if let Some(url) =
                                self.form_target_for_node(active_id, node_id, Some(node_id))
                            {
                                action = Some(ClickAction::NavigateUrl(url));
                            }
                            break;
                        }
                        if !matches!(input_type, "button" | "reset" | "hidden") {
                            focused_input = Some(node_id);
                            action = Some(ClickAction::FocusInPageInput);
                            break;
                        }
                    } else if &*element.name.local == "textarea" || &*element.name.local == "select"
                    {
                        focused_input = Some(node_id);
                        action = Some(ClickAction::FocusInPageInput);
                        break;
                    } else if &*element.name.local == "button" {
                        let button_type = element.get_attribute("type").unwrap_or("submit");
                        if button_type == "submit" {
                            if let Some(url) =
                                self.form_target_for_node(active_id, node_id, Some(node_id))
                            {
                                action = Some(ClickAction::NavigateUrl(url));
                            }
                            break;
                        }
                    }
                }
                current = page.document.parent(node_id);
            }
            let initial_value = focused_input.map(|node| {
                page.runtime_context
                    .as_ref()
                    .and_then(|context| {
                        context
                            .try_borrow()
                            .ok()
                            .map(|context| {
                                context.dom_state.borrow().input_values.get(&node).cloned()
                            })
                            .flatten()
                    })
                    .or_else(|| match page.document.get(node) {
                        Some(NodeData::Element(element)) => element
                            .get_attribute("value")
                            .map(str::to_string)
                            .or_else(|| {
                                (element.name.local.as_str() == "textarea")
                                    .then(|| page.document.inner_text(node))
                            }),
                        _ => None,
                    })
                    .unwrap_or_default()
            });
            (
                target,
                action,
                focused_input,
                initial_value,
                page.runtime_context.clone(),
            )
        };

        let mut deferred = false;
        let page_click_pos = self.pages.get(&active_id).map(|p| {
            (
                x - self.web_surface.bounds.x + p.scroll_x,
                y - self.web_surface.bounds.y + p.scroll_y,
            )
        });

        let target_selection = if let (Some(node), Some((cx, cy))) = (focused_input, page_click_pos) {
            let page = &self.pages[&active_id];
            rix_layout::hit_test::hit_test_input_cursor(&page.layout_tree, node, cx, cy)
                .map(TextSelection::collapsed)
        } else {
            None
        };
        let selection = target_selection.unwrap_or_else(|| {
            TextSelection::collapsed(
                initial_value
                    .as_ref()
                    .map_or(0, |value| value.chars().count()),
            )
        });

        if let Some(page) = self.pages.get(&active_id) {
            if let Some(worker) = &page.page_worker {
                worker
                    .borrow()
                    .focus(&page.document, focused_input, selection);
                if matches!(action, Some(ClickAction::NavigateUrl(_)))
                    && worker.borrow().is_running()
                {
                    deferred = true;
                }
                if let Some(id) = worker
                    .borrow()
                    .event(&page.document, target, "click", None, None)
                {
                    if let Some(action @ ClickAction::NavigateUrl(_)) = &action {
                        self.worker_defaults.insert((active_id, id), action.clone());
                        deferred = true;
                    }
                }
            }
        }
        if let Some(context) = page_runtime {
            if let Ok(mut context) = context.try_borrow_mut() {
                let _ =
                    dispatch_dom_event_to_js(target, "click", None, context.runtime.context_mut());
            }
        }
        if let Some(page) = self.pages.get_mut(&active_id) {
            page.in_page_focused = focused_input.is_some();
            page.focused_input = focused_input;
            page.input_selection = selection;
            if let Some(value) = initial_value {
                page.in_page_input = value;
            }
        }
        self.synchronize_page_selection(active_id);
        if deferred {
            Some(ClickAction::UnfocusOmnibox)
        } else {
            action.or(Some(ClickAction::UnfocusOmnibox))
        }
    }

    pub(super) fn dispatch_page_key(&mut self, input: &KeyboardInput) -> bool {
        let Some(page) = self.pages.get(&self.active_tab_id()) else {
            return true;
        };
        let Some(context) = page.runtime_context.clone() else {
            return true;
        };
        let target = page.focused_input.unwrap_or(page.document.root());
        let Ok(mut context) = context.try_borrow_mut() else {
            return false;
        };
        let (key_code, char_code, which) =
            rix_js::dom_bridge::legacy_key::legacy_key_values("keydown", &input.key, &input.code);
        let extra = [
            ("key", boa_engine::js_string!(input.key.as_str()).into()),
            ("code", boa_engine::js_string!(input.code.as_str()).into()),
            ("ctrlKey", boa_engine::JsValue::new(input.ctrl)),
            ("altKey", boa_engine::JsValue::new(input.alt)),
            ("shiftKey", boa_engine::JsValue::new(input.shift)),
            ("metaKey", boa_engine::JsValue::new(input.meta)),
            ("keyCode", boa_engine::JsValue::new(key_code)),
            ("charCode", boa_engine::JsValue::new(char_code)),
            ("which", boa_engine::JsValue::new(which)),
        ];
        dispatch_dom_event_to_js(
            target,
            "keydown",
            Some(&extra),
            context.runtime.context_mut(),
        )
        .map(|value| value.to_boolean())
        .unwrap_or(false)
    }

    pub(super) fn apply_page_key_default(&mut self, input: &KeyboardInput) {
        let active_id = self.active_tab_id();
        let key = input.key.as_str();
        let input_node = self
            .pages
            .get(&active_id)
            .and_then(|page| page.focused_input);
        let input_kind = input_node.and_then(|node| {
            let page = self.pages.get(&active_id)?;
            let NodeData::Element(element) = page.document.get(node)? else {
                return None;
            };
            Some((
                element.name.local.to_string(),
                element
                    .get_attribute("type")
                    .unwrap_or("text")
                    .to_ascii_lowercase(),
                element.get_attribute("readonly").is_some(),
                element.get_attribute("disabled").is_some(),
            ))
        });
        if let Some((tag, control_type, readonly, disabled)) = &input_kind {
            if *disabled {
                return;
            }
            let multiline = tag == "textarea";
            let text_control = multiline
                || tag == "input"
                    && matches!(
                        control_type.as_str(),
                        "text" | "search" | "tel" | "url" | "email" | "password" | "number"
                    );
            if text_control {
                let mut value = self.pages[&active_id].in_page_input.clone();
                let page = self.pages.get_mut(&active_id).unwrap();
                let result = edit_text(
                    &mut value,
                    &mut page.input_selection,
                    input,
                    multiline,
                    *readonly,
                );
                if result.handled {
                    if result.changed {
                        self.set_page_input_value(active_id, value);
                    }
                    self.synchronize_page_selection(active_id);
                    return;
                }
                if key == "Enter" && !input.ctrl && !input.alt && !input.meta {
                    if let Some(url) =
                        input_node.and_then(|node| self.form_target_for_node(active_id, node, None))
                    {
                        self.start_navigation(active_id, &url);
                    } else if !value.trim().is_empty() {
                        let target = self.normalize_navigation_target(&value);
                        self.start_navigation(active_id, &target);
                    }
                    return;
                }
            }
        }
        if input.ctrl && !input.alt && !input.meta && matches!(key, "Home" | "End") {
            if let Some(page) = self.pages.get_mut(&active_id) {
                page.scroll_y = if key == "Home" { 0.0 } else { f32::MAX };
            }
        } else if !input.ctrl && !input.alt && !input.meta {
            if let Some(page) = self.pages.get_mut(&active_id) {
                let viewport_height = self.web_surface.bounds.height;
                match key {
                    "ArrowLeft" => page.scroll_x = (page.scroll_x - 40.0).max(0.0),
                    "ArrowRight" => page.scroll_x += 40.0,
                    "ArrowUp" => page.scroll_y = (page.scroll_y - 40.0).max(0.0),
                    "ArrowDown" => page.scroll_y += 40.0,
                    " " => {
                        page.scroll_y = if input.shift {
                            (page.scroll_y - viewport_height).max(0.0)
                        } else {
                            page.scroll_y + viewport_height
                        };
                    }
                    "PageUp" => page.scroll_y = (page.scroll_y - viewport_height).max(0.0),
                    "PageDown" => page.scroll_y += viewport_height,
                    "Home" => page.scroll_y = 0.0,
                    "End" => page.scroll_y = f32::MAX,
                    _ => {}
                }
            }
        }
    }

    pub(super) fn synchronize_page_selection(&mut self, tab_id: u32) {
        let Some(page) = self.pages.get_mut(&tab_id) else {
            return;
        };
        page.input_selection
            .clamp(page.in_page_input.chars().count());
        page.document.set_focused(page.focused_input, true);
        page.document
            .set_text_selection(page.focused_input.map(|node| {
                (
                    node,
                    page.input_selection.anchor,
                    page.input_selection.cursor,
                )
            }));
        if let Some(context) = &page.runtime_context {
            if let Ok(context) = context.try_borrow() {
                let doc = context.dom_state.borrow().document.clone();
                let mut doc = doc.borrow_mut();
                doc.set_focused(page.focused_input, true);
                doc.set_text_selection(page.focused_input.map(|node| {
                    (
                        node,
                        page.input_selection.anchor,
                        page.input_selection.cursor,
                    )
                }));
            }
        }
        page.display_list = DisplayList::build_from_layout_with_dom(
            &page.layout_tree,
            Some(&page.document),
            &self.font_database,
        );
        if !page.images.is_empty() {
            bind_page_image_resources(page);
        }
        page.display_revision = next_page_display_revision();
    }

    pub(super) fn form_target_for_node(
        &self,
        tab_id: u32,
        node_id: rix_dom::NodeId,
        submitter: Option<rix_dom::NodeId>,
    ) -> Option<String> {
        let page = self.pages.get(&tab_id)?;
        let mut current = Some(node_id);
        let mut form = None;
        while let Some(node) = current {
            if matches!(page.document.get(node), Some(NodeData::Element(element)) if &*element.name.local == "form")
            {
                form = Some(node);
                break;
            }
            current = page.document.parent(node);
        }
        let form = form?;
        let NodeData::Element(form_element) = page.document.get(form)? else {
            return None;
        };
        if form_element
            .get_attribute("method")
            .unwrap_or("get")
            .eq_ignore_ascii_case("post")
        {
            return None;
        }
        let action = form_element.get_attribute("action").unwrap_or("");
        let base = RixUrl::parse(&document_base_url(&page.document, &page.url)).ok()?;
        let action_url = base.join(action).ok()?;
        let input_values = page
            .runtime_context
            .as_ref()
            .and_then(|context| {
                context
                    .try_borrow()
                    .ok()
                    .map(|context| context.dom_state.borrow().input_values.clone())
            })
            .unwrap_or_default();
        let mut fields = Vec::<(String, String)>::new();
        for control in page.document.descendants(form) {
            let Some(NodeData::Element(element)) = page.document.get(control) else {
                continue;
            };
            let tag = &*element.name.local;
            if !matches!(tag, "input" | "textarea" | "select" | "button") {
                continue;
            }
            if element.get_attribute("disabled").is_some() {
                continue;
            }
            let Some(name) = element.get_attribute("name") else {
                continue;
            };
            let control_type = element.get_attribute("type").unwrap_or("text");
            if tag == "input" && matches!(control_type, "submit" | "button" | "reset" | "image") {
                if Some(control) != submitter {
                    continue;
                }
            }
            if tag == "input"
                && matches!(control_type, "checkbox" | "radio")
                && element.get_attribute("checked").is_none()
            {
                continue;
            }
            let value = input_values
                .get(&control)
                .cloned()
                .or_else(|| element.get_attribute("value").map(str::to_string))
                .unwrap_or_else(|| {
                    if tag == "textarea" {
                        page.document.inner_text(control)
                    } else {
                        String::new()
                    }
                });
            fields.push((name.to_string(), value));
        }
        if let Some(submitter) = submitter {
            if let Some(NodeData::Element(element)) = page.document.get(submitter) {
                if matches!(&*element.name.local, "button" | "input") {
                    if let Some(name) = element.get_attribute("name") {
                        if !fields.iter().any(|(field, _)| field == name) {
                            fields.push((
                                name.to_string(),
                                element.get_attribute("value").unwrap_or("").to_string(),
                            ));
                        }
                    }
                }
            }
        }
        let mut resolved = url::Url::parse(action_url.as_str()).ok()?;
        resolved.query_pairs_mut().extend_pairs(
            fields
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
        Some(resolved.to_string())
    }

    pub(super) fn set_page_input_value(&mut self, tab_id: u32, value: String) {
        self.commit_page_input_value(tab_id, value, true);
    }

    pub(super) fn commit_page_input_value(&mut self, tab_id: u32, value: String, notify: bool) {
        let Some((node_id, page_runtime, selection)) = self.pages.get(&tab_id).and_then(|page| {
            page.focused_input
                .map(|node| (node, page.runtime_context.clone(), page.input_selection))
        }) else {
            return;
        };

        if !notify
            && self
                .pages
                .get(&tab_id)
                .and_then(|page| control_value(&page.document, node_id))
                .as_deref()
                == Some(value.as_str())
        {
            self.pages.get_mut(&tab_id).unwrap().in_page_input = value;
            self.synchronize_page_selection(tab_id);
            return;
        }
        if let Some(page) = self.pages.get_mut(&tab_id) {
            page.in_page_input = value.clone();
            page.document.set_focused(Some(node_id), true);
            page.document.set_text_selection(Some((
                node_id,
                page.input_selection.anchor,
                page.input_selection.cursor,
            )));
            if notify {
                if let Some(worker) = &page.page_worker {
                    worker.borrow().event(
                        &page.document,
                        node_id,
                        "input",
                        None,
                        Some(value.clone()),
                    );
                }
            }
            if let Some(NodeData::Element(element)) = page.document.get_mut(node_id) {
                element.set_attribute("value", &value);
            }
            page.layout_tree.update_input_text(node_id, &value, &self.font_database);
        }
        if let Some(context) = page_runtime.filter(|_| notify) {
            if let Ok(mut context) = context.try_borrow_mut() {
                let dom_state = context.dom_state.clone();
                let document = {
                    let mut state = dom_state.borrow_mut();
                    state.input_values.insert(node_id, value.clone());
                    state.document.clone()
                };
                document.borrow_mut().set_focused(Some(node_id), true);
                document.borrow_mut().set_text_selection(Some((
                    node_id,
                    selection.anchor,
                    selection.cursor,
                )));
                if let Some(NodeData::Element(element)) = document.borrow_mut().get_mut(node_id) {
                    element.set_attribute("value", &value);
                }
                let _ =
                    dispatch_dom_event_to_js(node_id, "input", None, context.runtime.context_mut());
            }
        }
        if let Some(page) = self.pages.get_mut(&tab_id) {
            page.in_page_input = value;
        }
        if !notify
            && self
                .pages
                .get(&tab_id)
                .is_some_and(|page| page.page_worker.is_some())
        {
            self.synchronize_page_selection(tab_id);
            return;
        }
        if let Some(page) = self
            .pages
            .get_mut(&tab_id)
            .filter(|page| page.runtime_context.is_none())
        {
            self.pending_page_layouts.remove(&tab_id);
            self.deferred_worker_snapshots.remove(&tab_id);
            page.html = page.document.to_html_string(page.document.root());
            page.layout_tree = compute_page_layout(
                &page.document,
                &page.url,
                &page.external_stylesheets,
                self.web_surface.bounds.width,
                self.web_surface.bounds.height,
                &self.font_database,
            );
            page.rendered_dom_revision = page.document.revision();
        } else {
            self.refresh_active_page_render_data();
        }
        self.synchronize_page_selection(tab_id);
    }

    pub(super) async fn synchronize_privileged_host(&mut self, previous_active_id: u32) {
        let (requested_theme, requested_orientation) = {
            let mut state = self.priv_state.borrow_mut();
            (
                state.requested_theme.take(),
                state.requested_tab_orientation.take(),
            )
        };
        let mut appearance_changed = false;
        if let Some(mode) = requested_theme {
            let theme = match mode.as_str() {
                "light" => ThemeMode::Light,
                "dark" => ThemeMode::Dark,
                _ => self.customization.theme_mode(),
            };
            if self.customization.theme_mode() != theme {
                self.customization.set_theme_mode(theme);
                appearance_changed = true;
            }
        }
        if let Some(orientation) = requested_orientation {
            let orientation = match orientation.as_str() {
                "vertical" => TabOrientation::Vertical,
                "horizontal" => TabOrientation::Horizontal,
                _ => self.customization.tab_orientation(),
            };
            if self.customization.tab_orientation() != orientation {
                self.customization.set_tab_orientation(orientation);
                appearance_changed = true;
            }
        }
        if appearance_changed {
            let _ = self.apply_customization_to_chrome();
        }
        let commands = {
            let mut state = self.priv_state.borrow_mut();
            std::mem::take(&mut state.navigation_history)
        };
        let draft = if commands.is_empty() {
            let doc = self.document.borrow();
            let input = doc.query_selector(doc.root(), "#omnibox");
            (input.is_some() && doc.focused_node() == input).then(|| {
                (
                    self.chrome_omnibox_value().unwrap_or_default(),
                    doc.text_selection(),
                )
            })
        } else {
            None
        };
        for command in commands {
            match command {
                rix_js::privileged::NavigationCommand::GoTo(url) => {
                    self.start_navigation_active_tab(&url);
                }
                rix_js::privileged::NavigationCommand::Back => {
                    let id = self.active_tab_id();
                    self.start_go_back(id);
                }
                rix_js::privileged::NavigationCommand::Forward => {
                    let id = self.active_tab_id();
                    self.start_go_forward(id);
                }
                rix_js::privileged::NavigationCommand::Reload => {
                    self.start_reload_active_tab();
                }
            }
        }

        let clipboard_text = self.priv_state.borrow_mut().clipboard_write.take();
        if let Some(text) = clipboard_text {
            crate::text_editing::set_clipboard(&text);
        }

        let cookie_cmds = {
            let mut state = self.priv_state.borrow_mut();
            std::mem::take(&mut state.cookie_commands)
        };
        let mut should_reload = false;
        let active_tab_url = self
            .pages
            .get(&self.active_tab_id())
            .map(|p| p.url.clone())
            .unwrap_or_default();
        let is_internal = self.is_active_internal();

        if let Some(ref storage) = self.storage {
            for cmd in cookie_cmds {
                match cmd {
                    rix_js::privileged::CookieCommand::DeleteForDomain(domain) => {
                        let _ = storage.delete_cookies_for_domain(&domain);
                        let _ = storage.clear_local_storage(&format!("https://{domain}"));
                        let _ = storage.clear_local_storage(&format!("http://{domain}"));
                        if domain.contains("youtube.com") {
                            let _ = storage.delete_cookies_for_domain("google.com");
                            let _ = storage.clear_local_storage("https://www.google.com");
                            let _ = storage.clear_local_storage("https://consent.youtube.com");
                        }
                        if active_tab_url.contains(&domain) {
                            should_reload = true;
                        }
                    }
                    rix_js::privileged::CookieCommand::ClearAll => {
                        let _ = storage.clear_cookies();
                        if !is_internal {
                            should_reload = true;
                        }
                    }
                }
            }
            if let Ok(cookies) = storage.get_all_cookies() {
                let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
                for c in cookies {
                    let domain = c.domain.trim_start_matches('.').to_string();
                    *counts.entry(domain).or_insert(0) += 1;
                }
                self.priv_state.borrow_mut().cookie_domains = counts.into_iter().collect();
            }
        }

        if should_reload {
            self.start_reload_active_tab();
        }

        let descriptors = self.priv_state.borrow().tabs.clone();
        let descriptor_ids: std::collections::HashSet<u32> =
            descriptors.iter().map(|tab| tab.id).collect();
        self.pages.retain(|id, _| descriptor_ids.contains(id));

        for tab in &descriptors {
            let page_url = self.pages.get(&tab.id).map(|page| page.url.clone());
            match page_url {
                None => self.start_navigation(tab.id, &tab.url),
                Some(url) if url != tab.url => self.start_navigation(tab.id, &tab.url),
                _ => {}
            }
        }

        let active_id = self.active_tab_id();
        let tabs_changed = descriptors.len() != self.pages.len()
            || descriptors.iter().any(|tab| {
                self.pages
                    .get(&tab.id)
                    .map(|page| page.url != tab.url || page.title != tab.title)
                    .unwrap_or(true)
            });
        if active_id != previous_active_id || tabs_changed {
            self.notify_chrome_tabs_changed();
        }
        if let Some((value, selection)) = draft {
            self.set_chrome_omnibox_value(&value);
            self.document.borrow_mut().set_text_selection(selection);
        }
        self.adopt_chrome_input_state();
    }

    pub(super) fn notify_chrome_tabs_changed(&mut self) {
        let active_id = self.active_tab_id();
        let can_back = self
            .pages
            .get(&active_id)
            .is_some_and(|page| !page.back_stack.is_empty());
        let can_forward = self
            .pages
            .get(&active_id)
            .is_some_and(|page| !page.forward_stack.is_empty());
        let active_loading = self
            .pages
            .get(&active_id)
            .is_some_and(|page| page.is_loading);
        {
            let mut doc = self.document.borrow_mut();
            for (selector, enabled) in [("#nav-back", can_back), ("#nav-forward", can_forward)] {
                if let Some(node) = doc.query_selector(doc.root(), selector) {
                    if let Some(NodeData::Element(element)) = doc.get_mut(node) {
                        if enabled {
                            element.remove_attribute("disabled");
                        } else {
                            element.set_attribute("disabled", "");
                        }
                    }
                }
            }
        }
        let loading_by_id: std::collections::HashMap<u32, bool> = self
            .pages
            .iter()
            .map(|(id, page)| (*id, page.is_loading))
            .collect();
        let state = self.priv_state.borrow();
        let tabs: Vec<serde_json::Value> = state
            .tabs
            .iter()
            .map(|tab| {
                serde_json::json!({
                    "id": tab.id,
                    "url": tab.url,
                    "title": tab.title,
                    "isActive": tab.is_active,
                    "isLoading": loading_by_id.get(&tab.id).copied().unwrap_or(false),
                })
            })
            .collect();
        let info = serde_json::json!({
            "tabs": tabs,
            "activeTabId": state.active_tab_id,
            "activeLoading": active_loading,
        });
        let active_url = state
            .tabs
            .iter()
            .find(|tab| tab.id == state.active_tab_id)
            .map(|tab| tab.url.clone())
            .unwrap_or_else(|| "about:blank".to_string());
        drop(state);
        if let Ok(json) = serde_json::to_string(&info) {
            let script = format!(
                "if (window.rixChrome && window.rixChrome.onTabUpdated) window.rixChrome.onTabUpdated({json});"
            );
            let _ = self.runtime.evaluate(&script);
        }
        let connection = if active_url.starts_with("https:") {
            "Secure connection"
        } else if active_url.starts_with("http:") {
            "HTTP connection (not secure)"
        } else {
            "Internal page"
        };
        let active_id = self.priv_state.borrow().active_tab_id;
        let page_status = self
            .pages
            .get(&active_id)
            .map(|page| page.status_message.trim())
            .unwrap_or("");
        let status_text = if page_status.is_empty() || page_status.eq_ignore_ascii_case(connection) {
            format!("{connection} • {active_url}")
        } else {
            format!("{connection} • {page_status} • {active_url}")
        };
        self.set_chrome_text_content("status-left", &status_text);
        self.needs_redraw = true;
    }
}
