
use crate::color::CssColor;
use crate::rule_index::RuleIndex;
use crate::specificity::Specificity;
use rix_dom::node::NodeData;
use rix_dom::query::PseudoElement;
use rix_dom::tree::Document;
use rix_dom::NodeId;
use std::collections::HashMap;
use std::sync::Arc;

mod cascade;
mod declaration;
mod inherit;
mod length;
mod shorthands;

pub use cascade::*;
use declaration::*;
use inherit::*;
pub use length::*;
use shorthands::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    None,
    #[default]
    Block,
    Inline,
    InlineBlock,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    Table,
    TableRow,
    TableCell,
    ListItem,
}

impl Display {
    pub fn parse(val: &str) -> Self {
        let lower = val.trim().to_lowercase();
        let keyword = lower.split_whitespace().last().unwrap_or("block");
        match keyword {
            "none" => Self::None,
            "inline" => Self::Inline,
            "inline-block" => Self::InlineBlock,
            "flex" => Self::Flex,
            "inline-flex" => Self::InlineFlex,
            "grid" => Self::Grid,
            "inline-grid" => Self::InlineGrid,
            "flow" | "flow-root" | "block" => Self::Block,
            "table" => Self::Table,
            "inline-table" => Self::Table,
            "table-row" => Self::TableRow,
            "table-cell" => Self::TableCell,
            "table-row-group" | "table-header-group" | "table-footer-group"
            | "table-column-group" | "table-column" | "table-caption" => Self::Table,
            "list-item" => Self::ListItem,
            "contents" => Self::Inline,
            "ruby" | "ruby-base" | "ruby-text" | "ruby-base-container" | "ruby-text-container" => {
                Self::Inline
            }
            _ => Self::Block,
        }
    }

    pub fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    pub fn is_inline_level(self) -> bool {
        matches!(
            self,
            Self::Inline | Self::InlineBlock | Self::InlineFlex | Self::InlineGrid
        )
    }

    pub fn is_block_level(self) -> bool {
        matches!(
            self,
            Self::Block | Self::ListItem | Self::Table | Self::Flex | Self::Grid
        )
    }

    pub fn is_atomic_inline(self) -> bool {
        matches!(self, Self::InlineBlock | Self::InlineFlex | Self::InlineGrid)
    }

    pub fn blockify(self) -> Self {
        match self {
            Self::Inline | Self::InlineBlock => Self::Block,
            Self::InlineFlex => Self::Flex,
            Self::InlineGrid => Self::Grid,
            Self::TableRow | Self::TableCell => Self::Block,
            Self::None | Self::Block | Self::Flex | Self::Grid | Self::Table | Self::ListItem => {
                self
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: CssColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundRepeatMode {
    #[default]
    Repeat,
    NoRepeat,
    Unsupported,
}

impl BoxShadow {
    pub fn parse(val: &str, font_size: f32, ctx: &LayoutContext) -> Option<Self> {
        let trimmed = val.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            return None;
        }
        let layer = split_shadow_layer(trimmed);
        let mut lengths: Vec<f32> = Vec::new();
        let mut color = CssColor::from_rgba_u8(0, 0, 0, 128);
        for token in split_shadow_tokens(&layer) {
            if lengths.len() < 4 {
                if let Some(px) = resolve_length(&token, font_size, ctx) {
                    lengths.push(px);
                    continue;
                }
            }
            if let Some(c) = CssColor::parse(&token) {
                color = c;
            }
        }
        if lengths.len() < 2 {
            return None;
        }
        Some(Self {
            offset_x: lengths[0],
            offset_y: lengths[1],
            blur_radius: lengths.get(2).copied().unwrap_or(0.0).max(0.0),
            spread_radius: lengths.get(3).copied().unwrap_or(0.0),
            color,
        })
    }
}

fn split_shadow_layer(val: &str) -> String {
    let mut depth = 0i32;
    for (i, c) in val.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => return val[..i].to_string(),
            _ => {}
        }
    }
    val.to_string()
}

fn split_shadow_tokens(layer: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in layer.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    out.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: CssColor,
    pub background_color: CssColor,
    pub background_image: Option<crate::gradient::LinearGradient>,
    pub background_repeat_x: BackgroundRepeatMode,
    pub background_repeat_y: BackgroundRepeatMode,
    pub font_size: f32,
    pub font_family: String,
    pub font_weight: u16,
    pub font_style: String,
    pub text_decoration_line: String,
    pub line_height: f32,
    pub line_height_factor: Option<f32>,
    pub line_height_normal: bool,
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_top_auto: bool,
    pub margin_right_auto: bool,
    pub margin_bottom_auto: bool,
    pub margin_left_auto: bool,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub padding_top_percent: Option<f32>,
    pub padding_right_percent: Option<f32>,
    pub padding_bottom_percent: Option<f32>,
    pub padding_left_percent: Option<f32>,
    pub width: Option<f32>,
    pub width_percent: Option<f32>,
    pub height: Option<f32>,
    pub height_percent: Option<f32>,
    pub min_width: Option<f32>,
    pub min_width_percent: Option<f32>,
    pub max_width: Option<f32>,
    pub max_width_percent: Option<f32>,
    pub min_height: Option<f32>,
    pub min_height_percent: Option<f32>,
    pub max_height: Option<f32>,
    pub max_height_percent: Option<f32>,
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
    pub top_percent: Option<f32>,
    pub right_percent: Option<f32>,
    pub bottom_percent: Option<f32>,
    pub left_percent: Option<f32>,
    pub aspect_ratio: Option<f32>,
    pub border_radius: f32,
    pub border_top_left_radius: f32,
    pub border_top_right_radius: f32,
    pub border_bottom_right_radius: f32,
    pub border_bottom_left_radius: f32,
    pub border_width: f32,
    pub border_color: CssColor,
    pub border_top_width: f32,
    pub border_right_width: f32,
    pub border_left_width: f32,
    pub border_top_color: CssColor,
    pub border_right_color: CssColor,
    pub border_left_color: CssColor,
    pub border_bottom_width: f32,
    pub border_bottom_color: CssColor,
    pub flex_direction: String,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Option<f32>,
    pub flex_basis_percent: Option<f32>,
    pub align_items: Option<String>,
    pub justify_content: Option<String>,
    pub align_self: Option<String>,
    pub justify_items: Option<String>,
    pub justify_self: Option<String>,
    pub grid_row_start: Option<i16>,
    pub grid_column_start: Option<i16>,
    pub grid_template_columns: crate::grid::GridTemplate,
    pub grid_template_rows: crate::grid::GridTemplate,
    pub box_sizing: String,
    pub gap: f32,
    pub row_gap: Option<f32>,
    pub column_gap: Option<f32>,
    pub position: String,
    pub visibility: String,
    pub pointer_events: String,
    pub cursor: String,
    pub opacity: f32,
    pub order: i32,
    pub flex_wrap: String,
    pub text_align: String,
    pub vertical_align: String,
    pub align_content: String,
    pub white_space: String,
    pub text_overflow: String,
    pub overflow_x: String,
    pub overflow_y: String,
    pub z_index: i32,
    pub box_shadow: Option<BoxShadow>,
    pub variables: Arc<HashMap<String, String>>,
    pub object_fit: String,
    pub fill: Option<CssColor>,
    pub fill_is_current_color: bool,
    pub stroke: Option<CssColor>,
    pub stroke_is_current_color: bool,
    pub stroke_width: f32,
    pub content: Option<String>,
    pub before: Option<Box<ComputedStyle>>,
    pub after: Option<Box<ComputedStyle>>,
    pub placeholder: Option<Box<ComputedStyle>>,
}

impl ComputedStyle {
    pub fn layout_eq(&self, other: &Self) -> bool {
        let mut adjusted = self.clone();
        adjusted.color = other.color;
        adjusted.background_color = other.background_color;
        adjusted
            .background_image
            .clone_from(&other.background_image);
        adjusted.background_repeat_x = other.background_repeat_x;
        adjusted.background_repeat_y = other.background_repeat_y;
        adjusted.border_color = other.border_color;
        adjusted.border_top_color = other.border_top_color;
        adjusted.border_right_color = other.border_right_color;
        adjusted.border_bottom_color = other.border_bottom_color;
        adjusted.border_left_color = other.border_left_color;
        adjusted.border_radius = other.border_radius;
        adjusted.border_top_left_radius = other.border_top_left_radius;
        adjusted.border_top_right_radius = other.border_top_right_radius;
        adjusted.border_bottom_right_radius = other.border_bottom_right_radius;
        adjusted.border_bottom_left_radius = other.border_bottom_left_radius;
        adjusted.box_shadow = other.box_shadow;
        adjusted.object_fit = other.object_fit.clone();
        adjusted.fill = other.fill;
        adjusted.fill_is_current_color = other.fill_is_current_color;
        adjusted.stroke = other.stroke;
        adjusted.stroke_is_current_color = other.stroke_is_current_color;
        adjusted.stroke_width = other.stroke_width;
        adjusted.opacity = other.opacity;
        adjusted.cursor = other.cursor.clone();
        adjusted.pointer_events = other.pointer_events.clone();
        adjusted.z_index = other.z_index;
        adjusted.text_decoration_line = other.text_decoration_line.clone();
        adjusted.variables.clone_from(&other.variables);
        adjusted == *other
    }

    pub fn corner_radii(&self) -> (f32, f32, f32, f32) {
        (
            self.border_top_left_radius,
            self.border_top_right_radius,
            self.border_bottom_right_radius,
            self.border_bottom_left_radius,
        )
    }
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Block,
            color: CssColor::BLACK,
            background_color: CssColor::TRANSPARENT,
            background_image: None,
            background_repeat_x: BackgroundRepeatMode::Repeat,
            background_repeat_y: BackgroundRepeatMode::Repeat,
            font_size: 16.0,
            font_family: "sans-serif".to_string(),
            font_weight: 400,
            font_style: "normal".to_string(),
            text_decoration_line: "none".to_string(),
            line_height: 19.2,
            line_height_factor: Some(1.2),
            line_height_normal: true,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            margin_top_auto: false,
            margin_right_auto: false,
            margin_bottom_auto: false,
            margin_left_auto: false,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_top_percent: None,
            padding_right_percent: None,
            padding_bottom_percent: None,
            padding_left_percent: None,
            width: None,
            width_percent: None,
            height: None,
            height_percent: None,
            min_width: None,
            min_width_percent: None,
            max_width: None,
            max_width_percent: None,
            min_height: None,
            min_height_percent: None,
            max_height: None,
            max_height_percent: None,
            top: None,
            right: None,
            bottom: None,
            left: None,
            top_percent: None,
            right_percent: None,
            bottom_percent: None,
            left_percent: None,
            aspect_ratio: None,
            border_radius: 0.0,
            border_top_left_radius: 0.0,
            border_top_right_radius: 0.0,
            border_bottom_right_radius: 0.0,
            border_bottom_left_radius: 0.0,
            border_width: 0.0,
            border_color: CssColor::TRANSPARENT,
            border_top_width: 0.0,
            border_right_width: 0.0,
            border_left_width: 0.0,
            border_top_color: CssColor::TRANSPARENT,
            border_right_color: CssColor::TRANSPARENT,
            border_left_color: CssColor::TRANSPARENT,
            border_bottom_width: 0.0,
            border_bottom_color: CssColor::TRANSPARENT,
            flex_direction: "row".to_string(),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: None,
            flex_basis_percent: None,
            align_items: None,
            justify_content: None,
            align_self: None,
            justify_items: None,
            justify_self: None,
            grid_row_start: None,
            grid_column_start: None,
            grid_template_columns: Default::default(),
            grid_template_rows: Default::default(),
            box_sizing: "content-box".to_string(),
            gap: 0.0,
            row_gap: None,
            column_gap: None,
            position: "static".to_string(),
            visibility: "visible".to_string(),
            pointer_events: "auto".to_string(),
            cursor: "auto".to_string(),
            opacity: 1.0,
            order: 0,
            flex_wrap: "nowrap".to_string(),
            text_align: "left".to_string(),
            vertical_align: "baseline".to_string(),
            align_content: "normal".to_string(),
            white_space: "normal".to_string(),
            text_overflow: "clip".to_string(),
            overflow_x: "visible".to_string(),
            overflow_y: "visible".to_string(),
            z_index: 0,
            box_shadow: None,
            variables: Arc::new(HashMap::new()),
            object_fit: "fill".to_string(),
            fill: None,
            fill_is_current_color: false,
            stroke: None,
            stroke_is_current_color: false,
            stroke_width: 1.0,
            content: None,
            before: None,
            after: None,
            placeholder: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutContext {
    pub root_font_size: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
}

impl Default for LayoutContext {
    fn default() -> Self {
        Self {
            root_font_size: 16.0,
            viewport_width: 1280.0,
            viewport_height: 800.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ContainerQueryContext<'a> {
    pub width: f32,
    pub height: f32,
    pub name: Option<&'a str>,
}

impl<'a> Default for ContainerQueryContext<'a> {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
            name: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    UserAgent,
    ShadowHostAuthor,
    Author,
    Inline,
}
