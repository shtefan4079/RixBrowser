
use crate::display_list::{DisplayList, DrawCommand, ObjectFit};
use rix_css::color::CssColor;
use rix_text::font_loader::FontDatabase;
use rix_text::layout::LayoutGlyph;
use rix_text::shaping::TextShaper;
use std::collections::HashMap;
use std::sync::OnceLock;
use vello::kurbo::{Affine, BezPath, Rect, RoundedRect, RoundedRectRadii, Shape, Stroke};
use vello::peniko::{
    BlendMode, Blob, Color, Fill, FontData as PenikoFontData, Gradient, ImageAlphaType, ImageBrush,
    ImageData as PenikoImageData, ImageFormat, InterpolationAlphaSpace,
};
use vello::{Glyph, Scene};

/// Proportionally scales corner radii according to W3C CSS Backgrounds and Borders Level 3 §5.5:
/// "Corner curves must not overlap: When the sum of any two adjacent border radii
/// exceeds the size of the border box, UAs must proportionally reduce the used values
/// of all border radii until none of them overlap."
#[inline]
pub(crate) fn scale_corner_radii(width: f64, height: f64, radii: [f32; 4]) -> [f64; 4] {
    let [tl, tr, br, bl] = radii.map(|r| (r as f64).max(0.0));
    let mut f = 1.0f64;
    let top_sum = tl + tr;
    if top_sum > width && top_sum > 0.0 {
        f = f.min(width / top_sum);
    }
    let bottom_sum = bl + br;
    if bottom_sum > width && bottom_sum > 0.0 {
        f = f.min(width / bottom_sum);
    }
    let left_sum = tl + bl;
    if left_sum > height && left_sum > 0.0 {
        f = f.min(height / left_sum);
    }
    let right_sum = tr + br;
    if right_sum > height && right_sum > 0.0 {
        f = f.min(height / right_sum);
    }
    [tl * f, tr * f, br * f, bl * f]
}

pub struct VelloSceneBuilder;

impl VelloSceneBuilder {
    pub fn build_scene(display_list: &DisplayList) -> Scene {
        Self::build_scene_with_fonts(display_list, default_font_db())
    }

    pub fn build_scene_with_fonts(display_list: &DisplayList, font_db: &FontDatabase) -> Scene {
        let mut scene = Scene::new();
        Self::render_to_scene_with_fonts(display_list, &mut scene, Affine::IDENTITY, font_db);
        scene
    }

    pub fn render_to_scene(display_list: &DisplayList, scene: &mut Scene, base_transform: Affine) {
        Self::render_to_scene_with_fonts(display_list, scene, base_transform, default_font_db());
    }

    pub fn render_to_scene_with_fonts(
        display_list: &DisplayList,
        scene: &mut Scene,
        base_transform: Affine,
        font_db: &FontDatabase,
    ) {
        let current_transform = base_transform;
        let shaper = TextShaper::new(font_db);
        let mut clip_bounds: Vec<rix_layout::box_model::Rect> = Vec::new();

        for item in &display_list.items {
            match &item.command {
                DrawCommand::DrawRect {
                    rect,
                    color,
                    border_radius,
                    corner_radii,
                } => {
                    let fill_color = Self::convert_color(*color);
                    if rect.width <= 0.0 || rect.height <= 0.0 {
                        continue;
                    }
                    let x0 = rect.x as f64;
                    let y0 = rect.y as f64;
                    let x1 = (rect.x + rect.width) as f64;
                    let y1 = (rect.y + rect.height) as f64;
                    let effective_radii = if corner_radii.iter().any(|&r| r > 0.0) {
                        *corner_radii
                    } else if *border_radius > 0.0 {
                        [*border_radius; 4]
                    } else {
                        [0.0; 4]
                    };
                    if effective_radii.iter().any(|&r| r > 0.0) {
                        let [tl, tr, br, bl] =
                            scale_corner_radii(rect.width as f64, rect.height as f64, effective_radii);
                        let rounded = RoundedRect::new(
                            x0,
                            y0,
                            x1,
                            y1,
                            RoundedRectRadii::new(tl, tr, br, bl),
                        );
                        scene.fill(Fill::NonZero, current_transform, fill_color, None, &rounded);
                    } else {
                        let r = Rect::new(x0, y0, x1, y1);
                        scene.fill(Fill::NonZero, current_transform, fill_color, None, &r);
                    }
                }
                DrawCommand::DrawLinearGradient {
                    rect,
                    clip_rect,
                    repeat_x,
                    repeat_y,
                    gradient,
                    border_radius,
                } => {
                    if rect.width <= 0.0
                        || rect.height <= 0.0
                        || clip_rect.width <= 0.0
                        || clip_rect.height <= 0.0
                    {
                        continue;
                    }
                    let visible_clip = clip_bounds
                        .last()
                        .copied()
                        .map(|parent| intersect_clip_bounds(*clip_rect, parent))
                        .unwrap_or(*clip_rect);
                    if visible_clip.width <= 0.0 || visible_clip.height <= 0.0 {
                        continue;
                    }
                    let x = f64::from(rect.x);
                    let y = f64::from(rect.y);
                    let clip_x = f64::from(clip_rect.x);
                    let clip_y = f64::from(clip_rect.y);
                    let stops = gradient
                        .stops
                        .iter()
                        .map(|(offset, color)| (*offset, Self::convert_color(*color)))
                        .collect::<Vec<_>>();
                    let Some((first_x, last_x)) = background_tile_range(
                        rect.x,
                        rect.width,
                        visible_clip.x,
                        visible_clip.width,
                        *repeat_x,
                    ) else {
                        continue;
                    };
                    let Some((first_y, last_y)) = background_tile_range(
                        rect.y,
                        rect.height,
                        visible_clip.y,
                        visible_clip.height,
                        *repeat_y,
                    ) else {
                        continue;
                    };
                    let clip_radius = f64::from(*border_radius)
                        .clamp(0.0, f64::from(clip_rect.width.min(clip_rect.height)) / 2.0);
                    let clip_shape = RoundedRect::new(
                        clip_x,
                        clip_y,
                        clip_x + f64::from(clip_rect.width),
                        clip_y + f64::from(clip_rect.height),
                        clip_radius,
                    );
                    scene.push_clip_layer(Fill::NonZero, current_transform, &clip_shape);
                    for tile_y in first_y..=last_y {
                        let tile_y = y + tile_y as f64 * f64::from(rect.height);
                        for tile_x in first_x..=last_x {
                            let tile_x = x + tile_x as f64 * f64::from(rect.width);
                            let brush = Gradient::new_linear(
                                (tile_x + gradient.start.0, tile_y + gradient.start.1),
                                (tile_x + gradient.end.0, tile_y + gradient.end.1),
                            )
                            .with_interpolation_alpha_space(InterpolationAlphaSpace::Premultiplied)
                            .with_stops(stops.as_slice());
                            let image_rect = Rect::new(
                                tile_x,
                                tile_y,
                                tile_x + f64::from(rect.width),
                                tile_y + f64::from(rect.height),
                            );
                            scene.fill(Fill::NonZero, current_transform, &brush, None, &image_rect);
                        }
                    }
                    scene.pop_layer();
                }
                DrawCommand::DrawShadow {
                    rect,
                    color,
                    blur_radius,
                    offset_x,
                    offset_y,
                    border_radius,
                    spread,
                } => {
                    if rect.width <= 0.0 || rect.height <= 0.0 {
                        continue;
                    }
                    if color.a <= 0.001 {
                        continue;
                    }
                    let shadow_color = Self::convert_color(*color);
                    let sx = rect.x + offset_x - spread;
                    let sy = rect.y + offset_y - spread;
                    let sw = (rect.width + spread * 2.0).max(1.0);
                    let sh = (rect.height + spread * 2.0).max(1.0);
                    let radius =
                        (*border_radius + *spread).clamp(0.0, (sw.min(sh) / 2.0).max(0.0)) as f64;
                    let std_dev = (*blur_radius as f64 / 2.0).max(0.0);
                    let shadow_rect =
                        Rect::new(sx as f64, sy as f64, (sx + sw) as f64, (sy + sh) as f64);
                    if std_dev > 0.01 {
                        scene.draw_blurred_rounded_rect(
                            current_transform,
                            shadow_rect,
                            shadow_color,
                            radius,
                            std_dev,
                        );
                    } else {
                        let rounded = RoundedRect::new(
                            sx as f64,
                            sy as f64,
                            (sx + sw) as f64,
                            (sy + sh) as f64,
                            radius,
                        );
                        scene.fill(
                            Fill::NonZero,
                            current_transform,
                            shadow_color,
                            None,
                            &rounded,
                        );
                    }
                }
                DrawCommand::DrawBorder {
                    rect,
                    color,
                    width,
                    border_radius,
                    corner_radii,
                    widths,
                } => {
                    if rect.width <= 0.0 || rect.height <= 0.0 || *width <= 0.0 {
                        continue;
                    }
                    let stroke_color = Self::convert_color(*color);
                    let is_uniform_width = widths.iter().all(|&w| (w - widths[0]).abs() < 1e-4);
                    let effective_radii = if corner_radii.iter().any(|&r| r > 0.0) {
                        *corner_radii
                    } else if *border_radius > 0.0 {
                        [*border_radius; 4]
                    } else {
                        [0.0; 4]
                    };
                    let has_radii = effective_radii.iter().any(|&r| r > 0.0);

                    if is_uniform_width {
                        let stroke = Stroke::new(*width as f64);
                        let x0 = rect.x as f64;
                        let y0 = rect.y as f64;
                        let x1 = (rect.x + rect.width) as f64;
                        let y1 = (rect.y + rect.height) as f64;

                        if has_radii {
                            let [tl, tr, br, bl] = scale_corner_radii(
                                rect.width as f64,
                                rect.height as f64,
                                effective_radii,
                            );
                            let rounded = RoundedRect::new(
                                x0,
                                y0,
                                x1,
                                y1,
                                RoundedRectRadii::new(tl, tr, br, bl),
                            );
                            scene.stroke(&stroke, current_transform, stroke_color, None, &rounded);
                        } else {
                            let r = Rect::new(x0, y0, x1, y1);
                            scene.stroke(&stroke, current_transform, stroke_color, None, &r);
                        }
                    } else {
                        let x0 = rect.x as f64;
                        let y0 = rect.y as f64;
                        let x1 = (rect.x + rect.width) as f64;
                        let y1 = (rect.y + rect.height) as f64;
                        let [tl, tr, br, bl] = scale_corner_radii(
                            rect.width as f64,
                            rect.height as f64,
                            effective_radii,
                        );
                        let outer_rounded = RoundedRect::new(
                            x0,
                            y0,
                            x1,
                            y1,
                            RoundedRectRadii::new(tl, tr, br, bl),
                        );

                        let top_w = widths[0] as f64;
                        let right_w = widths[1] as f64;
                        let bottom_w = widths[2] as f64;
                        let left_w = widths[3] as f64;

                        let in_x0 = x0 + left_w;
                        let in_y0 = y0 + top_w;
                        let in_x1 = (x1 - right_w).max(in_x0);
                        let in_y1 = (y1 - bottom_w).max(in_y0);

                        let in_tl = (tl - top_w.max(left_w)).max(0.0);
                        let in_tr = (tr - top_w.max(right_w)).max(0.0);
                        let in_br = (br - bottom_w.max(right_w)).max(0.0);
                        let in_bl = (bl - bottom_w.max(left_w)).max(0.0);
                        let inner_rounded = RoundedRect::new(
                            in_x0,
                            in_y0,
                            in_x1,
                            in_y1,
                            RoundedRectRadii::new(in_tl, in_tr, in_br, in_bl),
                        );

                        let mut path = outer_rounded.to_path(0.1);
                        path.extend(inner_rounded.path_elements(0.1));
                        scene.fill(Fill::EvenOdd, current_transform, stroke_color, None, &path);
                    }
                }
                DrawCommand::DrawText {
                    text,
                    font_id,
                    font_size,
                    color,
                    x,
                    y,
                } => {
                    if text.is_empty() || *font_size <= 0.0 {
                        continue;
                    }
                    Self::draw_shaped_text(
                        scene,
                        &shaper,
                        font_db,
                        text,
                        *font_id,
                        *font_size,
                        *color,
                        *x,
                        *y,
                        current_transform,
                    );
                }
                DrawCommand::DrawGlyphs { glyphs, x, y } => {
                    Self::draw_positioned_glyphs(scene, font_db, glyphs, *x, *y, current_transform);
                }
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
                } => {
                    if d.trim().is_empty() {
                        continue;
                    }
                    let Some(base_path) = cached_svg_path(d) else {
                        continue;
                    };
                    let path_transform = current_transform
                        * Affine::translate((*x as f64, *y as f64))
                        * Affine::scale_non_uniform(*scale_x as f64, *scale_y as f64);
                    if let Some(fill_color) = fill {
                        let brush = Self::convert_color(*fill_color);
                        let vello_fill = match fill_rule {
                            rix_dom::svg::SvgFillRule::EvenOdd => Fill::EvenOdd,
                            rix_dom::svg::SvgFillRule::NonZero => Fill::NonZero,
                        };
                        scene.fill(vello_fill, path_transform, brush, None, &*base_path);
                    }
                    if let Some(stroke_color) = stroke {
                        let st = Stroke::new(*stroke_width as f64);
                        let brush = Self::convert_color(*stroke_color);
                        scene.stroke(&st, path_transform, brush, None, &*base_path);
                    }
                }
                DrawCommand::DrawImage {
                    rect,
                    src,
                    border_radius,
                    object_fit,
                } => {
                    if rect.width <= 0.0 || rect.height <= 0.0 {
                        continue;
                    }
                    Self::draw_gpu_image(
                        scene,
                        rect,
                        src,
                        *border_radius,
                        *object_fit,
                        current_transform,
                    );
                }
                DrawCommand::DrawImageResource {
                    rect,
                    image,
                    border_radius,
                    object_fit,
                } => {
                    Self::draw_decoded_image(
                        scene,
                        rect,
                        image.data.clone(),
                        *border_radius,
                        *object_fit,
                        current_transform,
                    );
                }
                DrawCommand::PushClip { rect, corner_radii } => {
                    let current = clip_bounds.last().copied();
                    clip_bounds.push(
                        current
                            .map(|parent| intersect_clip_bounds(*rect, parent))
                            .unwrap_or(*rect),
                    );
                    let x0 = rect.x as f64;
                    let y0 = rect.y as f64;
                    let x1 = (rect.x + rect.width) as f64;
                    let y1 = (rect.y + rect.height) as f64;
                    let has_radii = corner_radii.iter().any(|&r| r > 0.0);
                    if has_radii {
                        let [tl, tr, br, bl] =
                            scale_corner_radii(rect.width as f64, rect.height as f64, *corner_radii);
                        let rounded = RoundedRect::new(
                            x0,
                            y0,
                            x1,
                            y1,
                            RoundedRectRadii::new(tl, tr, br, bl),
                        );
                        scene.push_clip_layer(Fill::NonZero, current_transform, &rounded);
                    } else {
                        let clip_rect = Rect::new(x0, y0, x1, y1);
                        scene.push_clip_layer(Fill::NonZero, current_transform, &clip_rect);
                    }
                }
                DrawCommand::PopClip => {
                    scene.pop_layer();
                    clip_bounds.pop();
                }
                DrawCommand::PushOpacity { opacity } => {
                    let alpha = opacity.clamp(0.0, 1.0);
                    let unbounded = Rect::new(-1e6, -1e6, 1e6, 1e6);
                    scene.push_layer(
                        Fill::NonZero,
                        BlendMode::default(),
                        alpha,
                        current_transform,
                        &unbounded,
                    );
                }
                DrawCommand::PopOpacity => {
                    scene.pop_layer();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_shaped_text(
        scene: &mut Scene,
        shaper: &TextShaper<'_>,
        font_db: &FontDatabase,
        text: &str,
        font_id: rix_text::font_loader::FontId,
        font_size: f32,
        color: CssColor,
        x: f32,
        y: f32,
        transform: Affine,
    ) {
        let effective_id = if font_db.get_font(font_id).is_some() {
            Some(font_id)
        } else {
            font_db.default_sans_serif
        };
        let Some(resolved_id) = effective_id else {
            return;
        };
        let runs = shaper.shape_text(text, resolved_id, font_size);
        if runs.is_empty() {
            return;
        }

        let mut pen_x = 0.0;
        let mut glyphs = Vec::new();
        for run in runs {
            for g in &run.glyphs {
                let gx = pen_x + g.x_offset;
                glyphs.push(LayoutGlyph {
                    glyph_id: g.glyph_id,
                    x: gx,
                    advance: g.x_advance,
                    y: -g.y_offset,
                    cluster: g.cluster,
                    source: None,
                    paint: true,
                    font_id: run.font_id,
                    font_size,
                    color: [color.r, color.g, color.b, color.a],
                });
                pen_x += g.x_advance;
            }
        }
        Self::draw_positioned_glyphs(scene, font_db, &glyphs, x, y, transform);
    }

    fn draw_positioned_glyphs(
        scene: &mut Scene,
        font_db: &FontDatabase,
        glyphs: &[LayoutGlyph],
        x: f32,
        y: f32,
        transform: Affine,
    ) {
        let mut offset = 0;
        while let Some(first) = glyphs.get(offset) {
            let mut end = offset + 1;
            while glyphs.get(end).is_some_and(|next| {
                next.font_id == first.font_id
                    && next.font_size == first.font_size
                    && next.color == first.color
            }) {
                end += 1;
            }
            let Some(font) = font_db.get_font(first.font_id) else {
                offset = end;
                continue;
            };
            let peniko_font = PenikoFontData::new(font.blob().clone(), font.face_index);
            let [r, g, b, a] = first.color;
            scene
                .draw_glyphs(&peniko_font)
                .font_size(first.font_size)
                .normalized_coords(&font.normalized_coords)
                .hint(true)
                .transform(transform)
                .brush(Self::convert_color(CssColor { r, g, b, a }))
                .draw(
                    Fill::NonZero,
                    glyphs[offset..end].iter().map(|glyph| Glyph {
                        id: glyph.glyph_id,
                        x: x + glyph.x,
                        y: y + glyph.y,
                    }),
                );
            offset = end;
        }
    }

    fn draw_gpu_image(
        scene: &mut Scene,
        rect: &rix_layout::box_model::Rect,
        src: &str,
        border_radius: f32,
        object_fit: ObjectFit,
        transform: Affine,
    ) {
        let Some(image) = cached_gpu_image_data(src) else {
            tracing::debug!(
                "VelloSceneBuilder: skipping undecodable image src (len={})",
                src.len()
            );
            return;
        };
        Self::draw_decoded_image(scene, rect, image, border_radius, object_fit, transform);
    }

    fn draw_decoded_image(
        scene: &mut Scene,
        rect: &rix_layout::box_model::Rect,
        image: PenikoImageData,
        border_radius: f32,
        object_fit: ObjectFit,
        transform: Affine,
    ) {
        if rect.width <= 0.0 || rect.height <= 0.0 || image.width == 0 || image.height == 0 {
            return;
        }
        let img_w = image.width;
        let img_h = image.height;
        let brush = ImageBrush::new(image).with_quality(vello::peniko::ImageQuality::High);
        let geometry = object_fit_geometry(
            object_fit,
            rect.x as f64,
            rect.y as f64,
            rect.width as f64,
            rect.height as f64,
            img_w as f64,
            img_h as f64,
        );

        let image_transform = transform
            * Affine::translate((geometry.offset_x, geometry.offset_y))
            * Affine::scale_non_uniform(geometry.scale_x, geometry.scale_y);

        if border_radius > 0.0 {
            let radius =
                (border_radius as f64).clamp(0.0, (rect.width.min(rect.height) / 2.0) as f64);
            let rounded = RoundedRect::new(
                rect.x as f64,
                rect.y as f64,
                (rect.x + rect.width) as f64,
                (rect.y + rect.height) as f64,
                radius,
            );
            scene.push_clip_layer(Fill::NonZero, transform, &rounded);
            scene.draw_image(&brush, image_transform);
            scene.pop_layer();
        } else if geometry.needs_clip {
            let clip_rect = Rect::new(
                rect.x as f64,
                rect.y as f64,
                (rect.x + rect.width) as f64,
                (rect.y + rect.height) as f64,
            );
            scene.push_clip_layer(Fill::NonZero, transform, &clip_rect);
            scene.draw_image(&brush, image_transform);
            scene.pop_layer();
        } else {
            scene.draw_image(&brush, image_transform);
        }
    }

    fn convert_color(color: CssColor) -> Color {
        Color::from_rgba8(
            (color.r * 255.0).clamp(0.0, 255.0) as u8,
            (color.g * 255.0).clamp(0.0, 255.0) as u8,
            (color.b * 255.0).clamp(0.0, 255.0) as u8,
            (color.a * 255.0).clamp(0.0, 255.0) as u8,
        )
    }
}

fn background_tile_range(
    image_origin: f32,
    image_extent: f32,
    clip_origin: f32,
    clip_extent: f32,
    repeat: bool,
) -> Option<(i64, i64)> {
    let (image_origin, image_extent, clip_origin, clip_extent) = (
        f64::from(image_origin),
        f64::from(image_extent),
        f64::from(clip_origin),
        f64::from(clip_extent),
    );
    if !image_origin.is_finite()
        || !image_extent.is_finite()
        || !clip_origin.is_finite()
        || !clip_extent.is_finite()
        || image_extent <= 0.0
        || clip_extent <= 0.0
    {
        return None;
    }

    let clip_end = clip_origin + clip_extent;
    if !repeat {
        let image_end = image_origin + image_extent;
        return (image_origin < clip_end && clip_origin < image_end).then_some((0, 0));
    }

    let first = ((clip_origin - image_origin) / image_extent).floor();
    let last = ((clip_end - image_origin) / image_extent).ceil() - 1.0;
    if !first.is_finite()
        || !last.is_finite()
        || first < i64::MIN as f64
        || last >= i64::MAX as f64
        || first > last
    {
        return None;
    }
    Some((first as i64, last as i64))
}

fn intersect_clip_bounds(
    first: rix_layout::box_model::Rect,
    second: rix_layout::box_model::Rect,
) -> rix_layout::box_model::Rect {
    if !first.x.is_finite()
        || !first.y.is_finite()
        || !first.width.is_finite()
        || !first.height.is_finite()
        || !second.x.is_finite()
        || !second.y.is_finite()
        || !second.width.is_finite()
        || !second.height.is_finite()
    {
        return rix_layout::box_model::Rect::new(0.0, 0.0, 0.0, 0.0);
    }
    let left = first.x.max(second.x);
    let top = first.y.max(second.y);
    let right = (first.x + first.width).min(second.x + second.width);
    let bottom = (first.y + first.height).min(second.y + second.height);
    rix_layout::box_model::Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

pub(crate) fn default_font_db() -> &'static FontDatabase {
    static DB: OnceLock<FontDatabase> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = FontDatabase::new();
        db.scan_system_fonts();
        db
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ImageFitGeometry {
    scale_x: f64,
    scale_y: f64,
    offset_x: f64,
    offset_y: f64,
    needs_clip: bool,
}

fn object_fit_geometry(
    object_fit: ObjectFit,
    box_x: f64,
    box_y: f64,
    box_w: f64,
    box_h: f64,
    img_w: f64,
    img_h: f64,
) -> ImageFitGeometry {
    debug_assert!(box_w > 0.0 && box_h > 0.0 && img_w > 0.0 && img_h > 0.0);
    let centered = |scale_x: f64, scale_y: f64, needs_clip: bool| ImageFitGeometry {
        offset_x: box_x + (box_w - img_w * scale_x) / 2.0,
        offset_y: box_y + (box_h - img_h * scale_y) / 2.0,
        scale_x,
        scale_y,
        needs_clip,
    };
    match object_fit {
        ObjectFit::Fill => ImageFitGeometry {
            scale_x: box_w / img_w,
            scale_y: box_h / img_h,
            offset_x: box_x,
            offset_y: box_y,
            needs_clip: false,
        },
        ObjectFit::Contain => {
            let scale = (box_w / img_w).min(box_h / img_h);
            centered(scale, scale, false)
        }
        ObjectFit::Cover => {
            let scale = (box_w / img_w).max(box_h / img_h);
            centered(scale, scale, true)
        }
        ObjectFit::None => {
            let needs_clip = img_w > box_w || img_h > box_h;
            centered(1.0, 1.0, needs_clip)
        }
        ObjectFit::ScaleDown => {
            let scale = (box_w / img_w).min(box_h / img_h).min(1.0);
            centered(scale, scale, false)
        }
    }
}

fn cached_gpu_image_data(src: &str) -> Option<PenikoImageData> {
    static CACHE: OnceLock<std::sync::RwLock<HashMap<String, PenikoImageData>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::RwLock::new(HashMap::with_capacity(128)));
    if let Ok(reader) = cache.read() {
        if let Some(image) = reader.get(src) {
            return Some(image.clone());
        }
    }
    let (pixels, img_w, img_h) = decode_image_to_rgba(src)?;
    if img_w == 0 || img_h == 0 {
        return None;
    }
    let image = PenikoImageData {
        data: Blob::from(pixels),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: img_w,
        height: img_h,
    };
    if let Ok(mut writer) = cache.write() {
        if writer.len() >= 512 {
            let to_remove: Vec<String> = writer.keys().take(256).cloned().collect();
            for key in to_remove {
                writer.remove(&key);
            }
        }
        writer.insert(src.to_string(), image.clone());
    }
    Some(image)
}

fn decode_image_to_rgba(src: &str) -> Option<(Vec<u8>, u32, u32)> {
    if let Some(data) = decode_data_uri(src) {
        return image_bytes_to_rgba(&data);
    }

    None
}

fn decode_data_uri(src: &str) -> Option<Vec<u8>> {
    if !src.starts_with("data:image/") {
        return None;
    }
    let comma = src.find(',')?;
    let (meta, payload) = src.split_at(comma);
    let payload = &payload[1..];
    if meta.contains(";base64") {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()
    } else {
        let decoded = percent_decode(payload)?;
        Some(decoded.into_bytes())
    }
}

fn percent_decode(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = bytes.next()?;
            let lo = bytes.next()?;
            let hex = |c: u8| (c as char).to_digit(16);
            let v = (hex(hi)? << 4) | hex(lo)?;
            out.push(v as u8 as char);
        } else {
            out.push(b as char);
        }
    }
    Some(out)
}

fn image_bytes_to_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return None;
    }
    Some((rgba.into_raw(), w, h))
}

fn cached_svg_path(d: &str) -> Option<std::sync::Arc<BezPath>> {
    static CACHE: OnceLock<std::sync::RwLock<std::collections::HashMap<String, std::sync::Arc<BezPath>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::with_capacity(1024)));
    if let Ok(reader) = cache.read() {
        if let Some(path) = reader.get(d) {
            return Some(std::sync::Arc::clone(path));
        }
    }
    let parsed = std::sync::Arc::new(svg_path_to_kurbo(d, 0.0, 0.0, 1.0, 1.0)?);
    if let Ok(mut writer) = cache.write() {
        if writer.len() < 4096 {
            writer.insert(d.to_string(), std::sync::Arc::clone(&parsed));
        }
    }
    Some(parsed)
}

fn svg_path_to_kurbo(d: &str, ox: f32, oy: f32, sx: f32, sy: f32) -> Option<BezPath> {
    use svgtypes::{PathParser, PathSegment};
    let mut path = BezPath::new();
    let mut cur_x = 0.0f32;
    let mut cur_y = 0.0f32;
    let mut start_x = 0.0f32;
    let mut start_y = 0.0f32;
    let mut last_cubic_x = 0.0f32;
    let mut last_cubic_y = 0.0f32;
    let mut last_quad_x = 0.0f32;
    let mut last_quad_y = 0.0f32;
    let mut last_was_cubic = false;
    let mut last_was_quad = false;
    let mut has_content = false;

    let tx = |x: f32| (ox + x * sx) as f64;
    let ty = |y: f32| (oy + y * sy) as f64;

    for segment in PathParser::from(d).flatten() {
        match segment {
            PathSegment::MoveTo { abs, x, y } => {
                let nx = if abs { x as f32 } else { cur_x + x as f32 };
                let ny = if abs { y as f32 } else { cur_y + y as f32 };
                path.move_to((tx(nx), ty(ny)));
                cur_x = nx;
                cur_y = ny;
                start_x = nx;
                start_y = ny;
                last_was_cubic = false;
                last_was_quad = false;
                has_content = true;
            }
            PathSegment::LineTo { abs, x, y } => {
                let nx = if abs { x as f32 } else { cur_x + x as f32 };
                let ny = if abs { y as f32 } else { cur_y + y as f32 };
                path.line_to((tx(nx), ty(ny)));
                cur_x = nx;
                cur_y = ny;
                last_was_cubic = false;
                last_was_quad = false;
                has_content = true;
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                let nx = if abs { x as f32 } else { cur_x + x as f32 };
                path.line_to((tx(nx), ty(cur_y)));
                cur_x = nx;
                last_was_cubic = false;
                last_was_quad = false;
                has_content = true;
            }
            PathSegment::VerticalLineTo { abs, y } => {
                let ny = if abs { y as f32 } else { cur_y + y as f32 };
                path.line_to((tx(cur_x), ty(ny)));
                cur_y = ny;
                last_was_cubic = false;
                last_was_quad = false;
                has_content = true;
            }
            PathSegment::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let p1x = if abs { x1 as f32 } else { cur_x + x1 as f32 };
                let p1y = if abs { y1 as f32 } else { cur_y + y1 as f32 };
                let p2x = if abs { x2 as f32 } else { cur_x + x2 as f32 };
                let p2y = if abs { y2 as f32 } else { cur_y + y2 as f32 };
                let nx = if abs { x as f32 } else { cur_x + x as f32 };
                let ny = if abs { y as f32 } else { cur_y + y as f32 };
                path.curve_to((tx(p1x), ty(p1y)), (tx(p2x), ty(p2y)), (tx(nx), ty(ny)));
                cur_x = nx;
                cur_y = ny;
                last_cubic_x = p2x;
                last_cubic_y = p2y;
                last_was_cubic = true;
                last_was_quad = false;
                has_content = true;
            }
            PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
                let p1x = if last_was_cubic {
                    2.0 * cur_x - last_cubic_x
                } else {
                    cur_x
                };
                let p1y = if last_was_cubic {
                    2.0 * cur_y - last_cubic_y
                } else {
                    cur_y
                };
                let p2x = if abs { x2 as f32 } else { cur_x + x2 as f32 };
                let p2y = if abs { y2 as f32 } else { cur_y + y2 as f32 };
                let nx = if abs { x as f32 } else { cur_x + x as f32 };
                let ny = if abs { y as f32 } else { cur_y + y as f32 };
                path.curve_to((tx(p1x), ty(p1y)), (tx(p2x), ty(p2y)), (tx(nx), ty(ny)));
                cur_x = nx;
                cur_y = ny;
                last_cubic_x = p2x;
                last_cubic_y = p2y;
                last_was_cubic = true;
                last_was_quad = false;
                has_content = true;
            }
            PathSegment::Quadratic { abs, x1, y1, x, y } => {
                let p1x = if abs { x1 as f32 } else { cur_x + x1 as f32 };
                let p1y = if abs { y1 as f32 } else { cur_y + y1 as f32 };
                let nx = if abs { x as f32 } else { cur_x + x as f32 };
                let ny = if abs { y as f32 } else { cur_y + y as f32 };
                path.quad_to((tx(p1x), ty(p1y)), (tx(nx), ty(ny)));
                cur_x = nx;
                cur_y = ny;
                last_quad_x = p1x;
                last_quad_y = p1y;
                last_was_quad = true;
                last_was_cubic = false;
                has_content = true;
            }
            PathSegment::SmoothQuadratic { abs, x, y } => {
                let p1x = if last_was_quad {
                    2.0 * cur_x - last_quad_x
                } else {
                    cur_x
                };
                let p1y = if last_was_quad {
                    2.0 * cur_y - last_quad_y
                } else {
                    cur_y
                };
                let nx = if abs { x as f32 } else { cur_x + x as f32 };
                let ny = if abs { y as f32 } else { cur_y + y as f32 };
                path.quad_to((tx(p1x), ty(p1y)), (tx(nx), ty(ny)));
                cur_x = nx;
                cur_y = ny;
                last_quad_x = p1x;
                last_quad_y = p1y;
                last_was_quad = true;
                last_was_cubic = false;
                has_content = true;
            }
            PathSegment::EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                let nx = if abs { x as f32 } else { cur_x + x as f32 };
                let ny = if abs { y as f32 } else { cur_y + y as f32 };
                arc_to_kurbo(
                    &mut path,
                    cur_x,
                    cur_y,
                    rx as f32,
                    ry as f32,
                    x_axis_rotation as f32,
                    large_arc,
                    sweep,
                    nx,
                    ny,
                    ox,
                    oy,
                    sx,
                    sy,
                );
                cur_x = nx;
                cur_y = ny;
                last_was_cubic = false;
                last_was_quad = false;
                has_content = true;
            }
            PathSegment::ClosePath { .. } => {
                path.close_path();
                cur_x = start_x;
                cur_y = start_y;
                last_was_cubic = false;
                last_was_quad = false;
            }
        }
    }
    if has_content {
        Some(path)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn arc_to_kurbo(
    path: &mut BezPath,
    x0: f32,
    y0: f32,
    mut rx: f32,
    mut ry: f32,
    x_axis_rotation: f32,
    large_arc: bool,
    sweep: bool,
    x: f32,
    y: f32,
    ox: f32,
    oy: f32,
    sx: f32,
    sy: f32,
) {
    let tx = |vx: f32| (ox + vx * sx) as f64;
    let ty = |vy: f32| (oy + vy * sy) as f64;
    if (x0 - x).abs() < 1e-4 && (y0 - y).abs() < 1e-4 {
        return;
    }
    if rx.abs() < 1e-4 || ry.abs() < 1e-4 {
        path.line_to((tx(x), ty(y)));
        return;
    }
    rx = rx.abs();
    ry = ry.abs();

    let phi = x_axis_rotation.to_radians();
    let cos_phi = phi.cos();
    let sin_phi = phi.sin();
    let dx = (x0 - x) / 2.0;
    let dy = (y0 - y) / 2.0;
    let x1_p = cos_phi * dx + sin_phi * dy;
    let y1_p = -sin_phi * dx + cos_phi * dy;

    let mut rx_sq = rx * rx;
    let mut ry_sq = ry * ry;
    let x1_p_sq = x1_p * x1_p;
    let y1_p_sq = y1_p * y1_p;
    let lambda = x1_p_sq / rx_sq + y1_p_sq / ry_sq;
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
        rx_sq = rx * rx;
        ry_sq = ry * ry;
    }
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let sq = ((rx_sq * ry_sq - rx_sq * y1_p_sq - ry_sq * x1_p_sq)
        / (rx_sq * y1_p_sq + ry_sq * x1_p_sq))
        .max(0.0);
    let coef = sign * sq.sqrt();
    let cx_p = coef * (rx * y1_p / ry);
    let cy_p = coef * (-(ry * x1_p / rx));
    let cx = cos_phi * cx_p - sin_phi * cy_p + (x0 + x) / 2.0;
    let cy = sin_phi * cx_p + cos_phi * cy_p + (y0 + y) / 2.0;

    let v1x = (x1_p - cx_p) / rx;
    let v1y = (y1_p - cy_p) / ry;
    let v2x = (-x1_p - cx_p) / rx;
    let v2y = (-y1_p - cy_p) / ry;
    let theta1 = v1y.atan2(v1x);
    let mut d_theta = v2y.atan2(v2x) - theta1;
    if !sweep && d_theta > 0.0 {
        d_theta -= std::f32::consts::TAU;
    } else if sweep && d_theta < 0.0 {
        d_theta += std::f32::consts::TAU;
    }
    let segments = (d_theta.abs() / std::f32::consts::FRAC_PI_2 + 0.001).ceil() as usize;
    let d_t = d_theta / segments.max(1) as f32;
    let alpha = (d_t / 2.0).sin() * 4.0 / 3.0 / (1.0 + (d_t / 2.0).cos());
    let mut current_theta = theta1;
    let mut cur_pt_x = x0;
    let mut cur_pt_y = y0;
    for _ in 0..segments.max(1) {
        let next_theta = current_theta + d_t;
        let sin_t = current_theta.sin();
        let cos_t = current_theta.cos();
        let sin_nt = next_theta.sin();
        let cos_nt = next_theta.cos();
        let e2x = cos_phi * rx * cos_nt - sin_phi * ry * sin_nt + cx;
        let e2y = sin_phi * rx * cos_nt + cos_phi * ry * sin_nt + cy;
        let d1x = -cos_phi * rx * sin_t - sin_phi * ry * cos_t;
        let d1y = -sin_phi * rx * sin_t + cos_phi * ry * cos_t;
        let d2x = -cos_phi * rx * sin_nt - sin_phi * ry * cos_nt;
        let d2y = -sin_phi * rx * sin_nt + cos_phi * ry * cos_nt;
        let cp1x = cur_pt_x + alpha * d1x;
        let cp1y = cur_pt_y + alpha * d1y;
        let cp2x = e2x - alpha * d2x;
        let cp2y = e2y - alpha * d2y;
        path.curve_to(
            (tx(cp1x), ty(cp1y)),
            (tx(cp2x), ty(cp2y)),
            (tx(e2x), ty(e2y)),
        );
        cur_pt_x = e2x;
        cur_pt_y = e2y;
        current_theta = next_theta;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VelloRenderer;

    #[test]
    fn test_cached_svg_path_dedup() {
        let d = "M10 10 H 90 V 90 H 10 Z";
        let path1 = cached_svg_path(d).expect("path should parse");
        let path2 = cached_svg_path(d).expect("path should parse from cache");
        assert!(std::sync::Arc::ptr_eq(&path1, &path2), "subsequent lookups must return the same cached Arc");
    }

    #[test]
    fn test_rounded_clip_rendering() {
        let mut list = DisplayList::default();
        list.push(
            DrawCommand::PushClip {
                rect: rix_layout::box_model::Rect::new(20.0, 20.0, 100.0, 100.0),
                corner_radii: [20.0, 20.0, 20.0, 20.0],
            },
            0,
        );
        list.push(
            DrawCommand::DrawRect {
                rect: rix_layout::box_model::Rect::new(20.0, 20.0, 100.0, 100.0),
                color: CssColor::WHITE,
                border_radius: 0.0,
                corner_radii: [0.0; 4],
            },
            0,
        );
        list.push(DrawCommand::PopClip, 0);
        let scene = VelloSceneBuilder::build_scene(&list);
        pollster::block_on(
            VelloRenderer::render_scene_to_png(&scene, 200, 200, "/tmp/clip_test.png")
        ).unwrap();
    }

    #[test]
    fn test_asymmetric_border_rendering() {
        let mut list = DisplayList::default();
        list.push(
            DrawCommand::DrawBorder {
                rect: rix_layout::box_model::Rect::new(20.0, 20.0, 200.0, 40.0),
                color: CssColor::WHITE,
                width: 1.0,
                border_radius: 40.0,
                corner_radii: [40.0, 0.0, 0.0, 40.0],
                widths: [1.0, 0.0, 1.0, 1.0],
            },
            0,
        );
        let scene = VelloSceneBuilder::build_scene(&list);
        pollster::block_on(
            VelloRenderer::render_scene_to_png(&scene, 300, 100, "/tmp/border_test.png")
        ).unwrap();
    }

    #[test]
    fn test_scale_corner_radii() {
        // If box is 100x100 and radii are [10, 10, 10, 10], no overlap -> unchanged
        let radii = scale_corner_radii(100.0, 100.0, [10.0, 10.0, 10.0, 10.0]);
        assert_eq!(radii, [10.0, 10.0, 10.0, 10.0]);

        // If box is 50 wide and top radii are [30, 30] (sum 60 > 50), factor f = 50/60 = 5/6
        let radii = scale_corner_radii(50.0, 100.0, [30.0, 30.0, 0.0, 0.0]);
        assert!((radii[0] - 25.0).abs() < 1e-4);
        assert!((radii[1] - 25.0).abs() < 1e-4);

        // If one side has sum 0, it doesn't divide by zero
        let radii = scale_corner_radii(100.0, 100.0, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(radii, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_cached_gpu_image_dedup() {
        let data_uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let img1 = cached_gpu_image_data(data_uri).expect("data uri should decode");
        assert_eq!(img1.width, 1);
        assert_eq!(img1.height, 1);
        let img2 = cached_gpu_image_data(data_uri).expect("data uri should hit cache");
        assert_eq!(img1.data.id(), img2.data.id(), "cached image blob IDs must match");
    }
}
