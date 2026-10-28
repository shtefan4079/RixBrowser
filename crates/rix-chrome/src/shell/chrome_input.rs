use super::*;

impl BrowserShell {
    fn connected_chrome_focused_node(&self) -> Option<rix_dom::NodeId> {
        let mut doc = self.document.borrow_mut();
        let focused = doc.focused_node()?;
        let connected = std::iter::once(focused)
            .chain(doc.ancestors(focused))
            .any(|ancestor| ancestor == doc.root());
        if !connected {
            doc.set_focused(None, false);
            doc.set_text_selection(None);
            return None;
        }
        Some(focused)
    }

    pub fn update_chrome_pointer(&mut self, position: Option<(f32, f32)>, pressed: bool) -> bool {
        let is_internal = self.is_active_internal();
        let target = position.and_then(|(x, y)| {
            if !is_internal && self.web_surface.contains(x, y) {
                return None;
            }
            self.layout_tree
                .as_ref()
                .and_then(|layout| rix_layout::hit_test::hit_test_tree(layout, x, y))
        });
        let mut doc = self.document.borrow_mut();
        let hovered = doc.set_hovered(target);
        let active = doc.set_active(if pressed { target } else { None });
        hovered || active
    }

    pub fn cursor_at(&self, x: f32, y: f32) -> &'static str {
        if !self.is_active_internal() && self.web_surface.contains(x, y) {
            let active_id = self.active_tab_id();
            let Some(page) = self.pages.get(&active_id) else {
                return "default";
            };
            let rel_x = x - self.web_surface.bounds.x;
            let rel_y = y - self.web_surface.bounds.y;
            let box_id = rix_layout::hit_test_box_with_scroll(
                &page.layout_tree,
                Some(&page.document),
                rel_x,
                rel_y,
                page.scroll_x,
                page.scroll_y,
            );
            if let Some(box_id) = box_id {
                if let Some(box_data) = page.layout_tree.get(box_id) {
                    let current_node = box_data.dom_node.or_else(|| {
                        box_data
                            .parent
                            .and_then(|p| page.layout_tree.get(p))
                            .and_then(|p| p.dom_node)
                    });

                    let mut inside_clickable = false;
                    let mut inside_text_input = false;
                    let mut node_iter = current_node;
                    while let Some(node) = node_iter {
                        if let Some(NodeData::Element(element)) = page.document.get(node) {
                            let tag = &*element.name.local;
                            if (tag == "a" && element.get_attribute("href").is_some())
                                || tag == "button"
                                || element.get_attribute("role") == Some("button")
                                || element.get_attribute("role") == Some("link")
                                || element.get_attribute("role") == Some("tab")
                                || element.get_attribute("role") == Some("menuitem")
                                || element.get_attribute("onclick").is_some()
                            {
                                inside_clickable = true;
                                break;
                            }
                            if tag == "input" {
                                let input_type = element.get_attribute("type").unwrap_or("text");
                                if matches!(
                                    input_type,
                                    "submit" | "button" | "reset" | "checkbox" | "radio"
                                ) {
                                    inside_clickable = true;
                                } else {
                                    inside_text_input = true;
                                }
                                break;
                            }
                            if tag == "textarea" {
                                inside_text_input = true;
                                break;
                            }
                            if tag == "select" {
                                inside_clickable = true;
                                break;
                            }
                        }
                        node_iter = page.document.parent(node);
                    }

                    if inside_clickable {
                        return "pointer";
                    }
                    if inside_text_input {
                        return "text";
                    }

                    let css_cursor = box_data.style.cursor.as_str();
                    match css_cursor {
                        "pointer" => return "pointer",
                        "text" => return "text",
                        "grab" => return "grab",
                        "grabbing" => return "grabbing",
                        "move" => return "move",
                        "not-allowed" => return "not-allowed",
                        "wait" => return "wait",
                        "progress" => return "progress",
                        "crosshair" => return "crosshair",
                        "zoom-in" => return "zoom-in",
                        "zoom-out" => return "zoom-out",
                        "col-resize" => return "col-resize",
                        "row-resize" => return "row-resize",
                        "help" => return "help",
                        "default" => return "default",
                        _ => {}
                    }

                    if box_data.text_layout.is_some()
                        || matches!(box_data.box_type, rix_layout::AnonymousBoxType::Text(_))
                    {
                        return "text";
                    }
                }
            }
            return "default";
        }

        let target = self
            .layout_tree
            .as_ref()
            .and_then(|layout| rix_layout::hit_test::hit_test_tree(layout, x, y));
        let Some(target) = target else {
            return "default";
        };

        let doc = self.document.borrow();
        let mut current = Some(target);
        while let Some(node_id) = current {
            if let Some(NodeData::Element(element)) = doc.get(node_id) {
                if element.get_attribute("id") == Some("omnibox") {
                    return "text";
                }
                match element.get_attribute("id") {
                    Some(
                        "new-tab-btn"
                        | "theme-toggle"
                        | "tabs-orientation-toggle"
                        | "nav-back"
                        | "nav-forward"
                        | "nav-reload"
                        | "nav-home",
                    ) => return "pointer",
                    _ => {}
                }
                if element.get_attribute("data-tab-id").is_some() {
                    return "pointer";
                }
                if element
                    .get_attribute("class")
                    .map(|class| {
                        class.contains("tab-close")
                            || class.contains("tab-title")
                            || class.contains("tab")
                    })
                    .unwrap_or(false)
                {
                    return "pointer";
                }
                if &*element.name.local == "button"
                    || element.get_attribute("role") == Some("button")
                {
                    return "pointer";
                }
            }
            current = doc.parent(node_id);
        }

        "default"
    }

    pub(super) fn adopt_chrome_input_state(&mut self) {
        if let Some(value) = self.chrome_omnibox_value() {
            self.omnibox_text = value;
        }
        let doc = self.document.borrow();
        let omnibox = doc.query_selector(doc.root(), "#omnibox");
        self.omnibox_focused = omnibox.is_some() && doc.focused_node() == omnibox;
        if self.omnibox_focused {
            if let Some((node, start, end)) = doc
                .text_selection()
                .filter(|selection| Some(selection.0) == omnibox)
            {
                let _ = node;
                let length = self.omnibox_text.chars().count();
                self.omnibox_selection = TextSelection {
                    anchor: start.min(length),
                    cursor: end.min(length),
                };
            }
        }
    }

    pub(super) fn synchronize_chrome_input_state(&mut self, keyboard: bool) {
        let mut doc = self.document.borrow_mut();
        if self.omnibox_focused {
            let node = doc.query_selector(doc.root(), "#omnibox");
            doc.set_focused(node, keyboard);
            self.omnibox_selection
                .clamp(self.omnibox_text.chars().count());
            let selection = node.map(|node| {
                (
                    node,
                    self.omnibox_selection.anchor,
                    self.omnibox_selection.cursor,
                )
            });
            doc.set_text_selection(selection);
        } else {
            let focused = doc.focused_node();
            if focused == doc.query_selector(doc.root(), "#omnibox") {
                doc.set_focused(None, keyboard);
            }
            doc.set_text_selection(None);
        }
    }

    pub(super) fn move_chrome_focus(&mut self, backwards: bool) {
        let mut doc = self.document.borrow_mut();
        let candidates: Vec<_> = doc
            .descendants(doc.root())
            .filter(|&node| {
                let Some(NodeData::Element(element)) = doc.get(node) else {
                    return false;
                };
                if element.get_attribute("disabled").is_some()
                    || element.get_attribute("hidden").is_some()
                {
                    return false;
                }
                let focusable = matches!(element.name.local.as_str(), "button" | "input")
                    || element.get_attribute("tabindex") == Some("0");
                focusable
                    && self.layout_tree.as_ref().is_some_and(|tree| {
                        tree.iter().any(|layout_box| {
                            layout_box.dom_node == Some(node)
                                && layout_box.dimensions.content.width > 0.0
                                && layout_box.dimensions.content.height > 0.0
                        })
                    })
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        let current = candidates
            .iter()
            .position(|node| Some(*node) == doc.focused_node());
        let index = match (current, backwards) {
            (Some(index), true) => (index + candidates.len() - 1) % candidates.len(),
            (Some(index), false) => (index + 1) % candidates.len(),
            (None, true) => candidates.len() - 1,
            (None, false) => 0,
        };
        let target = candidates[index];
        self.omnibox_focused = Some(target) == doc.query_selector(doc.root(), "#omnibox");
        self.omnibox_selection = TextSelection::collapsed(self.omnibox_text.chars().count());
        if self.omnibox_focused {
            self.omnibox_selection
                .select_all(self.omnibox_text.chars().count());
        }
        doc.set_focused(Some(target), true);
        drop(doc);
        self.synchronize_chrome_input_state(true);
    }

    pub async fn handle_input(&mut self, event: IpcInputEvent) -> Result<(), ShellError> {
        match event {
            IpcInputEvent::MouseDown { x, y, button } => {
                self.route_mouse_down(x, y, button).await;
            }
            IpcInputEvent::KeyDown { key, code } => {
                self.handle_native_key(&key, &code, false, false, false, false)
                    .await;
            }
            other => self.forward_input_to_active_tab(&other).await,
        }

        Ok(())
    }

    pub(super) async fn forward_input_to_active_tab(&mut self, event: &IpcInputEvent) {
        if self.is_active_internal() {
            return;
        }
        let is_in_surface = match event {
            IpcInputEvent::MouseMove { x, y }
            | IpcInputEvent::MouseDown { x, y, .. }
            | IpcInputEvent::MouseUp { x, y, .. } => self.web_surface.contains(*x, *y),
            IpcInputEvent::Scroll { .. } => true,
            _ => false,
        };
        if !is_in_surface {
            return;
        }

        if let Some(mapped_evt) = self.web_surface.map_input_event(event) {
            let active_tab_id = self.priv_state.borrow().active_tab_id;
            let _ = self
                .process_mgr
                .send_to_tab(
                    active_tab_id,
                    &IpcMessage::InputEvent {
                        tab_id: active_tab_id,
                        event: mapped_evt,
                    },
                )
                .await;
        }
    }

    pub(super) async fn route_mouse_down(&mut self, x: f32, y: f32, button: u8) {
        let hit_chrome_overlay = self.layout_tree.as_ref().and_then(|lt| {
            let node = rix_layout::hit_test::hit_test_tree(lt, x, y)?;
            let doc = self.document.borrow();
            let is_overlay = std::iter::once(node).chain(doc.ancestors(node)).any(|n| {
                matches!(doc.get(n), Some(NodeData::Element(el)) if matches!(el.get_attribute("id"), Some("site-info-popup" | "tab-context-menu" | "more-menu" | "omnibox-suggestions" | "command-palette")) || matches!(el.get_attribute("class"), Some(c) if c.contains("site-info-popup") || c.contains("context-menu") || c.contains("more-menu") || c.contains("omnibox-suggestions")))
            });
            is_overlay.then_some(node)
        });

        if !self.is_active_internal() && self.web_surface.contains(x, y) && hit_chrome_overlay.is_none() {
            self.document.borrow_mut().set_focused(None, false);
            self.document.borrow_mut().set_text_selection(None);
            self.omnibox_focused = false;
            if let Some(action) = self.dispatch_page_click(x, y) {
                self.execute_action(action).await;
            }
            let event = IpcInputEvent::MouseDown { x, y, button };
            self.forward_input_to_active_tab(&event).await;
        } else {
            let active_id = self.active_tab_id();
            if let Some(page) = self.pages.get_mut(&active_id) {
                page.focused_input = None;
                page.in_page_focused = false;
                page.document.set_focused(None, false);
                page.document.set_text_selection(None);
                if let Some(worker) = &page.page_worker {
                    worker
                        .borrow()
                        .focus(&page.document, None, TextSelection::default());
                }
            }
            let previous_active_id = self.active_tab_id();
            let previous_theme = self.customization.theme_mode();
            let previous_orientation = self.customization.tab_orientation();
            let previous_tabs_len = self.priv_state.borrow().tabs.len();

            let chrome_hit = self.dispatch_chrome_click_at(x, y, button);
            if chrome_hit.is_some() {
                self.synchronize_privileged_host(previous_active_id).await;
            }

            if button == 0 {
                if let Some(action) = self.click_at(x, y) {
                    let js_handled = match &action {
                        ClickAction::ToggleTheme => self.customization.theme_mode() != previous_theme,
                        ClickAction::ToggleTabOrientation => {
                            self.customization.tab_orientation() != previous_orientation
                        }
                        ClickAction::NewTab => {
                            self.priv_state.borrow().tabs.len() > previous_tabs_len
                        }
                        ClickAction::CloseTab(_) => {
                            self.priv_state.borrow().tabs.len() < previous_tabs_len
                        }
                        ClickAction::SwitchTab(_) => self.active_tab_id() != previous_active_id,
                        ClickAction::FocusOmnibox => self.omnibox_focused,
                        _ => false,
                    };
                    if !js_handled {
                        self.execute_action(action).await;
                    }
                } else {
                    self.omnibox_focused = false;
                }
            }
            self.needs_redraw = true;
        }
    }

    pub async fn handle_native_key(
        &mut self,
        key: &str,
        code: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) {
        let focused_node = self.connected_chrome_focused_node();
        let chrome_owns_focus = self.omnibox_focused || focused_node.is_some();
        if key == "Tab" && !ctrl && !alt && !meta && chrome_owns_focus {
            self.move_chrome_focus(shift);
            return;
        }
        let is_button = focused_node.is_some_and(|node| {
            let doc = self.document.borrow();
            matches!(doc.get(node), Some(NodeData::Element(el)) if &*el.name.local == "button" || el.get_attribute("role") == Some("button"))
        });
        if chrome_owns_focus && !self.omnibox_focused && is_button && matches!(key, "Enter" | " ") {
            if let Some(target) = focused_node {
                let previous_active_id = self.active_tab_id();
                let _ = dispatch_dom_event_to_js(target, "click", None, self.runtime.context_mut());
                self.synchronize_privileged_host(previous_active_id).await;
                return;
            }
        }
        let ctrl_l = (ctrl || meta)
            && if code.is_empty() {
                key.eq_ignore_ascii_case("l")
            } else {
                code == "KeyL"
            };
        let alt_left = alt
            && if code.is_empty() {
                key == "ArrowLeft"
            } else {
                code == "ArrowLeft"
            };
        let alt_right = alt
            && if code.is_empty() {
                key == "ArrowRight"
            } else {
                code == "ArrowRight"
            };
        let previous_active_id = self.active_tab_id();
        let doc = self.document.borrow();
        let target = if self.omnibox_focused {
            doc.query_selector(doc.root(), "#omnibox")
        } else {
            focused_node.or(Some(doc.root()))
        };
        drop(doc);

        let input = KeyboardInput {
            key: key.into(),
            code: code.into(),
            ctrl,
            alt,
            shift,
            meta,
        };
        let chrome_allowed = target
            .map(|target| self.dispatch_key_down_to(target, key, code, ctrl, alt, shift, meta))
            .unwrap_or(true);

        if self.omnibox_focused {
            if ctrl || alt || meta {
                self.synchronize_privileged_host(previous_active_id).await;
            }
            match key {
                "Enter" => {
                    self.synchronize_privileged_host(previous_active_id).await;
                    if let Some(value) = self.chrome_omnibox_value() {
                        self.omnibox_text = value;
                        self.omnibox_selection =
                            TextSelection::collapsed(self.omnibox_text.chars().count());
                    }
                    let has_pending_go_to =
                        self.priv_state
                            .borrow()
                            .navigation_history
                            .iter()
                            .any(|command| {
                                matches!(command, rix_js::privileged::NavigationCommand::GoTo(_))
                            });
                    if !has_pending_go_to {
                        let destination = self.omnibox_text.trim().to_string();
                        if !destination.is_empty() {
                            self.start_navigation_active_tab(&destination);
                        }
                    }
                    self.synchronize_privileged_host(previous_active_id).await;
                    self.omnibox_focused = false;
                    self.omnibox_selection =
                        TextSelection::collapsed(self.omnibox_text.chars().count());
                    self.needs_redraw = true;
                }
                "Escape" => {
                    self.omnibox_focused = false;
                    self.omnibox_selection =
                        TextSelection::collapsed(self.omnibox_text.chars().count());
                    self.needs_redraw = true;
                }
                _ => {
                    if key == "ArrowDown" || key == "ArrowUp" {
                        self.synchronize_privileged_host(previous_active_id).await;
                        if let Some(val) = self.chrome_omnibox_value() {
                            if val != self.omnibox_text {
                                self.omnibox_text = val;
                                self.omnibox_selection = TextSelection::collapsed(self.omnibox_text.chars().count());
                            }
                        }
                        self.needs_redraw = true;
                    } else if ctrl_l {
                        self.synchronize_privileged_host(previous_active_id).await;
                        self.omnibox_selection
                            .select_all(self.omnibox_text.chars().count());
                    } else if chrome_allowed {
                        let result = edit_text(
                            &mut self.omnibox_text,
                            &mut self.omnibox_selection,
                            &input,
                            false,
                            false,
                        );
                        if result.handled {
                            if result.changed {
                                self.set_chrome_omnibox_value(&self.omnibox_text.clone());
                                self.dispatch_chrome_input_event();
                            }
                            self.synchronize_chrome_input_state(true);
                            let _ = self.recompute_layout();
                        }
                    }
                }
            }
        } else if let Some(target_input) = focused_node.filter(|&n| {
            let doc = self.document.borrow();
            matches!(doc.get(n), Some(NodeData::Element(el)) if matches!(el.name.local.as_str(), "input" | "textarea"))
        }) {
            let mut current_val = {
                let doc = self.document.borrow();
                doc.get(target_input)
                    .and_then(|data| match data {
                        NodeData::Element(el) => el.get_attribute("value").map(String::from),
                        _ => None,
                    })
                    .unwrap_or_default()
            };
            let mut selection = TextSelection::collapsed(current_val.chars().count());
            match key {
                "Enter" => {
                    self.synchronize_privileged_host(previous_active_id).await;
                    let has_pending_go_to = self
                        .priv_state
                        .borrow()
                        .navigation_history
                        .iter()
                        .any(|c| matches!(c, rix_js::privileged::NavigationCommand::GoTo(_)));
                    if !has_pending_go_to && !current_val.trim().is_empty() {
                        self.start_navigation_active_tab(&current_val);
                    }
                    self.synchronize_privileged_host(previous_active_id).await;
                    self.needs_redraw = true;
                }
                "Escape" => {
                    self.document.borrow_mut().set_focused(None, false);
                    self.needs_redraw = true;
                }
                _ => {
                    let result = edit_text(&mut current_val, &mut selection, &input, false, false);
                    if result.handled {
                        if result.changed {
                            {
                                let mut doc = self.document.borrow_mut();
                                if let Some(NodeData::Element(el)) = doc.get_mut(target_input) {
                                    el.set_attribute("value", &current_val);
                                }
                            }
                            let _ = dispatch_dom_event_to_js(
                                target_input,
                                "input",
                                None,
                                self.runtime.context_mut(),
                            );
                            self.synchronize_privileged_host(previous_active_id).await;
                            let _ = self.recompute_layout();
                        }
                        self.needs_redraw = true;
                    }
                }
            }
        } else {
            let alt_back_command_pending = self
                .priv_state
                .borrow()
                .navigation_history
                .iter()
                .any(|command| matches!(command, rix_js::privileged::NavigationCommand::Back));
            let alt_forward_command_pending = self
                .priv_state
                .borrow()
                .navigation_history
                .iter()
                .any(|command| matches!(command, rix_js::privileged::NavigationCommand::Forward));
            self.synchronize_privileged_host(previous_active_id).await;
            if ctrl_l {
                self.omnibox_focused = true;
                self.omnibox_selection
                    .select_all(self.omnibox_text.chars().count());
            } else if alt_left && !alt_back_command_pending {
                let active_id = self.active_tab_id();
                self.start_go_back(active_id);
            } else if alt_right && !alt_forward_command_pending {
                let active_id = self.active_tab_id();
                self.start_go_forward(active_id);
            }
            let browser_shortcut = ctrl_l || alt_left || alt_right;
            if chrome_allowed && !browser_shortcut {
                let tab_id = self.active_tab_id();
                let queued = self.pages.get(&tab_id).and_then(|page| {
                    page.page_worker.as_ref().and_then(|worker| {
                        let target = page.focused_input.unwrap_or(page.document.root());
                        worker
                            .borrow()
                            .key_event(&page.document, target, input.clone())
                    })
                });
                if let Some(id) = queued {
                    let page = &self.pages[&tab_id];
                    self.worker_keys.insert(
                        (tab_id, id),
                        PendingPageKey {
                            input,
                            target: page.page_worker.as_ref().and_then(|worker| {
                                page.focused_input.and_then(|node| {
                                    worker.borrow().node_token(&page.document, node)
                                })
                            }),
                            focus_epoch: page
                                .page_worker
                                .as_ref()
                                .map_or(0, |worker| worker.borrow().focus_epoch()),
                            generation: page.image_generation,
                        },
                    );
                } else if self.pages.get(&tab_id).is_some_and(|page| {
                    page.page_worker
                        .as_ref()
                        .is_some_and(|worker| worker.borrow().is_running())
                }) {
                    tracing::warn!(tab_id, "Page input queue cannot accept a keyboard event");
                } else {
                    let allowed = self.dispatch_page_key(&input);
                    if allowed {
                        self.apply_page_key_default(&input);
                    }
                }
            }
        }
        self.synchronize_chrome_input_state(true);
    }

    pub(super) fn dispatch_chrome_click_at(&mut self, x: f32, y: f32, button: u8) -> Option<rix_dom::NodeId> {
        let is_overlay = self.layout_tree.as_ref().and_then(|lt| {
            let node = rix_layout::hit_test::hit_test_tree(lt, x, y)?;
            let doc = self.document.borrow();
            let is_overlay = std::iter::once(node).chain(doc.ancestors(node)).any(|n| {
                matches!(doc.get(n), Some(NodeData::Element(el)) if matches!(el.get_attribute("id"), Some("site-info-popup" | "tab-context-menu" | "more-menu" | "omnibox-suggestions" | "command-palette")) || matches!(el.get_attribute("class"), Some(c) if c.contains("site-info-popup") || c.contains("context-menu") || c.contains("more-menu") || c.contains("omnibox-suggestions")))
            });
            is_overlay.then_some(node)
        });

        let target_node = if let Some(ref lt) = self.layout_tree {
            if is_overlay.is_some() {
                is_overlay
            } else if self.is_active_internal() && self.web_surface.contains(x, y) && self.internal_scroll_y > 0.0 {
                rix_layout::hit_test::hit_test_tree_with_scroll(lt, Some(&*self.document.borrow()), x, y, 0.0, self.internal_scroll_y)
            } else {
                rix_layout::hit_test::hit_test_tree(lt, x, y)
            }
        } else {
            None
        };

        if let Some(target) = target_node {
            let disabled = {
                let doc = self.document.borrow();
                let disabled = std::iter::once(target).chain(doc.ancestors(target)).any(|node| matches!(doc.get(node), Some(NodeData::Element(element)) if element.get_attribute("disabled").is_some()));
                disabled
            };
            if disabled {
                return None;
            }
            self.set_chrome_focus_for_node(target, Some((x, y)));
            let event_name = if button == 2 { "contextmenu" } else { "click" };
            let coords = [
                ("clientX", boa_engine::JsValue::from(x as f64)),
                ("clientY", boa_engine::JsValue::from(y as f64)),
            ];
            if let Err(e) = dispatch_dom_event_to_js(target, event_name, Some(&coords), self.runtime.context_mut()) {
                tracing::debug!("Chrome DOM event dispatch error: {e}");
            }
            return Some(target);
        }
        None
    }

    pub(super) fn dispatch_key_down_to(
        &mut self,
        target: rix_dom::NodeId,
        key: &str,
        code: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> bool {
        let extra = [
            ("key", boa_engine::js_string!(key).into()),
            ("code", boa_engine::js_string!(code).into()),
            ("ctrlKey", boa_engine::JsValue::new(ctrl)),
            ("altKey", boa_engine::JsValue::new(alt)),
            ("shiftKey", boa_engine::JsValue::new(shift)),
            ("metaKey", boa_engine::JsValue::new(meta)),
        ];
        dispatch_dom_event_to_js(target, "keydown", Some(&extra), self.runtime.context_mut())
            .map(|value| value.to_boolean())
            .unwrap_or(false)
    }

    pub(super) fn set_chrome_focus_for_node(
        &mut self,
        target: rix_dom::NodeId,
        click_pos: Option<(f32, f32)>,
    ) {
        let doc = self.document.borrow();
        let mut current = Some(target);
        let mut is_omnibox = false;
        while let Some(node_id) = current {
            if let Some(NodeData::Element(element)) = doc.get(node_id) {
                if element.get_attribute("id") == Some("omnibox")
                    || element
                        .get_attribute("class")
                        .map(|classes| classes.split_whitespace().any(|class| class == "omnibox"))
                        .unwrap_or(false)
                {
                    is_omnibox = true;
                    break;
                }
            }
            current = doc.parent(node_id);
        }
        let focus_node = if is_omnibox {
            doc.query_selector(doc.root(), "#omnibox")
        } else {
            std::iter::once(target).chain(doc.ancestors(target)).find(|&node| {
                matches!(doc.get(node), Some(NodeData::Element(element)) if matches!(element.name.local.as_str(), "button" | "input" | "textarea") || element.get_attribute("tabindex") == Some("0"))
            })
        };
        drop(doc);
        let was_omnibox_focused = self.omnibox_focused;
        self.document.borrow_mut().set_focused(focus_node, false);
        self.omnibox_focused = is_omnibox;
        if is_omnibox {
            if !was_omnibox_focused {
                
                self.omnibox_selection = TextSelection {
                    anchor: 0,
                    cursor: self.omnibox_text.chars().count(),
                };
            } else if let Some((cx, cy)) = click_pos {
                
                let cursor = self.layout_tree.as_ref().and_then(|lt| {
                    let omni_node = self
                        .document
                        .borrow()
                        .query_selector(self.document.borrow().root(), "#omnibox")?;
                    rix_layout::hit_test::hit_test_input_cursor(lt, omni_node, cx, cy)
                }).unwrap_or_else(|| self.omnibox_text.chars().count());
                self.omnibox_selection = TextSelection::collapsed(cursor);
            }
            self.synchronize_chrome_input_state(true);
            let _ = self.recompute_layout();
        }
    }

    pub(super) fn set_chrome_omnibox_value(&mut self, value: &str) {
        let document = self.document.clone();
        let node = {
            let doc = document.borrow();
            doc.query_selector(doc.root(), "#omnibox")
        };
        if let Some(node) = node {
            let changed = matches!(document.borrow().get(node), Some(NodeData::Element(element)) if element.get_attribute("value") != Some(value));
            if changed {
                if let Some(NodeData::Element(element)) = document.borrow_mut().get_mut(node) {
                    element.set_attribute("value", value);
                }
            }
            let mut state = self.dom_state.borrow_mut();
            if state.input_values.get(&node).map(String::as_str) != Some(value) {
                state.input_values.insert(node, value.to_string());
            }
        }
    }

    pub(super) fn chrome_omnibox_value(&self) -> Option<String> {
        let node = {
            let document = self.document.borrow();
            document.query_selector(document.root(), "#omnibox")
        }?;
        self.dom_state
            .borrow()
            .input_values
            .get(&node)
            .cloned()
            .or_else(|| {
                let document = self.document.borrow();
                match document.get(node) {
                    Some(NodeData::Element(element)) => {
                        element.get_attribute("value").map(str::to_string)
                    }
                    _ => None,
                }
            })
    }

    pub fn set_cookie_persistence_notice(&mut self, value: &str) {
        self.set_chrome_text_content("status-right", value);
    }

    pub(super) fn set_chrome_text_content(&mut self, element_id: &str, value: &str) {
        let document = self.document.clone();
        let node = {
            let doc = document.borrow();
            doc.query_selector(doc.root(), &format!("#{element_id}"))
        };
        if let Some(node) = node {
            let mut document = document.borrow_mut();
            let children: Vec<_> = document.children(node).collect();
            for child in children {
                if let Some(NodeData::Text(text)) = document.get_mut(child) {
                    *text = value.to_string();
                    return;
                }
            }
        }
    }

    pub(super) fn dispatch_chrome_input_event(&mut self) {
        let node = {
            let document = self.document.borrow();
            document.query_selector(document.root(), "#omnibox")
        };
        if let Some(node) = node {
            let _ = dispatch_dom_event_to_js(node, "input", None, self.runtime.context_mut());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_chrome_button_clicks() {
        let mut shell = BrowserShell::new_with_customization(
            1280.0,
            800.0,
            crate::customization::CustomizationManager::load_or_default(),
        ).unwrap();
        shell.recompute_layout().unwrap();

        let doc = shell.document.borrow();
        let theme_node = doc.query_selector(doc.root(), "#theme-toggle").unwrap();
        let new_tab_node = doc
            .query_selector(doc.root(), "#sidebar-add-tab-btn")
            .or_else(|| doc.query_selector(doc.root(), "#new-tab-btn"))
            .unwrap();
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let theme_box = lt.iter().find(|b| b.dom_node == Some(theme_node)).map(|b| b.dimensions.border_box());
        let new_tab_box = lt.iter().find(|b| b.dom_node == Some(new_tab_node)).map(|b| b.dimensions.border_box());
        let omnibox_node = {
            let doc = shell.document.borrow();
            doc.query_selector(doc.root(), "#omnibox").unwrap()
        };
        let omnibox_box = lt.iter().find(|b| b.dom_node == Some(omnibox_node)).map(|b| b.dimensions.border_box());

        let test_tab_id = shell.priv_state.borrow().active_tab_id;

        if let Some(r) = theme_box {
            let cx = r.x + r.width / 2.0;
            let cy = r.y + r.height / 2.0;
            assert_eq!(shell.cursor_at(cx, cy), "pointer");
        }
        if let Some(r) = new_tab_box {
            let cx = r.x + r.width / 2.0;
            let cy = r.y + r.height / 2.0;
            assert_eq!(shell.cursor_at(cx, cy), "pointer");
        }
        if let Some(r) = omnibox_box {
            let cx = r.x + r.width / 2.0;
            let cy = r.y + r.height / 2.0;
            assert_eq!(shell.cursor_at(cx, cy), "text");
        }

        if let Some(r) = theme_box {
            let cx = r.x + r.width / 2.0;
            let cy = r.y + r.height / 2.0;

            let (_scene_before, _) = shell.render_frame();

            let page_html = r#"<!DOCTYPE html><html><head><style>
                body { background-color: rgb(100, 100, 100); }
                @media (prefers-color-scheme: dark) { body { background-color: rgb(15, 15, 15); } }
                @media (prefers-color-scheme: light) { body { background-color: rgb(240, 240, 240); } }
            </style></head><body><h1>Content</h1></body></html>"#;
            {
                let mut host = shell.priv_state.borrow_mut();
                if let Some(tab) = host.tabs.iter_mut().find(|tab| tab.id == test_tab_id) {
                    tab.url = "https://example.com".to_string();
                }
            }
            let mut test_page = shell.build_page_from_html("https://example.com", page_html);
            test_page.layout_viewport = (1280.0, 700.0);
            test_page.layout_tree = crate::page_style::compute_page_layout(
                &test_page.document,
                &test_page.url,
                &test_page.external_stylesheets,
                1280.0,
                700.0,
                &shell.font_database,
            );
            shell.pages.insert(test_tab_id, test_page);

            let dark_bg = {
                let page = shell.pages.get(&test_tab_id).unwrap();
                let body = page.document.query_selector(page.document.root(), "body").unwrap();
                page.layout_tree.iter().find(|b| b.dom_node == Some(body)).unwrap().style.background_color
            };
            assert_eq!(dark_bg, rix_css::color::CssColor::from_rgba_u8(15, 15, 15, 255));

            shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();
            assert_eq!(shell.customization.theme_mode(), crate::customization::ThemeMode::Light);

            let light_bg = {
                let page = shell.pages.get(&test_tab_id).unwrap();
                let body = page.document.query_selector(page.document.root(), "body").unwrap();
                page.layout_tree.iter().find(|b| b.dom_node == Some(body)).unwrap().style.background_color
            };
            assert_eq!(light_bg, rix_css::color::CssColor::from_rgba_u8(240, 240, 240, 255));

            let (_scene_after, _) = shell.render_frame();
        }

        if let Some(r) = new_tab_box {
            let r = shell
                .layout_tree
                .as_ref()
                .and_then(|lt| lt.iter().find(|b| b.dom_node == Some(new_tab_node)).map(|b| b.dimensions.border_box()))
                .unwrap_or(r);
            let cx = r.x + r.width / 2.0;
            let cy = r.y + r.height / 2.0;
            let initial_tabs = shell.priv_state.borrow().tabs.len();
            shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();
            let new_tabs = shell.priv_state.borrow().tabs.len();
            assert!(new_tabs > initial_tabs);

            let (_scene_after_tab, _) = shell.render_frame();
            let doc = shell.document.borrow();
            let tabs_in_dom = doc.descendants(doc.root()).filter(|n| {
                matches!(doc.get(*n), Some(NodeData::Element(e)) if e.get_attribute("data-tab-id").is_some())
            }).count();
            assert_eq!(tabs_in_dom, 2);
        }

        if let Some(r) = theme_box {
            let cx = r.x + r.width / 2.0;
            let cy = r.y + r.height / 2.0;
            shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();
            assert_eq!(shell.customization.theme_mode(), crate::customization::ThemeMode::Dark);

            let restored_dark_bg = {
                let page = shell.pages.get(&test_tab_id).unwrap();
                let body = page.document.query_selector(page.document.root(), "body").unwrap();
                page.layout_tree.iter().find(|b| b.dom_node == Some(body)).unwrap().style.background_color
            };
            assert_eq!(restored_dark_bg, rix_css::color::CssColor::from_rgba_u8(15, 15, 15, 255));

            let (_scene, _) = shell.render_frame();
            let doc = shell.document.borrow();
            let html_node = doc.query_selector(doc.root(), "html").unwrap();
            if let Some(NodeData::Element(el)) = doc.get(html_node) {
                assert_eq!(el.get_attribute("data-theme"), Some("dark"));
            }
        }
    }

    #[tokio::test]
    async fn test_site_info_lock_popup() {
        let mut shell = BrowserShell::new_with_customization(
            1280.0,
            800.0,
            crate::customization::CustomizationManager::load_or_default(),
        ).unwrap();
        shell.recompute_layout().unwrap();

        let doc = shell.document.borrow();
        let lock_node = doc.query_selector(doc.root(), "#security-lock-btn").unwrap();
        let popup_node = doc.query_selector(doc.root(), "#site-info-popup").unwrap();
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let lock_box = lt.iter().find(|b| b.dom_node == Some(lock_node)).map(|b| b.dimensions.border_box()).unwrap();

        let cx = lock_box.x + lock_box.width / 2.0;
        let cy = lock_box.y + lock_box.height / 2.0;

        shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();

        let doc = shell.document.borrow();
        if let Some(NodeData::Element(el)) = doc.get(popup_node) {
            assert!(el.get_attribute("hidden").is_none(), "site-info-popup should not be hidden after clicking lock button");
        }
        let sugg_node = doc.query_selector(doc.root(), "#omnibox-suggestions").unwrap();
        if let Some(NodeData::Element(el)) = doc.get(sugg_node) {
            assert!(el.get_attribute("hidden").is_some(), "omnibox suggestions should remain hidden");
        }
    }

    #[tokio::test]
    async fn test_tab_context_menu_right_click() {
        let mut shell = BrowserShell::new_with_customization(
            1280.0,
            800.0,
            crate::customization::CustomizationManager::load_or_default(),
        ).unwrap();
        shell.recompute_layout().unwrap();

        let doc = shell.document.borrow();
        let tab_node = doc.descendants(doc.root()).find(|n| {
            matches!(doc.get(*n), Some(NodeData::Element(e)) if e.get_attribute("data-tab-id").is_some())
        }).expect("should find active tab node in sidebar");
        let ctx_menu_node = doc.query_selector(doc.root(), "#tab-context-menu").expect("context menu node exists");
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let tab_box = lt.iter().find(|b| b.dom_node == Some(tab_node)).map(|b| b.dimensions.border_box()).unwrap();
        let cx = tab_box.x + tab_box.width / 2.0;
        let cy = tab_box.y + tab_box.height / 2.0;

        shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 2 }).await.unwrap();

        let doc = shell.document.borrow();
        if let Some(NodeData::Element(el)) = doc.get(ctx_menu_node) {
            assert!(el.get_attribute("hidden").is_none(), "Context menu should be visible after physical right-click");
        }
    }

    #[tokio::test]
    async fn test_newtab_input_and_navigation() {
        let mut shell = BrowserShell::new_with_customization(
            1280.0,
            800.0,
            crate::customization::CustomizationManager::load_or_default(),
        ).unwrap();
        shell.recompute_layout().unwrap();

        let doc = shell.document.borrow();
        let input_by_search = doc.descendants(doc.root()).find(|n| {
            matches!(doc.get(*n), Some(NodeData::Element(e)) if e.get_attribute("id") == Some("newtab-input"))
        });
        let newtab_input = input_by_search.expect("newtab input exists");
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let input_box = lt.iter().find(|b| b.dom_node == Some(newtab_input)).map(|b| b.dimensions.border_box()).unwrap();
        let cx = input_box.x + input_box.width / 2.0;
        let cy = input_box.y + input_box.height / 2.0;

        shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();

        shell.handle_input(IpcInputEvent::KeyDown { key: "r".to_string(), code: "KeyR".to_string() }).await.unwrap();
        shell.handle_input(IpcInputEvent::KeyDown { key: "u".to_string(), code: "KeyU".to_string() }).await.unwrap();
        shell.handle_input(IpcInputEvent::KeyDown { key: "s".to_string(), code: "KeyS".to_string() }).await.unwrap();
        shell.handle_input(IpcInputEvent::KeyDown { key: "t".to_string(), code: "KeyT".to_string() }).await.unwrap();

        let doc = shell.document.borrow();
        if let Some(NodeData::Element(el)) = doc.get(newtab_input) {
            assert_eq!(el.get_attribute("value"), Some("rust"));
        }
        drop(doc);

        shell.handle_input(IpcInputEvent::KeyDown { key: "Enter".to_string(), code: "Enter".to_string() }).await.unwrap();
        let active_id = shell.priv_state.borrow().active_tab_id;
        let active_tab = shell.priv_state.borrow().tabs.iter().find(|t| t.id == active_id).cloned().unwrap();
        assert!(active_tab.url.contains("google.com/search?q=rust") || active_tab.url.contains("rust"));
    }

    #[tokio::test]
    async fn test_history_input_and_clear() {
        let mut shell = BrowserShell::new_with_customization(
            1280.0,
            800.0,
            crate::customization::CustomizationManager::load_or_default(),
        ).unwrap();

        shell.runtime.evaluate("navigateTo('rix://history');").unwrap();
        shell.recompute_layout().unwrap();

        let doc = shell.document.borrow();
        let hist_input = doc.query_selector(doc.root(), "#history-search-input").expect("history search input exists");
        let clear_btn = doc.query_selector(doc.root(), "#btn-clear-history").expect("clear history button exists");
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let input_box = lt.iter().find(|b| b.dom_node == Some(hist_input)).map(|b| b.dimensions.border_box()).unwrap();
        let cx = input_box.x + input_box.width / 2.0;
        let cy = input_box.y + input_box.height / 2.0;

        shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();

        shell.handle_input(IpcInputEvent::KeyDown { key: "g".to_string(), code: "KeyG".to_string() }).await.unwrap();
        shell.handle_input(IpcInputEvent::KeyDown { key: "i".to_string(), code: "KeyI".to_string() }).await.unwrap();
        shell.handle_input(IpcInputEvent::KeyDown { key: "t".to_string(), code: "KeyT".to_string() }).await.unwrap();

        let doc = shell.document.borrow();
        if let Some(NodeData::Element(el)) = doc.get(hist_input) {
            assert_eq!(el.get_attribute("value"), Some("git"));
        }
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let clear_box = lt.iter().find(|b| b.dom_node == Some(clear_btn)).map(|b| b.dimensions.border_box()).unwrap();
        let bx = clear_box.x + clear_box.width / 2.0;
        let by = clear_box.y + clear_box.height / 2.0;

        shell.handle_input(IpcInputEvent::MouseDown { x: bx, y: by, button: 0 }).await.unwrap();

        let doc = shell.document.borrow();
        let has_no_entries = doc.descendants(doc.root()).any(|n| {
            matches!(doc.get(n), Some(NodeData::Text(t)) if t.contains("No history entries found"))
        });
        assert!(has_no_entries, "History should be empty and display 'No history entries found'");
    }

    #[tokio::test]
    async fn test_settings_privacy_cookies() {
        let mut shell = BrowserShell::new_with_customization(
            1280.0,
            800.0,
            crate::customization::CustomizationManager::load_or_default(),
        ).unwrap();

        if let Some(ref storage) = shell.storage {
            let _ = storage.set_cookie(rix_storage::models::Cookie {
                name: "session".to_string(),
                value: "abc".to_string(),
                domain: "github.com".to_string(),
                path: "/".to_string(),
                expires_at: Some(u64::MAX / 2),
                secure: true,
                http_only: false,
                same_site: rix_storage::models::SameSitePolicy::Lax,
            });
        }

        shell.priv_state.borrow_mut().cookie_domains = vec![
            ("github.com".to_string(), 1),
        ];

        shell.runtime.evaluate("window.navigateTo('rix://settings');").unwrap();
        shell.recompute_layout().unwrap();

        let doc = shell.document.borrow();
        let clear_cookies_btn = doc.query_selector(doc.root(), "#btn-clear-all-cookies").expect("clear all cookies button exists");
        let has_github = doc.descendants(doc.root()).any(|n| {
            matches!(doc.get(n), Some(NodeData::Text(t)) if t.contains("github.com"))
        });
        assert!(has_github, "github.com should be displayed in settings cookies list");
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let clear_box = lt.iter().find(|b| b.dom_node == Some(clear_cookies_btn)).map(|b| b.dimensions.border_box()).unwrap();
        let cx = clear_box.x + clear_box.width / 2.0;
        let cy = clear_box.y + clear_box.height / 2.0;

        shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();

        if let Some(ref storage) = shell.storage {
            let cookies = storage.get_all_cookies().unwrap();
            assert!(cookies.is_empty(), "All cookies should be deleted from storage");
        }
    }

    #[tokio::test]
    async fn test_context_menu_close_others_and_copy_url() {
        let mut shell = BrowserShell::new_with_customization(
            1280.0,
            800.0,
            crate::customization::CustomizationManager::load_or_default(),
        ).unwrap();

        shell.runtime.evaluate("createTab('https://github.com', false); createTab('https://youtube.com', false);").unwrap();
        shell.recompute_layout().unwrap();

        let initial_tabs_count = shell.priv_state.borrow().tabs.len();
        assert_eq!(initial_tabs_count, 3, "Should have 3 open tabs");

        let doc = shell.document.borrow();
        let tab1 = doc.query_selector(doc.root(), "[data-tab-id='1']").expect("tab 1 exists");
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let tab_box = lt.iter().find(|b| b.dom_node == Some(tab1)).map(|b| b.dimensions.border_box()).unwrap();
        let tx = tab_box.x + tab_box.width / 2.0;
        let ty = tab_box.y + tab_box.height / 2.0;

        shell.handle_input(IpcInputEvent::MouseDown { x: tx, y: ty, button: 2 }).await.unwrap();
        shell.recompute_layout().unwrap();

        let doc = shell.document.borrow();
        let ctx_menu = doc.query_selector(doc.root(), "#tab-context-menu").expect("context menu exists");
        assert!(matches!(doc.get(ctx_menu), Some(NodeData::Element(el)) if el.get_attribute("hidden").is_none()));

        let copy_btn = doc.query_selector(doc.root(), "#ctx-copy-url").expect("copy url button exists");
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let copy_box = lt.iter().find(|b| b.dom_node == Some(copy_btn)).map(|b| b.dimensions.border_box()).unwrap();
        shell.handle_input(IpcInputEvent::MouseDown { x: copy_box.x + 5.0, y: copy_box.y + 5.0, button: 0 }).await.unwrap();

        assert_eq!(crate::text_editing::get_clipboard(), "about:blank");

        shell.handle_input(IpcInputEvent::MouseDown { x: tx, y: ty, button: 2 }).await.unwrap();
        shell.recompute_layout().unwrap();

        let doc = shell.document.borrow();
        let close_others_btn = doc.query_selector(doc.root(), "#ctx-close-others").expect("close others button exists");
        drop(doc);

        let lt = shell.layout_tree.as_ref().unwrap();
        let close_box = lt.iter().find(|b| b.dom_node == Some(close_others_btn)).map(|b| b.dimensions.border_box()).unwrap();
        shell.handle_input(IpcInputEvent::MouseDown { x: close_box.x + 5.0, y: close_box.y + 5.0, button: 0 }).await.unwrap();

        let remaining_tabs = shell.priv_state.borrow().tabs.len();
        assert_eq!(remaining_tabs, 1, "Only 1 tab should remain after Close Other Tabs");
    }

    #[tokio::test]
    async fn test_internal_page_scrolling() {
        let mut shell = BrowserShell::new_with_customization(
            1280.0,
            800.0,
            crate::customization::CustomizationManager::load_or_default(),
        ).unwrap();

        shell.runtime.evaluate("window.navigateTo('rix://settings');").unwrap();
        shell.recompute_layout().unwrap();

        assert_eq!(shell.internal_scroll_y, 0.0);

        shell.scroll_active_tab(120.0);
        let _ = shell.render_frame();

        assert_eq!(shell.internal_scroll_y, 120.0);
    }
}
