
use rix_css::color::CssColor;
use rix_css::computed::BackgroundRepeatMode;
use rix_dom::ElementData;
use rix_layout::box_model::Rect;
use rix_layout::tree::{AnonymousBoxType, LayoutBoxId, LayoutTree};
use rix_text::font_loader::{FontDatabase, FontDescriptor, FontId, FontStyle, FontWeight};
use rix_text::layout::LayoutGlyph;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

impl ObjectFit {
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "contain" => Self::Contain,
            "cover" => Self::Cover,
            "none" => Self::None,
            "scale-down" => Self::ScaleDown,
            _ => Self::Fill,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    DrawRect {
        rect: Rect,
        color: CssColor,
        border_radius: f32,
        corner_radii: [f32; 4],
    },
    DrawLinearGradient {
        rect: Rect,
        clip_rect: Rect,
        repeat_x: bool,
        repeat_y: bool,
        gradient: rix_css::gradient::ResolvedLinearGradient,
        border_radius: f32,
    },
    DrawBorder {
        rect: Rect,
        color: CssColor,
        width: f32,
        border_radius: f32,
        corner_radii: [f32; 4],
        widths: [f32; 4],
    },
    DrawText {
        text: String,
        font_id: FontId,
        font_size: f32,
        color: CssColor,
        x: f32,
        y: f32,
    },
    DrawGlyphs {
        glyphs: Vec<LayoutGlyph>,
        x: f32,
        y: f32,
    },
    DrawPath {
        d: String,
        x: f32,
        y: f32,
        scale_x: f32,
        scale_y: f32,
        fill: Option<CssColor>,
        fill_rule: rix_dom::svg::SvgFillRule,
        stroke: Option<CssColor>,
        stroke_width: f32,
    },
    DrawImage {
        rect: Rect,
        src: String,
        border_radius: f32,
        object_fit: ObjectFit,
    },
    DrawImageResource {
        rect: Rect,
        image: crate::image_resource::ImageResource,
        border_radius: f32,
        object_fit: ObjectFit,
    },
    DrawShadow {
        rect: Rect,
        color: CssColor,
        blur_radius: f32,
        offset_x: f32,
        offset_y: f32,
        border_radius: f32,
        spread: f32,
    },
    PushClip {
        rect: Rect,
        corner_radii: [f32; 4],
    },
    PopClip,
    PushOpacity {
        opacity: f32,
    },
    PopOpacity,
}

impl DrawCommand {
    pub fn translate(&self, offset_x: f32, offset_y: f32) -> DrawCommand {
        let translate_rect = |r: Rect| Rect::new(r.x + offset_x, r.y + offset_y, r.width, r.height);
        match self {
            DrawCommand::DrawRect {
                rect,
                color,
                border_radius,
                corner_radii,
            } => DrawCommand::DrawRect {
                rect: translate_rect(*rect),
                color: *color,
                border_radius: *border_radius,
                corner_radii: *corner_radii,
            },
            DrawCommand::DrawLinearGradient {
                rect,
                clip_rect,
                repeat_x,
                repeat_y,
                gradient,
                border_radius,
            } => DrawCommand::DrawLinearGradient {
                rect: translate_rect(*rect),
                clip_rect: translate_rect(*clip_rect),
                repeat_x: *repeat_x,
                repeat_y: *repeat_y,
                gradient: gradient.clone(),
                border_radius: *border_radius,
            },
            DrawCommand::DrawBorder {
                rect,
                color,
                width,
                border_radius,
                corner_radii,
                widths,
            } => DrawCommand::DrawBorder {
                rect: translate_rect(*rect),
                color: *color,
                width: *width,
                border_radius: *border_radius,
                corner_radii: *corner_radii,
                widths: *widths,
            },
            DrawCommand::DrawText {
                text,
                font_id,
                font_size,
                color,
                x,
                y,
            } => DrawCommand::DrawText {
                text: text.clone(),
                font_id: *font_id,
                font_size: *font_size,
                color: *color,
                x: *x + offset_x,
                y: *y + offset_y,
            },
            DrawCommand::DrawGlyphs { glyphs, x, y } => DrawCommand::DrawGlyphs {
                glyphs: glyphs.clone(),
                x: *x + offset_x,
                y: *y + offset_y,
            },
            DrawCommand::DrawPath {
                d,
                x,
                y,
                scale_x,
                scale_y,
                fill,
                fill_rule,
                stroke,
                stroke_width,
            } => DrawCommand::DrawPath {
                d: d.clone(),
                x: *x + offset_x,
                y: *y + offset_y,
                scale_x: *scale_x,
                scale_y: *scale_y,
                fill: *fill,
                fill_rule: *fill_rule,
                stroke: *stroke,
                stroke_width: *stroke_width,
            },
            DrawCommand::DrawImage {
                rect,
                src,
                border_radius,
                object_fit,
            } => DrawCommand::DrawImage {
                rect: translate_rect(*rect),
                src: src.clone(),
                border_radius: *border_radius,
                object_fit: *object_fit,
            },
            DrawCommand::DrawImageResource {
                rect,
                image,
                border_radius,
                object_fit,
            } => DrawCommand::DrawImageResource {
                rect: translate_rect(*rect),
                image: image.clone(),
                border_radius: *border_radius,
                object_fit: *object_fit,
            },
            DrawCommand::DrawShadow {
                rect,
                color,
                blur_radius,
                offset_x: shadow_x,
                offset_y: shadow_y,
                border_radius,
                spread,
            } => DrawCommand::DrawShadow {
                rect: translate_rect(*rect),
                color: *color,
                blur_radius: *blur_radius,
                offset_x: *shadow_x,
                offset_y: *shadow_y,
                border_radius: *border_radius,
                spread: *spread,
            },
            DrawCommand::PushClip { rect, corner_radii } => DrawCommand::PushClip {
                rect: translate_rect(*rect),
                corner_radii: *corner_radii,
            },
            DrawCommand::PopClip => DrawCommand::PopClip,
            DrawCommand::PushOpacity { opacity } => DrawCommand::PushOpacity {
                opacity: *opacity,
            },
            DrawCommand::PopOpacity => DrawCommand::PopOpacity,
        }
    }
}

use rix_dom::node::NodeData;
use rix_dom::tree::Document;

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayItem {
    pub command: DrawCommand,
    pub z_index: i32,
    pub viewport_anchored: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
    viewport_anchored: bool,
}

impl PartialEq for DisplayList {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl DisplayList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, command: DrawCommand, z_index: i32) {
        self.items.push(DisplayItem {
            command,
            z_index,
            viewport_anchored: self.viewport_anchored,
        });
    }

    pub fn building_viewport_anchored(&self) -> bool {
        self.viewport_anchored
    }

    pub fn set_building_viewport_anchored(&mut self, anchored: bool) -> bool {
        std::mem::replace(&mut self.viewport_anchored, anchored)
    }

    pub fn translate(&self, offset_x: f32, offset_y: f32) -> DisplayList {
        let mut list = DisplayList::new();
        for item in &self.items {
            list.push(item.command.translate(offset_x, offset_y), item.z_index);
        }
        list
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn sort_by_stacking_order(&mut self) {
        self.items.sort_by_key(|item| item.z_index);
    }

    pub fn build_from_layout(tree: &LayoutTree, font_db: &FontDatabase) -> Self {
        Self::build_from_layout_with_dom(tree, None, font_db)
    }

    pub fn build_from_layout_with_dom(
        tree: &LayoutTree,
        doc: Option<&Document>,
        font_db: &FontDatabase,
    ) -> Self {
        Self::build_from_layout_with_dom_and_scroll(tree, doc, font_db, None, 0.0, 0.0)
    }

    pub fn build_from_layout_with_dom_and_scroll(
        tree: &LayoutTree,
        doc: Option<&Document>,
        font_db: &FontDatabase,
        scroll_node: Option<rix_dom::NodeId>,
        scroll_x: f32,
        scroll_y: f32,
    ) -> Self {
        let mut list = Self::new();
        let viewport_overflow_owner = rix_layout::overflow::viewport_overflow_owner(tree, doc);
        if let Some(root_id) = tree.root {
            Self::build_box_recursive(
                tree,
                root_id,
                0,
                &mut list,
                font_db,
                doc,
                0,
                viewport_overflow_owner,
                scroll_node,
                scroll_x,
                scroll_y,
            );
        }
        list.sort_by_stacking_order();
        list
    }

    #[allow(clippy::too_many_arguments)]
    fn build_box_recursive(
        tree: &LayoutTree,
        box_id: LayoutBoxId,
        current_z: i32,
        list: &mut DisplayList,
        font_db: &FontDatabase,
        doc: Option<&Document>,
        depth: usize,
        viewport_overflow_owner: Option<rix_dom::NodeId>,
        scroll_node: Option<rix_dom::NodeId>,
        scroll_x: f32,
        scroll_y: f32,
    ) {
        let box_data = match tree.get(box_id) {
            Some(b) => b,
            None => return,
        };

        if box_data.style.opacity <= 0.001 {
            return;
        }
        let anchored = list.building_viewport_anchored() || box_data.style.position == "fixed";
        let previous_anchored = list.set_building_viewport_anchored(anchored);
        let paint_visible = box_data.style.visibility == "visible";

        let rect = box_data.dimensions.border_box();
        let current_z = if box_data.style.position == "fixed" {
            if box_data.style.z_index != 0 {
                box_data.style.z_index.max(current_z)
            } else {
                current_z
            }
        } else {
            current_z.saturating_add(box_data.style.z_index)
        };

        let is_text_box = matches!(box_data.box_type, AnonymousBoxType::Text(_));

        let needs_opacity_layer = box_data.style.opacity < 0.999;
        if needs_opacity_layer {
            list.push(
                DrawCommand::PushOpacity {
                    opacity: box_data.style.opacity.clamp(0.0, 1.0),
                },
                current_z,
            );
        }

        if paint_visible && !is_text_box {
            if let Some(shadow) = box_data.style.box_shadow {
                if shadow.blur_radius >= 0.0
                    && (shadow.offset_x != 0.0
                        || shadow.offset_y != 0.0
                        || shadow.blur_radius > 0.0
                        || shadow.spread_radius != 0.0)
                    && shadow.color.a > 0.001
                {
                    list.push(
                        DrawCommand::DrawShadow {
                            rect,
                            color: shadow.color,
                            blur_radius: shadow.blur_radius,
                            offset_x: shadow.offset_x,
                            offset_y: shadow.offset_y,
                            border_radius: box_data.style.border_radius,
                            spread: shadow.spread_radius,
                        },
                        current_z,
                    );
                }
            }
        }

        if paint_visible
            && !is_text_box
            && box_data.style.background_color != CssColor::TRANSPARENT
            && box_data.style.background_color.a > 0.001
        {
            let (tl, tr, br, bl) = box_data.style.corner_radii();
            let corner_radii = [tl, tr, br, bl];
            if box_data.inline_fragments.is_empty() {
                list.push(
                    DrawCommand::DrawRect {
                        rect,
                        color: box_data.style.background_color,
                        border_radius: box_data.style.border_radius,
                        corner_radii,
                    },
                    current_z,
                );
            } else {
                for fragment in &box_data.inline_fragments {
                    list.push(
                        DrawCommand::DrawRect {
                            rect: *fragment,
                            color: box_data.style.background_color,
                            border_radius: box_data.style.border_radius,
                            corner_radii,
                        },
                        current_z,
                    );
                }
            }
        }

        if paint_visible && !is_text_box {
            let image_rect = box_data.dimensions.padding_box();
            let repeat = match (
                box_data.style.background_repeat_x,
                box_data.style.background_repeat_y,
            ) {
                (BackgroundRepeatMode::Repeat, BackgroundRepeatMode::Repeat) => Some((true, true)),
                (BackgroundRepeatMode::Repeat, BackgroundRepeatMode::NoRepeat) => {
                    Some((true, false))
                }
                (BackgroundRepeatMode::NoRepeat, BackgroundRepeatMode::Repeat) => {
                    Some((false, true))
                }
                (BackgroundRepeatMode::NoRepeat, BackgroundRepeatMode::NoRepeat) => {
                    Some((false, false))
                }
                _ => None,
            };
            if let (Some((repeat_x, repeat_y)), Some(gradient)) = (
                repeat,
                box_data
                    .style
                    .background_image
                    .as_ref()
                    .and_then(|gradient| {
                        gradient.resolve(image_rect.width, image_rect.height, box_data.style.color)
                    }),
            ) {
                list.push(
                    DrawCommand::DrawLinearGradient {
                        rect: image_rect,
                        clip_rect: rect,
                        repeat_x,
                        repeat_y,
                        gradient,
                        border_radius: box_data.style.border_radius,
                    },
                    current_z,
                );
            }
        }

        if paint_visible && !is_text_box {
            let style = &box_data.style;
            let widths = [
                style.border_top_width,
                style.border_right_width,
                style.border_bottom_width,
                style.border_left_width,
            ];
            let colors = [
                style.border_top_color,
                style.border_right_color,
                style.border_bottom_color,
                style.border_left_color,
            ];
            let uniform = widths.iter().all(|width| *width == widths[0])
                && colors.iter().all(|color| *color == colors[0]);
            let has_corner_radii =
                style.corner_radii() != (0.0, 0.0, 0.0, 0.0) || style.border_radius > 0.0;
            let active_colors: Vec<_> = widths
                .iter()
                .zip(colors.iter())
                .filter(|(&w, c)| w > 0.0 && c.a > 0.001)
                .map(|(_, c)| *c)
                .collect();
            let same_color = !active_colors.is_empty()
                && active_colors.iter().all(|c| *c == active_colors[0]);

            if uniform && widths[0] > 0.0 && colors[0].a > 0.001 {
                let bw = widths[0];
                let (tl, tr, br, bl) = style.corner_radii();
                let corner_radii = [
                    (tl - bw / 2.0).max(0.0),
                    (tr - bw / 2.0).max(0.0),
                    (br - bw / 2.0).max(0.0),
                    (bl - bw / 2.0).max(0.0),
                ];
                list.push(
                    DrawCommand::DrawBorder {
                        rect: Rect::new(
                            rect.x + bw / 2.0,
                            rect.y + bw / 2.0,
                            (rect.width - bw).max(0.0),
                            (rect.height - bw).max(0.0),
                        ),
                        color: colors[0],
                        width: bw,
                        border_radius: (style.border_radius - bw / 2.0).max(0.0),
                        corner_radii,
                        widths: [bw; 4],
                    },
                    current_z,
                );
            } else if has_corner_radii && same_color {
                let (tl, tr, br, bl) = style.corner_radii();
                list.push(
                    DrawCommand::DrawBorder {
                        rect,
                        color: active_colors[0],
                        width: widths.iter().copied().fold(0.0, f32::max),
                        border_radius: style.border_radius,
                        corner_radii: [tl, tr, br, bl],
                        widths,
                    },
                    current_z,
                );
            } else if !uniform {
                let sides = [
                    Rect::new(rect.x, rect.y, rect.width, widths[0]),
                    Rect::new(
                        rect.x + rect.width - widths[1],
                        rect.y,
                        widths[1],
                        rect.height,
                    ),
                    Rect::new(
                        rect.x,
                        rect.y + rect.height - widths[2],
                        rect.width,
                        widths[2],
                    ),
                    Rect::new(rect.x, rect.y, widths[3], rect.height),
                ];
                for ((side, color), width) in sides.into_iter().zip(colors).zip(widths) {
                    if width > 0.0 && color.a > 0.001 {
                        list.push(
                            DrawCommand::DrawRect {
                                rect: side,
                                color,
                                border_radius: 0.0,
                                corner_radii: [0.0; 4],
                            },
                            current_z,
                        );
                    }
                }
            }
        }

        let propagates_to_viewport = viewport_overflow_owner.is_some()
            && box_data.dom_node == viewport_overflow_owner;
        let clip_x = !is_text_box
            && !propagates_to_viewport
            && box_data.style.overflow_x != "visible";
        let clip_y = !is_text_box
            && !propagates_to_viewport
            && box_data.style.overflow_y != "visible";
        let needs_clip = clip_x || clip_y;
        if needs_clip {
            let padding = box_data.dimensions.padding_box();
            let (tl, tr, br, bl) = box_data.style.corner_radii();
            let border = &box_data.dimensions.border;
            let corner_radii = if clip_x && clip_y {
                [
                    (tl - border.top.max(border.left)).max(0.0),
                    (tr - border.top.max(border.right)).max(0.0),
                    (br - border.bottom.max(border.right)).max(0.0),
                    (bl - border.bottom.max(border.left)).max(0.0),
                ]
            } else {
                [0.0; 4]
            };
            list.push(
                DrawCommand::PushClip {
                    rect: Rect::new(
                        if clip_x { padding.x } else { -1e6 },
                        if clip_y { padding.y } else { -1e6 },
                        if clip_x { padding.width } else { 2e6 },
                        if clip_y { padding.height } else { 2e6 },
                    ),
                    corner_radii,
                },
                current_z,
            );
        }

        if let (AnonymousBoxType::Text(text), true) = (&box_data.box_type, paint_visible) {
            if let Some(layout) = &box_data.text_layout {
                let selection = doc
                    .and_then(|document| document.text_selection())
                    .filter(|(node, _, _)| Some(*node) == box_data.dom_node);
                if let Some((_, start, end)) = selection {
                    let selection_color = box_data
                        .style
                        .variables
                        .get("--accent")
                        .and_then(|value| CssColor::parse(value))
                        .unwrap_or(box_data.style.color);
                    for rect in
                        text_selection_rects(layout, text, start, end, box_data.dimensions.content)
                    {
                        list.push(
                            DrawCommand::DrawRect {
                                rect,
                                color: CssColor {
                                    a: 0.25,
                                    ..selection_color
                                },
                                border_radius: 0.0,
                                corner_radii: [0.0; 4],
                            },
                            current_z,
                        );
                    }
                }
                let total_glyphs: usize = layout.lines.iter().map(|l| l.glyphs.len()).sum();
                let mut glyphs = Vec::with_capacity(total_glyphs);
                for line in &layout.lines {
                    glyphs.extend(line.glyphs.iter().filter(|glyph| glyph.paint).copied());
                }
                list.push(
                    DrawCommand::DrawGlyphs {
                        glyphs,
                        x: box_data.dimensions.content.x,
                        y: box_data.dimensions.content.y,
                    },
                    current_z,
                );
                if let Some((_, start, end)) = selection {
                    if start == end {
                        if let Some(rect) =
                            text_caret_rect(layout, text, end, box_data.dimensions.content)
                        {
                            list.push(
                                DrawCommand::DrawRect {
                                    rect,
                                    color: box_data.style.color,
                                    border_radius: 0.0,
                                    corner_radii: [0.0; 4],
                                },
                                current_z,
                            );
                        }
                    }
                }
            } else {
                let font_style = match box_data.style.font_style.as_str() {
                    "italic" => FontStyle::Italic,
                    "oblique" => FontStyle::Oblique,
                    _ => FontStyle::Normal,
                };
                let font_id = font_db
                    .match_font(
                        &FontDescriptor::new(&box_data.style.font_family)
                            .with_weight(FontWeight(box_data.style.font_weight))
                            .with_style(font_style),
                    )
                    .or(font_db.default_sans_serif)
                    .unwrap_or(FontId(0));

                list.push(
                    DrawCommand::DrawText {
                        text: text.clone(),
                        font_id,
                        font_size: box_data.style.font_size,
                        color: box_data.style.color,
                        x: box_data.dimensions.content.x,
                        y: box_data.dimensions.content.y + box_data.style.font_size,
                    },
                    current_z,
                );
            }
        }

        if let (Some(doc), Some(dom_id), true) = (doc, box_data.dom_node, paint_visible) {
            if let Some(NodeData::Element(elem)) = doc.get(dom_id) {
                let tag = elem.name.local.as_str();
                if tag == "img" {
                    let source = rix_dom::select_image_source(
                        elem.get_attribute("src"),
                        elem.get_attribute("srcset"),
                    );
                    if let Some(src) = source {
                        if !src.is_empty() {
                            list.push(
                                DrawCommand::DrawImage {
                                    rect: box_data.dimensions.content,
                                    src: src.to_string(),
                                    border_radius: box_data.style.border_radius,
                                    object_fit: ObjectFit::parse(&box_data.style.object_fit),
                                },
                                current_z,
                            );
                        }
                    }
                } else if tag == "canvas" {
                    if let Some(bitmap) = elem.canvas_bitmap.as_ref() {
                        if let Some(pixels) = bitmap.rgba.as_ref() {
                            match crate::image_resource::ImageResource::from_rgba(
                                bitmap.width,
                                bitmap.height,
                                pixels,
                            ) {
                                Ok(image) => list.push(
                                    DrawCommand::DrawImageResource {
                                        rect: box_data.dimensions.content,
                                        image,
                                        border_radius: box_data.style.border_radius,
                                        object_fit: ObjectFit::parse(&box_data.style.object_fit),
                                    },
                                    current_z,
                                ),
                                Err(error) => tracing::warn!(
                                    width = bitmap.width,
                                    height = bitmap.height,
                                    %error,
                                    "Skipping invalid canvas bitmap in display list"
                                ),
                            }
                        }
                    }
                } else if tag == "video" {
                    if let Some(frame) = elem.video_frame.as_ref() {
                        if !frame.rgba.is_empty() {
                            match crate::image_resource::ImageResource::from_rgba(
                                frame.width,
                                frame.height,
                                &frame.rgba,
                            ) {
                                Ok(image) => list.push(
                                    DrawCommand::DrawImageResource {
                                        rect: box_data.dimensions.content,
                                        image,
                                        border_radius: box_data.style.border_radius,
                                        object_fit: ObjectFit::parse(&box_data.style.object_fit),
                                    },
                                    current_z,
                                ),
                                Err(error) => tracing::warn!(
                                    width = frame.width,
                                    height = frame.height,
                                    %error,
                                    "Skipping invalid video frame in display list"
                                ),
                            }
                        }
                    } else if let Some(poster) = elem.get_attribute("poster") {
                        if !poster.is_empty() {
                            list.push(
                                DrawCommand::DrawImage {
                                    rect: box_data.dimensions.content,
                                    src: poster.to_string(),
                                    border_radius: box_data.style.border_radius,
                                    object_fit: ObjectFit::parse(&box_data.style.object_fit),
                                },
                                current_z,
                            );
                        }
                    }
                } else if tag == "svg" {
                    let style_attr = elem.get_attribute("style").unwrap_or_default();
                    let is_explicit_zero = (style_attr.contains("width: 0") || style_attr.contains("width:0") || elem.get_attribute("width") == Some("0"))
                        && (style_attr.contains("height: 0") || style_attr.contains("height:0") || elem.get_attribute("height") == Some("0"));
                    if !is_explicit_zero {
                        let content_rect = box_data.dimensions.content;
                        let vb = elem
                            .get_attribute("viewBox")
                            .or_else(|| elem.get_attribute("viewbox"))
                            .and_then(rix_dom::svg::SvgViewBox::parse)
                            .unwrap_or(rix_dom::svg::SvgViewBox {
                                min_x: 0.0,
                                min_y: 0.0,
                                width: content_rect.width.max(1.0),
                                height: content_rect.height.max(1.0),
                            });

                        let preserve = elem
                            .get_attribute("preserveAspectRatio")
                            .or_else(|| elem.get_attribute("preserveaspectratio"))
                            .map(rix_dom::svg::SvgPreserveAspectRatio::parse)
                            .unwrap_or_default();

                        let (scale_x, scale_y, offset_x, offset_y) = preserve.compute_transform(
                            &vb,
                            content_rect.width,
                            content_rect.height,
                        );

                        let origin_x = content_rect.x + offset_x;
                        let origin_y = content_rect.y + offset_y;

                        for child_id in doc.children(dom_id) {
                            render_svg_element_or_use(
                                doc,
                                child_id,
                                dom_id,
                                origin_x,
                                origin_y,
                                vb.min_x,
                                vb.min_y,
                                scale_x,
                                scale_y,
                                0.0,
                                0.0,
                                box_data.style.color,
                                box_data.style.fill,
                                box_data.style.stroke,
                                box_data.style.stroke_width,
                                current_z,
                                list,
                                false,
                                0,
                            );
                        }
                    }
                } else if tag == "iframe" {
                    if let Some(content_doc_root) = elem.content_document {
                        Self::build_iframe_content(
                            doc,
                            content_doc_root,
                            box_data.dimensions.content,
                            current_z,
                            list,
                            font_db,
                            depth + 1,
                        );
                    }
                }
            }
        }

        if box_data.children.is_empty() {
            
        } else {
            let is_scrolling_container = scroll_node.is_some()
                && box_data.dom_node == scroll_node
                && (scroll_x != 0.0 || scroll_y != 0.0);

            if is_scrolling_container {
                let mut sub_list = DisplayList::new();
                for &child_id in &box_data.children {
                    Self::build_box_recursive(
                        tree,
                        child_id,
                        current_z,
                        &mut sub_list,
                        font_db,
                        doc,
                        depth + 1,
                        viewport_overflow_owner,
                        scroll_node,
                        scroll_x,
                        scroll_y,
                    );
                }
                let translated = sub_list.translate(-scroll_x, -scroll_y);
                for item in translated.items {
                    list.push(item.command, item.z_index);
                }
            } else if box_data.children.len() == 1 {
                Self::build_box_recursive(
                    tree,
                    box_data.children[0],
                    current_z,
                    list,
                    font_db,
                    doc,
                    depth + 1,
                    viewport_overflow_owner,
                    scroll_node,
                    scroll_x,
                    scroll_y,
                );
            } else {
                let has_z_index = box_data
                    .children
                    .iter()
                    .any(|&child_id| tree.get(child_id).is_some_and(|child| child.style.z_index != 0));
                if !has_z_index {
                    for &child_id in &box_data.children {
                        Self::build_box_recursive(
                            tree,
                            child_id,
                            current_z,
                            list,
                            font_db,
                            doc,
                            depth + 1,
                            viewport_overflow_owner,
                            scroll_node,
                            scroll_x,
                            scroll_y,
                        );
                    }
                } else {
                    let mut children = box_data.children.clone();
                    children.sort_by_key(|child_id| {
                        tree.get(*child_id)
                            .map(|child| child.style.z_index)
                            .unwrap_or(0)
                    });
                    for child_id in children {
                        Self::build_box_recursive(
                            tree,
                            child_id,
                            current_z,
                            list,
                            font_db,
                            doc,
                            depth + 1,
                            viewport_overflow_owner,
                            scroll_node,
                            scroll_x,
                            scroll_y,
                        );
                    }
                }
            }
        }

        if needs_clip {
            list.push(DrawCommand::PopClip, current_z);
        }

        if needs_opacity_layer {
            list.push(DrawCommand::PopOpacity, current_z);
        }

        list.set_building_viewport_anchored(previous_anchored);
    }

    #[allow(clippy::too_many_arguments)]
    fn build_iframe_content(
        doc: &Document,
        content_doc_root: rix_dom::NodeId,
        content_rect: Rect,
        current_z: i32,
        list: &mut DisplayList,
        font_db: &FontDatabase,
        depth: usize,
    ) {
        if depth > 8 || content_rect.width <= 0.0 || content_rect.height <= 0.0 {
            return;
        }

        let ua_index = rix_css::rule_index::RuleIndex::default_ua();

        let mut author_sheet = rix_css::stylesheet::StyleSheet::default();
        for desc_id in doc.descendants(content_doc_root) {
            if let Some(NodeData::Element(elem)) = doc.get(desc_id) {
                if elem.name.local.as_str() == "style" {
                    for child_id in doc.children(desc_id) {
                        if let Some(NodeData::Text(css)) = doc.get(child_id) {
                            author_sheet
                                .rules
                                .extend(rix_css::stylesheet::StyleSheet::parse(css).rules.clone());
                        }
                    }
                }
            }
        }
        let author_index = rix_css::rule_index::RuleIndex::from_stylesheet(&author_sheet);

        let mut styles = std::collections::HashMap::new();
        let ctx = rix_css::computed::LayoutContext {
            root_font_size: 16.0,
            viewport_width: content_rect.width,
            viewport_height: content_rect.height,
        };
        rix_css::computed::compute_tree_styles(
            doc,
            content_doc_root,
            None,
            &ua_index,
            &author_index,
            &ctx,
            &mut styles,
        );

        let mut sub_tree = LayoutTree::from_dom(doc, content_doc_root, &styles);
        let mut engine = rix_layout::TaffyLayoutEngine::new();
        let _ = engine.compute_layout_with_fonts(
            &mut sub_tree,
            content_rect.width,
            content_rect.height,
            font_db,
        );

        let mut sub_list = DisplayList::new();
        if let Some(sub_root_box) = sub_tree.root {
            Self::build_box_recursive(
                &sub_tree,
                sub_root_box,
                0,
                &mut sub_list,
                font_db,
                Some(doc),
                depth + 1,
                None,
                None,
                0.0,
                0.0,
            );
        }

        list.push(
            DrawCommand::PushClip {
                rect: content_rect,
                corner_radii: [0.0; 4],
            },
            current_z,
        );
        let translated = sub_list.translate(content_rect.x, content_rect.y);
        for item in translated.items {
            list.push(item.command, current_z.saturating_add(item.z_index));
        }
        list.push(DrawCommand::PopClip, current_z);
    }
}

fn get_inline_style_property<'a>(element: &'a ElementData, property: &str) -> Option<&'a str> {
    let style = element.get_attribute("style")?;
    for part in style.split(';') {
        let part = part.trim();
        if let Some((name, val)) = part.split_once(':') {
            if name.trim().eq_ignore_ascii_case(property) {
                return Some(val.trim());
            }
        }
    }
    None
}

fn resolve_svg_attribute<'a>(
    doc: &'a Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
    name: &str,
) -> Option<&'a str> {
    let mut current = Some(node);
    while let Some(id) = current {
        if let Some(NodeData::Element(element)) = doc.get(id) {
            if let Some(val) = get_inline_style_property(element, name) {
                return Some(val);
            }
            if let Some(val) = element.get_attribute(name) {
                return Some(val);
            }
        }
        if id == svg_root {
            break;
        }
        current = doc.parent(id);
    }
    None
}

#[allow(dead_code)]
fn inherited_svg_attribute<'a>(
    doc: &'a Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
    name: &str,
) -> Option<&'a str> {
    resolve_svg_attribute(doc, node, svg_root, name)
}

fn is_svg_element_visible(
    doc: &Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
    is_use_instance: bool,
) -> bool {
    let mut current = Some(node);
    while let Some(id) = current {
        if let Some(NodeData::Element(element)) = doc.get(id) {
            let tag = element.name.local.as_str();
            if !is_use_instance && (tag == "defs" || tag == "clipPath" || tag == "symbol") {
                return false;
            }
            if let Some(display) = get_inline_style_property(element, "display")
                .or_else(|| element.get_attribute("display"))
            {
                if display.trim().eq_ignore_ascii_case("none") {
                    return false;
                }
            }
            if let Some(visibility) = get_inline_style_property(element, "visibility")
                .or_else(|| element.get_attribute("visibility"))
            {
                if visibility.trim().eq_ignore_ascii_case("hidden")
                    || visibility.trim().eq_ignore_ascii_case("collapse")
                {
                    return false;
                }
            }
        }
        if id == svg_root {
            break;
        }
        current = doc.parent(id);
    }
    true
}

fn resolve_svg_color(
    doc: &Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
    default_color: CssColor,
) -> CssColor {
    if let Some(color_val) = resolve_svg_attribute(doc, node, svg_root, "color") {
        if let Some(parsed) = CssColor::parse(color_val) {
            return parsed;
        }
    }
    default_color
}

fn resolve_svg_fill(
    doc: &Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
    default_color: CssColor,
    inherited_fill: Option<CssColor>,
) -> Option<CssColor> {
    let fill_val = resolve_svg_attribute(doc, node, svg_root, "fill");
    match fill_val {
        Some(val) => {
            let trimmed = val.trim();
            if trimmed.eq_ignore_ascii_case("none") {
                None
            } else if trimmed.eq_ignore_ascii_case("currentColor") {
                Some(resolve_svg_color(doc, node, svg_root, default_color))
            } else {
                CssColor::parse(trimmed)
            }
        }
        None => inherited_fill.or(Some(default_color)),
    }
}

fn resolve_svg_stroke(
    doc: &Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
    default_color: CssColor,
    inherited_stroke: Option<CssColor>,
) -> Option<CssColor> {
    let stroke_val = resolve_svg_attribute(doc, node, svg_root, "stroke");
    match stroke_val {
        Some(val) => {
            let trimmed = val.trim();
            if trimmed.eq_ignore_ascii_case("none") {
                None
            } else if trimmed.eq_ignore_ascii_case("currentColor") {
                Some(resolve_svg_color(doc, node, svg_root, default_color))
            } else {
                CssColor::parse(trimmed)
            }
        }
        None => inherited_stroke,
    }
}

fn resolve_svg_stroke_width(
    doc: &Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
    inherited_width: f32,
) -> f32 {
    resolve_svg_attribute(doc, node, svg_root, "stroke-width")
        .and_then(|w| w.trim().trim_end_matches("px").parse::<f32>().ok())
        .unwrap_or(inherited_width)
}

fn resolve_svg_fill_rule(
    doc: &Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
) -> rix_dom::svg::SvgFillRule {
    resolve_svg_attribute(doc, node, svg_root, "fill-rule")
        .and_then(rix_dom::svg::SvgFillRule::parse)
        .unwrap_or(rix_dom::svg::SvgFillRule::NonZero)
}

fn resolve_svg_opacities(
    doc: &Document,
    node: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
) -> (f32, f32, f32) {
    let opacity = resolve_svg_attribute(doc, node, svg_root, "opacity")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let fill_opacity = resolve_svg_attribute(doc, node, svg_root, "fill-opacity")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let stroke_opacity = resolve_svg_attribute(doc, node, svg_root, "stroke-opacity")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    (opacity, fill_opacity, stroke_opacity)
}

fn find_element_by_id(doc: &Document, target_id: &str) -> Option<rix_dom::NodeId> {
    doc.get_element_by_id(target_id)
}

#[allow(clippy::too_many_arguments)]
fn render_svg_element_or_use(
    doc: &Document,
    element_id: rix_dom::NodeId,
    svg_root: rix_dom::NodeId,
    origin_x: f32,
    origin_y: f32,
    min_x: f32,
    min_y: f32,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
    box_color: CssColor,
    box_fill: Option<CssColor>,
    box_stroke: Option<CssColor>,
    box_stroke_width: f32,
    current_z: i32,
    list: &mut DisplayList,
    is_use_instance: bool,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let Some(NodeData::Element(elem)) = doc.get(element_id) else {
        return;
    };
    if !is_svg_element_visible(doc, element_id, svg_root, is_use_instance) {
        return;
    }
    let tag = elem.name.local.as_str();

    if tag == "use" {
        let href = elem
            .get_attribute("href")
            .or_else(|| elem.get_attribute("xlink:href"))
            .or_else(|| elem.get_attribute_ns(rix_dom::svg::XLINK_NAMESPACE, "href"));
        if let Some(href) = href {
            let target_id = href.trim_start_matches('#');
            if let Some(target_node) = find_element_by_id(doc, target_id) {
                let use_x = elem
                    .get_attribute("x")
                    .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
                    .unwrap_or(0.0);
                let use_y = elem
                    .get_attribute("y")
                    .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
                    .unwrap_or(0.0);
                render_svg_element_or_use(
                    doc,
                    target_node,
                    svg_root,
                    origin_x,
                    origin_y,
                    min_x,
                    min_y,
                    scale_x,
                    scale_y,
                    offset_x + use_x,
                    offset_y + use_y,
                    box_color,
                    box_fill,
                    box_stroke,
                    box_stroke_width,
                    current_z,
                    list,
                    true,
                    depth + 1,
                );
            }
        }
        return;
    }

    if let Some(d) = rix_dom::svg::svg_element_to_path_d(elem) {
        let mut fill = resolve_svg_fill(doc, element_id, svg_root, box_color, box_fill);
        let mut stroke = resolve_svg_stroke(doc, element_id, svg_root, box_color, box_stroke);
        let stroke_width = resolve_svg_stroke_width(doc, element_id, svg_root, box_stroke_width);
        let fill_rule = resolve_svg_fill_rule(doc, element_id, svg_root);
        let (opacity, fill_opacity, stroke_opacity) =
            resolve_svg_opacities(doc, element_id, svg_root);

        if let Some(fill_color) = &mut fill {
            fill_color.a *= opacity * fill_opacity;
        }
        if let Some(stroke_color) = &mut stroke {
            stroke_color.a *= opacity * stroke_opacity;
        }

        list.push(
            DrawCommand::DrawPath {
                d,
                x: origin_x - min_x * scale_x + offset_x * scale_x,
                y: origin_y - min_y * scale_y + offset_y * scale_y,
                scale_x,
                scale_y,
                fill,
                fill_rule,
                stroke,
                stroke_width,
            },
            current_z,
        );
    } else if tag == "g" || tag == "svg" || tag == "symbol" {
        let (child_scale_x, child_scale_y, child_min_x, child_min_y) = if tag == "symbol" || tag == "svg" {
            if let Some(vb) = elem
                .get_attribute("viewBox")
                .or_else(|| elem.get_attribute("viewbox"))
                .and_then(rix_dom::svg::SvgViewBox::parse)
            {
                (
                    if vb.width > 0.0 { scale_x } else { 1.0 },
                    if vb.height > 0.0 { scale_y } else { 1.0 },
                    vb.min_x,
                    vb.min_y,
                )
            } else {
                (scale_x, scale_y, min_x, min_y)
            }
        } else {
            (scale_x, scale_y, min_x, min_y)
        };
        for child in doc.children(element_id) {
            render_svg_element_or_use(
                doc,
                child,
                svg_root,
                origin_x,
                origin_y,
                child_min_x,
                child_min_y,
                child_scale_x,
                child_scale_y,
                offset_x,
                offset_y,
                box_color,
                box_fill,
                box_stroke,
                box_stroke_width,
                current_z,
                list,
                is_use_instance,
                depth + 1,
            );
        }
    }
}

fn char_byte_offset(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len())
}

fn line_boundary_x(line: &rix_text::LayoutLine, byte_offset: usize) -> f32 {
    line.glyphs
        .iter()
        .find(|glyph| glyph.cluster as usize >= byte_offset)
        .map(|glyph| glyph.x)
        .unwrap_or_else(|| line.glyphs.first().map(|glyph| glyph.x).unwrap_or(0.0) + line.width)
}

fn text_caret_rect(
    layout: &rix_text::ParagraphLayout,
    text: &str,
    character: usize,
    origin: Rect,
) -> Option<Rect> {
    let byte_offset = char_byte_offset(text, character);
    let line = layout
        .lines
        .iter()
        .find(|line| {
            line.glyphs
                .iter()
                .any(|glyph| glyph.cluster as usize >= byte_offset)
        })
        .or_else(|| layout.lines.last());
    if let Some(line) = line {
        let caret_h = if line.height > 0.0 { line.height } else { 18.0 };
        Some(Rect::new(
            origin.x + line_boundary_x(line, byte_offset),
            origin.y + line.y,
            2.0,
            caret_h,
        ))
    } else {
        let caret_h = if origin.height > 4.0 { origin.height.min(22.0) } else { 18.0 };
        let caret_y = origin.y + (origin.height - caret_h).max(0.0) / 2.0;
        Some(Rect::new(origin.x, caret_y, 2.0, caret_h))
    }
}

fn text_selection_rects(
    layout: &rix_text::ParagraphLayout,
    text: &str,
    start: usize,
    end: usize,
    origin: Rect,
) -> Vec<Rect> {
    let (start, end) = (start.min(end), start.max(end));
    let start = char_byte_offset(text, start);
    let end = char_byte_offset(text, end);
    if start == end {
        return Vec::new();
    }
    layout
        .lines
        .iter()
        .filter_map(|line| {
            let selected: Vec<_> = line
                .glyphs
                .iter()
                .filter(|glyph| glyph.cluster as usize >= start && (glyph.cluster as usize) < end)
                .collect();
            if selected.is_empty() {
                return None;
            }
            let x0 = selected
                .iter()
                .map(|glyph| glyph.x)
                .fold(f32::INFINITY, f32::min);
            let x1 = line_boundary_x(line, end);
            Some(Rect::new(
                origin.x + x0.min(x1),
                origin.y + line.y,
                (x1 - x0).abs(),
                line.height,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rix_layout::taffy_adapter::TaffyLayoutEngine;
    use rix_layout::tree::LayoutTree;
    use rix_text::font_loader::FontDatabase;

    #[test]
    fn test_svg_symbol_use_rendering() {
        let html = r##"
            <html>
                <body>
                    <svg viewBox="0 0 100 100">
                        <symbol id="icon-search" viewBox="0 0 24 24">
                            <path d="M 0 0 L 24 24 Z" fill="red" />
                        </symbol>
                        <use href="#icon-search" x="5" y="10" />
                    </svg>
                </body>
            </html>
        "##;
        let doc = rix_html::parse_html(html);
        let ua_index = rix_css::rule_index::RuleIndex::default_ua();
        let author_index = rix_css::rule_index::RuleIndex::default();
        let mut styles = std::collections::HashMap::new();
        let ctx = rix_css::computed::LayoutContext {
            root_font_size: 16.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
        };
        rix_css::computed::compute_tree_styles(
            &doc,
            doc.root(),
            None,
            &ua_index,
            &author_index,
            &ctx,
            &mut styles,
        );
        let mut tree = LayoutTree::from_dom(&doc, doc.root(), &styles);
        let font_db = FontDatabase::new();
        let _ = TaffyLayoutEngine::new().compute_layout_with_fonts(&mut tree, 800.0, 600.0, &font_db);
        let list = DisplayList::build_from_layout_with_dom(&tree, Some(&doc), &font_db);

        let draw_paths: Vec<_> = list
            .items
            .iter()
            .filter_map(|item| match &item.command {
                DrawCommand::DrawPath { d, fill, .. } => Some((d.as_str(), *fill)),
                _ => None,
            })
            .collect();

        assert_eq!(
            draw_paths.len(),
            1,
            "Exactly one path from <use> referencing <symbol> should be rendered"
        );
        assert_eq!(draw_paths[0].0, "M 0 0 L 24 24 Z");
        assert!(draw_paths[0].1.is_some(), "Fill color must be resolved");
    }

    #[test]
    fn test_display_list_children_traversal_and_z_index_sorting() {
        let html = r##"
            <html>
                <body>
                    <div id="container">
                        <div id="child1" style="background-color: red; z-index: 10;"></div>
                        <div id="child2" style="background-color: blue; z-index: 2;"></div>
                        <div id="child3" style="background-color: green; z-index: 5;"></div>
                    </div>
                </body>
            </html>
        "##;
        let doc = rix_html::parse_html(html);
        let ua_index = rix_css::rule_index::RuleIndex::default_ua();
        let author_index = rix_css::rule_index::RuleIndex::default();
        let mut styles = std::collections::HashMap::new();
        let ctx = rix_css::computed::LayoutContext {
            root_font_size: 16.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
        };
        rix_css::computed::compute_tree_styles(
            &doc,
            doc.root(),
            None,
            &ua_index,
            &author_index,
            &ctx,
            &mut styles,
        );
        let mut tree = LayoutTree::from_dom(&doc, doc.root(), &styles);
        let font_db = FontDatabase::new();
        let _ = TaffyLayoutEngine::new().compute_layout_with_fonts(&mut tree, 800.0, 600.0, &font_db);
        let list = DisplayList::build_from_layout_with_dom(&tree, Some(&doc), &font_db);

        let colors: Vec<_> = list
            .items
            .iter()
            .filter_map(|item| match &item.command {
                DrawCommand::DrawRect { color, .. } if color.a > 0.5 => Some(*color),
                _ => None,
            })
            .collect();

        assert!(colors.len() >= 3);
        let blue = CssColor::parse("blue").unwrap();
        let green = CssColor::parse("green").unwrap();
        let red = CssColor::parse("red").unwrap();

        let blue_idx = colors.iter().position(|&c| c == blue).unwrap();
        let green_idx = colors.iter().position(|&c| c == green).unwrap();
        let red_idx = colors.iter().position(|&c| c == red).unwrap();

        assert!(blue_idx < green_idx, "Blue (z=2) must render before Green (z=5)");
        assert!(green_idx < red_idx, "Green (z=5) must render before Red (z=10)");
    }
}
