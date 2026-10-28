
#![deny(unsafe_code)]

pub mod color;
pub mod computed;
pub mod gradient;
pub mod grid;
pub mod rule_index;
pub mod specificity;
pub mod stylesheet;

pub use color::CssColor;
pub use rix_core::InternedStr;
pub use computed::{
    compute_style, compute_style_full, compute_style_with_container, compute_tree_styles,
    compute_tree_styles_with_container, compute_tree_styles_with_scoped_author, resolve_length,
    resolve_length_percentage, BackgroundRepeatMode, ComputedStyle, ContainerQueryContext, Display,
    LayoutContext, LengthPercentage,
};
pub use rule_index::{IndexedRule, RuleIndex};
pub use specificity::Specificity;
pub use stylesheet::{
    clear_media_query_cache, default_user_agent_stylesheet, matches_container_query,
    matches_media_query, matches_supports_query, parse_cssom_rule, parse_cssom_stylesheet,
    parse_declarations, parse_inline_style, preferred_color_scheme, serialize_declarations,
    set_preferred_color_scheme, supports_declaration, ColorScheme, CssomParsedRule, Declaration,
    Rule, StyleSheet,
};
