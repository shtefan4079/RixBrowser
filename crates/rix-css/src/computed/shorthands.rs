use super::*;

pub(super) fn split_whitespace_preserve_parens(val: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (i, c) in val.char_indices() {
        match c {
            '(' => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
            }
            _ if c.is_whitespace() && depth == 0 => {
                if let Some(s) = start {
                    parts.push(&val[s..i]);
                    start = None;
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s) = start {
        parts.push(&val[s..]);
    }
    parts
}

pub(super) fn parse_margin_shorthand(val: &str, style: &mut ComputedStyle, ctx: &LayoutContext) {
    let parts = split_whitespace_preserve_parens(val);
    if parts.is_empty() {
        return;
    }
    let resolve_side = |part: &str, is_vertical: bool| -> (f32, bool) {
        if part.eq_ignore_ascii_case("auto") {
            (0.0, true)
        } else if let Some(lp) = resolve_length_percentage(part, style.font_size, ctx) {
            let basis = if is_vertical {
                ctx.viewport_height
            } else {
                ctx.viewport_width
            };
            (lp.resolve(basis), false)
        } else {
            (0.0, false)
        }
    };
    match parts.len() {
        1 => {
            let (top_bot, tb_auto) = resolve_side(parts[0], true);
            let (left_right, lr_auto) = resolve_side(parts[0], false);
            style.margin_top = top_bot;
            style.margin_top_auto = tb_auto;
            style.margin_right = left_right;
            style.margin_right_auto = lr_auto;
            style.margin_bottom = top_bot;
            style.margin_bottom_auto = tb_auto;
            style.margin_left = left_right;
            style.margin_left_auto = lr_auto;
        }
        2 => {
            let (top_bot, tb_auto) = resolve_side(parts[0], true);
            let (left_right, lr_auto) = resolve_side(parts[1], false);
            style.margin_top = top_bot;
            style.margin_top_auto = tb_auto;
            style.margin_bottom = top_bot;
            style.margin_bottom_auto = tb_auto;
            style.margin_left = left_right;
            style.margin_left_auto = lr_auto;
            style.margin_right = left_right;
            style.margin_right_auto = lr_auto;
        }
        3 => {
            let (top, t_auto) = resolve_side(parts[0], true);
            let (left_right, lr_auto) = resolve_side(parts[1], false);
            let (bot, b_auto) = resolve_side(parts[2], true);
            style.margin_top = top;
            style.margin_top_auto = t_auto;
            style.margin_left = left_right;
            style.margin_left_auto = lr_auto;
            style.margin_right = left_right;
            style.margin_right_auto = lr_auto;
            style.margin_bottom = bot;
            style.margin_bottom_auto = b_auto;
        }
        _ => {
            let (top, t_auto) = resolve_side(parts[0], true);
            let (right, r_auto) = resolve_side(parts[1], false);
            let (bot, b_auto) = resolve_side(parts[2], true);
            let (left, l_auto) = resolve_side(parts[3], false);
            style.margin_top = top;
            style.margin_top_auto = t_auto;
            style.margin_right = right;
            style.margin_right_auto = r_auto;
            style.margin_bottom = bot;
            style.margin_bottom_auto = b_auto;
            style.margin_left = left;
            style.margin_left_auto = l_auto;
        }
    }
}

pub(super) fn parse_padding_shorthand(val: &str, style: &mut ComputedStyle, ctx: &LayoutContext) {
    let parts = split_whitespace_preserve_parens(val);
    if parts.is_empty() {
        return;
    }
    let parse_side = |part: &str| -> (f32, Option<f32>) {
        match resolve_length_percentage(part, style.font_size, ctx) {
            Some(lp) => (lp.px, lp.percent_units()),
            None => (0.0, None),
        }
    };
    let apply = |top: (f32, Option<f32>),
                     right: (f32, Option<f32>),
                     bottom: (f32, Option<f32>),
                     left: (f32, Option<f32>),
                     style: &mut ComputedStyle| {
        style.padding_top = top.0;
        style.padding_top_percent = top.1;
        style.padding_right = right.0;
        style.padding_right_percent = right.1;
        style.padding_bottom = bottom.0;
        style.padding_bottom_percent = bottom.1;
        style.padding_left = left.0;
        style.padding_left_percent = left.1;
    };
    match parts.len() {
        1 => {
            let side = parse_side(parts[0]);
            apply(side, side, side, side, style);
        }
        2 => {
            let tb = parse_side(parts[0]);
            let lr = parse_side(parts[1]);
            apply(tb, lr, tb, lr, style);
        }
        3 => {
            let top = parse_side(parts[0]);
            let lr = parse_side(parts[1]);
            let bottom = parse_side(parts[2]);
            apply(top, lr, bottom, lr, style);
        }
        _ => {
            apply(
                parse_side(parts[0]),
                parse_side(parts[1]),
                parse_side(parts[2]),
                parse_side(parts[3]),
                style,
            );
        }
    }
}

pub(super) fn parse_font_shorthand(val: &str, style: &mut ComputedStyle, ctx: &LayoutContext) {
    let mut size_token: Option<String> = None;
    let mut line_token: Option<String> = None;
    let mut head: Vec<String> = Vec::new();
    let mut family_parts: Vec<String> = Vec::new();
    let mut seen_size = false;

    let tokens = tokenize_font_shorthand(val);
    let mut idx = 0;
    while idx < tokens.len() {
        let tok = tokens[idx].clone();
        if seen_size {
            if tok == "/" {
                if let Some(next) = tokens.get(idx + 1) {
                    line_token = Some(next.clone());
                    idx += 1;
                }
            } else {
                family_parts.push(tok);
            }
            idx += 1;
            continue;
        }
        if tok.contains('/') && size_token.is_none() {
            let mut split = tok.splitn(2, '/');
            let size_part = split.next().unwrap_or("").to_string();
            let line_part = split.next().unwrap_or("").to_string();
            if !size_part.is_empty() {
                size_token = Some(size_part);
                if !line_part.is_empty() {
                    line_token = Some(line_part);
                }
                seen_size = true;
                idx += 1;
                continue;
            }
        }
        if is_font_size_token(&tok) {
            size_token = Some(tok);
            seen_size = true;
            idx += 1;
            continue;
        }
        head.push(tok);
        idx += 1;
    }

    for tok in head {
        let low = tok.to_lowercase();
        match low.as_str() {
            "italic" | "oblique" => style.font_style = low,
            "normal" => {}
            "bold" => style.font_weight = 700,
            "bolder" => style.font_weight = 700,
            "lighter" => style.font_weight = 300,
            _ => {
                if let Ok(w) = low.parse::<u16>() {
                    if (1..=1000).contains(&w) {
                        style.font_weight = w;
                    }
                }
            }
        }
    }

    if let Some(size) = size_token {
        if let Some(px) = resolve_font_size(&size, style.font_size, ctx) {
            style.font_size = px;
            if let Some(line) = line_token {
                if line.eq_ignore_ascii_case("normal") {
                    style.line_height_normal = true;
                    style.line_height_factor = Some(1.2);
                    style.line_height = style.font_size * 1.2;
                } else if let Ok(mult) = line.parse::<f32>() {
                    style.line_height_normal = false;
                    style.line_height_factor = Some(mult);
                    style.line_height = style.font_size * mult;
                } else if let Some(px) = resolve_length(&line, style.font_size, ctx) {
                    style.line_height_normal = false;
                    style.line_height_factor = None;
                    style.line_height = px;
                }
            } else {
                style.line_height_normal = true;
                style.line_height_factor = Some(1.2);
                style.line_height = style.font_size * 1.2;
            }
        }
    }
    if !family_parts.is_empty() {
        let mut family_value = String::new();
        for part in family_parts {
            if part == "," {
                while family_value.chars().last().is_some_and(char::is_whitespace) {
                    family_value.pop();
                }
                family_value.push(',');
            } else {
                if !family_value.is_empty() {
                    family_value.push(' ');
                }
                family_value.push_str(&part);
            }
        }
        style.font_family = family_value;
    }
}

pub(super) fn tokenize_font_shorthand(val: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut characters = val.chars().peekable();
    while let Some(c) = characters.next() {
        if c == '\\' {
            current.push(c);
            if let Some(escaped) = characters.next() {
                current.push(escaped);
                if escaped.is_ascii_hexdigit() {
                    let mut digits = 1;
                    while digits < 6
                        && characters
                            .peek()
                            .is_some_and(|next| next.is_ascii_hexdigit())
                    {
                        if let Some(next) = characters.next() {
                            current.push(next);
                            digits += 1;
                        }
                    }
                    if characters
                        .peek()
                        .is_some_and(|next| next.is_ascii_whitespace())
                    {
                        if let Some(whitespace) = characters.next() {
                            current.push(whitespace);
                            if whitespace == '\r' && characters.peek() == Some(&'\n') {
                                if let Some(line_feed) = characters.next() {
                                    current.push(line_feed);
                                }
                            }
                        }
                    }
                }
            }
            continue;
        }
        if let Some(q) = quote {
            current.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
            current.push(c);
        } else if c == ',' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            tokens.push(",".to_string());
        } else if c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub(super) fn is_font_size_token(tok: &str) -> bool {
    let low = tok.to_lowercase();
    if [
        "xx-small",
        "x-small",
        "small",
        "medium",
        "large",
        "x-large",
        "xx-large",
        "xxx-large",
    ]
    .contains(&low.as_str())
    {
        return true;
    }
    low == "0"
        || low.ends_with('%')
        || low.chars().any(|ch| ch.is_ascii_alphabetic())
            && low
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
}

pub(super) fn parse_flex_basis(style: &mut ComputedStyle, value: &str, ctx: &LayoutContext) {
    if matches!(value, "auto" | "content") {
        style.flex_basis = None;
        style.flex_basis_percent = None;
    } else if let Some(lp) = resolve_length_percentage(value, style.font_size, ctx) {
        if lp.is_pure_percent() {
            style.flex_basis = None;
            style.flex_basis_percent = Some(lp.percent * 100.0);
        } else if lp.is_pure_px() {
            style.flex_basis = Some(lp.px.max(0.0));
            style.flex_basis_percent = None;
        } else {
            style.flex_basis = Some(lp.resolve(ctx.viewport_width).max(0.0));
            style.flex_basis_percent = None;
        }
    }
}

pub(super) fn resolve_font_size(
    value: &str,
    inherited_size: f32,
    ctx: &LayoutContext,
) -> Option<f32> {
    if let Some(percent) = value
        .strip_suffix('%')
        .and_then(|number| number.trim().parse::<f32>().ok())
    {
        return (percent >= 0.0 && percent.is_finite()).then_some(inherited_size * percent / 100.0);
    }
    match value {
        "xx-small" => Some(ctx.root_font_size * 0.6),
        "x-small" => Some(ctx.root_font_size * 0.75),
        "small" => Some(ctx.root_font_size * 0.89),
        "medium" => Some(ComputedStyle::default().font_size),
        "large" => Some(ctx.root_font_size * 1.2),
        "x-large" => Some(ctx.root_font_size * 1.5),
        "xx-large" => Some(ctx.root_font_size * 2.0),
        "xxx-large" => Some(ctx.root_font_size * 3.0),
        "smaller" => Some(inherited_size / 1.2),
        "larger" => Some(inherited_size * 1.2),
        _ => resolve_length(value, inherited_size, ctx)
            .filter(|size| size.is_finite() && *size >= 0.0),
    }
}
