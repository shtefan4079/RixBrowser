use crate::computed::{resolve_length_percentage, LayoutContext};
use lightningcss::{
    properties::grid as ast,
    traits::{Parse, ToCss},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridBreadth {
    Length(f32),
    Percent(f32),
    Flex(f32),
    Auto,
    MinContent,
    MaxContent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrack {
    Breadth(GridBreadth),
    MinMax(GridBreadth, GridBreadth),
    FitContent(GridBreadth),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridRepeat {
    Count(u16),
    AutoFill,
    AutoFit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GridComponent {
    Single(GridTrack),
    Repeat {
        count: GridRepeat,
        tracks: Vec<GridTrack>,
        line_names: Vec<Vec<String>>,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GridTemplate {
    pub components: Vec<GridComponent>,
    pub line_names: Vec<Vec<String>>,
}

impl GridTemplate {
    pub fn parse(value: &str, font_size: f32, context: &LayoutContext) -> Option<Self> {
        let parsed = ast::TrackSizing::parse_string(value).ok()?;
        let ast::TrackSizing::TrackList(list) = parsed else {
            return Some(Self::default());
        };
        let names = |lines: &[lightningcss::values::ident::CustomIdentList<'_>]| {
            lines
                .iter()
                .map(|line| line.iter().map(|name| name.0.to_string()).collect())
                .collect()
        };
        let mut components = Vec::new();
        let mut track_budget = 0usize;
        for item in list.items {
            components.push(match item {
                ast::TrackListItem::TrackSize(track) => {
                    track_budget += 1;
                    GridComponent::Single(lower_track(track, font_size, context)?)
                }
                ast::TrackListItem::TrackRepeat(repeat) => {
                    let count = match repeat.count {
                        ast::RepeatCount::Number(value) => {
                            GridRepeat::Count(u16::try_from(value).ok().filter(|count| *count > 0)?)
                        }
                        ast::RepeatCount::AutoFill => GridRepeat::AutoFill,
                        ast::RepeatCount::AutoFit => GridRepeat::AutoFit,
                    };
                    track_budget += match count {
                        GridRepeat::Count(count) => usize::from(count) * repeat.track_sizes.len(),
                        _ => repeat.track_sizes.len(),
                    };
                    GridComponent::Repeat {
                        count,
                        tracks: repeat
                            .track_sizes
                            .into_iter()
                            .map(|track| lower_track(track, font_size, context))
                            .collect::<Option<Vec<_>>>()?,
                        line_names: names(&repeat.line_names),
                    }
                }
            });
            if track_budget > 4096 {
                return None;
            }
        }
        Some(Self {
            components,
            line_names: names(&list.line_names),
        })
    }
}

fn lower_length(
    value: lightningcss::values::length::LengthPercentage,
    font_size: f32,
    context: &LayoutContext,
) -> Option<GridBreadth> {
    let value = value.to_css_string(Default::default()).ok()?;
    if let Some(lp) = resolve_length_percentage(&value, font_size, context) {
        if lp.is_pure_percent() {
            Some(GridBreadth::Percent(lp.percent))
        } else {
            Some(GridBreadth::Length(
                lp.resolve(context.viewport_width).max(0.0),
            ))
        }
    } else {
        None
    }
}

fn lower_breadth(
    value: ast::TrackBreadth,
    font_size: f32,
    context: &LayoutContext,
) -> Option<GridBreadth> {
    Some(match value {
        ast::TrackBreadth::Auto => GridBreadth::Auto,
        ast::TrackBreadth::MinContent => GridBreadth::MinContent,
        ast::TrackBreadth::MaxContent => GridBreadth::MaxContent,
        ast::TrackBreadth::Flex(value) => GridBreadth::Flex(value),
        ast::TrackBreadth::Length(value) => return lower_length(value, font_size, context),
    })
}

fn lower_track(
    value: ast::TrackSize,
    font_size: f32,
    context: &LayoutContext,
) -> Option<GridTrack> {
    Some(match value {
        ast::TrackSize::TrackBreadth(value) => {
            GridTrack::Breadth(lower_breadth(value, font_size, context)?)
        }
        ast::TrackSize::MinMax { min, max } => GridTrack::MinMax(
            lower_breadth(min, font_size, context)?,
            lower_breadth(max, font_size, context)?,
        ),
        ast::TrackSize::FitContent(value) => {
            GridTrack::FitContent(lower_length(value, font_size, context)?)
        }
    })
}
