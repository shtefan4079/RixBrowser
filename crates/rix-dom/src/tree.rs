
use crate::node::{CanvasBitmap, ElementData, NodeData, VideoFrameBitmap};
use indextree::{Arena, NodeEdge, NodeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct Document {
    arena: Arena<NodeData>,
    root: NodeId,
    revision: u64,
    style_revision: u64,
    script_revision: u64,
    hovered: Option<NodeId>,
    focused: Option<NodeId>,
    active: Option<NodeId>,
    focus_visible: bool,
    text_selection: Option<(NodeId, usize, usize)>,
    dynamic_stylesheets: HashMap<NodeId, String>,
    adopted_stylesheets: HashMap<NodeId, Vec<String>>,
    id_cache: Arc<RwLock<Option<(u64, HashMap<String, NodeId>)>>>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        let mut arena = Arena::new();
        let root = arena.new_node(NodeData::Document);
        Self {
            arena,
            root,
            revision: 0,
            style_revision: 0,
            script_revision: 0,
            hovered: None,
            focused: None,
            active: None,
            focus_visible: false,
            text_selection: None,
            dynamic_stylesheets: HashMap::new(),
            adopted_stylesheets: HashMap::new(),
            id_cache: Arc::new(RwLock::new(None)),
        }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn get_element_by_id(&self, target_id: &str) -> Option<NodeId> {
        if target_id.is_empty() {
            return None;
        }
        if let Ok(guard) = self.id_cache.read() {
            if let Some((rev, map)) = guard.as_ref() {
                if *rev == self.revision {
                    return map.get(target_id).copied();
                }
            }
        }
        let mut map = HashMap::new();
        for node in self.descendants_with_shadow_roots(self.root) {
            if let Some(NodeData::Element(element)) = self.get(node) {
                if let Some(id) = element.get_attribute("id") {
                    map.entry(id.to_string()).or_insert(node);
                }
            }
        }
        let result = map.get(target_id).copied();
        if let Ok(mut writer) = self.id_cache.write() {
            *writer = Some((self.revision, map));
        }
        result
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn style_revision(&self) -> u64 {
        self.style_revision
    }

    pub fn script_revision(&self) -> u64 {
        self.script_revision
    }

    pub fn bump_script_revision(&mut self) {
        self.script_revision = self.script_revision.wrapping_add(1);
    }

    pub fn dynamic_stylesheet(&self, node: NodeId) -> Option<&str> {
        self.dynamic_stylesheets.get(&node).map(|s| s.as_str())
    }

    pub fn set_dynamic_stylesheet(&mut self, node: NodeId, css: String) {
        self.dynamic_stylesheets.insert(node, css);
        self.bump_style_revision();
    }

    pub fn remove_dynamic_stylesheet(&mut self, node: NodeId) {
        if self.dynamic_stylesheets.remove(&node).is_some() {
            self.bump_style_revision();
        }
    }

    pub fn all_dynamic_stylesheets(&self) -> &HashMap<NodeId, String> {
        &self.dynamic_stylesheets
    }

    pub fn adopted_stylesheets(&self, node: NodeId) -> &[String] {
        self.adopted_stylesheets
            .get(&node)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn set_adopted_stylesheets(&mut self, node: NodeId, sheets: Vec<String>) {
        self.adopted_stylesheets.insert(node, sheets);
        self.bump_style_revision();
    }

    pub fn all_adopted_stylesheets(&self) -> &HashMap<NodeId, Vec<String>> {
        &self.adopted_stylesheets
    }

    pub fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn bump_style_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.style_revision = self.style_revision.wrapping_add(1);
    }

    pub fn set_hovered(&mut self, node: Option<NodeId>) -> bool {
        if self.hovered == node {
            return false;
        }
        self.hovered = node;
        self.revision = self.revision.wrapping_add(1);
        self.style_revision = self.style_revision.wrapping_add(1);
        true
    }

    pub fn set_active(&mut self, node: Option<NodeId>) -> bool {
        if self.active == node {
            return false;
        }
        self.active = node;
        self.revision = self.revision.wrapping_add(1);
        self.style_revision = self.style_revision.wrapping_add(1);
        true
    }

    pub fn set_focused(&mut self, node: Option<NodeId>, visible: bool) -> bool {
        if self.focused == node && self.focus_visible == visible {
            return false;
        }
        self.focused = node;
        self.focus_visible = visible;
        self.revision = self.revision.wrapping_add(1);
        self.style_revision = self.style_revision.wrapping_add(1);
        true
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.focused
    }

    pub fn text_selection(&self) -> Option<(NodeId, usize, usize)> {
        self.text_selection
    }

    pub fn set_text_selection(&mut self, selection: Option<(NodeId, usize, usize)>) {
        if self.text_selection != selection {
            self.text_selection = selection;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn is_hovered(&self, node: NodeId) -> bool {
        self.state_contains(self.hovered, node)
    }

    pub fn is_active(&self, node: NodeId) -> bool {
        self.state_contains(self.active, node)
    }

    pub fn is_focused(&self, node: NodeId) -> bool {
        self.focused == Some(node)
    }

    pub fn is_focus_visible(&self, node: NodeId) -> bool {
        self.is_focused(node) && self.focus_visible
    }

    pub fn is_focus_within(&self, node: NodeId) -> bool {
        self.state_contains(self.focused, node)
    }

    fn state_contains(&self, state: Option<NodeId>, node: NodeId) -> bool {
        state.is_some_and(|target| {
            target == node || self.ancestors(target).any(|ancestor| ancestor == node)
        })
    }

    pub fn create_element(&mut self, local_name: &str) -> NodeId {
        let data = NodeData::Element(ElementData::new_html(local_name));
        self.arena.new_node(data)
    }

    pub fn create_element_ns(
        &mut self,
        namespace: &str,
        prefix: Option<&str>,
        local_name: &str,
    ) -> NodeId {
        self.new_node(NodeData::Element(crate::node::ElementData::new(
            markup5ever::QualName::new(
                prefix.map(markup5ever::Prefix::from),
                markup5ever::Namespace::from(namespace),
                markup5ever::LocalName::from(local_name),
            ),
        )))
    }

    pub fn create_text(&mut self, text: &str) -> NodeId {
        let data = NodeData::Text(text.to_string());
        self.arena.new_node(data)
    }

    pub fn create_comment(&mut self, comment: &str) -> NodeId {
        let data = NodeData::Comment(comment.to_string());
        self.arena.new_node(data)
    }

    pub fn new_node(&mut self, data: NodeData) -> NodeId {
        self.arena.new_node(data)
    }

    pub fn is_connected(&self, node: NodeId) -> bool {
        if node == self.root {
            return true;
        }
        let mut current = node;
        for _ in 0..256 {
            if current == self.root || matches!(self.get(current), Some(NodeData::Document)) {
                return true;
            }
            if let Some(parent) = self.parent(current) {
                current = parent;
                continue;
            }
            match self.get(current) {
                Some(NodeData::ShadowRoot { host, .. }) => current = *host,
                _ => return false,
            }
        }
        false
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        parent.append(child, &mut self.arena);
        if self.is_connected(parent) {
            self.revision = self.revision.wrapping_add(1);
            self.style_revision = self.style_revision.wrapping_add(1);
            if matches!(self.get(child), Some(NodeData::Element(e)) if e.name.local.as_str() == "script") {
                self.script_revision = self.script_revision.wrapping_add(1);
            }
        }
    }

    pub fn insert_before(&mut self, target: NodeId, new_child: NodeId) {
        target.insert_before(new_child, &mut self.arena);
        if self.is_connected(target) {
            self.revision = self.revision.wrapping_add(1);
            self.style_revision = self.style_revision.wrapping_add(1);
            if matches!(self.get(new_child), Some(NodeData::Element(e)) if e.name.local.as_str() == "script") {
                self.script_revision = self.script_revision.wrapping_add(1);
            }
        }
    }

    pub fn remove_child(&mut self, child: NodeId) {
        let is_script = matches!(self.get(child), Some(NodeData::Element(e)) if e.name.local.as_str() == "script");
        let connected = self.is_connected(child);
        if self.state_contains(self.hovered, child) {
            self.hovered = None;
        }
        if self.state_contains(self.focused, child) {
            self.focused = None;
            self.focus_visible = false;
        }
        if self.state_contains(self.active, child) {
            self.active = None;
        }
        if self.state_contains(self.text_selection.map(|selection| selection.0), child) {
            self.text_selection = None;
        }
        child.detach(&mut self.arena);
        if connected {
            self.revision = self.revision.wrapping_add(1);
            self.style_revision = self.style_revision.wrapping_add(1);
            if is_script {
                self.script_revision = self.script_revision.wrapping_add(1);
            }
        }
    }

    pub fn replace_child(&mut self, old_child: NodeId, new_child: NodeId) {
        self.insert_before(old_child, new_child);
        self.remove_child(old_child);
    }

    pub fn get(&self, id: NodeId) -> Option<&NodeData> {
        self.arena.get(id).map(|n| n.get())
    }

    pub fn get_mut_untracked(&mut self, id: NodeId) -> Option<&mut NodeData> {
        self.arena.get_mut(id).map(|n| n.get_mut())
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut NodeData> {
        let connected = self.is_connected(id);
        let node = self.arena.get_mut(id);
        if node.is_some() && connected {
            self.revision = self.revision.wrapping_add(1);
            self.style_revision = self.style_revision.wrapping_add(1);
        }
        node.map(|n| n.get_mut())
    }

    pub fn canvas_bitmap(&self, id: NodeId) -> Option<&CanvasBitmap> {
        match self.get(id) {
            Some(NodeData::Element(element)) => element.canvas_bitmap.as_ref(),
            _ => None,
        }
    }

    pub fn video_frame(&self, id: NodeId) -> Option<&VideoFrameBitmap> {
        match self.get(id) {
            Some(NodeData::Element(element)) => element.video_frame.as_ref(),
            _ => None,
        }
    }

    pub fn content_document(&self, id: NodeId) -> Option<NodeId> {
        match self.get(id) {
            Some(NodeData::Element(element)) => element.content_document,
            _ => None,
        }
    }

    pub fn for_each_composed_child<F>(&self, parent: NodeId, mut f: F)
    where
        F: FnMut(NodeId),
    {
        let target_root = match self.get(parent) {
            Some(NodeData::Element(element)) => element.shadow_root.unwrap_or(parent),
            _ => parent,
        };
        for child in self.children(target_root) {
            if self.is_slot_in_shadow_tree(child) {
                let assigned = self.assigned_nodes_for_slot(child);
                if assigned.is_empty() {
                    for fallback in self.children(child) {
                        f(fallback);
                    }
                } else {
                    for assigned_node in assigned {
                        f(assigned_node);
                    }
                }
            } else {
                f(child);
            }
        }
    }

    pub fn composed_children(&self, parent: NodeId) -> Vec<NodeId> {
        let mut composed = Vec::new();
        self.for_each_composed_child(parent, |child| composed.push(child));
        composed
    }

    fn is_slot_in_shadow_tree(&self, node: NodeId) -> bool {
        matches!(self.get(node), Some(NodeData::Element(element)) if &*element.name.local == "slot")
            && self.shadow_root_containing(node).is_some()
    }

    pub fn shadow_root_containing(&self, node: NodeId) -> Option<NodeId> {
        std::iter::once(node)
            .chain(self.ancestors(node))
            .find(|ancestor| matches!(self.get(*ancestor), Some(NodeData::ShadowRoot { .. })))
    }

    pub fn assigned_slot_for(&self, node: NodeId) -> Option<NodeId> {
        let host = self.parent(node)?;
        let root = match self.get(host) {
            Some(NodeData::Element(host_element)) => host_element.shadow_root?,
            _ => return None,
        };
        std::iter::once(root)
            .chain(self.descendants(root))
            .find(|slot| {
                matches!(self.get(*slot), Some(NodeData::Element(element)) if &*element.name.local == "slot")
                    && self.assigned_nodes_for_slot(*slot).contains(&node)
            })
    }

    fn assigned_nodes_for_slot(&self, slot: NodeId) -> Vec<NodeId> {
        let Some(root) = self.shadow_root_containing(slot) else {
            return Vec::new();
        };
        let Some(NodeData::ShadowRoot { host, .. }) = self.get(root) else {
            return Vec::new();
        };
        let Some(NodeData::Element(slot_element)) = self.get(slot) else {
            return Vec::new();
        };
        let slot_name = slot_element.get_attribute("name").unwrap_or("");
        let is_first_slot = std::iter::once(root)
            .chain(self.descendants(root))
            .filter(|node| matches!(self.get(*node), Some(NodeData::Element(element)) if &*element.name.local == "slot"))
            .find(|node| matches!(self.get(*node), Some(NodeData::Element(element)) if element.get_attribute("name").unwrap_or("") == slot_name))
            == Some(slot);
        if !is_first_slot {
            return Vec::new();
        }
        self.children(*host)
            .filter(|child| match self.get(*child) {
                Some(NodeData::Element(element)) => {
                    element.get_attribute("slot").unwrap_or("") == slot_name
                }
                Some(NodeData::Text(_)) => slot_name.is_empty(),
                _ => false,
            })
            .collect()
    }

    pub fn set_canvas_bitmap(&mut self, id: NodeId, bitmap: Option<CanvasBitmap>) -> bool {
        let changed = {
            let Some(node) = self.arena.get_mut(id) else {
                return false;
            };
            let NodeData::Element(element) = node.get_mut() else {
                return false;
            };
            if element.canvas_bitmap == bitmap {
                false
            } else {
                element.canvas_bitmap = bitmap;
                true
            }
        };
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    pub fn mutate_canvas_bitmap(
        &mut self,
        id: NodeId,
        mutate: impl FnOnce(&mut CanvasBitmap) -> bool,
    ) -> bool {
        let changed = {
            let Some(node) = self.arena.get_mut(id) else {
                return false;
            };
            let NodeData::Element(element) = node.get_mut() else {
                return false;
            };
            let Some(bitmap) = element.canvas_bitmap.as_mut() else {
                return false;
            };
            mutate(bitmap)
        };
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    pub fn set_video_frame(&mut self, id: NodeId, frame: Option<VideoFrameBitmap>) -> bool {
        let changed = {
            let Some(node) = self.arena.get_mut(id) else {
                return false;
            };
            let NodeData::Element(element) = node.get_mut() else {
                return false;
            };
            if element.video_frame == frame {
                false
            } else {
                element.video_frame = frame;
                true
            }
        };
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    pub fn set_content_document(&mut self, id: NodeId, content_doc: Option<NodeId>) -> bool {
        let changed = {
            let Some(node) = self.arena.get_mut(id) else {
                return false;
            };
            let NodeData::Element(element) = node.get_mut() else {
                return false;
            };
            if element.content_document == content_doc {
                false
            } else {
                element.content_document = content_doc;
                true
            }
        };
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    pub fn clone_subtree_from(&mut self, source_document: &Document, source: NodeId) -> NodeId {
        fn shallow(
            source_document: &Document,
            source: NodeId,
            destination: &mut Document,
        ) -> NodeId {
            let mut data = source_document
                .get(source)
                .cloned()
                .unwrap_or(NodeData::Text(String::new()));
            if let NodeData::Element(element) = &mut data {
                element.template_contents = element
                    .template_contents
                    .map(|_| destination.new_node(NodeData::DocumentFragment));
                element.shadow_root = None;
                element.canvas_bitmap = None;
                element.video_frame = None;
                element.content_document = None;
            }
            destination.new_node(data)
        }
        let copy = shallow(source_document, source, self);
        let mut pending = vec![(source, copy)];
        while let Some((source, parent)) = pending.pop() {
            let (source_fragment, copy_fragment, shadow_root_info, source_content_doc) = {
                let source_elem = match source_document.get(source) {
                    Some(NodeData::Element(e)) => {
                        Some((e.template_contents, e.shadow_root, e.content_document))
                    }
                    _ => None,
                };
                let copy_fragment = match self.get(parent) {
                    Some(NodeData::Element(e)) => e.template_contents,
                    _ => None,
                };
                if let Some((src_frag, src_sr, src_cd)) = source_elem {
                    let sr_info = src_sr.and_then(|sr| {
                        if let Some(NodeData::ShadowRoot {
                            mode,
                            delegates_focus,
                            clonable,
                            ..
                        }) = source_document.get(sr)
                        {
                            Some((sr, mode.clone(), *delegates_focus, *clonable))
                        } else {
                            None
                        }
                    });
                    (src_frag, copy_fragment, sr_info, src_cd)
                } else {
                    (None, None, None, None)
                }
            };
            if let (Some(sf), Some(cf)) = (source_fragment, copy_fragment) {
                pending.push((sf, cf));
            }
            if let Some((src_root, mode, delegates_focus, clonable)) = shadow_root_info {
                let copy_root = self.new_node(NodeData::ShadowRoot {
                    host: parent,
                    mode,
                    delegates_focus,
                    clonable,
                });
                if let Some(NodeData::Element(copy_elem)) = self.get_mut(parent) {
                    copy_elem.shadow_root = Some(copy_root);
                }
                pending.push((src_root, copy_root));
            }
            if let Some(src_cd) = source_content_doc {
                let copy_cd = self.new_node(NodeData::Document);
                if let Some(NodeData::Element(copy_elem)) = self.get_mut(parent) {
                    copy_elem.content_document = Some(copy_cd);
                }
                pending.push((src_cd, copy_cd));
            }
            for child in source_document.children(source) {
                let copy = shallow(source_document, child, self);
                self.append_child(parent, copy);
                pending.push((child, copy));
            }
        }
        copy
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(id).and_then(|n| n.parent())
    }

    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(id).and_then(|n| n.first_child())
    }

    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(id).and_then(|n| n.last_child())
    }

    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(id).and_then(|n| n.previous_sibling())
    }

    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(id).and_then(|n| n.next_sibling())
    }

    pub fn previous_element_sibling(&self, mut id: NodeId) -> Option<NodeId> {
        while let Some(prev) = self.previous_sibling(id) {
            if matches!(self.get(prev), Some(NodeData::Element(_))) {
                return Some(prev);
            }
            id = prev;
        }
        None
    }

    pub fn next_element_sibling(&self, mut id: NodeId) -> Option<NodeId> {
        while let Some(next) = self.next_sibling(id) {
            if matches!(self.get(next), Some(NodeData::Element(_))) {
                return Some(next);
            }
            id = next;
        }
        None
    }

    pub fn first_element_child(&self, id: NodeId) -> Option<NodeId> {
        let mut child = self.first_child(id);
        while let Some(c) = child {
            if matches!(self.get(c), Some(NodeData::Element(_))) {
                return Some(c);
            }
            child = self.next_sibling(c);
        }
        None
    }

    pub fn last_element_child(&self, id: NodeId) -> Option<NodeId> {
        let mut child = self.last_child(id);
        while let Some(c) = child {
            if matches!(self.get(c), Some(NodeData::Element(_))) {
                return Some(c);
            }
            child = self.previous_sibling(c);
        }
        None
    }

    pub fn child_element_count(&self, id: NodeId) -> usize {
        let mut count = 0;
        let mut child = self.first_child(id);
        while let Some(c) = child {
            if matches!(self.get(c), Some(NodeData::Element(_))) {
                count += 1;
            }
            child = self.next_sibling(c);
        }
        count
    }

    pub fn children(&self, id: NodeId) -> impl DoubleEndedIterator<Item = NodeId> + '_ {
        id.children(&self.arena)
    }

    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        id.descendants(&self.arena)
    }

    pub fn descendants_with_shadow_roots(&self, id: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        let mut stack = vec![id];
        let mut visited = std::collections::HashSet::new();

        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if current != id {
                result.push(current);
            }
            if let Some(NodeData::Element(element)) = self.get(current) {
                if let Some(shadow_root) = element.shadow_root {
                    stack.push(shadow_root);
                }
            }
            stack.extend(self.children(current).rev());
        }
        result
    }

    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        id.ancestors(&self.arena)
    }

    pub fn inner_text(&self, id: NodeId) -> String {
        let mut result = String::new();
        for desc in self.descendants(id) {
            if let Some(NodeData::Text(t)) = self.get(desc) {
                result.push_str(t);
            }
        }
        result
    }

    pub fn document_title_element(&self, document: NodeId) -> Option<NodeId> {
        let document_element = self.children(document).find(|child| {
            matches!(self.get(*child), Some(NodeData::Element(_)))
        });
        if let Some(document_element) = document_element {
            if let Some(NodeData::Element(root)) = self.get(document_element) {
                if &*root.name.ns == crate::svg::SVG_NAMESPACE && &*root.name.local == "svg" {
                    return self.children(document_element).find(|child| {
                        matches!(self.get(*child), Some(NodeData::Element(element))
                            if &*element.name.ns == crate::svg::SVG_NAMESPACE
                                && &*element.name.local == "title")
                    });
                }
            }
        }
        self.descendants(document).find(|node| {
            matches!(self.get(*node), Some(NodeData::Element(element))
                if &*element.name.ns == "http://www.w3.org/1999/xhtml"
                    && &*element.name.local == "title")
        })
    }

    pub fn document_title(&self, document: NodeId) -> String {
        self.document_title_element(document)
            .map_or_else(String::new, |title| self.inner_text(title))
    }

    pub fn to_html_string(&self, node: NodeId) -> String {
        let mut output = String::new();
        let mut edges = vec![NodeEdge::Start(node)];
        while let Some(edge) = edges.pop() {
            match edge {
                NodeEdge::Start(node) => {
                    let mut child_container = node;
                    match self.get(node) {
                        Some(NodeData::Element(element)) => {
                            output.push('<');
                            serialize_tag_name(&element.name, &mut output);
                            for (name, value) in &element.attributes {
                                output.push(' ');
                                let prefix = match &*name.ns {
                                    "http://www.w3.org/XML/1998/namespace" => Some("xml"),
                                    "http://www.w3.org/2000/xmlns/" if &*name.local != "xmlns" => {
                                        Some("xmlns")
                                    }
                                    "http://www.w3.org/1999/xlink" => Some("xlink"),
                                    _ => name.prefix.as_ref().map(|prefix| &**prefix),
                                };
                                if let Some(prefix) = prefix {
                                    output.push_str(prefix);
                                    output.push(':');
                                }
                                output.push_str(&name.local);
                                output.push_str("=\"");
                                serialize_escaped(value, true, &mut output);
                                output.push('"');
                            }
                            output.push('>');
                            if &*element.name.ns == "http://www.w3.org/1999/xhtml"
                                && matches!(
                                    &*element.name.local,
                                    "area"
                                        | "base"
                                        | "br"
                                        | "col"
                                        | "embed"
                                        | "hr"
                                        | "img"
                                        | "input"
                                        | "link"
                                        | "meta"
                                        | "source"
                                        | "track"
                                        | "wbr"
                                        | "basefont"
                                        | "bgsound"
                                        | "frame"
                                        | "keygen"
                                        | "param"
                                )
                            {
                                continue;
                            }
                            edges.push(NodeEdge::End(node));
                            if let Some(fragment) = element.template_contents {
                                child_container = fragment;
                            }
                        }
                        Some(NodeData::Text(text)) => {
                            let raw = self.parent(node).and_then(|parent| self.get(parent)).is_some_and(|parent|
                            matches!(parent, NodeData::Element(element) if &*element.name.ns == "http://www.w3.org/1999/xhtml" && matches!(&*element.name.local,
                                "style" | "script" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext" | "noscript")));
                            if raw {
                                output.push_str(text);
                            } else {
                                serialize_escaped(text, false, &mut output);
                            }
                        }
                        Some(NodeData::Comment(comment)) => {
                            output.push_str("<!--");
                            output.push_str(comment);
                            output.push_str("-->");
                        }
                        Some(NodeData::DocumentType { name, .. }) => {
                            output.push_str("<!DOCTYPE ");
                            output.push_str(name);
                            output.push('>');
                        }
                        Some(
                            NodeData::Document
                            | NodeData::DocumentFragment
                            | NodeData::ShadowRoot { .. },
                        )
                        | None => {}
                    }
                    let children: Vec<_> = self.children(child_container).collect();
                    edges.extend(children.into_iter().rev().map(NodeEdge::Start));
                }
                NodeEdge::End(node) => {
                    if let Some(NodeData::Element(element)) = self.get(node) {
                        output.push_str("</");
                        serialize_tag_name(&element.name, &mut output);
                        output.push('>');
                    }
                }
            }
        }
        output
    }
}

fn serialize_escaped(value: &str, attribute: bool, output: &mut String) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '\u{00a0}' => output.push_str("&nbsp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' if attribute => output.push_str("&quot;"),
            character => output.push(character),
        }
    }
}

fn serialize_tag_name(name: &markup5ever::QualName, output: &mut String) {
    if !matches!(
        &*name.ns,
        "http://www.w3.org/1999/xhtml"
            | "http://www.w3.org/2000/svg"
            | "http://www.w3.org/1998/Math/MathML"
    ) {
        if let Some(prefix) = &name.prefix {
            output.push_str(prefix);
            output.push(':');
        }
    }
    output.push_str(&name.local);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::ElementData;

    #[test]
    fn test_is_connected_and_revision_stability() {
        let mut doc = Document::new();
        assert!(doc.is_connected(doc.root()));
        assert_eq!(doc.revision(), 0);
        assert_eq!(doc.style_revision(), 0);

        let frag = doc.new_node(NodeData::DocumentFragment);
        let div = doc.new_node(NodeData::Element(ElementData::new(markup5ever::QualName::new(
            None,
            "http://www.w3.org/1999/xhtml".into(),
            "div".into(),
        ))));
        let span = doc.new_node(NodeData::Element(ElementData::new(markup5ever::QualName::new(
            None,
            "http://www.w3.org/1999/xhtml".into(),
            "span".into(),
        ))));

        assert!(!doc.is_connected(frag));
        assert!(!doc.is_connected(div));
        assert!(!doc.is_connected(span));

        doc.append_child(div, span);
        doc.append_child(frag, div);
        assert_eq!(doc.revision(), 0);
        assert_eq!(doc.style_revision(), 0);
        assert!(!doc.is_connected(div));
        assert!(!doc.is_connected(span));

        if let Some(NodeData::Element(elem)) = doc.get_mut(span) {
            elem.set_attribute("class", "active");
        }
        assert_eq!(doc.revision(), 0);
        assert_eq!(doc.style_revision(), 0);

        doc.append_child(doc.root(), div);
        assert_eq!(doc.revision(), 1);
        assert_eq!(doc.style_revision(), 1);
        assert!(doc.is_connected(div));
        assert!(doc.is_connected(span));

        if let Some(NodeData::Element(elem)) = doc.get_mut(span) {
            elem.set_attribute("id", "main");
        }
        assert_eq!(doc.revision(), 2);
        assert_eq!(doc.style_revision(), 2);

        let host = doc.new_node(NodeData::Element(ElementData::new(markup5ever::QualName::new(
            None,
            "http://www.w3.org/1999/xhtml".into(),
            "custom-elem".into(),
        ))));
        let shadow = doc.new_node(NodeData::ShadowRoot {
            host,
            mode: "open".into(),
            delegates_focus: false,
            clonable: true,
        });
        let inner = doc.new_node(NodeData::Element(ElementData::new(markup5ever::QualName::new(
            None,
            "http://www.w3.org/1999/xhtml".into(),
            "p".into(),
        ))));
        doc.append_child(shadow, inner);
        assert!(!doc.is_connected(shadow));
        assert!(!doc.is_connected(inner));

        doc.append_child(div, host);
        assert!(doc.is_connected(host));
        assert!(doc.is_connected(shadow));
        assert!(doc.is_connected(inner));
    }

    #[test]
    fn test_get_element_by_id_caching_and_invalidation() {
        let mut doc = Document::new();
        let div = doc.new_node(NodeData::Element(ElementData::new(markup5ever::QualName::new(
            None,
            "http://www.w3.org/1999/xhtml".into(),
            "div".into(),
        ))));
        let button = doc.new_node(NodeData::Element(ElementData::new(markup5ever::QualName::new(
            None,
            "http://www.w3.org/1999/xhtml".into(),
            "button".into(),
        ))));
        doc.append_child(doc.root(), div);
        doc.append_child(div, button);

        assert_eq!(doc.get_element_by_id("submit-btn"), None);

        if let Some(NodeData::Element(elem)) = doc.get_mut(button) {
            elem.set_attribute("id", "submit-btn");
        }
        
        assert_eq!(doc.get_element_by_id("submit-btn"), Some(button));
        
        assert_eq!(doc.get_element_by_id("submit-btn"), Some(button));
        assert_eq!(doc.get_element_by_id("non-existent"), None);

        assert_eq!(doc.query_selector(doc.root(), "#submit-btn"), Some(button));
        assert_eq!(doc.query_selector(doc.root(), "#non-existent"), None);

        if let Some(NodeData::Element(elem)) = doc.get_mut(button) {
            elem.set_attribute("id", "new-id");
        }
        assert_eq!(doc.get_element_by_id("submit-btn"), None);
        assert_eq!(doc.get_element_by_id("new-id"), Some(button));
        assert_eq!(doc.query_selector(doc.root(), "#new-id"), Some(button));
    }

    #[test]
    fn test_tree_sibling_and_child_traversal() {
        let mut doc = Document::new();
        let parent = doc.create_element("div");
        let child1 = doc.create_element("span");
        let text = doc.create_text("hello");
        let child2 = doc.create_element("p");

        doc.append_child(parent, child1);
        doc.append_child(parent, text);
        doc.append_child(parent, child2);

        assert_eq!(doc.first_child(parent), Some(child1));
        assert_eq!(doc.last_child(parent), Some(child2));

        assert_eq!(doc.next_sibling(child1), Some(text));
        assert_eq!(doc.previous_sibling(child1), None);

        assert_eq!(doc.next_sibling(text), Some(child2));
        assert_eq!(doc.previous_sibling(text), Some(child1));

        assert_eq!(doc.next_sibling(child2), None);
        assert_eq!(doc.previous_sibling(child2), Some(text));

        assert_eq!(doc.first_element_child(parent), Some(child1));
        assert_eq!(doc.last_element_child(parent), Some(child2));
        assert_eq!(doc.child_element_count(parent), 2);

        assert_eq!(doc.next_element_sibling(child1), Some(child2));
        assert_eq!(doc.previous_element_sibling(child2), Some(child1));
        assert_eq!(doc.previous_element_sibling(child1), None);
        assert_eq!(doc.next_element_sibling(child2), None);

        let mut visited = Vec::new();
        doc.for_each_composed_child(parent, |c| visited.push(c));
        assert_eq!(visited, doc.composed_children(parent));
        assert_eq!(visited, vec![child1, text, child2]);
    }
}
