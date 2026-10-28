
use crate::box_model::BoxDimensions;
use crate::inline::{collect_inline_contexts, InlineFormattingContext};
use rix_css::computed::{ComputedStyle, Display};
use rix_dom::node::{ElementData, NodeData};
use rix_dom::tree::Document;
use rix_dom::NodeId;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayoutBoxId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum AnonymousBoxType {
    None,
    AnonymousBlock,
    AnonymousInline,
    Text(String),
    PseudoElement(rix_dom::query::PseudoElement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutBoxData {
    pub id: LayoutBoxId,
    pub dom_node: Option<NodeId>,
    pub style: ComputedStyle,
    pub box_type: AnonymousBoxType,
    pub dimensions: BoxDimensions,
    pub text_layout: Option<rix_text::ParagraphLayout>,
    pub inline_fragments: Vec<crate::box_model::Rect>,
    pub is_replaced: bool,
    pub is_svg: bool,
    pub intrinsic_size: Option<(f32, f32)>,
    pub intrinsic_ratio: Option<f32>,
    pub parent: Option<LayoutBoxId>,
    pub children: Vec<LayoutBoxId>,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutTree {
    boxes: Vec<LayoutBoxData>,
    pub root: Option<LayoutBoxId>,
    inline_contexts: Vec<InlineFormattingContext>,
    node_to_box: HashMap<NodeId, LayoutBoxId>,
    inline_context_map: HashMap<LayoutBoxId, usize>,
}

pub type LayoutBox = LayoutBoxData;

impl LayoutTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            boxes: Vec::with_capacity(capacity),
            root: None,
            inline_contexts: Vec::new(),
            node_to_box: HashMap::with_capacity(capacity),
            inline_context_map: HashMap::new(),
        }
    }

    pub fn push(&mut self, mut box_data: LayoutBoxData) -> LayoutBoxId {
        let id = LayoutBoxId(self.boxes.len());
        box_data.id = id;
        self.boxes.push(box_data);
        id
    }

    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    pub fn get(&self, id: LayoutBoxId) -> Option<&LayoutBoxData> {
        self.boxes.get(id.0)
    }

    pub fn get_mut(&mut self, id: LayoutBoxId) -> Option<&mut LayoutBoxData> {
        self.boxes.get_mut(id.0)
    }

    pub fn iter(&self) -> impl Iterator<Item = &LayoutBoxData> {
        self.boxes.iter()
    }

    pub fn boxes(&self) -> &[LayoutBoxData] {
        &self.boxes
    }

    pub fn is_viewport_anchored(&self, id: LayoutBoxId) -> bool {
        let mut current = Some(id);
        while let Some(box_id) = current {
            let Some(box_data) = self.get(box_id) else {
                return false;
            };
            if box_data.style.position == "fixed" {
                return true;
            }
            current = box_data.parent;
        }
        false
    }

    pub fn inline_contexts(&self) -> &[InlineFormattingContext] {
        &self.inline_contexts
    }

    pub fn inline_context(&self, block: LayoutBoxId) -> Option<&InlineFormattingContext> {
        if let Some(&idx) = self.inline_context_map.get(&block) {
            return self.inline_contexts.get(idx);
        }
        if self.inline_context_map.is_empty() {
            return self
                .inline_contexts
                .iter()
                .find(|context| context.block == block);
        }
        None
    }

    pub fn node_to_box(&self) -> &HashMap<NodeId, LayoutBoxId> {
        &self.node_to_box
    }

    pub fn rebuild_node_index(&mut self) {
        self.node_to_box.clear();
        self.node_to_box.reserve(self.boxes.len());
        for b in &self.boxes {
            if let Some(node) = b.dom_node {
                self.node_to_box.entry(node).or_insert(b.id);
            }
        }
    }

    pub fn reuse_geometry_from(&mut self, previous: &Self) -> bool {
        if self.root != previous.root
            || self.boxes.len() != previous.boxes.len()
            || self.boxes.iter().zip(&previous.boxes).any(|(next, old)| {
                next.dom_node != old.dom_node
                    || next.parent != old.parent
                    || next.children != old.children
                    || next.box_type != old.box_type
                    || next.is_replaced != old.is_replaced
                    || next.is_svg != old.is_svg
                    || next.intrinsic_size != old.intrinsic_size
                    || next.intrinsic_ratio != old.intrinsic_ratio
                    || !next.style.layout_eq(&old.style)
            })
        {
            return false;
        }
        for (next, old) in self.boxes.iter_mut().zip(&previous.boxes) {
            next.dimensions = old.dimensions;
            next.text_layout.clone_from(&old.text_layout);
            next.inline_fragments.clone_from(&old.inline_fragments);
            if let Some(paragraph) = &mut next.text_layout {
                let color = next.style.color;
                for glyph in paragraph.lines.iter_mut().flat_map(|line| &mut line.glyphs) {
                    glyph.color = [color.r, color.g, color.b, color.a];
                }
            }
        }
        true
    }

    pub fn update_input_text(
        &mut self,
        node: NodeId,
        new_text: &str,
        font_db: &rix_text::FontDatabase,
    ) -> bool {
        let mut updated = false;
        for b in &mut self.boxes {
            if b.dom_node == Some(node) {
                if let AnonymousBoxType::Text(ref mut text) = b.box_type {
                    *text = new_text.to_string();
                    let font_style = match b.style.font_style.as_str() {
                        "italic" => rix_text::FontStyle::Italic,
                        "oblique" => rix_text::FontStyle::Oblique,
                        _ => rix_text::FontStyle::Normal,
                    };
                    let font_id = font_db
                        .match_font(
                            &rix_text::FontDescriptor::new(&b.style.font_family)
                                .with_weight(rix_text::FontWeight(b.style.font_weight))
                                .with_style(font_style),
                        )
                        .or(font_db.default_sans_serif)
                        .unwrap_or(rix_text::FontId(0));

                    let color = b.style.color;
                    let span = rix_text::TextSpan::new(new_text, font_id, b.style.font_size)
                        .with_color([color.r, color.g, color.b, color.a]);
                    let available_width = b.dimensions.content.width.max(1000.0);
                    let layout = rix_text::ParagraphLayout::compute_with_options(
                        &[span],
                        rix_text::TextAlign::Left,
                        Some(available_width),
                        font_db,
                        rix_text::ParagraphOptions {
                            wrap: false,
                            ..Default::default()
                        },
                    );
                    b.text_layout = Some(layout);
                    updated = true;
                }
            }
        }
        updated
    }

    pub fn set_image_intrinsic_size(&mut self, node: NodeId, width: f32, height: f32) -> bool {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return false;
        }
        let box_data = if let Some(&box_id) = self.node_to_box.get(&node) {
            self.boxes.get_mut(box_id.0)
        } else if self.node_to_box.is_empty() {
            self.boxes
                .iter_mut()
                .find(|box_data| box_data.dom_node == Some(node) && box_data.is_replaced)
        } else {
            None
        };
        let Some(image) = box_data else {
            return false;
        };
        if !image.is_replaced {
            return false;
        }
        if image.intrinsic_size == Some((width, height)) {
            return false;
        }
        image.intrinsic_size = Some((width, height));
        true
    }

    pub fn box_for_node(&self, node: NodeId) -> Option<&LayoutBoxData> {
        if let Some(&box_id) = self.node_to_box.get(&node) {
            return self.boxes.get(box_id.0);
        }
        if self.node_to_box.is_empty() {
            return self.boxes.iter().find(|box_data| box_data.dom_node == Some(node));
        }
        None
    }

    pub fn from_dom(
        doc: &Document,
        root_node: NodeId,
        styles: &HashMap<NodeId, ComputedStyle>,
    ) -> Self {
        let mut tree = Self::with_capacity(styles.len());
        if let Some(root_box) = tree.build_box_recursive(doc, root_node, None, styles) {
            tree.root = Some(root_box);
        }
        tree.rebuild_node_index();
        let (contexts, context_map) = collect_inline_contexts(doc, styles, &tree);
        tree.inline_contexts = contexts;
        tree.inline_context_map = context_map;
        tree
    }

    fn push_generated_box(
        &mut self,
        node: NodeId,
        parent: LayoutBoxId,
        pseudo: rix_dom::query::PseudoElement,
        style: &ComputedStyle,
    ) -> Option<LayoutBoxId> {
        if style.content.is_none() || style.display == Display::None {
            return None;
        }
        let box_id = LayoutBoxId(self.boxes.len());
        let box_data = LayoutBoxData {
            id: box_id,
            dom_node: Some(node),
            style: style.clone(),
            box_type: AnonymousBoxType::PseudoElement(pseudo),
            dimensions: BoxDimensions {
                padding: crate::box_model::EdgeSizes::new(
                    style.padding_top,
                    style.padding_right,
                    style.padding_bottom,
                    style.padding_left,
                ),
                margin: crate::box_model::EdgeSizes::new(
                    style.margin_top,
                    style.margin_right,
                    style.margin_bottom,
                    style.margin_left,
                ),
                border: crate::box_model::EdgeSizes::new(
                    style.border_top_width,
                    style.border_right_width,
                    style.border_bottom_width,
                    style.border_left_width,
                ),
                ..Default::default()
            },
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: Some(parent),
            children: Vec::new(),
        };
        self.boxes.push(box_data);

        if let Some(text) = style.content.as_deref().filter(|text| !text.is_empty()) {
            let text_box_id = LayoutBoxId(self.boxes.len());
            let text_box = LayoutBoxData {
                id: text_box_id,
                dom_node: Some(node),
                style: text_style_from_parent(style),
                box_type: AnonymousBoxType::Text(text.to_string()),
                dimensions: BoxDimensions::default(),
                text_layout: None,
                inline_fragments: Vec::new(),
                is_replaced: false,
                is_svg: false,
                intrinsic_size: None,
                intrinsic_ratio: None,
                parent: Some(box_id),
                children: Vec::new(),
            };
            self.boxes.push(text_box);
            if let Some(parent_box) = self.boxes.get_mut(box_id.0) {
                parent_box.children.push(text_box_id);
            }
        }

        Some(box_id)
    }

    fn build_box_recursive(
        &mut self,
        doc: &Document,
        node: NodeId,
        parent_id: Option<LayoutBoxId>,
        styles: &HashMap<NodeId, ComputedStyle>,
    ) -> Option<LayoutBoxId> {
        let style = styles.get(&node).cloned().unwrap_or_default();

        if style.display == Display::None {
            return None;
        }

        let node_data = doc.get(node)?;
        match node_data {
            NodeData::Document | NodeData::DocumentFragment | NodeData::ShadowRoot { .. } => {
                let mut result = None;
                doc.for_each_composed_child(node, |child| {
                    if result.is_none() {
                        if let Some(child_box) = self.build_box_recursive(doc, child, parent_id, styles) {
                            result = Some(child_box);
                        }
                    }
                });
                result
            }
            NodeData::Element(element) => {
                let is_text_control = matches!(element.name.local.as_str(), "input" | "textarea");
                let is_replaced = matches!(
                    element.name.local.as_str(),
                    "img" | "svg" | "video" | "canvas" | "iframe" | "object" | "embed"
                );
                let box_id = LayoutBoxId(self.boxes.len());
                let box_data = LayoutBoxData {
                    id: box_id,
                    dom_node: Some(node),
                    style: style.clone(),
                    box_type: AnonymousBoxType::None,
                    dimensions: BoxDimensions {
                        padding: crate::box_model::EdgeSizes::new(
                            style.padding_top,
                            style.padding_right,
                            style.padding_bottom,
                            style.padding_left,
                        ),
                        margin: crate::box_model::EdgeSizes::new(
                            style.margin_top,
                            style.margin_right,
                            style.margin_bottom,
                            style.margin_left,
                        ),
                        border: crate::box_model::EdgeSizes::new(
                            style.border_top_width,
                            style.border_right_width,
                            style.border_bottom_width,
                            style.border_left_width,
                        ),
                        ..Default::default()
                    },
                    text_layout: None,
                    inline_fragments: Vec::new(),
                    is_replaced,
                    is_svg: element.name.local.as_str() == "svg",
                    intrinsic_size: match element.name.local.as_str() {
                        "svg" => svg_intrinsic_size(element),
                        "canvas" => Some(canvas_intrinsic_size(element)),
                        "video" => Some(video_intrinsic_size(element)),
                        "iframe" => Some((300.0, 150.0)),
                        _ => None,
                    },
                    intrinsic_ratio: match element.name.local.as_str() {
                        "svg" => svg_viewbox_ratio(element),
                        "video" => element
                            .video_frame
                            .as_ref()
                            .filter(|f| f.width > 0 && f.height > 0)
                            .map(|f| f.width as f32 / f.height as f32),
                        _ => None,
                    },
                    parent: parent_id,
                    children: Vec::new(),
                };

                self.boxes.push(box_data);

                let mut child_boxes = Vec::new();
                if !is_replaced && !is_text_control {
                    if let Some(before) = style.before.as_deref() {
                        if let Some(generated) = self.push_generated_box(
                            node,
                            box_id,
                            rix_dom::query::PseudoElement::Before,
                            before,
                        ) {
                            child_boxes.push(generated);
                        }
                    }
                }
                if !is_replaced && !is_text_control {
                    doc.for_each_composed_child(node, |child| {
                        if let Some(child_node_data) = doc.get(child) {
                            match child_node_data {
                                NodeData::Element(_) => {
                                    if let Some(cb) =
                                        self.build_box_recursive(doc, child, Some(box_id), styles)
                                    {
                                        child_boxes.push(cb);
                                    }
                                }
                                NodeData::Text(text) => {
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty()
                                        || matches!(
                                            style.white_space.as_str(),
                                            "pre" | "pre-wrap" | "break-spaces"
                                        )
                                    {
                                        let text_box_id = LayoutBoxId(self.boxes.len());
                                        let text_style = text_style_from_parent(&style);
                                        let text_box = LayoutBoxData {
                                            id: text_box_id,
                                            dom_node: Some(child),
                                            style: text_style,
                                            box_type: AnonymousBoxType::Text(text.clone()),
                                            dimensions: BoxDimensions::default(),
                                            text_layout: None,
                                            inline_fragments: Vec::new(),
                                            is_replaced: false,
                                            is_svg: false,
                                            intrinsic_size: None,
                                            intrinsic_ratio: None,
                                            parent: Some(box_id),
                                            children: Vec::new(),
                                        };
                                        self.boxes.push(text_box);
                                        child_boxes.push(text_box_id);
                                    }
                                }
                                _ => {}
                            }
                        }
                    });
                }

                if is_text_control {
                    if let Some(NodeData::Element(elem)) = doc.get(node) {
                        let default_value = if &*elem.name.local == "textarea" {
                            doc.inner_text(node)
                        } else {
                            String::new()
                        };
                        let value = elem.get_attribute("value").unwrap_or(&default_value);
                        let text = Some(if value.is_empty() {
                            elem.get_attribute("placeholder").unwrap_or("")
                        } else {
                            value
                        });

                        if let Some(t) = text {
                            let text_box_id = LayoutBoxId(self.boxes.len());
                            let mut text_style = match (&style.placeholder, value.is_empty()) {
                                (Some(placeholder_style), true) => {
                                    text_style_from_parent(placeholder_style)
                                }
                                _ => text_style_from_parent(&style),
                            };
                            if &*elem.name.local == "input" {
                                text_style.white_space = "pre".to_string();
                            } else if text_style.white_space == "normal" {
                                text_style.white_space = "pre-wrap".to_string();
                            }
                            let text_box = LayoutBoxData {
                                id: text_box_id,
                                dom_node: Some(node),
                                style: text_style,
                                box_type: AnonymousBoxType::Text(t.to_string()),
                                dimensions: BoxDimensions::default(),
                                text_layout: None,
                                inline_fragments: Vec::new(),
                                is_replaced: false,
                                is_svg: false,
                                intrinsic_size: None,
                                intrinsic_ratio: None,
                                parent: Some(box_id),
                                children: Vec::new(),
                            };
                            self.boxes.push(text_box);
                            child_boxes.push(text_box_id);
                        }
                    }
                }

                if !is_replaced && !is_text_control {
                    if let Some(after) = style.after.as_deref() {
                        if let Some(generated) = self.push_generated_box(
                            node,
                            box_id,
                            rix_dom::query::PseudoElement::After,
                            after,
                        ) {
                            child_boxes.push(generated);
                        }
                    }
                }

                let generates_box = |b: &LayoutBoxData| b.style.display != Display::None;
                let is_inline_level = |b: &LayoutBoxData| {
                    matches!(b.box_type, AnonymousBoxType::Text(_))
                        || b.style.display.is_inline_level()
                };
                let has_block = child_boxes.iter().any(|&cb| {
                    let b = &self.boxes[cb.0];
                    generates_box(b) && !is_inline_level(b)
                });
                let has_inline_or_text = child_boxes
                    .iter()
                    .any(|&cb| is_inline_level(&self.boxes[cb.0]));

                if style.display == Display::Block && has_block && has_inline_or_text {
                    let mut normalized_children = Vec::new();
                    let mut pending_inline = Vec::new();

                    for cb in child_boxes {
                        let is_inline = is_inline_level(&self.boxes[cb.0]);

                        if is_inline {
                            pending_inline.push(cb);
                        } else {
                            if !pending_inline.is_empty() {
                                let anon_id = self.wrap_in_anonymous_block(
                                    box_id,
                                    std::mem::take(&mut pending_inline),
                                    &style,
                                );
                                normalized_children.push(anon_id);
                            }
                            normalized_children.push(cb);
                        }
                    }

                    if !pending_inline.is_empty() {
                        let anon_id = self.wrap_in_anonymous_block(box_id, pending_inline, &style);
                        normalized_children.push(anon_id);
                    }

                    self.boxes[box_id.0].children = normalized_children;
                } else {
                    self.boxes[box_id.0].children = child_boxes;
                }

                Some(box_id)
            }
            _ => None,
        }
    }

    fn wrap_in_anonymous_block(
        &mut self,
        parent_id: LayoutBoxId,
        inlines: Vec<LayoutBoxId>,
        parent_style: &ComputedStyle,
    ) -> LayoutBoxId {
        let anon_id = LayoutBoxId(self.boxes.len());
        let mut anon_style = parent_style.clone();
        anon_style.display = Display::Block;
        anon_style.padding_top = 0.0;
        anon_style.padding_right = 0.0;
        anon_style.padding_bottom = 0.0;
        anon_style.padding_left = 0.0;
        anon_style.margin_top = 0.0;
        anon_style.margin_right = 0.0;
        anon_style.margin_bottom = 0.0;
        anon_style.margin_left = 0.0;
        anon_style.border_width = 0.0;
        anon_style.border_top_width = 0.0;
        anon_style.border_right_width = 0.0;
        anon_style.border_bottom_width = 0.0;
        anon_style.border_left_width = 0.0;
        anon_style.background_color = rix_css::CssColor::TRANSPARENT;
        let anon_box = LayoutBoxData {
            id: anon_id,
            dom_node: None,
            style: anon_style,
            box_type: AnonymousBoxType::AnonymousBlock,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: Some(parent_id),
            children: inlines.clone(),
        };

        for &child_id in &inlines {
            self.boxes[child_id.0].parent = Some(anon_id);
        }

        self.boxes.push(anon_box);
        anon_id
    }
}

fn text_style_from_parent(parent: &ComputedStyle) -> ComputedStyle {
    ComputedStyle {
        display: Display::Inline,
        color: parent.color,
        font_family: parent.font_family.clone(),
        font_size: parent.font_size,
        font_weight: parent.font_weight,
        font_style: parent.font_style.clone(),
        line_height: parent.line_height,
        line_height_factor: parent.line_height_factor,
        line_height_normal: parent.line_height_normal,
        text_align: parent.text_align.clone(),
        text_decoration_line: parent.text_decoration_line.clone(),
        white_space: parent.white_space.clone(),
        text_overflow: parent.text_overflow.clone(),
        overflow_x: parent.overflow_x.clone(),
        overflow_y: parent.overflow_y.clone(),
        visibility: parent.visibility.clone(),
        cursor: parent.cursor.clone(),
        variables: parent.variables.clone(),
        ..Default::default()
    }
}

fn svg_intrinsic_size(element: &ElementData) -> Option<(f32, f32)> {
    let dimension = |name| {
        let value = element.get_attribute(name)?.trim();
        value
            .strip_suffix("px")
            .unwrap_or(value)
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
    };
    let width = dimension("width");
    let height = dimension("height");
    let vb_dim = svg_viewbox_dimensions(element);
    let ratio = vb_dim.map(|(w, h)| w / h);
    match (width, height, ratio, vb_dim) {
        (Some(width), Some(height), _, _) => Some((width, height)),
        (Some(width), None, Some(ratio), _) => Some((width, width / ratio)),
        (None, Some(height), Some(ratio), _) => Some((height * ratio, height)),
        (None, None, _, Some((vb_w, vb_h))) => Some((vb_w, vb_h)),
        (None, None, _, None) => None,
        (width, height, None, _) => Some((width.unwrap_or(300.0), height.unwrap_or(150.0))),
    }
}

fn canvas_intrinsic_size(element: &ElementData) -> (f32, f32) {
    let dimension = |name, default| {
        element
            .get_attribute(name)
            .and_then(rix_dom::node::parse_non_negative_integer)
            .unwrap_or(default) as f32
    };
    (dimension("width", 300), dimension("height", 150))
}

fn video_intrinsic_size(element: &ElementData) -> (f32, f32) {
    if let Some(ref frame) = element.video_frame {
        if frame.width > 0 && frame.height > 0 {
            return (frame.width as f32, frame.height as f32);
        }
    }
    let dimension = |name, default| {
        element
            .get_attribute(name)
            .and_then(rix_dom::node::parse_non_negative_integer)
            .unwrap_or(default) as f32
    };
    (dimension("width", 300), dimension("height", 150))
}

fn svg_viewbox_dimensions(element: &ElementData) -> Option<(f32, f32)> {
    element
        .get_attribute("viewBox")
        .or_else(|| element.get_attribute("viewbox"))
        .and_then(|value| {
            let values = value
                .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
                .filter(|part| !part.is_empty())
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            if values.len() == 4
                && values.iter().all(|value| value.is_finite())
                && values[2] > 0.0
                && values[3] > 0.0
            {
                Some((values[2], values[3]))
            } else {
                None
            }
        })
}

fn svg_viewbox_ratio(element: &ElementData) -> Option<f32> {
    element
        .get_attribute("viewBox")
        .or_else(|| element.get_attribute("viewbox"))
        .and_then(|value| {
            let values = value
                .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
                .filter(|part| !part.is_empty())
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            (values.len() == 4
                && values.iter().all(|value| value.is_finite())
                && values[2] > 0.0
                && values[3] > 0.0)
                .then(|| values[2] / values[3])
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rix_dom::{LocalName, Namespace, QualName};

    #[test]
    fn test_svg_viewbox_intrinsic_dimensions() {
        let mut element = ElementData::new(QualName::new(
            None,
            Namespace::from("http://www.w3.org/2000/svg"),
            LocalName::from("svg"),
        ));
        element.set_attribute("viewBox", "0 0 24 24");

        assert_eq!(svg_viewbox_dimensions(&element), Some((24.0, 24.0)));
        assert_eq!(svg_intrinsic_size(&element), Some((24.0, 24.0)));

        element.set_attribute("width", "48");
        element.set_attribute("height", "48");
        assert_eq!(svg_intrinsic_size(&element), Some((48.0, 48.0)));
    }

    #[test]
    fn test_set_image_intrinsic_size_dedup_and_box_for_node() {
        let mut doc = Document::new();
        let node_id = doc.new_node(NodeData::Comment("test".into()));
        let other_node = doc.new_node(NodeData::Comment("other".into()));
        let mut tree = LayoutTree::new();
        let box_data = LayoutBoxData {
            id: LayoutBoxId(0),
            dom_node: Some(node_id),
            style: ComputedStyle::default(),
            box_type: AnonymousBoxType::None,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: true,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: None,
            children: Vec::new(),
        };
        tree.boxes.push(box_data);

        assert!(tree.box_for_node(node_id).is_some());
        assert_eq!(tree.box_for_node(node_id).unwrap().id, LayoutBoxId(0));
        assert!(tree.box_for_node(other_node).is_none());

        assert!(tree.set_image_intrinsic_size(node_id, 320.0, 180.0));
        assert_eq!(tree.get(LayoutBoxId(0)).unwrap().intrinsic_size, Some((320.0, 180.0)));

        assert!(!tree.set_image_intrinsic_size(node_id, 320.0, 180.0));

        assert!(tree.set_image_intrinsic_size(node_id, 640.0, 360.0));
        assert_eq!(tree.get(LayoutBoxId(0)).unwrap().intrinsic_size, Some((640.0, 360.0)));
    }

    #[test]
    fn test_layout_tree_node_to_box_and_inline_context_indexing() {
        let mut doc = Document::new();
        let html_elem = ElementData::new_html("div");
        let node_id = doc.new_node(NodeData::Element(html_elem));
        doc.append_child(doc.root(), node_id);

        let mut styles = HashMap::new();
        let mut block_style = ComputedStyle::default();
        block_style.display = Display::Block;
        styles.insert(node_id, block_style);

        let tree = LayoutTree::from_dom(&doc, doc.root(), &styles);
        assert!(!tree.node_to_box().is_empty());
        assert!(tree.box_for_node(node_id).is_some());
        let box_data = tree.box_for_node(node_id).unwrap();
        assert_eq!(box_data.dom_node, Some(node_id));

        for ctx in tree.inline_contexts() {
            assert!(tree.inline_context(ctx.block).is_some());
            assert_eq!(tree.inline_context(ctx.block).unwrap().block, ctx.block);
        }
    }
}
