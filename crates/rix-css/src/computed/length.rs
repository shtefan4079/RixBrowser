use super::*;

pub fn resolve_length(val: &str, current_font_size: f32, ctx: &LayoutContext) -> Option<f32> {
    let s = val.trim();
    if s.is_empty() {
        return None;
    }
    let low = s.to_lowercase();
    if low == "auto" || low == "none" {
        return None;
    }
    if low.starts_with("calc(")
        || low.starts_with("min(")
        || low.starts_with("max(")
        || low.starts_with("clamp(")
    {
        return eval_math_function(s, current_font_size, ctx);
    }
    resolve_single_length(s, current_font_size, ctx)
}

pub(super) fn resolve_single_length(s: &str, font_size: f32, ctx: &LayoutContext) -> Option<f32> {
    let t = s.trim();
    if t == "0" {
        return Some(0.0);
    }
    let low = t.to_lowercase();
    if let Some(num) = strip_unit(&low, "px") {
        return num.parse().ok();
    }
    if let Some(num) = strip_unit(&low, "rem") {
        let rem: f32 = num.parse().ok()?;
        return Some(rem * ctx.root_font_size);
    }
    if let Some(num) = strip_unit(&low, "em") {
        let em: f32 = num.parse().ok()?;
        return Some(em * font_size);
    }
    if let Some(num) = strip_unit(&low, "lh") {
        let lh: f32 = num.parse().ok()?;
        return Some(lh * font_size);
    }
    if let Some(num) = strip_unit(&low, "ex").or_else(|| strip_unit(&low, "ch")) {
        let units: f32 = num.parse().ok()?;
        return Some(units * font_size * 0.5);
    }
    if let Some(num) = strip_unit(&low, "vw") {
        let vw: f32 = num.parse().ok()?;
        return Some(vw * ctx.viewport_width / 100.0);
    }
    if let Some(num) = strip_unit(&low, "vh") {
        let vh: f32 = num.parse().ok()?;
        return Some(vh * ctx.viewport_height / 100.0);
    }
    if let Some(num) = strip_unit(&low, "vmin") {
        let v: f32 = num.parse().ok()?;
        return Some(v * ctx.viewport_width.min(ctx.viewport_height) / 100.0);
    }
    if let Some(num) = strip_unit(&low, "vmax") {
        let v: f32 = num.parse().ok()?;
        return Some(v * ctx.viewport_width.max(ctx.viewport_height) / 100.0);
    }
    if let Some(num) = strip_unit(&low, "cqw") {
        let v: f32 = num.parse().ok()?;
        return Some(v * ctx.viewport_width / 100.0);
    }
    if let Some(num) = strip_unit(&low, "cqh") {
        let v: f32 = num.parse().ok()?;
        return Some(v * ctx.viewport_height / 100.0);
    }
    if let Some(num) = strip_unit(&low, "in") {
        let v: f32 = num.parse().ok()?;
        return Some(v * 96.0);
    }
    if let Some(num) = strip_unit(&low, "cm") {
        let v: f32 = num.parse().ok()?;
        return Some(v * 96.0 / 2.54);
    }
    if let Some(num) = strip_unit(&low, "mm") {
        let v: f32 = num.parse().ok()?;
        return Some(v * 96.0 / 25.4);
    }
    if let Some(num) = strip_unit(&low, "q") {
        let v: f32 = num.parse().ok()?;
        return Some(v * 96.0 / 101.6);
    }
    if let Some(num) = strip_unit(&low, "pt") {
        let v: f32 = num.parse().ok()?;
        return Some(v * 96.0 / 72.0);
    }
    if let Some(num) = strip_unit(&low, "pc") {
        let v: f32 = num.parse().ok()?;
        return Some(v * 16.0);
    }
    if t.ends_with('%') {
        return None;
    }
    if let Ok(num) = t.parse::<f32>() {
        return Some(num);
    }
    None
}

pub(super) fn strip_unit<'a>(low: &'a str, unit: &str) -> Option<&'a str> {
    low.strip_suffix(unit).and_then(|num| {
        let n = num.trim();
        if n.is_empty() {
            return None;
        }
        if n.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'e' | 'E'))
        {
            Some(n)
        } else {
            None
        }
    })
}

pub(super) fn eval_math_function(expr: &str, font_size: f32, ctx: &LayoutContext) -> Option<f32> {
    eval_math_function_value(expr, font_size, ctx).map(|lp| lp.resolve(ctx.viewport_width))
}

pub(super) fn eval_math_function_value(
    expr: &str,
    font_size: f32,
    ctx: &LayoutContext,
) -> Option<LengthPercentage> {
    let t = expr.trim();
    let low = t.to_lowercase();
    if let Some(inner) = match_function(&low, t, "calc") {
        return eval_calc_value(&inner, font_size, ctx);
    }
    if let Some(inner) = match_function(&low, t, "min") {
        let mut best = f32::INFINITY;
        for arg in split_math_args(&inner)? {
            let lp = eval_calc_value(&arg, font_size, ctx)?;
            best = best.min(lp.resolve(ctx.viewport_width));
        }
        return Some(LengthPercentage::from_px(best));
    }
    if let Some(inner) = match_function(&low, t, "max") {
        let mut best = f32::NEG_INFINITY;
        for arg in split_math_args(&inner)? {
            let lp = eval_calc_value(&arg, font_size, ctx)?;
            best = best.max(lp.resolve(ctx.viewport_width));
        }
        return Some(LengthPercentage::from_px(best));
    }
    if let Some(inner) = match_function(&low, t, "clamp") {
        let args = split_math_args(&inner)?;
        if args.len() != 3 {
            return None;
        }
        let min = eval_calc_value(&args[0], font_size, ctx)?.resolve(ctx.viewport_width);
        let val = eval_calc_value(&args[1], font_size, ctx)?.resolve(ctx.viewport_width);
        let max = eval_calc_value(&args[2], font_size, ctx)?.resolve(ctx.viewport_width);
        return Some(LengthPercentage::from_px(min.max(val.min(max))));
    }
    None
}

pub(super) fn match_function(low: &str, original: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}(");
    if low.starts_with(&prefix) && original.trim_end().ends_with(')') {
        let start = original.find('(')? + 1;
        let end = original.trim_end().len() - 1;
        Some(original[start..end].to_string())
    } else {
        None
    }
}

pub(super) fn split_math_args(inner: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(inner[start..].trim().to_string());
    if args.iter().any(|a| a.is_empty()) {
        return None;
    }
    Some(args)
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LengthPercentage {
    pub percent: f32,
    pub px: f32,
}

impl LengthPercentage {
    pub const ZERO: Self = Self {
        percent: 0.0,
        px: 0.0,
    };

    #[inline]
    pub fn from_px(px: f32) -> Self {
        Self { percent: 0.0, px }
    }

    #[inline]
    pub fn from_percent(percent_factor: f32) -> Self {
        Self {
            percent: percent_factor,
            px: 0.0,
        }
    }

    #[inline]
    pub fn resolve(&self, basis: f32) -> f32 {
        self.percent * basis + self.px
    }

    #[inline]
    pub fn is_pure_px(&self) -> bool {
        self.percent.abs() < 1e-6
    }

    #[inline]
    pub fn is_pure_percent(&self) -> bool {
        self.px.abs() < 1e-6 && self.percent.abs() >= 1e-6
    }

    #[inline]
    pub fn percent_units(&self) -> Option<f32> {
        (self.percent.abs() >= 1e-6).then_some(self.percent * 100.0)
    }
}

pub fn resolve_length_percentage(
    val: &str,
    current_font_size: f32,
    ctx: &LayoutContext,
) -> Option<LengthPercentage> {
    let s = val.trim();
    if s.is_empty() {
        return None;
    }
    let low = s.to_lowercase();
    if low == "auto" || low == "none" {
        return None;
    }
    if low == "0" {
        return Some(LengthPercentage::ZERO);
    }
    if low.starts_with("calc(")
        || low.starts_with("min(")
        || low.starts_with("max(")
        || low.starts_with("clamp(")
    {
        return eval_math_function_value(s, current_font_size, ctx);
    }
    if let Some(pct) = s.strip_suffix('%') {
        let p: f32 = pct.trim().parse().ok()?;
        return Some(LengthPercentage::from_percent(p / 100.0));
    }
    resolve_single_length(s, current_font_size, ctx).map(LengthPercentage::from_px)
}

pub fn eval_calc_value(
    expr: &str,
    font_size: f32,
    ctx: &LayoutContext,
) -> Option<LengthPercentage> {
    let tokens = tokenize_calc(expr)?;
    let mut parser = CalcParser {
        tokens,
        pos: 0,
        font_size,
        ctx,
    };
    let value = parser.parse_sum()?;
    if parser.pos != parser.tokens.len() {
        return None;
    }
    value.into_length_percentage()
}

#[allow(dead_code)]
pub(super) fn eval_calc_expr(expr: &str, font_size: f32, ctx: &LayoutContext) -> Option<f32> {
    eval_calc_value(expr, font_size, ctx).map(|lp| lp.resolve(ctx.viewport_width))
}

#[derive(Debug, Clone)]
pub(super) enum CalcToken {
    Num(f32),
    Percent(f32),
    Unit(f32, String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CalcValue {
    Length(LengthPercentage),
    Num(f32),
}

impl CalcValue {
    fn into_length_percentage(self) -> Option<LengthPercentage> {
        match self {
            Self::Length(lp) => Some(lp),
            Self::Num(n) => Some(LengthPercentage::from_px(n)),
        }
    }
}

pub(super) fn tokenize_calc(expr: &str) -> Option<Vec<CalcToken>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if chars[i..].starts_with(&['c', 'a', 'l', 'c', '(']) {
            tokens.push(CalcToken::LParen);
            i += 5;
            continue;
        }
        match c {
            '+' => {
                tokens.push(CalcToken::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(CalcToken::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(CalcToken::Star);
                i += 1;
            }
            '/' => {
                tokens.push(CalcToken::Slash);
                i += 1;
            }
            '(' => {
                tokens.push(CalcToken::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(CalcToken::RParen);
                i += 1;
            }
            _ if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        i = j;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let num_str: String = chars[start..i].iter().collect();
                let num: f32 = num_str.parse().ok()?;
                if i < chars.len() && chars[i] == '%' {
                    i += 1;
                    tokens.push(CalcToken::Percent(num));
                } else {
                    let unit_start = i;
                    while i < chars.len() && chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let unit: String = chars[unit_start..i].iter().collect();
                    if unit.is_empty() {
                        tokens.push(CalcToken::Num(num));
                    } else {
                        tokens.push(CalcToken::Unit(num, unit.to_lowercase()));
                    }
                }
            }
            _ => return None,
        }
    }
    Some(tokens)
}

pub(super) struct CalcParser<'a> {
    pub(super) tokens: Vec<CalcToken>,
    pub(super) pos: usize,
    pub(super) font_size: f32,
    pub(super) ctx: &'a LayoutContext,
}

impl<'a> CalcParser<'a> {
    fn peek(&self) -> Option<&CalcToken> {
        self.tokens.get(self.pos)
    }

    fn parse_sum(&mut self) -> Option<CalcValue> {
        let mut left = self.parse_product()?;
        loop {
            let op_is_plus = matches!(self.peek(), Some(CalcToken::Plus));
            let op_is_minus = matches!(self.peek(), Some(CalcToken::Minus));
            if !op_is_plus && !op_is_minus {
                break;
            }
            self.pos += 1;
            let right = self.parse_product()?;
            left = match (left, right) {
                (CalcValue::Length(a), CalcValue::Length(b)) => {
                    CalcValue::Length(LengthPercentage {
                        percent: if op_is_plus {
                            a.percent + b.percent
                        } else {
                            a.percent - b.percent
                        },
                        px: if op_is_plus { a.px + b.px } else { a.px - b.px },
                    })
                }
                (CalcValue::Num(a), CalcValue::Num(b)) => {
                    CalcValue::Num(if op_is_plus { a + b } else { a - b })
                }
                (CalcValue::Length(a), CalcValue::Num(b)) if b.abs() < 1e-6 => CalcValue::Length(a),
                (CalcValue::Num(a), CalcValue::Length(b)) if a.abs() < 1e-6 && op_is_plus => {
                    CalcValue::Length(b)
                }
                _ => return None,
            };
        }
        Some(left)
    }

    fn parse_product(&mut self) -> Option<CalcValue> {
        let mut left = self.parse_unary()?;
        loop {
            let op_is_star = matches!(self.peek(), Some(CalcToken::Star));
            let op_is_slash = matches!(self.peek(), Some(CalcToken::Slash));
            if !op_is_star && !op_is_slash {
                break;
            }
            self.pos += 1;
            let right = self.parse_unary()?;
            left = if op_is_star {
                match (left, right) {
                    (CalcValue::Length(a), CalcValue::Num(b))
                    | (CalcValue::Num(b), CalcValue::Length(a)) => {
                        CalcValue::Length(LengthPercentage {
                            percent: a.percent * b,
                            px: a.px * b,
                        })
                    }
                    (CalcValue::Num(a), CalcValue::Num(b)) => CalcValue::Num(a * b),
                    _ => return None,
                }
            } else {
                match (left, right) {
                    (CalcValue::Length(a), CalcValue::Num(b)) if b != 0.0 => {
                        CalcValue::Length(LengthPercentage {
                            percent: a.percent / b,
                            px: a.px / b,
                        })
                    }
                    (CalcValue::Num(a), CalcValue::Num(b)) if b != 0.0 => CalcValue::Num(a / b),
                    _ => return None,
                }
            };
        }
        Some(left)
    }

    fn parse_unary(&mut self) -> Option<CalcValue> {
        if matches!(self.peek(), Some(CalcToken::Minus)) {
            self.pos += 1;
            return match self.parse_unary()? {
                CalcValue::Length(lp) => Some(CalcValue::Length(LengthPercentage {
                    percent: -lp.percent,
                    px: -lp.px,
                })),
                CalcValue::Num(v) => Some(CalcValue::Num(-v)),
            };
        }
        if matches!(self.peek(), Some(CalcToken::Plus)) {
            self.pos += 1;
            return self.parse_unary();
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Option<CalcValue> {
        match self.tokens.get(self.pos)?.clone() {
            CalcToken::Num(n) => {
                self.pos += 1;
                Some(CalcValue::Num(n))
            }
            CalcToken::Percent(p) => {
                self.pos += 1;
                Some(CalcValue::Length(LengthPercentage::from_percent(p / 100.0)))
            }
            CalcToken::Unit(n, unit) => {
                self.pos += 1;
                let text = format!("{n}{unit}");
                resolve_single_length(&text, self.font_size, self.ctx)
                    .map(|px| CalcValue::Length(LengthPercentage::from_px(px)))
            }
            CalcToken::LParen => {
                self.pos += 1;
                let value = self.parse_sum()?;
                if !matches!(self.peek(), Some(CalcToken::RParen)) {
                    return None;
                }
                self.pos += 1;
                Some(value)
            }
            _ => None,
        }
    }
}
