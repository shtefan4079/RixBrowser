use super::*;

pub(super) fn control_value(doc: &Document, node: NodeId) -> Option<String> {
    let NodeData::Element(element) = doc.get(node)? else {
        return None;
    };
    element
        .get_attribute("value")
        .map(str::to_string)
        .or_else(|| (element.name.local.as_str() == "textarea").then(|| doc.inner_text(node)))
}

pub(super) fn apply_page_image_resources(page: &mut PageContent, fonts: &FontDatabase) {
    let base = document_base_url(&page.document, &page.url);
    let mut layout_changed = false;
    for node in page.document.descendants_with_shadow_roots(page.document.root()) {
        let Some(NodeData::Element(element)) = page.document.get(node) else {
            continue;
        };
        let source = match element.name.local.as_str() {
            "img" => rix_dom::select_image_source(
                element.get_attribute("src"),
                element.get_attribute("srcset"),
            ),
            "video" => element.get_attribute("poster"),
            _ => continue,
        };
        if let Some((url, _)) = source.and_then(|s| resolve_page_image(&base, s)) {
            if let Some(image) = page.images.get(&url) {
                layout_changed |= page.layout_tree.set_image_intrinsic_size(
                    node,
                    image.data.width as f32,
                    image.data.height as f32,
                );
            }
        }
    }
    if layout_changed {
        let (width, height) = page.layout_viewport;
        let _ = TaffyLayoutEngine::new().compute_layout_with_fonts(
            &mut page.layout_tree,
            width,
            height,
            fonts,
        );
        page.display_list =
            DisplayList::build_from_layout_with_dom(&page.layout_tree, Some(&page.document), fonts);
    }
    bind_page_image_resources(page);
}

pub(super) fn bind_page_image_resources(page: &mut PageContent) {
    let base = document_base_url(&page.document, &page.url);
    for item in &mut page.display_list.items {
        if let DrawCommand::DrawImage {
            rect,
            src,
            border_radius,
            object_fit,
        } = &item.command
        {
            if let Some((url, _)) = resolve_page_image(&base, src) {
                if let Some(image) = page.images.get(&url) {
                    item.command = DrawCommand::DrawImageResource {
                        rect: *rect,
                        image: image.clone(),
                        border_radius: *border_radius,
                        object_fit: *object_fit,
                    };
                }
            }
        }
    }
}

pub(super) enum PageImageSource {
    Network(RixUrl),
    Inline(String),
}

pub(super) fn resolve_page_image(base: &str, source: &str) -> Option<(String, PageImageSource)> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    if source.starts_with("data:image/") && source.len() <= MAX_RESOURCE_BYTES {
        Some((source.into(), PageImageSource::Inline(source.into())))
    } else {
        resolve_external_resource(base, source)
            .map(|(url, target)| (url, PageImageSource::Network(target)))
    }
}

pub(super) fn translated_rect(
    rect: rix_layout::box_model::Rect,
    x: f32,
    y: f32,
) -> rix_layout::box_model::Rect {
    rix_layout::box_model::Rect::new(rect.x + x, rect.y + y, rect.width, rect.height)
}

pub(super) fn translate_page_display_list(
    source: &DisplayList,
    bounds: rix_layout::box_model::Rect,
    scroll_x: f32,
    scroll_y: f32,
) -> DisplayList {
    use rix_render::display_list::DrawCommand;

    let mut translated = DisplayList::new();
    translated.push(DrawCommand::PushClip { rect: bounds }, i32::MIN);
    let scrolled_x = bounds.x - scroll_x;
    let scrolled_y = bounds.y - scroll_y;
    for item in &source.items {
        let (offset_x, offset_y) = if item.viewport_anchored {
            (bounds.x, bounds.y)
        } else {
            (scrolled_x, scrolled_y)
        };
        let command = match &item.command {
            DrawCommand::DrawRect {
                rect,
                color,
                border_radius,
                corner_radii,
            } => DrawCommand::DrawRect {
                rect: translated_rect(*rect, offset_x, offset_y),
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
                rect: translated_rect(*rect, offset_x, offset_y),
                clip_rect: translated_rect(*clip_rect, offset_x, offset_y),
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
            } => DrawCommand::DrawBorder {
                rect: translated_rect(*rect, offset_x, offset_y),
                color: *color,
                width: *width,
                border_radius: *border_radius,
                corner_radii: *corner_radii,
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
                rect: translated_rect(*rect, offset_x, offset_y),
                src: src.clone(),
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
                rect: translated_rect(*rect, offset_x, offset_y),
                color: *color,
                blur_radius: *blur_radius,
                offset_x: *shadow_x,
                offset_y: *shadow_y,
                border_radius: *border_radius,
                spread: *spread,
            },
            DrawCommand::DrawImageResource {
                rect,
                image,
                border_radius,
                object_fit,
            } => DrawCommand::DrawImageResource {
                rect: translated_rect(*rect, offset_x, offset_y),
                image: image.clone(),
                border_radius: *border_radius,
                object_fit: *object_fit,
            },
            DrawCommand::PushClip { rect } => DrawCommand::PushClip {
                rect: translated_rect(*rect, offset_x, offset_y),
            },
            DrawCommand::PopClip => DrawCommand::PopClip,
            DrawCommand::PushOpacity { opacity } => DrawCommand::PushOpacity { opacity: *opacity },
            DrawCommand::PopOpacity => DrawCommand::PopOpacity,
        };
        translated.push(command, item.z_index);
    }
    translated.push(DrawCommand::PopClip, i32::MAX);
    translated
}
