
#![deny(unsafe_code)]

pub mod parser;
pub mod sink;

pub use markup5ever::QualName;
pub use parser::{
    attach_declarative_shadow_roots, parse_html, parse_html_fragment, HtmlParserStream,
    ParserStreamStep,
};
pub use sink::DomTreeSink;
