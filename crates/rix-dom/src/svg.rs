
use crate::node::ElementData;

pub const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

pub const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvgViewBox {
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

impl SvgViewBox {
    pub fn new(min_x: f32, min_y: f32, width: f32, height: f32) -> Option<Self> {
        if min_x.is_finite()
            && min_y.is_finite()
            && width.is_finite()
            && height.is_finite()
            && width > 0.0
            && height > 0.0
        {
            Some(Self {
                min_x,
                min_y,
                width,
                height,
            })
        } else {
            None
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        let parts: Vec<f32> = input
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f32>().ok())
            .collect();

        if parts.len() == 4 {
            Self::new(parts[0], parts[1], parts[2], parts[3])
        } else {
            None
        }
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.width / self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvgAlign {
    None,
    XMinYMin,
    XMidYMin,
    XMaxYMin,
    XMinYMid,
    #[default]
    XMidYMid,
    XMaxYMid,
    XMinYMax,
    XMidYMax,
    XMaxYMax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvgMeetOrSlice {
    #[default]
    Meet,
    Slice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SvgPreserveAspectRatio {
    pub align: SvgAlign,
    pub meet_or_slice: SvgMeetOrSlice,
}

impl SvgPreserveAspectRatio {
    pub fn parse(input: &str) -> Self {
        let mut tokens = input.split_ascii_whitespace();
        let mut align_token = tokens.next().unwrap_or("xMidYMid");
        if align_token.eq_ignore_ascii_case("defer") {
            align_token = tokens.next().unwrap_or("xMidYMid");
        }

        let align = match align_token {
            s if s.eq_ignore_ascii_case("none") => SvgAlign::None,
            s if s.eq_ignore_ascii_case("xMinYMin") => SvgAlign::XMinYMin,
            s if s.eq_ignore_ascii_case("xMidYMin") => SvgAlign::XMidYMin,
            s if s.eq_ignore_ascii_case("xMaxYMin") => SvgAlign::XMaxYMin,
            s if s.eq_ignore_ascii_case("xMinYMid") => SvgAlign::XMinYMid,
            s if s.eq_ignore_ascii_case("xMidYMid") => SvgAlign::XMidYMid,
            s if s.eq_ignore_ascii_case("xMaxYMid") => SvgAlign::XMaxYMid,
            s if s.eq_ignore_ascii_case("xMinYMax") => SvgAlign::XMinYMax,
            s if s.eq_ignore_ascii_case("xMidYMax") => SvgAlign::XMidYMax,
            s if s.eq_ignore_ascii_case("xMaxYMax") => SvgAlign::XMaxYMax,
            _ => SvgAlign::XMidYMid,
        };

        let mode_token = tokens.next().unwrap_or("meet");
        let meet_or_slice = if mode_token.eq_ignore_ascii_case("slice") {
            SvgMeetOrSlice::Slice
        } else {
            SvgMeetOrSlice::Meet
        };

        Self {
            align,
            meet_or_slice,
        }
    }

    pub fn compute_transform(
        &self,
        vb: &SvgViewBox,
        viewport_w: f32,
        viewport_h: f32,
    ) -> (f32, f32, f32, f32) {
        if vb.width <= 0.0 || vb.height <= 0.0 || viewport_w <= 0.0 || viewport_h <= 0.0 {
            return (1.0, 1.0, 0.0, 0.0);
        }

        let sx = viewport_w / vb.width;
        let sy = viewport_h / vb.height;

        if self.align == SvgAlign::None {
            return (sx, sy, 0.0, 0.0);
        }

        let uniform_scale = match self.meet_or_slice {
            SvgMeetOrSlice::Meet => sx.min(sy),
            SvgMeetOrSlice::Slice => sx.max(sy),
        };

        let free_x = viewport_w - vb.width * uniform_scale;
        let free_y = viewport_h - vb.height * uniform_scale;

        let offset_x = match self.align {
            SvgAlign::XMinYMin | SvgAlign::XMinYMid | SvgAlign::XMinYMax => 0.0,
            SvgAlign::XMidYMin | SvgAlign::XMidYMid | SvgAlign::XMidYMax => free_x / 2.0,
            SvgAlign::XMaxYMin | SvgAlign::XMaxYMid | SvgAlign::XMaxYMax => free_x,
            SvgAlign::None => 0.0,
        };

        let offset_y = match self.align {
            SvgAlign::XMinYMin | SvgAlign::XMidYMin | SvgAlign::XMaxYMin => 0.0,
            SvgAlign::XMinYMid | SvgAlign::XMidYMid | SvgAlign::XMaxYMid => free_y / 2.0,
            SvgAlign::XMinYMax | SvgAlign::XMidYMax | SvgAlign::XMaxYMax => free_y,
            SvgAlign::None => 0.0,
        };

        (uniform_scale, uniform_scale, offset_x, offset_y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvgFillRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl SvgFillRule {
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("evenodd") {
            Some(Self::EvenOdd)
        } else if trimmed.eq_ignore_ascii_case("nonzero") {
            Some(Self::NonZero)
        } else {
            None
        }
    }
}

pub fn rect_to_path_d(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    rx: Option<f32>,
    ry: Option<f32>,
) -> String {
    let mut rx = rx.unwrap_or(0.0).max(0.0);
    let mut ry = ry.unwrap_or(0.0).max(0.0);

    if rx > 0.0 && ry == 0.0 {
        ry = rx;
    } else if ry > 0.0 && rx == 0.0 {
        rx = ry;
    }

    let rx = rx.min(width / 2.0);
    let ry = ry.min(height / 2.0);

    if rx > 0.0 && ry > 0.0 {
        format!(
            "M {} {} h {} a {} {} 0 0 1 {} {} v {} a {} {} 0 0 1 {} {} h {} a {} {} 0 0 1 {} {} v {} a {} {} 0 0 1 {} {} z",
            x + rx, y,
            width - 2.0 * rx,
            rx, ry, rx, ry,
            height - 2.0 * ry,
            rx, ry, -rx, ry,
            -(width - 2.0 * rx),
            rx, ry, -rx, -ry,
            -(height - 2.0 * ry),
            rx, ry, rx, -ry
        )
    } else {
        format!(
            "M {} {} h {} v {} h {} z",
            x, y, width, height, -width
        )
    }
}

pub fn circle_to_path_d(cx: f32, cy: f32, r: f32) -> String {
    ellipse_to_path_d(cx, cy, r, r)
}

pub fn ellipse_to_path_d(cx: f32, cy: f32, rx: f32, ry: f32) -> String {
    format!(
        "M {} {} a {} {} 0 1 0 {} 0 a {} {} 0 1 0 {} 0 z",
        cx - rx,
        cy,
        rx,
        ry,
        2.0 * rx,
        rx,
        ry,
        -2.0 * rx
    )
}

pub fn line_to_path_d(x1: f32, y1: f32, x2: f32, y2: f32) -> String {
    format!("M {} {} L {} {}", x1, y1, x2, y2)
}

pub fn points_to_path_d(points: &str, close: bool) -> Option<String> {
    let nums: Vec<f32> = points
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f32>().ok())
        .collect();

    if nums.len() < 4 || nums.len() % 2 != 0 {
        return None;
    }

    let mut d = format!("M {} {}", nums[0], nums[1]);
    for chunk in nums[2..].chunks_exact(2) {
        d.push_str(&format!(" L {} {}", chunk[0], chunk[1]));
    }
    if close {
        d.push_str(" Z");
    }
    Some(d)
}

pub fn svg_element_to_path_d(element: &ElementData) -> Option<String> {
    let tag = element.name.local.as_str();
    match tag {
        "path" => element.get_attribute("d").map(ToString::to_string),
        "rect" => {
            let x = element
                .get_attribute("x")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let y = element
                .get_attribute("y")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let width = element
                .get_attribute("width")
                .and_then(|v| v.parse::<f32>().ok())?;
            let height = element
                .get_attribute("height")
                .and_then(|v| v.parse::<f32>().ok())?;
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            let rx = element
                .get_attribute("rx")
                .and_then(|v| v.parse::<f32>().ok());
            let ry = element
                .get_attribute("ry")
                .and_then(|v| v.parse::<f32>().ok());
            Some(rect_to_path_d(x, y, width, height, rx, ry))
        }
        "circle" => {
            let cx = element
                .get_attribute("cx")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let cy = element
                .get_attribute("cy")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let r = element
                .get_attribute("r")
                .and_then(|v| v.parse::<f32>().ok())?;
            if r <= 0.0 {
                return None;
            }
            Some(circle_to_path_d(cx, cy, r))
        }
        "ellipse" => {
            let cx = element
                .get_attribute("cx")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let cy = element
                .get_attribute("cy")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let rx = element
                .get_attribute("rx")
                .and_then(|v| v.parse::<f32>().ok())?;
            let ry = element
                .get_attribute("ry")
                .and_then(|v| v.parse::<f32>().ok())?;
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            Some(ellipse_to_path_d(cx, cy, rx, ry))
        }
        "line" => {
            let x1 = element
                .get_attribute("x1")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let y1 = element
                .get_attribute("y1")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let x2 = element
                .get_attribute("x2")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let y2 = element
                .get_attribute("y2")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            Some(line_to_path_d(x1, y1, x2, y2))
        }
        "polyline" => {
            let points = element.get_attribute("points")?;
            points_to_path_d(points, false)
        }
        "polygon" => {
            let points = element.get_attribute("points")?;
            points_to_path_d(points, true)
        }
        _ => None,
    }
}
