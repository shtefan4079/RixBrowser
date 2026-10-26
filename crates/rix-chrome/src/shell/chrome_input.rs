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
        let target = position.and_then(|(x, y)| {
            if self.web_surface.contains(x, y) {
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
        if self.web_surface.contains(x, y) {
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
        if self.web_surface.contains(x, y) {
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

            let chrome_hit = self.dispatch_chrome_click_at(x, y);
            if chrome_hit.is_some() {
                self.synchronize_privileged_host(previous_active_id).await;
            }

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
        if chrome_owns_focus && !self.omnibox_focused && matches!(key, "Enter" | " ") {
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
                "Enter" if chrome_allowed => {
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
                }
                "Escape" => {
                    self.omnibox_focused = false;
                    self.omnibox_selection =
                        TextSelection::collapsed(self.omnibox_text.chars().count());
                }
                _ => {
                    if ctrl_l {
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

    pub(super) fn dispatch_chrome_click_at(&mut self, x: f32, y: f32) -> Option<rix_dom::NodeId> {
        let target_node = if let Some(ref lt) = self.layout_tree {
            rix_layout::hit_test::hit_test_tree(lt, x, y)
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
            if let Err(e) = dispatch_dom_event_to_js(target, "click", None, self.runtime.context_mut()) {
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
                matches!(doc.get(node), Some(NodeData::Element(element)) if element.name.local.as_str() == "button" || element.get_attribute("tabindex") == Some("0"))
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

        println!("Web surface bounds: {:?}", shell.web_surface.bounds);

        let doc = shell.document.borrow();
        let theme_node = doc.query_selector(doc.root(), "#theme-toggle").unwrap();
        let new_tab_node = doc.query_selector(doc.root(), "#new-tab-btn").unwrap();
        drop(doc);

        println!("Theme node: {:?}", theme_node);
        println!("New tab node: {:?}", new_tab_node);

        let lt = shell.layout_tree.as_ref().unwrap();
        let theme_box = lt.iter().find(|b| b.dom_node == Some(theme_node)).map(|b| b.dimensions.border_box());
        let new_tab_box = lt.iter().find(|b| b.dom_node == Some(new_tab_node)).map(|b| b.dimensions.border_box());
        let omnibox_node = {
            let doc = shell.document.borrow();
            doc.query_selector(doc.root(), "#omnibox").unwrap()
        };
        let omnibox_box = lt.iter().find(|b| b.dom_node == Some(omnibox_node)).map(|b| b.dimensions.border_box());
        println!("Theme box: {:?}", theme_box);
        println!("New tab box: {:?}", new_tab_box);

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
            println!("Clicking theme at ({cx}, {cy})");

            let hit = shell.layout_tree.as_ref().and_then(|lt| rix_layout::hit_test::hit_test_tree(lt, cx, cy));
            println!("Hit node: {:?}", hit);
            if let Some(n) = hit {
                let doc = shell.document.borrow();
                println!("Hit node info: {:?}", doc.get(n));
                println!("Hit node parent: {:?}", doc.parent(n));
            }

            println!("Customization theme before: {:?}", shell.customization.theme_mode());
            let (_scene_before, _) = shell.render_frame();
            println!("Chrome layer commands before: {}", shell.compositor.layers().get(shell.chrome_layer_id).map(|l| l.display_list.items.len()).unwrap_or(0));

            shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();
            println!("Customization theme after: {:?}", shell.customization.theme_mode());
            assert_eq!(shell.customization.theme_mode(), crate::customization::ThemeMode::Light);

            let (_scene_after, _) = shell.render_frame();
            println!("Chrome layer commands after: {}", shell.compositor.layers().get(shell.chrome_layer_id).map(|l| l.display_list.items.len()).unwrap_or(0));

            if let Some(layer) = shell.compositor.layers().get(shell.chrome_layer_id) {
                for item in &layer.display_list.items {
                    if let rix_render::display_list::DrawCommand::DrawRect { rect, color, .. } = &item.command {
                        if rect.width >= 1200.0 {
                            println!("Chrome rect: {:?} color: {:?}", rect, color);
                        }
                    }
                }
            }
        }

        if let Some(r) = new_tab_box {
            let cx = r.x + r.width / 2.0;
            let cy = r.y + r.height / 2.0;
            println!("Clicking new tab at ({cx}, {cy})");
            let initial_tabs = shell.priv_state.borrow().tabs.len();
            shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();
            let new_tabs = shell.priv_state.borrow().tabs.len();
            println!("Tabs: {initial_tabs} -> {new_tabs}");
            assert!(new_tabs > initial_tabs);

            let (_scene_after_tab, _) = shell.render_frame();
            let doc = shell.document.borrow();
            let tabs_in_dom = doc.descendants(doc.root()).filter(|n| {
                matches!(doc.get(*n), Some(NodeData::Element(e)) if e.get_attribute("data-tab-id").is_some())
            }).count();
            println!("Tabs in DOM after render_frame: {tabs_in_dom}");
            assert_eq!(tabs_in_dom, 2);
        }

        if let Some(r) = theme_box {
            let cx = r.x + r.width / 2.0;
            let cy = r.y + r.height / 2.0;
            shell.handle_input(IpcInputEvent::MouseDown { x: cx, y: cy, button: 0 }).await.unwrap();
            assert_eq!(shell.customization.theme_mode(), crate::customization::ThemeMode::Dark);
            let (_scene, _) = shell.render_frame();
            let doc = shell.document.borrow();
            let html_node = doc.query_selector(doc.root(), "html").unwrap();
            if let Some(NodeData::Element(el)) = doc.get(html_node) {
                assert_eq!(el.get_attribute("data-theme"), Some("dark"));
            }
        }
    }
}
