use super::*;

impl BrowserShell {
    pub fn click_at(&mut self, x: f32, y: f32) -> Option<ClickAction> {
        let hit_chrome_overlay = self.hit_chrome_overlay(x, y);

        if !self.is_active_internal() && self.web_surface.contains(x, y) && hit_chrome_overlay.is_none() {
            return self
                .page_action_at(x, y)
                .or(Some(ClickAction::UnfocusOmnibox));
        }

        if hit_chrome_overlay.is_some() {
            return None;
        }

        let target = {
            let layout = self.layout_tree.as_ref()?;
            rix_layout::hit_test::hit_test_tree(layout, x, y)?
        };
        let doc = self.document.borrow();
        let mut current = Some(target);
        while let Some(node_id) = current {
            if let Some(NodeData::Element(element)) = doc.get(node_id) {
                match element.get_attribute("id") {
                    Some("omnibox") => return Some(ClickAction::FocusOmnibox),
                    Some("new-tab-btn") | Some("sidebar-add-tab-btn") => return Some(ClickAction::NewTab),
                    Some("theme-toggle") => return Some(ClickAction::ToggleTheme),
                    Some("tabs-orientation-toggle")
                    | Some("rix-toggle-sidebar")
                    | Some("toolbar-toggle-sidebar") => return Some(ClickAction::ToggleTabOrientation),
                    Some("nav-back") => return Some(ClickAction::NavBack),
                    Some("nav-forward") => return Some(ClickAction::NavForward),
                    Some("nav-reload") => return Some(ClickAction::NavReload),
                    Some("nav-home") | Some("brand-home-btn") => {
                        return Some(ClickAction::NavigateUrl(self.homepage.clone()))
                    }
                    _ => {}
                }
                if let Some(tab_id) = element
                    .get_attribute("data-tab-id")
                    .and_then(|id| id.parse::<u32>().ok())
                {
                    if element
                        .get_attribute("class")
                        .map(|class| class.contains("tab-close"))
                        .unwrap_or(false)
                    {
                        return Some(ClickAction::CloseTab(tab_id));
                    }
                    return Some(ClickAction::SwitchTab(tab_id));
                }
                if element
                    .get_attribute("class")
                    .map(|class| class.contains("tab-close"))
                    .unwrap_or(false)
                {
                    let mut ancestor = doc.parent(node_id);
                    while let Some(parent) = ancestor {
                        if let Some(NodeData::Element(tab)) = doc.get(parent) {
                            if let Some(tab_id) = tab
                                .get_attribute("data-tab-id")
                                .and_then(|id| id.parse::<u32>().ok())
                            {
                                return Some(ClickAction::CloseTab(tab_id));
                            }
                        }
                        ancestor = doc.parent(parent);
                    }
                }
            }
            current = doc.parent(node_id);
        }
        None
    }

    pub(super) fn page_action_at(&self, x: f32, y: f32) -> Option<ClickAction> {
        let active_id = self.active_tab_id();
        let page = self.pages.get(&active_id)?;
        let base_url = document_base_url(&page.document, &page.url);
        let node_id = rix_layout::hit_test::hit_test_tree_with_scroll(
            &page.layout_tree,
            Some(&page.document),
            x - self.web_surface.bounds.x,
            y - self.web_surface.bounds.y,
            page.scroll_x,
            page.scroll_y,
        )?;
        let mut current = Some(node_id);
        while let Some(node) = current {
            if let Some(NodeData::Element(element)) = page.document.get(node) {
                if &*element.name.local == "a" {
                    if let Some(href) = element.get_attribute("href") {
                        return resolve_page_link(&base_url, href).map(ClickAction::NavigateUrl);
                    }
                } else if &*element.name.local == "input" {
                    let input_type = element.get_attribute("type").unwrap_or("text");
                    if matches!(input_type, "submit" | "image") {
                        return self
                            .form_target_for_node(active_id, node, Some(node))
                            .map(ClickAction::NavigateUrl);
                    }
                    if !matches!(input_type, "button" | "reset" | "hidden") {
                        return Some(ClickAction::FocusInPageInput);
                    }
                } else if &*element.name.local == "textarea" || &*element.name.local == "select" {
                    return Some(ClickAction::FocusInPageInput);
                } else if &*element.name.local == "button"
                    && element.get_attribute("type").unwrap_or("submit") == "submit"
                {
                    return self
                        .form_target_for_node(active_id, node, Some(node))
                        .map(ClickAction::NavigateUrl);
                }
            }
            current = page.document.parent(node);
        }
        None
    }

    pub async fn execute_action(&mut self, action: ClickAction) {
        match action {
            ClickAction::SwitchTab(id) => self.switch_tab(id),
            ClickAction::CloseTab(id) => self.close_tab(id),
            ClickAction::NewTab => {
                let homepage = self.homepage.clone();
                let _ = self.start_new_tab(&homepage);
            }
            ClickAction::ToggleTheme => {
                let next_theme = match self.customization.theme_mode() {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                    ThemeMode::Custom => ThemeMode::Light,
                };
                self.customization.set_theme_mode(next_theme);
                let _ = self.apply_customization_to_chrome();
                self.needs_redraw = true;
            }
            ClickAction::ToggleTabOrientation => {
                let next_orientation = match self.customization.tab_orientation() {
                    TabOrientation::Horizontal => TabOrientation::Vertical,
                    TabOrientation::Vertical => TabOrientation::Horizontal,
                };
                self.customization.set_tab_orientation(next_orientation);
                let _ = self.apply_customization_to_chrome();
                self.needs_redraw = true;
            }
            ClickAction::NavBack => {
                let active_id = self.priv_state.borrow().active_tab_id;
                self.start_go_back(active_id);
            }
            ClickAction::NavForward => {
                let active_id = self.priv_state.borrow().active_tab_id;
                self.start_go_forward(active_id);
            }
            ClickAction::NavReload => {
                self.start_reload_active_tab();
            }
            ClickAction::FocusOmnibox => {
                self.omnibox_focused = true;
                self.omnibox_selection =
                    TextSelection::collapsed(self.omnibox_text.chars().count());
                self.synchronize_chrome_input_state(true);
                let _ = self.recompute_layout();
                self.needs_redraw = true;
            }
            ClickAction::UnfocusOmnibox => {
                self.omnibox_focused = false;
                self.synchronize_chrome_input_state(false);
                let _ = self.recompute_layout();
                self.needs_redraw = true;
            }
            ClickAction::NavigateUrl(url) => {
                let active_id = self.priv_state.borrow().active_tab_id;
                self.start_navigation(active_id, &url);
            }
            ClickAction::FocusInPageInput => {
                let active_id = self.priv_state.borrow().active_tab_id;
                if let Some(page) = self.pages.get_mut(&active_id) {
                    page.in_page_focused = true;
                }
                self.omnibox_focused = false;
            }
        }
    }

    pub fn active_tab_id(&self) -> u32 {
        self.priv_state.borrow().active_tab_id
    }

    pub fn is_active_internal(&self) -> bool {
        let active_id = self.active_tab_id();
        if let Some(tab) = self.priv_state.borrow().tabs.iter().find(|t| t.id == active_id) {
            let u = tab.url.trim();
            return u.is_empty()
                || u.starts_with("rix://")
                || u == "about:blank";
        }
        if let Some(page) = self.pages.get(&active_id) {
            let u = page.url.trim();
            return u.is_empty()
                || u.starts_with("rix://")
                || u == "about:blank";
        }
        true
    }

    pub fn refresh_in_page_display(&mut self, tab_id: u32) {
        if tab_id == self.active_tab_id() {
            self.refresh_active_page_render_data();
        }
    }

    pub async fn handle_control_message(&mut self, msg: crate::server::ControlMessage) {
        match msg {
            crate::server::ControlMessage::Status { resp } => {
                let active_id = self.priv_state.borrow().active_tab_id;
                let tabs: Vec<crate::server::TabInfo> = self
                    .priv_state
                    .borrow()
                    .tabs
                    .iter()
                    .map(|t| crate::server::TabInfo {
                        id: t.id,
                        url: t.url.clone(),
                        title: t.title.clone(),
                    })
                    .collect();

                let title = self
                    .pages
                    .get(&active_id)
                    .map(|p| p.title.clone())
                    .unwrap_or_default();
                let _ = resp.send(Ok(serde_json::json!({
                    "engine": "RixBrowser 0.1.0 (100% Pure Rust)",
                    "active_tab": active_id,
                    "title": title,
                    "url": self.omnibox_text,
                    "tabs": tabs,
                })));
            }
            crate::server::ControlMessage::Navigate { url, resp } => {
                let active_id = self.priv_state.borrow().active_tab_id;
                self.start_navigation(active_id, &url);
                let (title, is_loading) = if let Some(p) = self.pages.get(&active_id) {
                    (p.title.clone(), p.is_loading)
                } else {
                    (String::new(), false)
                };
                let _ = resp.send(Ok(serde_json::json!({
                    "ok": true,
                    "active_tab": active_id,
                    "url": self.omnibox_text,
                    "title": title,
                    "is_loading": is_loading,
                })));
            }
            crate::server::ControlMessage::Screenshot { path, resp } => {
                let target_path = if path.trim().is_empty() {
                    std::env::temp_dir()
                        .join("rixbrowser-screenshot.png")
                        .to_string_lossy()
                        .to_string()
                } else {
                    path
                };
                let w = self.width.max(1.0) as u32;
                let h = self.height.max(1.0) as u32;
                let (scene, _) = self.render_frame();
                match rix_render::VelloRenderer::render_scene_to_png(&scene, w, h, &target_path)
                    .await
                {
                    Ok(()) => {
                        let _ = resp.send(Ok(serde_json::json!({
                            "ok": true,
                            "path": target_path,
                            "width": self.width,
                            "height": self.height,
                        })));
                    }
                    Err(e) => {
                        let _ = resp.send(Err(format!("Screenshot error: {e}")));
                    }
                }
            }
            crate::server::ControlMessage::Click { x, y, button, resp } => {
                let action = if button == 0 { self.click_at(x, y) } else { None };
                let _ = self
                    .handle_input(IpcInputEvent::MouseDown { x, y, button })
                    .await;
                let _ = resp.send(Ok(serde_json::json!({
                    "ok": true,
                    "action": format!("{:?}", action),
                })));
            }
            crate::server::ControlMessage::Type { text, resp } => {
                let active_id = self.priv_state.borrow().active_tab_id;
                let input = KeyboardInput {
                    key: text,
                    ..Default::default()
                };
                if !self.omnibox_focused
                    && self
                        .pages
                        .get(&active_id)
                        .is_some_and(|page| page.focused_input.is_some())
                {
                    self.apply_page_key_default(&input);
                } else {
                    let result = edit_text(
                        &mut self.omnibox_text,
                        &mut self.omnibox_selection,
                        &input,
                        false,
                        false,
                    );
                    if result.changed {
                        self.set_chrome_omnibox_value(&self.omnibox_text.clone());
                        self.dispatch_chrome_input_event();
                    }
                    self.synchronize_chrome_input_state(true);
                }

                let in_page_val = self
                    .pages
                    .get(&active_id)
                    .map(|p| p.in_page_input.clone())
                    .unwrap_or_default();
                let _ = resp.send(Ok(serde_json::json!({
                    "ok": true,
                    "omnibox_text": self.omnibox_text,
                    "in_page_input": in_page_val,
                })));
            }
            crate::server::ControlMessage::Key {
                key,
                code,
                ctrl,
                alt,
                shift,
                meta,
                resp,
            } => {
                let active_id = self.priv_state.borrow().active_tab_id;
                self.handle_native_key(&key, &code, ctrl, alt, shift, meta)
                    .await;
                let in_page_val = self
                    .pages
                    .get(&active_id)
                    .map(|p| p.in_page_input.clone())
                    .unwrap_or_default();
                let _ = resp.send(Ok(serde_json::json!({
                    "ok": true,
                    "key": key,
                    "omnibox_text": self.omnibox_text,
                    "in_page_input": in_page_val,
                })));
            }
            crate::server::ControlMessage::GetDom { resp } => {
                let active_id = self.priv_state.borrow().active_tab_id;
                let mut dialog_boxes = Vec::new();
                let (title, is_loading, text_snippet, node_count, display_items) = if let Some(p) = self.pages.get(&active_id)
                {
                    for b in p.layout_tree.boxes() {
                        let elem_info = b.dom_node.and_then(|id| p.document.get(id)).and_then(|n| match n {
                            NodeData::Element(e) => Some((e.name.local.to_string(), e.get_attribute("id").unwrap_or("").to_string(), e.get_attribute("class").unwrap_or("").to_string())),
                            _ => None,
                        });
                        if let Some((tag, id, class)) = elem_info {
                            let parent_elem = b.dom_node
                                .and_then(|nid| p.document.parent(nid))
                                .and_then(|pid| p.document.get(pid))
                                .and_then(|n| match n {
                                    NodeData::Element(e) => Some(format!("{}:#{}", e.name.local, e.get_attribute("id").unwrap_or(""))),
                                    _ => None,
                                });
                            if tag.contains("dialog") || tag.contains("lightbox") || tag.contains("consent") || id.contains("content") || id.contains("dialog") || tag == "tp-yt-iron-overlay-backdrop" || tag.contains("masthead") || id.contains("masthead") || id == "end" || id == "center" || id == "start" || id == "buttons" || tag.contains("search") || tag.contains("topbar") || tag == "ytd-app" || (b.dimensions.border_box().y < 60.0 && b.dimensions.border_box().height > 0.0 && b.dimensions.border_box().width > 0.0) {
                                dialog_boxes.push(serde_json::json!({
                                    "tag": tag,
                                    "id": id,
                                    "class": class,
                                    "parent": parent_elem,
                                    "text": match &b.box_type {
                                        rix_layout::AnonymousBoxType::Text(t) => Some(t.clone()),
                                        _ => None,
                                    },
                                    "content_rect": [b.dimensions.content.x, b.dimensions.content.y, b.dimensions.content.width, b.dimensions.content.height],
                                    "border_rect": [b.dimensions.border_box().x, b.dimensions.border_box().y, b.dimensions.border_box().width, b.dimensions.border_box().height],
                                    "bg": format!("{:?}", b.style.background_color),
                                    "color": format!("{:?}", b.style.color),
                                    "display": format!("{:?}", b.style.display),
                                    "pos": b.style.position,
                                    "overflow_y": b.style.overflow_y,
                                    "opacity": b.style.opacity,
                                    "visibility": b.style.visibility,
                                    "z": b.style.z_index,
                                    "child_count": b.children.len(),
                                }));
                            }
                        }
                    }
                    let items_summary = p.display_list.items.iter().map(|item| {
                        let desc = match &item.command {
                            rix_render::display_list::DrawCommand::DrawRect { rect, color, .. } => {
                                format!("Rect [{}, {}, {}, {}] {:?}", rect.x, rect.y, rect.width, rect.height, color)
                            }
                            rix_render::display_list::DrawCommand::DrawText { text, x, y, color, font_size, .. } => {
                                format!("Text '{}' at ({}, {}) sz={} {:?}", text, x, y, font_size, color)
                            }
                            rix_render::display_list::DrawCommand::DrawGlyphs { glyphs, x, y } => {
                                format!("Glyphs count={} at ({}, {})", glyphs.len(), x, y)
                            }
                            rix_render::display_list::DrawCommand::DrawPath { d, x, y, fill, stroke, .. } => {
                                format!("Path len={} at ({}, {}) fill={:?} stroke={:?}", d.len(), x, y, fill, stroke)
                            }
                            rix_render::display_list::DrawCommand::DrawBorder { rect, color, width, .. } => {
                                format!("Border [{}, {}, {}, {}] w={} {:?}", rect.x, rect.y, rect.width, rect.height, width, color)
                            }
                            rix_render::display_list::DrawCommand::PushClip { rect } => {
                                format!("PushClip [{}, {}, {}, {}]", rect.x, rect.y, rect.width, rect.height)
                            }
                            rix_render::display_list::DrawCommand::PopClip => "PopClip".to_string(),
                            rix_render::display_list::DrawCommand::PushOpacity { opacity } => {
                                format!("PushOpacity {}", opacity)
                            }
                            rix_render::display_list::DrawCommand::PopOpacity => "PopOpacity".to_string(),
                            rix_render::display_list::DrawCommand::DrawImage { rect, src, .. } => {
                                format!("Image [{}, {}, {}, {}] src={}", rect.x, rect.y, rect.width, rect.height, src)
                            }
                            rix_render::display_list::DrawCommand::DrawImageResource { rect, .. } => {
                                format!("ImageRes [{}, {}, {}, {}]", rect.x, rect.y, rect.width, rect.height)
                            }
                            rix_render::display_list::DrawCommand::DrawShadow { rect, .. } => {
                                format!("Shadow [{}, {}, {}, {}]", rect.x, rect.y, rect.width, rect.height)
                            }
                            rix_render::display_list::DrawCommand::DrawLinearGradient { rect, .. } => {
                                format!("Gradient [{}, {}, {}, {}]", rect.x, rect.y, rect.width, rect.height)
                            }
                        };
                        serde_json::json!({
                            "z": item.z_index,
                            "a": item.viewport_anchored,
                            "c": desc,
                        })
                    }).collect::<Vec<_>>();
                    (p.title.clone(), p.is_loading, String::new(), p.node_count, items_summary)
                } else {
                    (String::new(), false, String::new(), 0, Vec::new())
                };
                let _ = resp.send(Ok(serde_json::json!({
                    "active_tab": active_id,
                    "url": self.omnibox_text,
                    "title": title,
                    "is_loading": is_loading,
                    "text_snippet": text_snippet,
                    "node_count": node_count,
                    "dialog_boxes": dialog_boxes,
                    "display_items": display_items,
                })));
            }
            crate::server::ControlMessage::EvalJs { script, resp } => {
                match self.runtime.evaluate(&script) {
                    Ok(val) => {
                        let _ = resp.send(Ok(serde_json::json!({
                            "ok": true,
                            "result": val.display().to_string(),
                        })));
                    }
                    Err(e) => {
                        let _ = resp.send(Err(format!("JS error: {e}")));
                    }
                }
            }
        }
    }
}
