use super::*;
use lightningcss::properties::background::{
    Background, BackgroundOrigin, BackgroundRepeat, BackgroundRepeatKeyword,
};
use lightningcss::properties::{Property as ParsedProperty, PropertyId};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions};
use lightningcss::traits::ToCss;
use lightningcss::values::gradient::Gradient as ParsedGradient;
use lightningcss::values::image::Image;

pub(super) fn apply_declaration(    style: &mut ComputedStyle,
    name: &str,
    val: &str,
    ctx: &LayoutContext,
) {
    let trimmed = val.trim();

    match name {
        "display" => {
            style.display = Display::parse(trimmed);
        }
        "white-space" => {
            if matches!(
                trimmed,
                "normal" | "nowrap" | "pre" | "pre-wrap" | "pre-line" | "break-spaces"
            ) {
                style.white_space = trimmed.to_string();
            }
        }
        "text-overflow" => {
            if matches!(trimmed, "clip" | "ellipsis") {
                style.text_overflow = trimmed.to_string();
            }
        }
        "object-fit" => {
            if matches!(
                trimmed,
                "fill" | "contain" | "cover" | "none" | "scale-down"
            ) {
                style.object_fit = trimmed.to_string();
            }
        }
        "content" => {
            style.content = parse_content_value(trimmed);
        }
        "overflow" | "overflow-x" | "overflow-y" => {
            let values: Vec<_> = trimmed.split_whitespace().collect();
            if !values.is_empty()
                && values.len() <= 2
                && values.iter().all(|value| {
                    matches!(*value, "visible" | "hidden" | "clip" | "auto" | "scroll")
                })
            {
                if name != "overflow-y" {
                    style.overflow_x = values[0].to_string();
                }
                if name != "overflow-x" {
                    style.overflow_y = values.get(1).unwrap_or(&values[0]).to_string();
                }
            }
        }
        "z-index" => {
            if trimmed == "auto" {
                style.z_index = 0;
            } else if let Ok(value) = trimmed.parse() {
                style.z_index = value;
            }
        }
        "color" => {
            if let Some(c) = CssColor::parse(trimmed) {
                style.color = c;
            }
        }
        "background-color" => {
            if let Some(c) = CssColor::parse(trimmed) {
                style.background_color = c;
            }
        }
        "background" => {
            apply_background_shorthand(style, trimmed);
        }
        "background-image" => {
            apply_background_image(style, trimmed);
        }
        "background-repeat" => {
            apply_background_repeat(style, trimmed);
        }
        "font" => {
            parse_font_shorthand(trimmed, style, ctx);
        }
        "font-size" => {
            if let Some(px) = resolve_font_size(trimmed, style.font_size, ctx) {
                style.font_size = px;
            }
        }
        "font-family" => {
            style.font_family = trimmed.to_string();
        }
        "font-weight" => {
            if trimmed.eq_ignore_ascii_case("bold") {
                style.font_weight = 700;
            } else if trimmed.eq_ignore_ascii_case("normal") {
                style.font_weight = 400;
            } else if trimmed.eq_ignore_ascii_case("bolder") {
                style.font_weight = 700;
            } else if trimmed.eq_ignore_ascii_case("lighter") {
                style.font_weight = 300;
            } else if let Ok(weight) = trimmed.parse::<u16>() {
                style.font_weight = weight;
            }
        }
        "font-style" => {
            style.font_style = trimmed.to_lowercase();
        }
        "text-decoration" | "text-decoration-line" => {
            style.text_decoration_line = trimmed.to_lowercase();
        }
        "line-height" => {
            if trimmed == "normal" {
                style.line_height_factor = Some(1.2);
                style.line_height = style.font_size * 1.2;
                style.line_height_normal = true;
            } else if let Ok(multiplier) = trimmed.parse::<f32>() {
                if multiplier >= 0.0 {
                    style.line_height_normal = false;
                    style.line_height_factor = Some(multiplier);
                    style.line_height = style.font_size * multiplier;
                }
            } else if let Some(px) = resolve_font_size(trimmed, style.font_size, ctx) {
                style.line_height_normal = false;
                style.line_height_factor = None;
                style.line_height = px;
            }
        }
        "margin" => {
            parse_margin_shorthand(trimmed, style, ctx);
        }
        "margin-top" => {
            if trimmed.eq_ignore_ascii_case("auto") {
                style.margin_top_auto = true;
                style.margin_top = 0.0;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.margin_top = lp.resolve(ctx.viewport_height);
                style.margin_top_auto = false;
            }
        }
        "margin-bottom" => {
            if trimmed.eq_ignore_ascii_case("auto") {
                style.margin_bottom_auto = true;
                style.margin_bottom = 0.0;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.margin_bottom = lp.resolve(ctx.viewport_height);
                style.margin_bottom_auto = false;
            }
        }
        "margin-left" => {
            if trimmed.eq_ignore_ascii_case("auto") {
                style.margin_left_auto = true;
                style.margin_left = 0.0;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.margin_left = lp.resolve(ctx.viewport_width);
                style.margin_left_auto = false;
            }
        }
        "margin-right" => {
            if trimmed.eq_ignore_ascii_case("auto") {
                style.margin_right_auto = true;
                style.margin_right = 0.0;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.margin_right = lp.resolve(ctx.viewport_width);
                style.margin_right_auto = false;
            }
        }
        "padding" => {
            parse_padding_shorthand(trimmed, style, ctx);
        }
        "padding-top" => {
            if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.padding_top = lp.px;
                style.padding_top_percent = lp.percent_units();
            }
        }
        "padding-bottom" => {
            if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.padding_bottom = lp.px;
                style.padding_bottom_percent = lp.percent_units();
            }
        }
        "padding-left" => {
            if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.padding_left = lp.px;
                style.padding_left_percent = lp.percent_units();
            }
        }
        "padding-right" => {
            if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.padding_right = lp.px;
                style.padding_right_percent = lp.percent_units();
            }
        }
        "flex-direction" => {
            style.flex_direction = trimmed.to_lowercase();
        }
        "flex-wrap" => {
            style.flex_wrap = trimmed.to_lowercase();
        }
        "order" => {
            if let Ok(ord) = trimmed.parse::<i32>() {
                style.order = ord;
            }
        }
        "text-align" => {
            style.text_align = trimmed.to_lowercase();
        }
        "vertical-align" => {
            style.vertical_align = trimmed.to_lowercase();
        }
        "align-content" => {
            style.align_content = trimmed.to_lowercase();
        }
        "align-items" => {
            style.align_items = Some(trimmed.to_lowercase());
        }
        "justify-content" => {
            style.justify_content = Some(trimmed.to_lowercase());
        }
        "align-self" => {
            style.align_self = Some(trimmed.to_lowercase());
        }
        "justify-items" => {
            style.justify_items = Some(trimmed.to_lowercase());
        }
        "justify-self" => {
            style.justify_self = Some(trimmed.to_lowercase());
        }
        "gap" => {
            let parts = split_whitespace_preserve_parens(trimmed);
            if let Some(row) = parts
                .first()
                .and_then(|value| resolve_length_percentage(value, style.font_size, ctx))
                .map(|lp| lp.resolve(ctx.viewport_height))
            {
                let column = parts
                    .get(1)
                    .and_then(|value| resolve_length_percentage(value, style.font_size, ctx))
                    .map(|lp| lp.resolve(ctx.viewport_width))
                    .unwrap_or(row);
                if parts.len() <= 2 {
                    style.gap = row;
                    style.row_gap = Some(row);
                    style.column_gap = Some(column);
                }
            }
        }
        "row-gap" => {
            if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.row_gap = Some(lp.resolve(ctx.viewport_height));
            }
        }
        "column-gap" => {
            if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                style.column_gap = Some(lp.resolve(ctx.viewport_width));
            }
        }
        "grid-template-columns" => {
            if let Some(template) = crate::grid::GridTemplate::parse(trimmed, style.font_size, ctx)
            {
                style.grid_template_columns = template;
            }
        }
        "grid-template-rows" => {
            if let Some(template) = crate::grid::GridTemplate::parse(trimmed, style.font_size, ctx)
            {
                style.grid_template_rows = template;
            }
        }
        "flex-grow" => {
            if let Ok(g) = trimmed.parse::<f32>() {
                style.flex_grow = g;
            }
        }
        "flex-shrink" => {
            if let Ok(s) = trimmed.parse::<f32>() {
                style.flex_shrink = s;
            }
        }
        "flex-basis" => {
            parse_flex_basis(style, trimmed, ctx);
        }
        "flex" => {
            style.flex_grow = 0.0;
            style.flex_shrink = 1.0;
            style.flex_basis = None;
            style.flex_basis_percent = None;
            match trimmed {
                "none" => style.flex_shrink = 0.0,
                "auto" => style.flex_grow = 1.0,
                _ => {
                    let parts: Vec<_> = trimmed.split_whitespace().collect();
                    if let Some(grow) = parts.first().and_then(|part| part.parse::<f32>().ok()) {
                        style.flex_grow = grow.max(0.0);
                        style.flex_basis_percent = Some(0.0);
                        if let Some(second) = parts.get(1) {
                            if let Ok(shrink) = second.parse::<f32>() {
                                style.flex_shrink = shrink.max(0.0);
                                if let Some(basis) = parts.get(2) {
                                    parse_flex_basis(style, basis, ctx);
                                }
                            } else {
                                parse_flex_basis(style, second, ctx);
                            }
                        }
                    } else {
                        parse_flex_basis(style, trimmed, ctx);
                    }
                }
            }
        }
        "width" => {
            if trimmed.eq_ignore_ascii_case("auto") {
                style.width = None;
                style.width_percent = None;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                if lp.is_pure_percent() {
                    style.width_percent = Some(lp.percent * 100.0);
                    style.width = None;
                } else if lp.is_pure_px() {
                    style.width = Some(lp.px);
                    style.width_percent = None;
                } else {
                    style.width = Some(lp.resolve(ctx.viewport_width));
                    style.width_percent = None;
                }
            }
        }
        "height" => {
            if trimmed.eq_ignore_ascii_case("auto") {
                style.height = None;
                style.height_percent = None;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                if lp.is_pure_percent() {
                    style.height_percent = Some(lp.percent * 100.0);
                    style.height = None;
                } else if lp.is_pure_px() {
                    style.height = Some(lp.px);
                    style.height_percent = None;
                } else {
                    style.height = Some(lp.resolve(ctx.viewport_height));
                    style.height_percent = None;
                }
            }
        }
        "min-width" => {
            if trimmed.eq_ignore_ascii_case("auto") {
                style.min_width = None;
                style.min_width_percent = None;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                if lp.is_pure_percent() {
                    style.min_width_percent = Some(lp.percent * 100.0);
                    style.min_width = None;
                } else if lp.is_pure_px() {
                    style.min_width = Some(lp.px);
                    style.min_width_percent = None;
                } else {
                    style.min_width = Some(lp.resolve(ctx.viewport_width));
                    style.min_width_percent = None;
                }
            }
        }
        "max-width" => {
            if trimmed.eq_ignore_ascii_case("none") {
                style.max_width = None;
                style.max_width_percent = None;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                if lp.is_pure_percent() {
                    style.max_width_percent = Some(lp.percent * 100.0);
                    style.max_width = None;
                } else if lp.is_pure_px() {
                    style.max_width = Some(lp.px);
                    style.max_width_percent = None;
                } else {
                    style.max_width = Some(lp.resolve(ctx.viewport_width));
                    style.max_width_percent = None;
                }
            }
        }
        "min-height" => {
            if trimmed.eq_ignore_ascii_case("auto") {
                style.min_height = None;
                style.min_height_percent = None;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                if lp.is_pure_percent() {
                    style.min_height_percent = Some(lp.percent * 100.0);
                    style.min_height = None;
                } else if lp.is_pure_px() {
                    style.min_height = Some(lp.px);
                    style.min_height_percent = None;
                } else {
                    style.min_height = Some(lp.resolve(ctx.viewport_height));
                    style.min_height_percent = None;
                }
            }
        }
        "max-height" => {
            if trimmed.eq_ignore_ascii_case("none") {
                style.max_height = None;
                style.max_height_percent = None;
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                if lp.is_pure_percent() {
                    style.max_height_percent = Some(lp.percent * 100.0);
                    style.max_height = None;
                } else if lp.is_pure_px() {
                    style.max_height = Some(lp.px);
                    style.max_height_percent = None;
                } else {
                    style.max_height = Some(lp.resolve(ctx.viewport_height));
                    style.max_height_percent = None;
                }
            }
        }
        "inset" => {
            let parts = split_whitespace_preserve_parens(trimmed);
            let values = match parts.as_slice() {
                [all] => [*all; 4],
                [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
                [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
                [top, right, bottom, left] => [*top, *right, *bottom, *left],
                _ => return,
            };
            for (property, value) in ["top", "right", "bottom", "left"].into_iter().zip(values) {
                apply_declaration(style, property, value, ctx);
            }
        }
        "top" | "right" | "bottom" | "left" => {
            let (length, percent) = if trimmed.eq_ignore_ascii_case("auto") {
                (None, None)
            } else if let Some(lp) = resolve_length_percentage(trimmed, style.font_size, ctx) {
                if lp.is_pure_percent() {
                    (None, Some(lp.percent * 100.0))
                } else if lp.is_pure_px() {
                    (Some(lp.px), None)
                } else {
                    let basis = if matches!(name, "top" | "bottom") {
                        ctx.viewport_height
                    } else {
                        ctx.viewport_width
                    };
                    (Some(lp.resolve(basis)), None)
                }
            } else {
                return;
            };
            match name {
                "top" => {
                    style.top = length;
                    style.top_percent = percent;
                }
                "right" => {
                    style.right = length;
                    style.right_percent = percent;
                }
                "bottom" => {
                    style.bottom = length;
                    style.bottom_percent = percent;
                }
                "left" => {
                    style.left = length;
                    style.left_percent = percent;
                }
                _ => unreachable!(),
            }
        }
        "aspect-ratio" => {
            if trimmed == "auto" {
                style.aspect_ratio = None;
            } else {
                let ratio = trimmed.strip_prefix("auto ").unwrap_or(trimmed);
                let (width, height) = ratio
                    .split_once('/')
                    .map_or((ratio, "1"), |(width, height)| {
                        (width.trim(), height.trim())
                    });
                if let (Ok(width), Ok(height)) = (width.parse::<f32>(), height.parse::<f32>()) {
                    let value = width / height;
                    if width > 0.0 && height > 0.0 && value.is_finite() {
                        style.aspect_ratio = Some(value);
                    }
                }
            }
        }
        "grid-area" => {
            let parts: Vec<&str> = trimmed.split('/').map(str::trim).collect();
            if let Some(r_str) = parts.first() {
                if let Ok(r) = r_str.parse::<i16>() {
                    style.grid_row_start = Some(r);
                }
            }
            if let Some(c_str) = parts.get(1) {
                if let Ok(c) = c_str.parse::<i16>() {
                    style.grid_column_start = Some(c);
                }
            }
        }
        "grid-row" | "grid-row-start" => {
            let first = trimmed.split('/').next().unwrap_or("").trim();
            if let Ok(r) = first.parse::<i16>() {
                style.grid_row_start = Some(r);
            }
        }
        "grid-column" | "grid-column-start" => {
            let first = trimmed.split('/').next().unwrap_or("").trim();
            if let Ok(c) = first.parse::<i16>() {
                style.grid_column_start = Some(c);
            }
        }
        "box-sizing" => {
            style.box_sizing = trimmed.to_lowercase();
        }
        "border-radius" => {
            if let Some(px) = resolve_length(trimmed, style.font_size, ctx) {
                style.border_radius = px;
            }
        }
        "border-width" => {
            let lower = trimmed.to_lowercase();
            if lower == "none" || lower == "0" {
                style.border_width = 0.0;
            } else if let Some(px) = resolve_length(trimmed, style.font_size, ctx) {
                style.border_width = px;
            }
        }
        "border-color" => {
            if let Some(c) = CssColor::parse(trimmed) {
                style.border_color = c;
            }
        }
        "border" => {
            let lower = trimmed.to_lowercase();
            if lower == "none" || lower == "0" || lower == "hidden" {
                style.border_width = 0.0;
                style.border_color = CssColor::TRANSPARENT;
            } else {
                for part in split_border_parts(trimmed) {
                    let part_lower = part.to_lowercase();
                    if part_lower == "none" || part_lower == "hidden" || part == "0" {
                        style.border_width = 0.0;
                    } else if part_lower == "thin" {
                        style.border_width = 1.0;
                    } else if part_lower == "medium" {
                        style.border_width = 3.0;
                    } else if part_lower == "thick" {
                        style.border_width = 5.0;
                    } else if let Some(px) = resolve_length(&part, style.font_size, ctx) {
                        style.border_width = px;
                    } else if let Some(c) = CssColor::parse(&part) {
                        style.border_color = c;
                    }
                }
            }
        }
        "border-bottom" => {
            let lower = trimmed.to_lowercase();
            if lower == "none" || lower == "0" || lower == "hidden" {
                style.border_bottom_width = 0.0;
                style.border_bottom_color = CssColor::TRANSPARENT;
            } else {
                for part in split_border_parts(trimmed) {
                    let part_lower = part.to_lowercase();
                    if part_lower == "none" || part_lower == "hidden" || part == "0" {
                        style.border_bottom_width = 0.0;
                    } else if let Some(px) = resolve_length(&part, style.font_size, ctx) {
                        style.border_bottom_width = px;
                    } else if let Some(c) = CssColor::parse(&part) {
                        style.border_bottom_color = c;
                    }
                }
            }
        }
        "border-top" | "border-left" | "border-right" => {
            let (mut width, mut color) = (0.0, style.color);
            for part in split_border_parts(trimmed) {
                if let Some(px) = resolve_length(&part, style.font_size, ctx) {
                    width = px;
                } else if let Some(parsed) = CssColor::parse(&part) {
                    color = parsed;
                } else if part == "thin" {
                    width = 1.0;
                } else if part == "medium" {
                    width = 3.0;
                } else if part == "thick" {
                    width = 5.0;
                }
            }
            if matches!(trimmed, "none" | "hidden" | "0") {
                width = 0.0;
            }
            match name {
                "border-top" => {
                    style.border_top_width = width;
                    style.border_top_color = color;
                }
                "border-right" => {
                    style.border_right_width = width;
                    style.border_right_color = color;
                }
                _ => {
                    style.border_left_width = width;
                    style.border_left_color = color;
                }
            }
        }
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            if let Some(px) = resolve_length(trimmed, style.font_size, ctx) {
                match name {
                    "border-top-width" => style.border_top_width = px,
                    "border-right-width" => style.border_right_width = px,
                    "border-bottom-width" => style.border_bottom_width = px,
                    _ => style.border_left_width = px,
                }
            }
        }
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            if let Some(color) = CssColor::parse(trimmed) {
                match name {
                    "border-top-color" => style.border_top_color = color,
                    "border-right-color" => style.border_right_color = color,
                    "border-bottom-color" => style.border_bottom_color = color,
                    _ => style.border_left_color = color,
                }
            }
        }
        "position" => {
            style.position = trimmed.to_lowercase();
        }
        "visibility" => {
            style.visibility = trimmed.to_lowercase();
        }
        "pointer-events" => {
            style.pointer_events = trimmed.to_lowercase();
        }
        "cursor" => {
            style.cursor = trimmed.to_lowercase();
        }
        "opacity" => {
            if let Ok(op) = trimmed.parse::<f32>() {
                style.opacity = op.clamp(0.0, 1.0);
            }
        }
        "box-shadow" => {
            style.box_shadow = BoxShadow::parse(trimmed, style.font_size, ctx);
        }
        _ => {}
    }
    if matches!(name, "border" | "border-width") {
        style.border_top_width = style.border_width;
        style.border_right_width = style.border_width;
        style.border_bottom_width = style.border_width;
        style.border_left_width = style.border_width;
    }
    if matches!(name, "border" | "border-color") {
        style.border_top_color = style.border_color;
        style.border_right_color = style.border_color;
        style.border_bottom_color = style.border_color;
        style.border_left_color = style.border_color;
    }
}

fn parsed_background_images(value: &str) -> Option<Option<crate::gradient::LinearGradient>> {
    let parsed = ParsedProperty::parse_string(
        PropertyId::from("background-image"),
        value,
        ParserOptions::default(),
    )
    .ok()?;
    let ParsedProperty::BackgroundImage(images) = parsed else {
        return None;
    };
    if images.len() != 1 {
        return Some(None);
    }
    let image = images.first()?;
    Some(match image {
        Image::None => None,
        Image::Gradient(gradient) => match gradient.as_ref() {
            ParsedGradient::Linear(linear) => {
                crate::gradient::LinearGradient::from_parsed(linear.clone())
            }
            _ => None,
        },
        Image::Url(_) | Image::ImageSet(_) => None,
    })
}

fn apply_background_image(style: &mut ComputedStyle, value: &str) {
    if let Some(gradient) = parsed_background_images(value) {
        style.background_image = gradient;
    }
}

fn apply_background_shorthand(style: &mut ComputedStyle, value: &str) {
    let parsed = ParsedProperty::parse_string(
        PropertyId::from("background"),
        value,
        ParserOptions::default(),
    );
    let Ok(ParsedProperty::Background(layers)) = parsed else {
        return;
    };
    let Some(last_layer) = layers.last() else {
        return;
    };

    style.background_color =
        lower_background_color(&last_layer.color, style.color).unwrap_or(CssColor::TRANSPARENT);
    if layers.len() == 1 {
        (style.background_repeat_x, style.background_repeat_y) =
            background_repeat_modes(&last_layer.repeat);
    } else {
        style.background_repeat_x = BackgroundRepeatMode::Unsupported;
        style.background_repeat_y = BackgroundRepeatMode::Unsupported;
    }
    style.background_image = if layers.len() == 1
        && background_layer_uses_supported_settings(last_layer)
        && style.background_repeat_x != BackgroundRepeatMode::Unsupported
        && style.background_repeat_y != BackgroundRepeatMode::Unsupported
    {
        match &last_layer.image {
            Image::Gradient(gradient) => match gradient.as_ref() {
                ParsedGradient::Linear(linear) => {
                    crate::gradient::LinearGradient::from_parsed(linear.clone())
                }
                _ => None,
            },
            Image::None | Image::Url(_) | Image::ImageSet(_) => None,
        }
    } else {
        None
    };
}

fn background_layer_uses_supported_settings(layer: &Background<'_>) -> bool {
    layer.position == Default::default()
        && layer.size == Default::default()
        && layer.attachment == Default::default()
        && layer.origin == BackgroundOrigin::PaddingBox
        && layer.clip == lightningcss::properties::background::BackgroundClip::BorderBox
}

fn apply_background_repeat(style: &mut ComputedStyle, value: &str) {
    let parsed = ParsedProperty::parse_string(
        PropertyId::from("background-repeat"),
        value,
        ParserOptions::default(),
    );
    let Ok(ParsedProperty::BackgroundRepeat(repeats)) = parsed else {
        return;
    };
    if let Some(repeat) = repeats.first() {
        (style.background_repeat_x, style.background_repeat_y) = background_repeat_modes(repeat);
    }
}

fn background_repeat_modes(
    repeat: &BackgroundRepeat,
) -> (BackgroundRepeatMode, BackgroundRepeatMode) {
    fn mode(keyword: &BackgroundRepeatKeyword) -> BackgroundRepeatMode {
        match keyword {
            BackgroundRepeatKeyword::Repeat => BackgroundRepeatMode::Repeat,
            BackgroundRepeatKeyword::NoRepeat => BackgroundRepeatMode::NoRepeat,
            BackgroundRepeatKeyword::Space | BackgroundRepeatKeyword::Round => {
                BackgroundRepeatMode::Unsupported
            }
        }
    }
    (mode(&repeat.x), mode(&repeat.y))
}

fn lower_background_color(
    color: &lightningcss::values::color::CssColor,
    current_color: CssColor,
) -> Option<CssColor> {
    match color {
        lightningcss::values::color::CssColor::CurrentColor => Some(current_color),
        lightningcss::values::color::CssColor::RGBA(color) => Some(CssColor::from_rgba_u8(
            color.red,
            color.green,
            color.blue,
            color.alpha,
        )),
        other => CssColor::parse(&other.to_css_string(PrinterOptions::default()).ok()?),
    }
}

pub(super) fn split_border_parts(val: &str) -> Vec<String> {    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in val.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            _ if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn parse_content_value(value: &str) -> Option<String> {
    let tokens = split_content_tokens(value);
    if tokens.is_empty() {
        return None;
    }
    if tokens.len() == 1 && matches!(tokens[0].as_str(), "normal" | "none") {
        return None;
    }
    let mut text = String::new();
    for token in &tokens {
        let parsed = parse_content_string(token)?;
        text.push_str(&parsed);
    }
    Some(text)
}

fn split_content_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0i32;
    let mut escaped = false;
    for c in value.chars() {
        if let Some(open) = quote {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                current.push(c);
            }
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            _ if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn parse_content_string(token: &str) -> Option<String> {
    let mut chars = token.chars();
    let open = chars.next()?;
    if open != '"' && open != '\'' {
        return None;
    }
    let body = token.strip_suffix(open)?;
    let body = body.get(open.len_utf8()..)?;
    let mut text = String::new();
    let mut escaped = false;
    for c in body.chars() {
        if escaped {
            match c {
                'n' => text.push('\n'),
                't' => text.push('\t'),
                other => text.push(other),
            }
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            text.push(c);
        }
    }
    if escaped {
        return None;
    }
    Some(text)
}
