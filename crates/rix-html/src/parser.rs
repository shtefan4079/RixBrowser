
use crate::sink::DomTreeSink;
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{parse_document, parse_fragment, ParseOpts, Parser};
use markup5ever::{QualName, TokenizerResult};
use rix_dom::tree::Document;
use std::cell::RefCell;
use std::rc::Rc;

use rix_dom::node::NodeData;
use rix_dom::NodeId;

#[derive(Debug, PartialEq, Eq)]
pub enum ParserStreamStep {
    Script(NodeId),
    Done,
}

pub struct HtmlParserStream {
    parser: Parser<DomTreeSink>,
    document: Rc<RefCell<Document>>,
    finished: bool,
}

impl Default for HtmlParserStream {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlParserStream {
    pub fn new() -> Self {
        let doc = Rc::new(RefCell::new(Document::new()));
        Self::with_document(doc)
    }

    pub fn with_document(document: Rc<RefCell<Document>>) -> Self {
        let sink = DomTreeSink::with_document(document.clone());
        let opts = ParseOpts::default();
        let parser = parse_document(sink, opts);
        Self {
            parser,
            document,
            finished: false,
        }
    }

    pub fn document(&self) -> Rc<RefCell<Document>> {
        self.document.clone()
    }

    pub fn feed(&mut self, chunk: &str) {
        self.parser.input_buffer.push_back(StrTendril::from_slice(chunk));
    }

    pub fn step(&mut self) -> ParserStreamStep {
        if self.finished {
            return ParserStreamStep::Done;
        }
        loop {
            match self.parser.tokenizer.feed(&self.parser.input_buffer) {
                TokenizerResult::Script(node_id) => {
                    attach_declarative_shadow_roots(&mut self.document.borrow_mut());
                    return ParserStreamStep::Script(node_id);
                }
                TokenizerResult::Done => return ParserStreamStep::Done,
                TokenizerResult::EncodingIndicator(_) => continue,
            }
        }
    }

    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        loop {
            match self.parser.tokenizer.feed(&self.parser.input_buffer) {
                TokenizerResult::Done => break,
                TokenizerResult::Script(_) | TokenizerResult::EncodingIndicator(_) => continue,
            }
        }
        self.parser.tokenizer.end();
        attach_declarative_shadow_roots(&mut self.document.borrow_mut());
        self.finished = true;
    }
}

pub fn parse_html(html: &str) -> Document {
    let sink = DomTreeSink::new();
    let opts = ParseOpts::default();
    let mut doc = parse_document(sink, opts).one(html);
    attach_declarative_shadow_roots(&mut doc);
    doc
}

pub fn parse_html_fragment(html: &str, context: Option<&QualName>) -> (Document, Vec<NodeId>) {
    let sink = DomTreeSink::new();
    let opts = ParseOpts::default();
    let context_name = match context {
        Some(qname)
            if !(&*qname.local == "html"
                && (&*qname.ns == "http://www.w3.org/1999/xhtml"
                    || qname.ns.is_empty())) =>
        {
            qname.clone()
        }
        _ => QualName::new(None, markup5ever::ns!(html), markup5ever::local_name!("body")),
    };
    let mut doc = parse_fragment(sink, opts, context_name, Vec::new(), true).one(html);
    attach_declarative_shadow_roots(&mut doc);
    let mut fragment_nodes = Vec::new();
    for child in doc.children(doc.root()) {
        if let Some(NodeData::Element(elem)) = doc.get(child) {
            if &*elem.name.local == "html" {
                for node in doc.children(child) {
                    fragment_nodes.push(node);
                }
                break;
            }
        }
    }
    if fragment_nodes.is_empty() {
        fragment_nodes.extend(doc.children(doc.root()));
    }
    (doc, fragment_nodes)
}

pub fn attach_declarative_shadow_roots(doc: &mut Document) {
    attach_declarative_shadow_roots_in_subtree(doc, doc.root());
}

fn attach_declarative_shadow_roots_in_subtree(doc: &mut Document, root: NodeId) {
    let children: Vec<NodeId> = doc.children(root).collect();
    for child in children {
        let template_info = if let Some(NodeData::Element(elem)) = doc.get(child) {
            if &*elem.name.local == "template" {
                let shadow_mode = elem
                    .get_attribute("shadowrootmode")
                    .or_else(|| elem.get_attribute("shadowroot"))
                    .map(|s| s.to_ascii_lowercase());
                if let Some(mode_lower) = shadow_mode {
                    if mode_lower == "open" || mode_lower == "closed" {
                        let delegates_focus = elem.has_attribute("shadowrootdelegatesfocus");
                        let clonable = elem.has_attribute("shadowrootclonable");
                        let frag = elem.template_contents.unwrap_or(child);
                        Some((mode_lower, delegates_focus, clonable, frag))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some((mode_lower, delegates_focus, clonable, frag)) = template_info {
            let parent = doc.parent(child);
            if let Some(parent) = parent {
                if can_attach_declarative_shadow(doc, parent) {
                    let shadow_root = doc.new_node(NodeData::ShadowRoot {
                        host: parent,
                        mode: mode_lower,
                        delegates_focus,
                        clonable,
                    });
                    if let Some(NodeData::Element(parent_elem)) = doc.get_mut(parent) {
                        parent_elem.shadow_root = Some(shadow_root);
                    }
                    let template_children: Vec<NodeId> = doc.children(frag).collect();
                    for tc in template_children {
                        doc.append_child(shadow_root, tc);
                    }
                    doc.remove_child(child);
                    attach_declarative_shadow_roots_in_subtree(doc, shadow_root);
                    continue;
                }
            }
        }

        attach_declarative_shadow_roots_in_subtree(doc, child);
        let shadow_root_to_recurse = if let Some(NodeData::Element(elem)) = doc.get(child) {
            elem.shadow_root
        } else {
            None
        };
        if let Some(sr) = shadow_root_to_recurse {
            attach_declarative_shadow_roots_in_subtree(doc, sr);
        }
    }
}

fn can_attach_declarative_shadow(doc: &Document, host: NodeId) -> bool {
    let Some(NodeData::Element(elem)) = doc.get(host) else {
        return false;
    };
    if elem.shadow_root.is_some() {
        return false;
    }
    let html_ns = &*elem.name.ns == "http://www.w3.org/1999/xhtml";
    html_ns && rix_dom::is_valid_shadow_host_name(&elem.name.local)
}
