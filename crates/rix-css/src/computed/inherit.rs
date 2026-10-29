use super::*;

pub(super) fn apply_cascade_declaration(
    style: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    name: &str,
    val: &str,
    ctx: &LayoutContext,
) {
    if name.starts_with("--") {
        return;
    }
    match val.trim().to_lowercase().as_str() {
        "inherit" => apply_inherit(style, parent, name),
        "initial" => apply_initial(style, name),
        "unset" => {
            if is_inherited_property(name) {
                apply_inherit(style, parent, name);
            } else {
                apply_initial(style, name);
            }
        }
        "revert" | "revert-layer" => {
            if is_inherited_property(name) {
                apply_inherit(style, parent, name);
            } else {
                apply_initial(style, name);
            }
        }
        _ => apply_declaration(style, name, val, ctx),
    }
}

pub(super) fn apply_inherit(style: &mut ComputedStyle, parent: Option<&ComputedStyle>, name: &str) {
    let Some(p) = parent else { return };
    let defaults = ComputedStyle::default();
    match name {
        "display" => style.display = p.display,
        "color" => style.color = p.color,
        "fill" => {
            style.fill = p.fill;
            style.fill_is_current_color = p.fill_is_current_color;
        }
        "stroke" => {
            style.stroke = p.stroke;
            style.stroke_is_current_color = p.stroke_is_current_color;
        }
        "stroke-width" => {
            style.stroke_width = p.stroke_width;
        }
        "background-color" => style.background_color = p.background_color,
        "background-image" => style.background_image.clone_from(&p.background_image),
        "background-repeat" => {
            style.background_repeat_x = p.background_repeat_x;
            style.background_repeat_y = p.background_repeat_y;
        }
        "background" => {
            style.background_color = p.background_color;
            style.background_image.clone_from(&p.background_image);
            style.background_repeat_x = p.background_repeat_x;
            style.background_repeat_y = p.background_repeat_y;
        }
        "font-size" => style.font_size = p.font_size,
        "font-family" => style.font_family = p.font_family.clone(),
        "font-weight" => style.font_weight = p.font_weight,
        "font-style" => style.font_style = p.font_style.clone(),
        "font" => {
            style.font_size = p.font_size;
            style.font_family = p.font_family.clone();
            style.font_weight = p.font_weight;
            style.font_style = p.font_style.clone();
            style.line_height_factor = p.line_height_factor;
            style.line_height_normal = p.line_height_normal;
            style.line_height = p
                .line_height_factor
                .map_or(p.line_height, |factor| style.font_size * factor);
        }
        "line-height" => {
            style.line_height_normal = p.line_height_normal;
            style.line_height_factor = p.line_height_factor;
            style.line_height = p
                .line_height_factor
                .map_or(p.line_height, |factor| style.font_size * factor);
        }
        "white-space" => style.white_space = p.white_space.clone(),
        "text-overflow" => style.text_overflow = p.text_overflow.clone(),
        "object-fit" => style.object_fit = p.object_fit.clone(),
        "overflow-x" => style.overflow_x = p.overflow_x.clone(),
        "overflow-y" => style.overflow_y = p.overflow_y.clone(),
        "overflow" => {
            style.overflow_x = p.overflow_x.clone();
            style.overflow_y = p.overflow_y.clone();
        }
        "z-index" => style.z_index = p.z_index,
        "text-decoration" | "text-decoration-line" => {
            style.text_decoration_line = p.text_decoration_line.clone();
        }
        "margin" => {
            style.margin_top = p.margin_top;
            style.margin_right = p.margin_right;
            style.margin_bottom = p.margin_bottom;
            style.margin_left = p.margin_left;
            style.margin_top_auto = p.margin_top_auto;
            style.margin_right_auto = p.margin_right_auto;
            style.margin_bottom_auto = p.margin_bottom_auto;
            style.margin_left_auto = p.margin_left_auto;
        }
        "margin-top" => {
            style.margin_top = p.margin_top;
            style.margin_top_auto = p.margin_top_auto;
        }
        "margin-right" => {
            style.margin_right = p.margin_right;
            style.margin_right_auto = p.margin_right_auto;
        }
        "margin-bottom" => {
            style.margin_bottom = p.margin_bottom;
            style.margin_bottom_auto = p.margin_bottom_auto;
        }
        "margin-left" => {
            style.margin_left = p.margin_left;
            style.margin_left_auto = p.margin_left_auto;
        }
        "padding" => {
            style.padding_top = p.padding_top;
            style.padding_right = p.padding_right;
            style.padding_bottom = p.padding_bottom;
            style.padding_left = p.padding_left;
        }
        "padding-top" => style.padding_top = p.padding_top,
        "padding-right" => style.padding_right = p.padding_right,
        "padding-bottom" => style.padding_bottom = p.padding_bottom,
        "padding-left" => style.padding_left = p.padding_left,
        "width" => {
            style.width = p.width;
            style.width_percent = p.width_percent;
        }
        "height" => {
            style.height = p.height;
            style.height_percent = p.height_percent;
        }
        "position" => style.position = p.position.clone(),
        "visibility" => style.visibility = p.visibility.clone(),
        "pointer-events" => style.pointer_events = p.pointer_events.clone(),
        "cursor" => style.cursor = p.cursor.clone(),
        "opacity" => style.opacity = p.opacity,
        "text-align" => style.text_align = p.text_align.clone(),
        "vertical-align" => style.vertical_align = p.vertical_align.clone(),
        "align-content" => style.align_content = p.align_content.clone(),
        "box-sizing" => style.box_sizing = p.box_sizing.clone(),
        "flex-direction" => style.flex_direction = p.flex_direction.clone(),
        "flex-wrap" => style.flex_wrap = p.flex_wrap.clone(),
        "flex-basis" => {
            style.flex_basis = p.flex_basis;
            style.flex_basis_percent = p.flex_basis_percent;
        }
        "flex" => {
            style.flex_grow = p.flex_grow;
            style.flex_shrink = p.flex_shrink;
            style.flex_basis = p.flex_basis;
            style.flex_basis_percent = p.flex_basis_percent;
        }
        "order" => style.order = p.order,
        "gap" => {
            style.gap = p.gap;
            style.row_gap = p.row_gap;
            style.column_gap = p.column_gap;
        }
        "row-gap" => style.row_gap = Some(p.row_gap.unwrap_or(p.gap)),
        "column-gap" => style.column_gap = Some(p.column_gap.unwrap_or(p.gap)),
        "grid-template-columns" => style.grid_template_columns = p.grid_template_columns.clone(),
        "grid-template-rows" => style.grid_template_rows = p.grid_template_rows.clone(),
        "border-radius" => {
            style.border_radius = p.border_radius;
            style.border_top_left_radius = p.border_top_left_radius;
            style.border_top_right_radius = p.border_top_right_radius;
            style.border_bottom_right_radius = p.border_bottom_right_radius;
            style.border_bottom_left_radius = p.border_bottom_left_radius;
        }
        "border-top-left-radius" => style.border_top_left_radius = p.border_top_left_radius,
        "border-top-right-radius" => style.border_top_right_radius = p.border_top_right_radius,
        "border-bottom-right-radius" => style.border_bottom_right_radius = p.border_bottom_right_radius,
        "border-bottom-left-radius" => style.border_bottom_left_radius = p.border_bottom_left_radius,
        "box-shadow" => style.box_shadow = p.box_shadow,
        "border-width" => style.border_width = p.border_width,
        "border-color" => style.border_color = p.border_color,
        _ => {
            if name == "all" {
                *style = ComputedStyle {
                    variables: std::mem::take(&mut style.variables),
                    ..p.clone()
                };
            } else {
                let _ = &defaults;
            }
        }
    }
}

pub(super) fn apply_initial(style: &mut ComputedStyle, name: &str) {
    let d = ComputedStyle::default();
    match name {
        "display" => style.display = d.display,
        "color" => style.color = d.color,
        "fill" => {
            style.fill = d.fill;
            style.fill_is_current_color = d.fill_is_current_color;
        }
        "stroke" => {
            style.stroke = d.stroke;
            style.stroke_is_current_color = d.stroke_is_current_color;
        }
        "stroke-width" => {
            style.stroke_width = d.stroke_width;
        }
        "background-color" => style.background_color = d.background_color,
        "background-image" => style.background_image = None,
        "background-repeat" => {
            style.background_repeat_x = d.background_repeat_x;
            style.background_repeat_y = d.background_repeat_y;
        }
        "background" => {
            style.background_color = d.background_color;
            style.background_image = None;
            style.background_repeat_x = d.background_repeat_x;
            style.background_repeat_y = d.background_repeat_y;
        }
        "font-size" => style.font_size = d.font_size,
        "font-family" => style.font_family = d.font_family,
        "font-weight" => style.font_weight = d.font_weight,
        "font-style" => style.font_style = d.font_style,
        "font" => {
            style.font_size = d.font_size;
            style.font_family = d.font_family;
            style.font_weight = d.font_weight;
            style.font_style = d.font_style;
            style.line_height_factor = d.line_height_factor;
            style.line_height_normal = d.line_height_normal;
            style.line_height = style.font_size * d.line_height_factor.unwrap();
        }
        "line-height" => {
            style.line_height_normal = d.line_height_normal;
            style.line_height_factor = d.line_height_factor;
            style.line_height = style.font_size * d.line_height_factor.unwrap();
        }
        "white-space" => style.white_space = d.white_space,
        "text-overflow" => style.text_overflow = d.text_overflow,
        "object-fit" => style.object_fit = d.object_fit,
        "overflow-x" => style.overflow_x = d.overflow_x,
        "overflow-y" => style.overflow_y = d.overflow_y,
        "overflow" => {
            style.overflow_x = d.overflow_x;
            style.overflow_y = d.overflow_y;
        }
        "z-index" => style.z_index = d.z_index,
        "text-decoration" | "text-decoration-line" => {
            style.text_decoration_line = d.text_decoration_line;
        }
        "margin" => {
            style.margin_top = d.margin_top;
            style.margin_right = d.margin_right;
            style.margin_bottom = d.margin_bottom;
            style.margin_left = d.margin_left;
            style.margin_top_auto = false;
            style.margin_right_auto = false;
            style.margin_bottom_auto = false;
            style.margin_left_auto = false;
        }
        "margin-top" => {
            style.margin_top = d.margin_top;
            style.margin_top_auto = false;
        }
        "margin-right" => {
            style.margin_right = d.margin_right;
            style.margin_right_auto = false;
        }
        "margin-bottom" => {
            style.margin_bottom = d.margin_bottom;
            style.margin_bottom_auto = false;
        }
        "margin-left" => {
            style.margin_left = d.margin_left;
            style.margin_left_auto = false;
        }
        "padding" => {
            style.padding_top = d.padding_top;
            style.padding_right = d.padding_right;
            style.padding_bottom = d.padding_bottom;
            style.padding_left = d.padding_left;
        }
        "padding-top" => style.padding_top = d.padding_top,
        "padding-right" => style.padding_right = d.padding_right,
        "padding-bottom" => style.padding_bottom = d.padding_bottom,
        "padding-left" => style.padding_left = d.padding_left,
        "width" => {
            style.width = None;
            style.width_percent = None;
        }
        "height" => {
            style.height = None;
            style.height_percent = None;
        }
        "min-width" => {
            style.min_width = None;
            style.min_width_percent = None;
        }
        "max-width" => {
            style.max_width = None;
            style.max_width_percent = None;
        }
        "min-height" => {
            style.min_height = None;
            style.min_height_percent = None;
        }
        "max-height" => {
            style.max_height = None;
            style.max_height_percent = None;
        }
        "top" => {
            style.top = None;
            style.top_percent = None;
        }
        "right" => {
            style.right = None;
            style.right_percent = None;
        }
        "bottom" => {
            style.bottom = None;
            style.bottom_percent = None;
        }
        "left" => {
            style.left = None;
            style.left_percent = None;
        }
        "aspect-ratio" => style.aspect_ratio = None,
        "inset" => {
            style.top = None;
            style.right = None;
            style.bottom = None;
            style.left = None;
            style.top_percent = None;
            style.right_percent = None;
            style.bottom_percent = None;
            style.left_percent = None;
        }
        "position" => style.position = d.position,
        "visibility" => style.visibility = d.visibility,
        "pointer-events" => style.pointer_events = d.pointer_events,
        "cursor" => style.cursor = d.cursor,
        "opacity" => style.opacity = d.opacity,
        "text-align" => style.text_align = d.text_align,
        "vertical-align" => style.vertical_align = d.vertical_align,
        "align-content" => style.align_content = d.align_content,
        "box-sizing" => style.box_sizing = d.box_sizing,
        "flex-direction" => style.flex_direction = d.flex_direction,
        "flex-wrap" => style.flex_wrap = d.flex_wrap,
        "flex-basis" => {
            style.flex_basis = d.flex_basis;
            style.flex_basis_percent = d.flex_basis_percent;
        }
        "flex-grow" => style.flex_grow = d.flex_grow,
        "flex-shrink" => style.flex_shrink = d.flex_shrink,
        "flex" => {
            style.flex_grow = d.flex_grow;
            style.flex_shrink = d.flex_shrink;
            style.flex_basis = d.flex_basis;
            style.flex_basis_percent = d.flex_basis_percent;
        }
        "order" => style.order = d.order,
        "gap" => {
            style.gap = d.gap;
            style.row_gap = None;
            style.column_gap = None;
        }
        "row-gap" => style.row_gap = Some(0.0),
        "column-gap" => style.column_gap = Some(0.0),
        "grid-template-columns" => style.grid_template_columns = Default::default(),
        "grid-template-rows" => style.grid_template_rows = Default::default(),
        "align-items" => style.align_items = None,
        "justify-content" => style.justify_content = None,
        "align-self" => style.align_self = None,
        "justify-items" => style.justify_items = None,
        "justify-self" => style.justify_self = None,
        "grid-row" | "grid-row-start" => style.grid_row_start = None,
        "grid-column" | "grid-column-start" => style.grid_column_start = None,
        "border-radius" => {
            style.border_radius = d.border_radius;
            style.border_top_left_radius = d.border_top_left_radius;
            style.border_top_right_radius = d.border_top_right_radius;
            style.border_bottom_right_radius = d.border_bottom_right_radius;
            style.border_bottom_left_radius = d.border_bottom_left_radius;
        }
        "border-top-left-radius" => style.border_top_left_radius = d.border_top_left_radius,
        "border-top-right-radius" => style.border_top_right_radius = d.border_top_right_radius,
        "border-bottom-right-radius" => style.border_bottom_right_radius = d.border_bottom_right_radius,
        "border-bottom-left-radius" => style.border_bottom_left_radius = d.border_bottom_left_radius,
        "box-shadow" => style.box_shadow = d.box_shadow,
        "border-width" => style.border_width = d.border_width,
        "border-color" => style.border_color = d.border_color,
        "border" => {
            style.border_width = d.border_width;
            style.border_color = d.border_color;
        }
        "border-bottom" => {
            style.border_bottom_width = d.border_bottom_width;
            style.border_bottom_color = d.border_bottom_color;
        }
        "all" => {
            let vars = std::mem::take(&mut style.variables);
            *style = ComputedStyle {
                variables: vars,
                ..d
            };
        }
        _ => {}
    }
}
