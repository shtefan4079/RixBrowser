
use crate::CssColor;
use lightningcss::traits::Parse;
use lightningcss::values::{
    color::CssColor as ParsedColor,
    gradient::{
        Gradient as ParsedGradient, GradientItem, LineDirection,
        LinearGradient as ParsedLinearGradient,
    },
    image::Image,
    length::LengthValue,
    percentage::DimensionPercentage,
    position::{HorizontalPositionKeyword, VerticalPositionKeyword},
};
use lightningcss::vendor_prefix::VendorPrefix;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinearDirection {
    Angle(f32),
    Corner {
        right: bool,
        bottom: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StopPosition {
    Fraction(f32),
    Pixels(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StopColor {
    Color(CssColor),
    CurrentColor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearColorStop {
    pub color: StopColor,
    pub position: Option<StopPosition>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub direction: LinearDirection,
    pub stops: Vec<LinearColorStop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLinearGradient {
    pub start: (f64, f64),
    pub end: (f64, f64),
    pub stops: Vec<(f32, CssColor)>,
}

impl LinearGradient {
    pub fn parse(input: &str) -> Option<Self> {
        let Image::Gradient(gradient) = Image::parse_string(input).ok()? else {
            return None;
        };
        let ParsedGradient::Linear(parsed) = *gradient else {
            return None;
        };
        Self::from_parsed(parsed)
    }

    pub(crate) fn from_parsed(parsed: ParsedLinearGradient) -> Option<Self> {
        if parsed.vendor_prefix != VendorPrefix::None {
            return None;
        }
        let direction = match parsed.direction {
            LineDirection::Angle(angle) => {
                let degrees = angle.to_degrees();
                if !degrees.is_finite() {
                    return None;
                }
                LinearDirection::Angle(degrees.rem_euclid(360.0))
            }
            LineDirection::Horizontal(side) => LinearDirection::Angle(match side {
                HorizontalPositionKeyword::Left => 270.0,
                HorizontalPositionKeyword::Right => 90.0,
            }),
            LineDirection::Vertical(side) => LinearDirection::Angle(match side {
                VerticalPositionKeyword::Top => 0.0,
                VerticalPositionKeyword::Bottom => 180.0,
            }),
            LineDirection::Corner {
                horizontal,
                vertical,
            } => LinearDirection::Corner {
                right: horizontal == HorizontalPositionKeyword::Right,
                bottom: vertical == VerticalPositionKeyword::Bottom,
            },
        };
        let mut stops = Vec::with_capacity(parsed.items.len());
        for item in parsed.items {
            let GradientItem::ColorStop(stop) = item else {
                return None;
            };
            let color = match stop.color {
                ParsedColor::CurrentColor => StopColor::CurrentColor,
                ParsedColor::RGBA(color) => StopColor::Color(CssColor::from_rgba_u8(
                    color.red,
                    color.green,
                    color.blue,
                    color.alpha,
                )),
                _ => return None,
            };
            let position = match stop.position {
                None => None,
                Some(DimensionPercentage::Percentage(value)) if value.0.is_finite() => {
                    Some(StopPosition::Fraction(value.0))
                }
                Some(DimensionPercentage::Dimension(LengthValue::Px(value)))
                    if value.is_finite() =>
                {
                    Some(StopPosition::Pixels(value))
                }
                _ => return None,
            };
            stops.push(LinearColorStop { color, position });
        }
        (stops.len() >= 2).then_some(Self { direction, stops })
    }

    pub fn resolve(
        &self,
        width: f32,
        height: f32,
        current_color: CssColor,
    ) -> Option<ResolvedLinearGradient> {
        let (width, height) = (f64::from(width), f64::from(height));
        if !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
            || self.stops.len() < 2
        {
            return None;
        }
        let (dx, dy) = match self.direction {
            LinearDirection::Angle(degrees) if degrees.is_finite() => {
                let angle = f64::from(degrees).to_radians();
                (angle.sin(), -angle.cos())
            }
            LinearDirection::Corner { right, bottom } => {
                let diagonal = width.hypot(height);
                (
                    height / diagonal * if right { 1.0 } else { -1.0 },
                    width / diagonal * if bottom { 1.0 } else { -1.0 },
                )
            }
            _ => return None,
        };
        let length = dx.abs() * width + dy.abs() * height;
        let mut offsets = self
            .stops
            .iter()
            .map(|stop| match stop.position {
                Some(StopPosition::Fraction(value)) => Some(f64::from(value)),
                Some(StopPosition::Pixels(value)) => Some(f64::from(value) / length),
                None => None,
            })
            .collect::<Vec<_>>();
        if offsets.iter().flatten().any(|value| !value.is_finite()) {
            return None;
        }
        let last = offsets.len() - 1;
        offsets[0].get_or_insert(0.0);
        offsets[last].get_or_insert(1.0);
        let mut preceding = f64::NEG_INFINITY;
        for offset in offsets.iter_mut().flatten() {
            *offset = offset.max(preceding);
            preceding = *offset;
        }
        let mut previous = 0;
        for next in 1..offsets.len() {
            if let Some(end) = offsets[next] {
                let start = offsets[previous]?;
                let count = (next - previous) as f64;
                for (index, offset) in offsets.iter_mut().enumerate().take(next).skip(previous + 1)
                {
                    *offset = Some(start + (end - start) * (index - previous) as f64 / count);
                }
                previous = next;
            }
        }
        let minimum = offsets[0]?.min(0.0);
        let maximum = offsets[last]?.max(1.0);
        let extent = maximum - minimum;
        let point = |offset: f64| {
            (
                width / 2.0 + dx * length * (offset - 0.5),
                height / 2.0 + dy * length * (offset - 0.5),
            )
        };
        let stops = self
            .stops
            .iter()
            .zip(offsets)
            .map(|(stop, offset)| {
                let color = match stop.color {
                    StopColor::Color(color) => color,
                    StopColor::CurrentColor => current_color,
                };
                Some((((offset? - minimum) / extent) as f32, color))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(ResolvedLinearGradient {
            start: point(minimum),
            end: point(maximum),
            stops,
        })
    }
}
