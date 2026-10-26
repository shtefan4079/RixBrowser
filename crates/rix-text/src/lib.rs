
#![deny(unsafe_code)]

pub mod font_loader;
pub mod font_match;
pub mod fontconfig;
pub mod layout;
pub mod shaping;

pub use font_loader::{
    FontData, FontDatabase, FontDescriptor, FontId, FontMetrics, FontStyle, FontWeight,
};
pub use layout::{
    InlineObjectMetrics, LayoutLine, ParagraphItem, ParagraphLayout, ParagraphOptions,
    PositionedInlineObject, SpaceBreakBehavior, TextAlign, TextSourceRange, TextSpan,
    TrailingSpaceBehavior,
};
pub use shaping::{contains_rtl_runs, GlyphPosition, ShapedRun, TextShaper};
