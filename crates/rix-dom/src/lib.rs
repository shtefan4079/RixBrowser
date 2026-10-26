
#![deny(unsafe_code)]

pub mod events;
pub mod node;
pub mod query;
pub mod srcset;
pub mod svg;
pub mod tree;

pub use events::{
    Event, EventCallback, EventDispatcher, EventPhase, FocusEvent, InputEvent, KeyboardEvent,
    MouseButton, MouseEvent,
};
pub use indextree::NodeId;
pub use markup5ever::{LocalName, Namespace, QualName};
pub use node::{
    is_valid_custom_element_name, is_valid_shadow_host_name, CanvasBitmap, ElementData, NodeData,
    VideoFrameBitmap,
};
pub use query::{Combinator, ComplexSelector, CompoundSelector, PseudoClass, SimpleSelector};
pub use srcset::{parse_srcset, select_image_source, ImageCandidate};
pub use svg::{
    circle_to_path_d, ellipse_to_path_d, line_to_path_d, points_to_path_d, rect_to_path_d,
    svg_element_to_path_d, SvgAlign, SvgFillRule, SvgMeetOrSlice, SvgPreserveAspectRatio,
    SvgViewBox, SVG_NAMESPACE, XLINK_NAMESPACE,
};
pub use tree::Document;
