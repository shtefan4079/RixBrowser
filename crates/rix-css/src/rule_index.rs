
use crate::specificity::Specificity;
use crate::stylesheet::{matches_container_query, matches_media_query, matches_supports_query, Declaration, StyleSheet};
use rix_dom::node::NodeData;
use rix_dom::query::{matches_complex, ComplexSelector, SimpleSelector};
use rix_dom::tree::Document;
use rix_dom::NodeId;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct IndexedRule {
    pub selector: ComplexSelector,
    pub declarations: Vec<Declaration>,
    pub specificity: Specificity,
    pub order: usize,
    pub media: Option<String>,
    pub layer: Option<String>,
    pub layer_order: usize,
    pub container: Option<String>,
    pub container_name: Option<String>,
    pub supports: Option<String>,
}

impl IndexedRule {
    pub fn layer_rank(&self) -> usize {
        self.layer_order
    }

    pub fn important_layer_rank(&self) -> usize {
        if self.layer_order == usize::MAX {
            0
        } else {
            usize::MAX - self.layer_order
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuleIndex {
    by_id: HashMap<String, Vec<Rc<IndexedRule>>>,
    by_class: HashMap<String, Vec<Rc<IndexedRule>>>,
    by_attr: HashMap<String, Vec<Rc<IndexedRule>>>,
    by_tag: HashMap<String, Vec<Rc<IndexedRule>>>,
    by_root: Vec<Rc<IndexedRule>>,
    universal: Vec<Rc<IndexedRule>>,
    slotted: Vec<Rc<IndexedRule>>,
    layer_order: Vec<String>,
    next_order: usize,
    rule_count: usize,
}

thread_local! {
    static THREAD_UA_INDEX: std::cell::RefCell<Option<RuleIndex>> = const { std::cell::RefCell::new(None) };
}

impl RuleIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn default_ua() -> Self {
        THREAD_UA_INDEX.with(|cell| {
            let mut guard = cell.borrow_mut();
            if let Some(existing) = guard.as_ref() {
                existing.clone()
            } else {
                let index = RuleIndex::from_stylesheet(&crate::stylesheet::default_user_agent_stylesheet());
                *guard = Some(index.clone());
                index
            }
        })
    }

    pub fn from_stylesheet(sheet: &StyleSheet) -> Self {
        let mut index = Self::new();
        index.add_stylesheet(sheet);
        index
    }

    pub fn layers(&self) -> &[String] {
        &self.layer_order
    }

    pub fn len(&self) -> usize {
        self.rule_count
    }

    pub fn is_empty(&self) -> bool {
        self.rule_count == 0
    }

    fn ensure_layer(&mut self, name: &str) -> usize {
        if let Some(pos) = self.layer_order.iter().position(|l| l == name) {
            return pos;
        }
        self.layer_order.push(name.to_string());
        self.layer_order.len() - 1
    }

    pub fn add_stylesheet(&mut self, sheet: &StyleSheet) {
        for name in &sheet.layer_order {
            self.ensure_layer(name);
        }
        for rule in &sheet.rules {
            let order = self.next_order;
            self.next_order += 1;
            for sel in &rule.selectors {
                let specificity = Specificity::of_complex(sel);
                let layer_rank = match rule.layer.as_deref() {
                    Some(name) => self.ensure_layer(name),
                    None => usize::MAX,
                };
                let candidate = IndexedRule {
                    selector: sel.clone(),
                    declarations: rule.declarations.clone(),
                    specificity,
                    order,
                    media: rule.media.clone(),
                    layer: rule.layer.clone(),
                    layer_order: layer_rank,
                    container: rule.container.clone(),
                    container_name: rule.container_name.clone(),
                    supports: rule.supports.clone(),
                };
                self.push_candidate(Rc::new(candidate));
                self.rule_count += 1;
            }
        }
    }

    fn push_candidate(&mut self, candidate: Rc<IndexedRule>) {
        let Some(key) = candidate.selector.chain.last() else {
            self.universal.push(candidate);
            return;
        };

        let is_slotted = key.predicates.iter().any(|p| {
            matches!(
                p,
                SimpleSelector::Pseudo(rix_dom::query::PseudoClass::Slotted(_))
            )
        });
        if is_slotted {
            self.slotted.push(candidate);
            return;
        }

        let mut id_bucket: Option<String> = None;
        let mut class_bucket: Option<String> = None;
        let mut attr_bucket: Option<String> = None;
        let mut tag_bucket: Option<String> = None;
        let mut is_root = false;

        for pred in &key.predicates {
            match pred {
                SimpleSelector::Id(id) if id_bucket.is_none() => {
                    id_bucket = Some(id.clone());
                }
                SimpleSelector::Class(cls) if class_bucket.is_none() => {
                    class_bucket = Some(cls.clone());
                }
                SimpleSelector::AttributeExact(attr, _, _)
                | SimpleSelector::AttributePrefix(attr, _, _)
                | SimpleSelector::AttributeSuffix(attr, _, _)
                | SimpleSelector::AttributeContains(attr, _, _)
                | SimpleSelector::AttributeWord(attr, _, _)
                | SimpleSelector::AttributeDashMatch(attr, _, _)
                | SimpleSelector::AttributeExists(attr)
                    if attr_bucket.is_none() =>
                {
                    attr_bucket = Some(attr.to_ascii_lowercase());
                }
                SimpleSelector::Tag(tag) if tag_bucket.is_none() => {
                    tag_bucket = Some(tag.to_lowercase());
                }
                SimpleSelector::Pseudo(rix_dom::query::PseudoClass::Root) => {
                    is_root = true;
                }
                _ => {}
            }
        }

        if let Some(id) = id_bucket {
            self.by_id.entry(id).or_default().push(candidate);
        } else if let Some(cls) = class_bucket {
            self.by_class.entry(cls).or_default().push(candidate);
        } else if let Some(attr) = attr_bucket {
            self.by_attr.entry(attr).or_default().push(candidate);
        } else if let Some(tag) = tag_bucket {
            self.by_tag.entry(tag).or_default().push(candidate);
        } else if is_root {
            self.by_root.push(candidate);
        } else {
            self.universal.push(candidate);
        }
    }

    pub fn match_element(&self, doc: &Document, node: NodeId) -> Vec<Rc<IndexedRule>> {
        self.match_element_with_viewport(doc, node, 1280.0, 800.0)
    }

    pub fn match_element_with_viewport(
        &self,
        doc: &Document,
        node: NodeId,
        viewport_w: f32,
        viewport_h: f32,
    ) -> Vec<Rc<IndexedRule>> {
        self.match_element_with_container(doc, node, viewport_w, viewport_h, viewport_w, viewport_h, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn match_element_with_container(
        &self,
        doc: &Document,
        node: NodeId,
        viewport_w: f32,
        viewport_h: f32,
        container_w: f32,
        container_h: f32,
        container_name: Option<&str>,
    ) -> Vec<Rc<IndexedRule>> {
        let elem = match doc.get(node) {
            Some(NodeData::Element(e)) => e,
            _ => return Vec::new(),
        };

        let mut matches: Vec<Rc<IndexedRule>> = Vec::new();

        let check_candidate = |r: &IndexedRule| -> bool {
            if let Some(ref m) = r.media {
                if !matches_media_query(m, viewport_w, viewport_h) {
                    return false;
                }
            }
            if let Some(ref s) = r.supports {
                if !matches_supports_query(s) {
                    return false;
                }
            }
            if let Some(ref c) = r.container {
                if let Some(ref wanted) = r.container_name {
                    if container_name != Some(wanted.as_str()) {
                        return false;
                    }
                }
                if !matches_container_query(c, container_w, container_h) {
                    return false;
                }
            }
            matches_complex(doc, node, &r.selector)
        };

        if let Some(id) = elem.id() {
            if let Some(rules) = self.by_id.get(id) {
                for r in rules {
                    if check_candidate(r) {
                        matches.push(Rc::clone(r));
                    }
                }
            }
        }

        for cls in elem.classes_iter() {
            if let Some(rules) = self.by_class.get(cls) {
                for r in rules {
                    if check_candidate(r) {
                        matches.push(Rc::clone(r));
                    }
                }
            }
        }

        for (qual_name, _) in &elem.attributes {
            let local: &str = &qual_name.local;
            if let Some(rules) = self.by_attr.get(local) {
                for r in rules {
                    if check_candidate(r) {
                        matches.push(Rc::clone(r));
                    }
                }
            }
        }

        let tag_rules = self
            .by_tag
            .get(&*elem.name.local)
            .or_else(|| self.by_tag.get(&elem.name.local.to_lowercase()));
        if let Some(rules) = tag_rules {
            for r in rules {
                if check_candidate(r) {
                    matches.push(Rc::clone(r));
                }
            }
        }

        let is_root_node = doc.parent(node) == Some(doc.root());
        if is_root_node {
            for r in &self.by_root {
                if check_candidate(r) {
                    matches.push(Rc::clone(r));
                }
            }
        }

        for r in &self.universal {
            if check_candidate(r) {
                matches.push(Rc::clone(r));
            }
        }

        matches
    }

    #[allow(clippy::too_many_arguments)]
    pub fn match_slotted_element_with_container(
        &self,
        doc: &Document,
        node: NodeId,
        slot: NodeId,
        viewport_w: f32,
        viewport_h: f32,
        container_w: f32,
        container_h: f32,
        container_name: Option<&str>,
    ) -> Vec<Rc<IndexedRule>> {
        let mut matches: Vec<Rc<IndexedRule>> = Vec::new();
        for r in &self.slotted {
            if let Some(ref m) = r.media {
                if !matches_media_query(m, viewport_w, viewport_h) {
                    continue;
                }
            }
            if let Some(ref s) = r.supports {
                if !matches_supports_query(s) {
                    continue;
                }
            }
            if let Some(ref c) = r.container {
                if let Some(ref wanted) = r.container_name {
                    if container_name != Some(wanted.as_str()) {
                        continue;
                    }
                }
                if !matches_container_query(c, container_w, container_h) {
                    continue;
                }
            }
            if match_slotted_rule(doc, node, slot, r) {
                matches.push(Rc::clone(r));
            }
        }
        matches.sort_by(|a, b| {
            if a.layer_order != b.layer_order {
                return a.layer_order.cmp(&b.layer_order);
            }
            if a.specificity != b.specificity {
                return a.specificity.cmp(&b.specificity);
            }
            a.order.cmp(&b.order)
        });
        matches
    }
}

fn match_slotted_rule(doc: &Document, node: NodeId, slot: NodeId, rule: &IndexedRule) -> bool {
    let Some(key_compound) = rule.selector.chain.last() else {
        return false;
    };
    let Some(slotted_inner) = key_compound.predicates.iter().find_map(|p| match p {
        SimpleSelector::Pseudo(rix_dom::query::PseudoClass::Slotted(inner)) => Some(inner),
        _ => None,
    }) else {
        return false;
    };
    if !rix_dom::query::matches_compound(doc, node, &slotted_inner.predicates) {
        return false;
    }
    let slot_predicates: Vec<_> = key_compound
        .predicates
        .iter()
        .filter(|p| !matches!(p, SimpleSelector::Pseudo(rix_dom::query::PseudoClass::Slotted(_))))
        .cloned()
        .collect();
    if !slot_predicates.is_empty() && !rix_dom::query::matches_compound(doc, slot, &slot_predicates) {
        return false;
    }
    if rule.selector.chain.len() > 1 {
        let mut slot_selector = rule.selector.clone();
        let last_idx = slot_selector.chain.len() - 1;
        slot_selector.chain[last_idx] = rix_dom::query::CompoundSelector {
            predicates: if slot_predicates.is_empty() {
                vec![rix_dom::query::SimpleSelector::Universal]
            } else {
                slot_predicates
            },
            combinator_to_next: None,
        };
        if !rix_dom::query::matches_complex(doc, slot, &slot_selector) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rix_html::parse_html;

    #[test]
    fn test_rule_index_bucket_separation() {
        let css = r#"
            #my-id { color: red; }
            .my-class { color: blue; }
            [data-role="btn"] { color: green; }
            div { color: yellow; }
            :root { --main-color: pink; }
            * { box-sizing: border-box; }
        "#;
        let sheet = StyleSheet::parse(css);
        let index = RuleIndex::from_stylesheet(&sheet);

        assert_eq!(index.by_id.len(), 1);
        assert_eq!(index.by_class.len(), 1);
        assert_eq!(index.by_attr.len(), 1);
        assert_eq!(index.by_tag.len(), 1);
        assert_eq!(index.by_root.len(), 1);
        assert_eq!(index.universal.len(), 1);

        let doc = parse_html(r#"<div id="my-id" class="my-class" data-role="btn">Hello</div>"#);
        let div = doc.query_selector(doc.root(), "div").unwrap();
        let matches = index.match_element(&doc, div);

        assert_eq!(matches.len(), 5);
        assert!(!matches.iter().any(|r| r.selector.chain[0].predicates.iter().any(|p| matches!(p, SimpleSelector::Pseudo(rix_dom::query::PseudoClass::Root)))));

        let html = doc.query_selector(doc.root(), "html").unwrap();
        let root_matches = index.match_element(&doc, html);
        
        assert!(root_matches.iter().any(|r| r.selector.chain[0].predicates.iter().any(|p| matches!(p, SimpleSelector::Pseudo(rix_dom::query::PseudoClass::Root)))));
    }

    #[test]
    fn test_media_and_supports_query_caching() {
        use crate::stylesheet::{matches_media_query, matches_supports_query};
        
        assert!(matches_media_query("(min-width: 500px)", 1280.0, 800.0));
        assert!(matches_media_query("(min-width: 500px)", 1280.0, 800.0)); 
        assert!(!matches_media_query("(min-width: 1500px)", 1280.0, 800.0));

        assert!(matches_supports_query("(display: flex)"));
        assert!(matches_supports_query("(display: flex)")); 
        assert!(!matches_supports_query("(unknown-unsupported-property-xyz: 123)"));
    }

    #[test]
    fn test_inline_style_caching_and_default_ua() {
        use crate::stylesheet::parse_inline_style_shared;
        let d1 = parse_inline_style_shared("color: red; display: none;");
        let d2 = parse_inline_style_shared("color: red; display: none;");
        assert!(std::sync::Arc::ptr_eq(&d1, &d2));

        let ua1 = RuleIndex::default_ua();
        let ua2 = RuleIndex::default_ua();
        assert_eq!(ua1.len(), ua2.len());
    }
}
