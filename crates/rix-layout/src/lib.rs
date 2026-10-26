
#![deny(unsafe_code)]

pub mod box_model;
pub mod inline;
mod inline_layout;
mod isolated;
pub mod overflow;
pub mod tree;
pub mod taffy_adapter;
pub mod hit_test;

pub use box_model::{BoxDimensions, EdgeSizes, Rect};
pub use inline::{InlineFlowItem, InlineFormattingContext, InlineTextRun};
pub use overflow::{ViewportScrollRange, viewport_scroll_range};
pub use tree::{AnonymousBoxType, LayoutBox, LayoutBoxData, LayoutTree};
pub use taffy_adapter::TaffyLayoutEngine;
pub use hit_test::{hit_test_box_with_scroll, hit_test_tree, hit_test_tree_with_scroll};
