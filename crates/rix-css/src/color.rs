
#![allow(clippy::excessive_precision)]

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CssColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for CssColor {
    fn default() -> Self {
        Self::BLACK
    }
}

impl CssColor {
    pub const TRANSPARENT: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const BLACK: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Self = Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const RED: Self = Self { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const GREEN: Self = Self { r: 0.0, g: 128.0 / 255.0, b: 0.0, a: 1.0 };
    pub const LIME: Self = Self { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
    pub const BLUE: Self = Self { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r: r.clamp(0.0, 1.0),
            g: g.clamp(0.0, 1.0),
            b: b.clamp(0.0, 1.0),
            a: a.clamp(0.0, 1.0),
        }
    }

    pub fn from_rgba_u8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        )
    }

    pub fn to_rgba_u8(self) -> (u8, u8, u8, u8) {
        (
            (self.r * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.g * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.b * 255.0).round().clamp(0.0, 255.0) as u8,
            (self.a * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }

    pub fn to_u32_argb(&self) -> u32 {
        let r = (self.r * 255.0).clamp(0.0, 255.0) as u32;
        let g = (self.g * 255.0).clamp(0.0, 255.0) as u32;
        let b = (self.b * 255.0).clamp(0.0, 255.0) as u32;
        (r << 16) | (g << 8) | b
    }

    pub fn to_hex_string(self) -> String {
        let (r, g, b, a) = self.to_rgba_u8();
        if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    pub fn to_css_string(self) -> String {
        let (r, g, b, a) = self.to_rgba_u8();
        if a == 255 {
            format!("rgb({r}, {g}, {b})")
        } else {
            format!("rgba({r}, {g}, {b}, {:.3})", self.a.clamp(0.0, 1.0))
        }
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        Self::new(self.r, self.g, self.b, alpha)
    }

    pub fn mix(self, other: Self, weight_self: f32) -> Self {
        let t = weight_self.clamp(0.0, 1.0);
        Self::new(
            self.r * t + other.r * (1.0 - t),
            self.g * t + other.g * (1.0 - t),
            self.b * t + other.b * (1.0 - t),
            self.a * t + other.a * (1.0 - t),
        )
    }

    pub fn luminance(self) -> f32 {
        fn lin(c: f32) -> f32 {
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    pub fn is_light(self) -> bool {
        self.luminance() > 0.5
    }

    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let lower = trimmed.to_lowercase();

        if lower == "currentcolor" || lower == "current-color" {
            return None;
        }

        if let Some(hex) = lower.strip_prefix('#') {
            return Self::parse_hex(hex);
        }

        if lower.starts_with("rgb(") || lower.starts_with("rgba(") {
            return Self::parse_rgb_fn(&lower);
        }
        if lower.starts_with("hsl(") || lower.starts_with("hsla(") {
            return Self::parse_hsl_fn(&lower);
        }
        if lower.starts_with("hwb(") {
            return Self::parse_hwb_fn(&lower);
        }
        if lower.starts_with("lab(") {
            return Self::parse_lab_fn(&lower);
        }
        if lower.starts_with("lch(") {
            return Self::parse_lch_fn(&lower);
        }
        if lower.starts_with("oklab(") {
            return Self::parse_oklab_fn(&lower);
        }
        if lower.starts_with("oklch(") {
            return Self::parse_oklch_fn(&lower);
        }
        if lower.starts_with("color-mix(") {
            return Self::parse_color_mix_fn(&lower);
        }
        if lower.starts_with("light-dark(") {
            return Self::parse_light_dark_fn(trimmed);
        }
        if lower.starts_with("color(") {
            return Self::parse_color_fn(&lower);
        }
        if lower.starts_with("gray(") {
            return Self::parse_gray_fn(&lower);
        }
        if lower.starts_with("hsv(") || lower.starts_with("hsva(") {
            return Self::parse_hsv_fn(&lower);
        }

        Self::from_named_color(&lower)
    }

    fn parse_hex(hex: &str) -> Option<Self> {
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some(Self::from_rgba_u8(r, g, b, 255))
            }
            4 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
                Some(Self::from_rgba_u8(r, g, b, a))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Self::from_rgba_u8(r, g, b, 255))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(Self::from_rgba_u8(r, g, b, a))
            }
            _ => None,
        }
    }

    fn split_channels(inner: &str) -> (Vec<String>, Option<String>) {
        let (channels_part, alpha_part) = match inner.split_once('/') {
            Some((l, r)) => (l, Some(r.trim().to_string())),
            None => (inner, None),
        };
        let normalized = channels_part.replace(',', " ");
        let channels: Vec<String> = normalized
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        (channels, alpha_part)
    }

    fn parse_rgb_fn(input: &str) -> Option<Self> {
        let inner = input
            .trim_start_matches("rgba(")
            .trim_start_matches("rgb(")
            .trim_end_matches(')')
            .trim();
        if inner.contains(',') {
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() < 3 {
                return None;
            }
            let r = parse_rgb_component(parts[0])?;
            let g = parse_rgb_component(parts[1])?;
            let (b_str, inline_alpha) = match parts[2].split_once('/') {
                Some((b, a)) => (b.trim(), Some(a.trim())),
                None => (parts[2].trim(), None),
            };
            let b = parse_rgb_component(b_str)?;
            let a = if let Some(ab) = inline_alpha {
                parse_alpha(ab)?
            } else if parts.len() >= 4 {
                let fourth = parts[3].trim();
                let a_str = fourth.split('/').next_back().unwrap_or(fourth).trim();
                parse_alpha(a_str)?
            } else {
                1.0
            };
            return Some(Self::new(r, g, b, a));
        }
        let (channels, alpha) = Self::split_channels(inner);
        if channels.len() < 3 {
            return None;
        }
        let r = parse_rgb_component(&channels[0])?;
        let g = parse_rgb_component(&channels[1])?;
        let b = parse_rgb_component(&channels[2])?;
        let a = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        Some(Self::new(r, g, b, a))
    }

    fn parse_hsl_fn(input: &str) -> Option<Self> {
        let inner = input
            .trim_start_matches("hsla(")
            .trim_start_matches("hsl(")
            .trim_end_matches(')')
            .trim();
        if inner.contains(',') {
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() < 3 {
                return None;
            }
            let h = parse_hue(parts[0])?;
            let s = parse_percent_01(parts[1])?;
            let l = parse_percent_01(parts[2])?;
            let a = if parts.len() >= 4 {
                let fourth = parts[3].trim();
                let a_str = fourth.split('/').next_back().unwrap_or(fourth).trim();
                parse_alpha(a_str)?
            } else {
                1.0
            };
            let (r, g, b) = hsl_to_rgb(h, s, l);
            return Some(Self::new(r, g, b, a));
        }
        let (channels, alpha) = Self::split_channels(inner);
        if channels.len() < 3 {
            return None;
        }
        let h = parse_hue(&channels[0])?;
        let s = parse_percent_01(&channels[1])?;
        let l = parse_percent_01(&channels[2])?;
        let a = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        let (r, g, b) = hsl_to_rgb(h, s, l);
        Some(Self::new(r, g, b, a))
    }

    fn parse_hsv_fn(input: &str) -> Option<Self> {
        let inner = input
            .trim_start_matches("hsva(")
            .trim_start_matches("hsv(")
            .trim_end_matches(')')
            .trim();
        let (channels, alpha) = if inner.contains(',') {
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() < 3 {
                return None;
            }
            let a = if parts.len() >= 4 {
                Some(parts[3].trim().to_string())
            } else {
                None
            };
            (
                vec![
                    parts[0].trim().to_string(),
                    parts[1].trim().to_string(),
                    parts[2].trim().to_string(),
                ],
                a,
            )
        } else {
            Self::split_channels(inner)
        };
        if channels.len() < 3 {
            return None;
        }
        let h = parse_hue(&channels[0])?;
        let s = parse_percent_01(&channels[1])?;
        let v = parse_percent_01(&channels[2])?;
        let a = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        let (r, g, b) = hsv_to_rgb(h, s, v);
        Some(Self::new(r, g, b, a))
    }

    fn parse_hwb_fn(input: &str) -> Option<Self> {
        let inner = input
            .trim_start_matches("hwb(")
            .trim_end_matches(')')
            .trim();
        let (channels, alpha) = Self::split_channels(inner);
        if channels.len() < 3 {
            return None;
        }
        let h = parse_hue(&channels[0])?;
        let w = parse_percent_01(&channels[1])?;
        let b = parse_percent_01(&channels[2])?;
        let a = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        let (r, g, bl) = hwb_to_rgb(h, w, b);
        Some(Self::new(r, g, bl, a))
    }

    fn parse_lab_fn(input: &str) -> Option<Self> {
        let inner = input.trim_start_matches("lab(").trim_end_matches(')').trim();
        let (channels, alpha) = Self::split_channels(inner);
        if channels.len() < 3 {
            return None;
        }
        let l = parse_lab_l(&channels[0])?;
        let a_v = parse_lab_ab(&channels[1])?;
        let b_v = parse_lab_ab(&channels[2])?;
        let alpha = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        let (r, g, b) = lab_to_rgb(l, a_v, b_v);
        Some(Self::new(r, g, b, alpha))
    }

    fn parse_lch_fn(input: &str) -> Option<Self> {
        let inner = input.trim_start_matches("lch(").trim_end_matches(')').trim();
        let (channels, alpha) = Self::split_channels(inner);
        if channels.len() < 3 {
            return None;
        }
        let l = parse_lab_l(&channels[0])?;
        let c = parse_lch_chroma(&channels[1])?;
        let h = parse_hue(&channels[2])?;
        let alpha = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        let (r, g, b) = lch_to_rgb(l, c, h);
        Some(Self::new(r, g, b, alpha))
    }

    fn parse_oklab_fn(input: &str) -> Option<Self> {
        let inner = input
            .trim_start_matches("oklab(")
            .trim_end_matches(')')
            .trim();
        let (channels, alpha) = Self::split_channels(inner);
        if channels.len() < 3 {
            return None;
        }
        let l = parse_oklab_l(&channels[0])?;
        let a_v = parse_oklab_ab(&channels[1])?;
        let b_v = parse_oklab_ab(&channels[2])?;
        let alpha = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        let (r, g, b) = oklab_to_rgb(l, a_v, b_v);
        Some(Self::new(r, g, b, alpha))
    }

    fn parse_oklch_fn(input: &str) -> Option<Self> {
        let inner = input
            .trim_start_matches("oklch(")
            .trim_end_matches(')')
            .trim();
        let (channels, alpha) = Self::split_channels(inner);
        if channels.len() < 3 {
            return None;
        }
        let l = parse_oklab_l(&channels[0])?;
        let c = parse_oklab_chroma(&channels[1])?;
        let h = parse_hue(&channels[2])?;
        let alpha = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        let (r, g, b) = oklch_to_rgb(l, c, h);
        Some(Self::new(r, g, b, alpha))
    }

    fn parse_color_fn(input: &str) -> Option<Self> {
        let inner = input.trim_start_matches("color(").trim_end_matches(')').trim();
        let (space_part, rest) = inner.split_whitespace().collect::<Vec<_>>().split_first()
            .map(|(h, t)| (h.to_string(), t.join(" ")))
            .unwrap_or_default();
        let space = space_part.to_lowercase();
        let (channels, alpha) = Self::split_channels(&rest);
        if channels.len() < 3 {
            return None;
        }
        let c = |s: &str| parse_color_channel(s);
        let (r, g, b) = (c(&channels[0])?, c(&channels[1])?, c(&channels[2])?);
        let alpha = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 4 {
                    parse_alpha(&channels[3])?
                } else {
                    1.0
                }
            }
        };
        let (r, g, b) = match space.as_str() {
            "srgb" => (r, g, b),
            "srgb-linear" => (
                linear_to_srgb(r),
                linear_to_srgb(g),
                linear_to_srgb(b),
            ),
            "display-p3" => display_p3_to_srgb(r, g, b),
            "a98-rgb" => a98_to_srgb(r, g, b),
            "prophoto-rgb" => prophoto_to_srgb(r, g, b),
            "rec2020" => rec2020_to_srgb(r, g, b),
            "xyz" | "xyz-d50" | "xyz-d65" => xyz_to_srgb(r, g, b),
            _ => return None,
        };
        Some(Self::new(r, g, b, alpha))
    }

    fn parse_gray_fn(input: &str) -> Option<Self> {
        let inner = input.trim_start_matches("gray(").trim_end_matches(')').trim();
        if inner.contains(',') {
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.is_empty() {
                return None;
            }
            let v = parse_rgb_component(parts[0])?;
            let a = if parts.len() >= 2 { parse_alpha(parts[1])? } else { 1.0 };
            return Some(Self::new(v, v, v, a));
        }
        let (channels, alpha) = Self::split_channels(inner);
        if channels.is_empty() {
            return None;
        }
        let v = parse_rgb_component(&channels[0])?;
        let a = match alpha {
            Some(a) => parse_alpha(&a)?,
            None => {
                if channels.len() >= 2 {
                    parse_alpha(&channels[1])?
                } else {
                    1.0
                }
            }
        };
        Some(Self::new(v, v, v, a))
    }

    fn parse_color_mix_fn(input: &str) -> Option<Self> {
        let inner = input
            .trim_start_matches("color-mix(")
            .trim_end_matches(')')
            .trim();
        let in_pos = inner.to_lowercase().find("in ")?;
        let after_in = inner[in_pos + 3..].trim();
        let space_end = after_in.find(',')?;
        let space = after_in[..space_end].trim().to_lowercase();
        let rest = after_in[space_end + 1..].trim();
        let (first_str, second_str) = split_top_level_comma(rest)?;
        let (c1, w1) = parse_mix_component(&first_str)?;
        let (c2, w2) = parse_mix_component(&second_str)?;
        let total = w1 + w2;
        if total <= 0.0 {
            return None;
        }
        let t = w1 / total;
        if space.contains("srgb") {
            return Some(c1.mix(c2, t));
        }
        let mixed = match space.as_str() {
            "hsl" => mix_hsl(c1, c2, t),
            "hwb" => mix_hwb(c1, c2, t),
            "lab" => mix_lab(c1, c2, t),
            "lch" => mix_lch(c1, c2, t),
            "oklab" => mix_oklab(c1, c2, t),
            "oklch" => mix_oklch(c1, c2, t),
            "xyz" | "xyz-d50" | "xyz-d65" => mix_xyz(c1, c2, t),
            _ => c1.mix(c2, t),
        };
        Some(mixed)
    }

    fn parse_light_dark_fn(original: &str) -> Option<Self> {
        let lower = original.to_lowercase();
        let inner = lower
            .trim_start_matches("light-dark(")
            .trim_end_matches(')')
            .trim()
            .to_string();
        let (light_str, _) = split_top_level_comma(&inner)?;
        Self::parse(light_str.trim())
    }

    #[allow(clippy::too_many_lines)]
    fn from_named_color(name: &str) -> Option<Self> {
        match name {
            "transparent" => Some(Self::TRANSPARENT),
            "aliceblue" => Some(Self::from_rgba_u8(240, 248, 255, 255)),
            "antiquewhite" => Some(Self::from_rgba_u8(250, 235, 215, 255)),
            "aqua" | "cyan" => Some(Self::from_rgba_u8(0, 255, 255, 255)),
            "aquamarine" => Some(Self::from_rgba_u8(127, 255, 212, 255)),
            "azure" => Some(Self::from_rgba_u8(240, 255, 255, 255)),
            "beige" => Some(Self::from_rgba_u8(245, 245, 220, 255)),
            "bisque" => Some(Self::from_rgba_u8(255, 228, 196, 255)),
            "black" => Some(Self::BLACK),
            "blanchedalmond" => Some(Self::from_rgba_u8(255, 235, 205, 255)),
            "blue" => Some(Self::BLUE),
            "blueviolet" => Some(Self::from_rgba_u8(138, 43, 226, 255)),
            "brown" => Some(Self::from_rgba_u8(165, 42, 42, 255)),
            "burlywood" => Some(Self::from_rgba_u8(222, 184, 135, 255)),
            "cadetblue" => Some(Self::from_rgba_u8(95, 158, 160, 255)),
            "chartreuse" => Some(Self::from_rgba_u8(127, 255, 0, 255)),
            "chocolate" => Some(Self::from_rgba_u8(210, 105, 30, 255)),
            "coral" => Some(Self::from_rgba_u8(255, 127, 80, 255)),
            "cornflowerblue" => Some(Self::from_rgba_u8(100, 149, 237, 255)),
            "cornsilk" => Some(Self::from_rgba_u8(255, 248, 220, 255)),
            "crimson" => Some(Self::from_rgba_u8(220, 20, 60, 255)),
            "darkblue" => Some(Self::from_rgba_u8(0, 0, 139, 255)),
            "darkcyan" => Some(Self::from_rgba_u8(0, 139, 139, 255)),
            "darkgoldenrod" => Some(Self::from_rgba_u8(184, 134, 11, 255)),
            "darkgray" | "darkgrey" => Some(Self::from_rgba_u8(169, 169, 169, 255)),
            "darkgreen" => Some(Self::from_rgba_u8(0, 100, 0, 255)),
            "darkkhaki" => Some(Self::from_rgba_u8(189, 183, 107, 255)),
            "darkmagenta" => Some(Self::from_rgba_u8(139, 0, 139, 255)),
            "darkolivegreen" => Some(Self::from_rgba_u8(85, 107, 47, 255)),
            "darkorange" => Some(Self::from_rgba_u8(255, 140, 0, 255)),
            "darkorchid" => Some(Self::from_rgba_u8(153, 50, 204, 255)),
            "darkred" => Some(Self::from_rgba_u8(139, 0, 0, 255)),
            "darksalmon" => Some(Self::from_rgba_u8(233, 150, 122, 255)),
            "darkseagreen" => Some(Self::from_rgba_u8(143, 188, 143, 255)),
            "darkslateblue" => Some(Self::from_rgba_u8(72, 61, 139, 255)),
            "darkslategray" | "darkslategrey" => Some(Self::from_rgba_u8(47, 79, 79, 255)),
            "darkturquoise" => Some(Self::from_rgba_u8(0, 206, 209, 255)),
            "darkviolet" => Some(Self::from_rgba_u8(148, 0, 211, 255)),
            "deeppink" => Some(Self::from_rgba_u8(255, 20, 147, 255)),
            "deepskyblue" => Some(Self::from_rgba_u8(0, 191, 255, 255)),
            "dimgray" | "dimgrey" => Some(Self::from_rgba_u8(105, 105, 105, 255)),
            "dodgerblue" => Some(Self::from_rgba_u8(30, 144, 255, 255)),
            "firebrick" => Some(Self::from_rgba_u8(178, 34, 34, 255)),
            "floralwhite" => Some(Self::from_rgba_u8(255, 250, 240, 255)),
            "forestgreen" => Some(Self::from_rgba_u8(34, 139, 34, 255)),
            "fuchsia" | "magenta" => Some(Self::from_rgba_u8(255, 0, 255, 255)),
            "gainsboro" => Some(Self::from_rgba_u8(220, 220, 220, 255)),
            "ghostwhite" => Some(Self::from_rgba_u8(248, 248, 255, 255)),
            "gold" => Some(Self::from_rgba_u8(255, 215, 0, 255)),
            "goldenrod" => Some(Self::from_rgba_u8(218, 165, 32, 255)),
            "gray" | "grey" => Some(Self::from_rgba_u8(128, 128, 128, 255)),
            "green" => Some(Self::GREEN),
            "greenyellow" => Some(Self::from_rgba_u8(173, 255, 47, 255)),
            "honeydew" => Some(Self::from_rgba_u8(240, 255, 240, 255)),
            "hotpink" => Some(Self::from_rgba_u8(255, 105, 180, 255)),
            "indianred" => Some(Self::from_rgba_u8(205, 92, 92, 255)),
            "indigo" => Some(Self::from_rgba_u8(75, 0, 130, 255)),
            "ivory" => Some(Self::from_rgba_u8(255, 255, 240, 255)),
            "khaki" => Some(Self::from_rgba_u8(240, 230, 140, 255)),
            "lavender" => Some(Self::from_rgba_u8(230, 230, 250, 255)),
            "lavenderblush" => Some(Self::from_rgba_u8(255, 240, 245, 255)),
            "lawngreen" => Some(Self::from_rgba_u8(124, 252, 0, 255)),
            "lemonchiffon" => Some(Self::from_rgba_u8(255, 250, 205, 255)),
            "lightblue" => Some(Self::from_rgba_u8(173, 216, 230, 255)),
            "lightcoral" => Some(Self::from_rgba_u8(240, 128, 128, 255)),
            "lightcyan" => Some(Self::from_rgba_u8(224, 255, 255, 255)),
            "lightgoldenrodyellow" => Some(Self::from_rgba_u8(250, 250, 210, 255)),
            "lightgray" | "lightgrey" => Some(Self::from_rgba_u8(211, 211, 211, 255)),
            "lightgreen" => Some(Self::from_rgba_u8(144, 238, 144, 255)),
            "lightpink" => Some(Self::from_rgba_u8(255, 182, 193, 255)),
            "lightsalmon" => Some(Self::from_rgba_u8(255, 160, 122, 255)),
            "lightseagreen" => Some(Self::from_rgba_u8(32, 178, 170, 255)),
            "lightskyblue" => Some(Self::from_rgba_u8(135, 206, 250, 255)),
            "lightslategray" | "lightslategrey" => Some(Self::from_rgba_u8(119, 136, 153, 255)),
            "lightsteelblue" => Some(Self::from_rgba_u8(176, 196, 222, 255)),
            "lightyellow" => Some(Self::from_rgba_u8(255, 255, 224, 255)),
            "lime" => Some(Self::LIME),
            "limegreen" => Some(Self::from_rgba_u8(50, 205, 50, 255)),
            "linen" => Some(Self::from_rgba_u8(250, 240, 230, 255)),
            "maroon" => Some(Self::from_rgba_u8(128, 0, 0, 255)),
            "mediumaquamarine" => Some(Self::from_rgba_u8(102, 205, 170, 255)),
            "mediumblue" => Some(Self::from_rgba_u8(0, 0, 205, 255)),
            "mediumorchid" => Some(Self::from_rgba_u8(186, 85, 211, 255)),
            "mediumpurple" => Some(Self::from_rgba_u8(147, 112, 219, 255)),
            "mediumseagreen" => Some(Self::from_rgba_u8(60, 179, 113, 255)),
            "mediumslateblue" => Some(Self::from_rgba_u8(123, 104, 238, 255)),
            "mediumspringgreen" => Some(Self::from_rgba_u8(0, 250, 154, 255)),
            "mediumturquoise" => Some(Self::from_rgba_u8(72, 209, 204, 255)),
            "mediumvioletred" => Some(Self::from_rgba_u8(199, 21, 133, 255)),
            "midnightblue" => Some(Self::from_rgba_u8(25, 25, 112, 255)),
            "mintcream" => Some(Self::from_rgba_u8(245, 255, 250, 255)),
            "mistyrose" => Some(Self::from_rgba_u8(255, 228, 225, 255)),
            "moccasin" => Some(Self::from_rgba_u8(255, 228, 181, 255)),
            "navajowhite" => Some(Self::from_rgba_u8(255, 222, 173, 255)),
            "navy" => Some(Self::from_rgba_u8(0, 0, 128, 255)),
            "oldlace" => Some(Self::from_rgba_u8(253, 245, 230, 255)),
            "olive" => Some(Self::from_rgba_u8(128, 128, 0, 255)),
            "olivedrab" => Some(Self::from_rgba_u8(107, 142, 35, 255)),
            "orange" => Some(Self::from_rgba_u8(255, 165, 0, 255)),
            "orangered" => Some(Self::from_rgba_u8(255, 69, 0, 255)),
            "orchid" => Some(Self::from_rgba_u8(218, 112, 214, 255)),
            "palegoldenrod" => Some(Self::from_rgba_u8(238, 232, 170, 255)),
            "palegreen" => Some(Self::from_rgba_u8(152, 251, 152, 255)),
            "paleturquoise" => Some(Self::from_rgba_u8(175, 238, 238, 255)),
            "palevioletred" => Some(Self::from_rgba_u8(219, 112, 147, 255)),
            "papayawhip" => Some(Self::from_rgba_u8(255, 239, 213, 255)),
            "peachpuff" => Some(Self::from_rgba_u8(255, 218, 185, 255)),
            "peru" => Some(Self::from_rgba_u8(205, 133, 63, 255)),
            "pink" => Some(Self::from_rgba_u8(255, 192, 203, 255)),
            "plum" => Some(Self::from_rgba_u8(221, 160, 221, 255)),
            "powderblue" => Some(Self::from_rgba_u8(176, 224, 230, 255)),
            "purple" => Some(Self::from_rgba_u8(128, 0, 128, 255)),
            "rebeccapurple" => Some(Self::from_rgba_u8(102, 51, 153, 255)),
            "red" => Some(Self::RED),
            "rosybrown" => Some(Self::from_rgba_u8(188, 143, 143, 255)),
            "royalblue" => Some(Self::from_rgba_u8(65, 105, 225, 255)),
            "saddlebrown" => Some(Self::from_rgba_u8(139, 69, 19, 255)),
            "salmon" => Some(Self::from_rgba_u8(250, 128, 114, 255)),
            "sandybrown" => Some(Self::from_rgba_u8(244, 164, 96, 255)),
            "seagreen" => Some(Self::from_rgba_u8(46, 139, 87, 255)),
            "seashell" => Some(Self::from_rgba_u8(255, 245, 238, 255)),
            "sienna" => Some(Self::from_rgba_u8(160, 82, 45, 255)),
            "silver" => Some(Self::from_rgba_u8(192, 192, 192, 255)),
            "skyblue" => Some(Self::from_rgba_u8(135, 206, 235, 255)),
            "slateblue" => Some(Self::from_rgba_u8(106, 90, 205, 255)),
            "slategray" | "slategrey" => Some(Self::from_rgba_u8(112, 128, 144, 255)),
            "snow" => Some(Self::from_rgba_u8(255, 250, 250, 255)),
            "springgreen" => Some(Self::from_rgba_u8(0, 255, 127, 255)),
            "steelblue" => Some(Self::from_rgba_u8(70, 130, 180, 255)),
            "tan" => Some(Self::from_rgba_u8(210, 180, 140, 255)),
            "teal" => Some(Self::from_rgba_u8(0, 128, 128, 255)),
            "thistle" => Some(Self::from_rgba_u8(216, 191, 216, 255)),
            "tomato" => Some(Self::from_rgba_u8(255, 99, 71, 255)),
            "turquoise" => Some(Self::from_rgba_u8(64, 224, 208, 255)),
            "violet" => Some(Self::from_rgba_u8(238, 130, 238, 255)),
            "wheat" => Some(Self::from_rgba_u8(245, 222, 179, 255)),
            "white" => Some(Self::WHITE),
            "whitesmoke" => Some(Self::from_rgba_u8(245, 245, 245, 255)),
            "yellow" => Some(Self::from_rgba_u8(255, 255, 0, 255)),
            "yellowgreen" => Some(Self::from_rgba_u8(154, 205, 50, 255)),
            "canvas" => Some(Self::WHITE),
            "canvastext" => Some(Self::BLACK),
            "linktext" => Some(Self::from_rgba_u8(0, 0, 238, 255)),
            "visitedtext" => Some(Self::from_rgba_u8(85, 26, 139, 255)),
            "activetext" => Some(Self::from_rgba_u8(255, 0, 0, 255)),
            "buttonface" => Some(Self::from_rgba_u8(240, 240, 240, 255)),
            "buttontext" => Some(Self::BLACK),
            "buttonborder" => Some(Self::from_rgba_u8(128, 128, 128, 255)),
            "field" => Some(Self::WHITE),
            "fieldtext" => Some(Self::BLACK),
            "highlight" => Some(Self::from_rgba_u8(0, 120, 215, 255)),
            "highlighttext" => Some(Self::WHITE),
            "mark" => Some(Self::from_rgba_u8(255, 255, 0, 255)),
            "marktext" => Some(Self::BLACK),
            "selectedcontent" | "selecteditem" => Some(Self::from_rgba_u8(0, 120, 215, 255)),
            "background" => Some(Self::WHITE),
            "foreground" | "windowtext" => Some(Self::BLACK),
            "window" => Some(Self::WHITE),
            "windowframe" => Some(Self::from_rgba_u8(128, 128, 128, 255)),
            "infobackground" => Some(Self::from_rgba_u8(255, 255, 224, 255)),
            "infotext" => Some(Self::BLACK),
            "accentcolor" | "accentcolortext" => Some(Self::from_rgba_u8(0, 120, 215, 255)),
            "graytext" | "greytext" => Some(Self::from_rgba_u8(128, 128, 128, 255)),
            _ => None,
        }
    }
}

fn parse_rgb_component(s: &str) -> Option<f32> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(percent) = trimmed.strip_suffix('%') {
        let val: f32 = percent.trim().parse().ok()?;
        Some((val / 100.0).clamp(0.0, 1.0))
    } else {
        let val: f32 = trimmed.parse().ok()?;
        Some((val / 255.0).clamp(0.0, 1.0))
    }
}

fn parse_color_channel(s: &str) -> Option<f32> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(percent) = trimmed.strip_suffix('%') {
        let val: f32 = percent.trim().parse().ok()?;
        Some((val / 100.0).clamp(0.0, 1.0))
    } else {
        let val: f32 = trimmed.parse().ok()?;
        Some(val.clamp(0.0, 1.0))
    }
}

fn parse_alpha(s: &str) -> Option<f32> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(percent) = trimmed.strip_suffix('%') {
        let val: f32 = percent.trim().parse().ok()?;
        Some((val / 100.0).clamp(0.0, 1.0))
    } else {
        let val: f32 = trimmed.parse().ok()?;
        Some(val.clamp(0.0, 1.0))
    }
}

fn parse_hue(s: &str) -> Option<f32> {
    let t = s.trim().to_lowercase();
    if t == "none" {
        return Some(0.0);
    }
    if let Some(v) = t.strip_suffix("deg") {
        return v.trim().parse::<f32>().ok();
    }
    if let Some(v) = t.strip_suffix("grad") {
        let g: f32 = v.trim().parse().ok()?;
        return Some(g * 360.0 / 400.0);
    }
    if let Some(v) = t.strip_suffix("turn") {
        let turns: f32 = v.trim().parse().ok()?;
        return Some(turns * 360.0);
    }
    if let Some(v) = t.strip_suffix("rad") {
        let rad: f32 = v.trim().parse().ok()?;
        return Some(rad * 180.0 / std::f32::consts::PI);
    }
    t.parse::<f32>().ok()
}

fn parse_percent_01(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(v) = t.strip_suffix('%') {
        let n: f32 = v.trim().parse().ok()?;
        Some((n / 100.0).clamp(0.0, 1.0))
    } else {
        let n: f32 = t.parse().ok()?;
        Some(n.clamp(0.0, 1.0))
    }
}

fn parse_lab_l(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(v) = t.strip_suffix('%') {
        let n: f32 = v.trim().parse().ok()?;
        Some(n.clamp(0.0, 100.0))
    } else {
        let n: f32 = t.parse().ok()?;
        Some(n.clamp(0.0, 100.0))
    }
}

fn parse_lab_ab(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    t.parse::<f32>().ok()
}

fn parse_lch_chroma(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(v) = t.strip_suffix('%') {
        let n: f32 = v.trim().parse().ok()?;
        Some((n * 1.5).max(0.0))
    } else {
        let n: f32 = t.parse().ok()?;
        Some(n.max(0.0))
    }
}

fn parse_oklab_l(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(v) = t.strip_suffix('%') {
        let n: f32 = v.trim().parse().ok()?;
        Some((n / 100.0).clamp(0.0, 1.0))
    } else {
        let n: f32 = t.parse().ok()?;
        Some(n.clamp(0.0, 1.0))
    }
}

fn parse_oklab_ab(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(v) = t.strip_suffix('%') {
        let n: f32 = v.trim().parse().ok()?;
        Some(n / 100.0 * 0.4)
    } else {
        t.parse::<f32>().ok()
    }
}

fn parse_oklab_chroma(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if let Some(v) = t.strip_suffix('%') {
        let n: f32 = v.trim().parse().ok()?;
        Some((n / 100.0 * 0.4).max(0.0))
    } else {
        let n: f32 = t.parse().ok()?;
        Some(n.max(0.0))
    }
}

fn split_top_level_comma(s: &str) -> Option<(String, String)> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                return Some((s[..i].trim().to_string(), s[i + 1..].trim().to_string()));
            }
            _ => {}
        }
    }
    None
}

fn parse_mix_component(s: &str) -> Option<(CssColor, f32)> {
    let s = s.trim();
    let (color_str, weight) = match s.rsplit_once(char::is_whitespace) {
        Some((c, w)) if w.trim().ends_with('%') => {
            let pct = w.trim().trim_end_matches('%').trim().parse::<f32>().ok()?;
            (c.trim(), pct / 100.0)
        }
        _ => (s, 0.5),
    };
    let color = CssColor::parse(color_str)?;
    Some((color, weight.clamp(0.0, 1.0)))
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = (h % 360.0 + 360.0) % 360.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r_prime, g_prime, b_prime) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (r_prime + m, g_prime + m, b_prime + m)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h = (h % 360.0 + 360.0) % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    (r1 + m, g1 + m, b1 + m)
}

fn hwb_to_rgb(h: f32, w: f32, b: f32) -> (f32, f32, f32) {
    let (r, g, bl) = hsl_to_rgb(h, 1.0, 0.5);
    let mut w = w;
    let mut b = b;
    if w + b > 1.0 {
        w /= w + b;
        b = 1.0 - w;
    }
    (
        r * (1.0 - w - b) + w,
        g * (1.0 - w - b) + w,
        bl * (1.0 - w - b) + w,
    )
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn lab_to_rgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    const EPS: f32 = 216.0 / 24389.0;
    const KAPPA: f32 = 24389.0 / 27.0;
    const XN: f32 = 95.047;
    const YN: f32 = 100.0;
    const ZN: f32 = 108.883;

    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;

    let fx3 = fx * fx * fx;
    let fz3 = fz * fz * fz;
    let xr = if fx3 > EPS { fx3 } else { (116.0 * fx - 16.0) / KAPPA };
    let yr = if l > KAPPA * EPS {
        let f = (l + 16.0) / 116.0;
        f * f * f
    } else {
        l / KAPPA
    };
    let zr = if fz3 > EPS { fz3 } else { (116.0 * fz - 16.0) / KAPPA };

    xyz_to_rgb(xr * XN / 100.0, yr * YN / 100.0, zr * ZN / 100.0)
}

fn lch_to_rgb(l: f32, c: f32, h: f32) -> (f32, f32, f32) {
    let rad = h.to_radians();
    lab_to_rgb(l, c * rad.cos(), c * rad.sin())
}

fn xyz_to_rgb(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let r_lin = 3.2404542 * x - 1.5371385 * y - 0.4985314 * z;
    let g_lin = -0.9692660 * x + 1.8760108 * y + 0.0415560 * z;
    let b_lin = 0.0556434 * x - 0.2040259 * y + 1.0572252 * z;
    (linear_to_srgb(r_lin), linear_to_srgb(g_lin), linear_to_srgb(b_lin))
}

fn oklab_to_rgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    (
        linear_to_srgb(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
        linear_to_srgb(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
        linear_to_srgb(-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s),
    )
}

fn oklch_to_rgb(l: f32, c: f32, h: f32) -> (f32, f32, f32) {
    let rad = h.to_radians();
    oklab_to_rgb(l, c * rad.cos(), c * rad.sin())
}

fn display_p3_to_srgb(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let rl = srgb_to_linear(r);
    let gl = srgb_to_linear(g);
    let bl = srgb_to_linear(b);
    let x = 0.4865709 * rl + 0.2656677 * gl + 0.1982173 * bl;
    let y = 0.2289746 * rl + 0.6917385 * gl + 0.0792869 * bl;
    let z = 0.0000000 * rl + 0.0451134 * gl + 1.0439444 * bl;
    xyz_to_rgb(x, y, z)
}

fn a98_to_srgb(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let lin = |c: f32| c.powf(2.2);
    let rl = lin(r);
    let gl = lin(g);
    let bl = lin(b);
    let x = 0.5766690 * rl + 0.1855580 * gl + 0.1882280 * bl;
    let y = 0.2973449 * rl + 0.6273636 * gl + 0.0752915 * bl;
    let z = 0.0270314 * rl + 0.0706889 * gl + 0.9913375 * bl;
    xyz_to_rgb(x, y, z)
}

fn prophoto_to_srgb(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let lin = |c: f32| {
        if c <= 0.03125 {
            c / 16.0
        } else {
            ((c + 0.03125) / 1.03125).powf(1.8)
        }
    };
    let rl = lin(r);
    let gl = lin(g);
    let bl = lin(b);
    let x50 = 0.7976749 * rl + 0.1351917 * gl + 0.0313534 * bl;
    let y50 = 0.2880402 * rl + 0.7118741 * gl + 0.0000857 * bl;
    let z50 = 0.0000000 * rl + 0.0000000 * gl + 0.8252100 * bl;
    let x = x50 * 0.9555766;
    let y = y50;
    let z = z50 * 1.0890585;
    xyz_to_rgb(x, y, z)
}

fn rec2020_to_srgb(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let gamma = |c: f32| {
        if c < 0.081 {
            c / 4.5
        } else {
            ((c + 0.0993) / 1.0993).powf(1.0 / 0.45)
        }
    };
    let rl = gamma(r);
    let gl = gamma(g);
    let bl = gamma(b);
    let x = 0.6369580 * rl + 0.1446169 * gl + 0.1688809 * bl;
    let y = 0.2627002 * rl + 0.6779981 * gl + 0.0593017 * bl;
    let z = 0.0000000 * rl + 0.0280727 * gl + 1.0609851 * bl;
    xyz_to_rgb(x, y, z)
}

fn xyz_to_srgb(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    xyz_to_rgb(x, y, z)
}

fn rgb_to_hsl(c: CssColor) -> (f32, f32, f32) {
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == c.r {
        60.0 * (((c.g - c.b) / d) % 6.0)
    } else if max == c.g {
        60.0 * ((c.b - c.r) / d + 2.0)
    } else {
        60.0 * ((c.r - c.g) / d + 4.0)
    };
    ((h + 360.0) % 360.0, s, l)
}

fn rgb_to_hwb(c: CssColor) -> (f32, f32, f32) {
    let (h, _, _) = rgb_to_hsl(c);
    let w = c.r.min(c.g).min(c.b);
    let b = 1.0 - c.r.max(c.g).max(c.b);
    (h, w, b)
}

fn srgb_to_xyz(c: CssColor) -> (f32, f32, f32) {
    let r = srgb_to_linear(c.r);
    let g = srgb_to_linear(c.g);
    let b = srgb_to_linear(c.b);
    (
        0.4124564 * r + 0.3575761 * g + 0.1804375 * b,
        0.2126729 * r + 0.7151522 * g + 0.0721750 * b,
        0.0193339 * r + 0.1191920 * g + 0.9503041 * b,
    )
}

fn xyz_to_lab(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    const EPS: f32 = 216.0 / 24389.0;
    const KAPPA: f32 = 24389.0 / 27.0;
    let f = |t: f32| {
        if t > EPS {
            t.powf(1.0 / 3.0)
        } else {
            (KAPPA * t + 16.0) / 116.0
        }
    };
    let fx = f(x * 100.0 / 95.047);
    let fy = f(y * 100.0 / 100.0);
    let fz = f(z * 100.0 / 108.883);
    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

fn rgb_to_lab(c: CssColor) -> (f32, f32, f32) {
    let (x, y, z) = srgb_to_xyz(c);
    xyz_to_lab(x, y, z)
}

fn rgb_to_lch(c: CssColor) -> (f32, f32, f32) {
    let (l, a, b) = rgb_to_lab(c);
    (l, (a * a + b * b).sqrt(), b.atan2(a).to_degrees().rem_euclid(360.0))
}

fn rgb_to_oklab(c: CssColor) -> (f32, f32, f32) {
    let r = srgb_to_linear(c.r);
    let g = srgb_to_linear(c.g);
    let b = srgb_to_linear(c.b);
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    (
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    )
}

fn rgb_to_oklch(c: CssColor) -> (f32, f32, f32) {
    let (l, a, b) = rgb_to_oklab(c);
    (l, (a * a + b * b).sqrt(), b.atan2(a).to_degrees().rem_euclid(360.0))
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = ((b - a + 540.0) % 360.0) - 180.0;
    (a + d * t).rem_euclid(360.0)
}

fn mix_hsl(a: CssColor, b: CssColor, t: f32) -> CssColor {
    let (h1, s1, l1) = rgb_to_hsl(a);
    let (h2, s2, l2) = rgb_to_hsl(b);
    let (r, g, bl) = hsl_to_rgb(
        lerp_angle(h1, h2, 1.0 - t),
        s1 * t + s2 * (1.0 - t),
        l1 * t + l2 * (1.0 - t),
    );
    CssColor::new(r, g, bl, a.a * t + b.a * (1.0 - t))
}

fn mix_hwb(a: CssColor, b: CssColor, t: f32) -> CssColor {
    let (h1, w1, b1) = rgb_to_hwb(a);
    let (h2, w2, b2) = rgb_to_hwb(b);
    let (r, g, bl) = hwb_to_rgb(
        lerp_angle(h1, h2, 1.0 - t),
        w1 * t + w2 * (1.0 - t),
        b1 * t + b2 * (1.0 - t),
    );
    CssColor::new(r, g, bl, a.a * t + b.a * (1.0 - t))
}

fn mix_lab(a: CssColor, b: CssColor, t: f32) -> CssColor {
    let (l1, a1, b1) = rgb_to_lab(a);
    let (l2, a2, b2) = rgb_to_lab(b);
    let (r, g, bl) = lab_to_rgb(
        l1 * t + l2 * (1.0 - t),
        a1 * t + a2 * (1.0 - t),
        b1 * t + b2 * (1.0 - t),
    );
    CssColor::new(r, g, bl, a.a * t + b.a * (1.0 - t))
}

fn mix_lch(a: CssColor, b: CssColor, t: f32) -> CssColor {
    let (l1, c1, h1) = rgb_to_lch(a);
    let (l2, c2, h2) = rgb_to_lch(b);
    let (r, g, bl) = lch_to_rgb(
        l1 * t + l2 * (1.0 - t),
        c1 * t + c2 * (1.0 - t),
        lerp_angle(h1, h2, 1.0 - t),
    );
    CssColor::new(r, g, bl, a.a * t + b.a * (1.0 - t))
}

fn mix_oklab(a: CssColor, b: CssColor, t: f32) -> CssColor {
    let (l1, x1, y1) = rgb_to_oklab(a);
    let (l2, x2, y2) = rgb_to_oklab(b);
    let (r, g, bl) = oklab_to_rgb(
        l1 * t + l2 * (1.0 - t),
        x1 * t + x2 * (1.0 - t),
        y1 * t + y2 * (1.0 - t),
    );
    CssColor::new(r, g, bl, a.a * t + b.a * (1.0 - t))
}

fn mix_oklch(a: CssColor, b: CssColor, t: f32) -> CssColor {
    let (l1, c1, h1) = rgb_to_oklch(a);
    let (l2, c2, h2) = rgb_to_oklch(b);
    let (r, g, bl) = oklch_to_rgb(
        l1 * t + l2 * (1.0 - t),
        c1 * t + c2 * (1.0 - t),
        lerp_angle(h1, h2, 1.0 - t),
    );
    CssColor::new(r, g, bl, a.a * t + b.a * (1.0 - t))
}

fn mix_xyz(a: CssColor, b: CssColor, t: f32) -> CssColor {
    let (x1, y1, z1) = srgb_to_xyz(a);
    let (x2, y2, z2) = srgb_to_xyz(b);
    let (r, g, bl) = xyz_to_rgb(
        x1 * t + x2 * (1.0 - t),
        y1 * t + y2 * (1.0 - t),
        z1 * t + z2 * (1.0 - t),
    );
    CssColor::new(r, g, bl, a.a * t + b.a * (1.0 - t))
}
