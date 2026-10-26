
use html5ever::tendril::StrTendril;
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use markup5ever::{Attribute, QualName};
use rix_dom::node::{ElementData, NodeData};
use rix_dom::tree::Document;
use rix_dom::NodeId;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct DomTreeSink {
    document: Rc<RefCell<Document>>,
    quirks_mode: Cell<QuirksMode>,
}

impl Default for DomTreeSink {
    fn default() -> Self {
        Self::new()
    }
}

impl DomTreeSink {
    pub fn new() -> Self {
        Self {
            document: Rc::new(RefCell::new(Document::new())),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
        }
    }

    pub fn with_document(document: Rc<RefCell<Document>>) -> Self {
        Self {
            document,
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
        }
    }

    pub fn document(&self) -> Rc<RefCell<Document>> {
        self.document.clone()
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode.get()
    }
}

impl TreeSink for DomTreeSink {
    type Handle = NodeId;
    type Output = Document;
    type ElemName<'a> = std::cell::Ref<'a, QualName>;

    fn finish(self) -> Self::Output {
        match Rc::try_unwrap(self.document) {
            Ok(ref_cell) => ref_cell.into_inner(),
            Err(rc) => rc.borrow().clone(),
        }
    }

    fn parse_error(&self, _msg: std::borrow::Cow<'static, str>) {
    }

    fn get_document(&self) -> Self::Handle {
        self.document.borrow().root()
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        std::cell::Ref::map(self.document.borrow(), |doc| match doc.get(*target) {
            Some(NodeData::Element(elem)) => &elem.name,
            _ => panic!("Expected element node in elem_name"),
        })
    }

    fn create_element(
        &self,
        name: QualName,
        html_attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        let mut elem_data = ElementData::new(name);
        for attr in html_attrs {
            elem_data
                .attributes
                .push((attr.name, attr.value.to_string()));
        }

        if &*elem_data.name.local == "template" {
            let frag = self
                .document
                .borrow_mut()
                .new_node(NodeData::DocumentFragment);
            elem_data.template_contents = Some(frag);
        }

        if &*elem_data.name.local == "iframe" {
            if let Some(srcdoc) = elem_data
                .attributes
                .iter()
                .find(|(k, _)| &*k.local == "srcdoc")
                .map(|(_, v)| v.clone())
            {
                let parsed = crate::parse_html(&srcdoc);
                let parsed_root = parsed.root();
                let mut doc = self.document.borrow_mut();
                let sub_root = doc.new_node(NodeData::Document);
                for child in parsed.children(parsed_root) {
                    let cloned = doc.clone_subtree_from(&parsed, child);
                    doc.append_child(sub_root, cloned);
                }
                elem_data.content_document = Some(sub_root);
            }
        }

        self.document
            .borrow_mut()
            .new_node(NodeData::Element(elem_data))
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.document
            .borrow_mut()
            .new_node(NodeData::Comment(text.to_string()))
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Self::Handle {
        self.document
            .borrow_mut()
            .new_node(NodeData::Comment("pi".into()))
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        let mut doc = self.document.borrow_mut();
        match child {
            NodeOrText::AppendNode(node) => {
                doc.append_child(*parent, node);
            }
            NodeOrText::AppendText(text) => {
                let last_child = doc.children(*parent).last();
                if let Some(last) = last_child {
                    if let Some(NodeData::Text(existing)) = doc.get_mut(last) {
                        existing.push_str(&text);
                        return;
                    }
                }
                let text_node = doc.create_text(&text);
                doc.append_child(*parent, text_node);
            }
        }
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        let mut doc = self.document.borrow_mut();
        match new_node {
            NodeOrText::AppendNode(node) => {
                doc.insert_before(*sibling, node);
            }
            NodeOrText::AppendText(text) => {
                let text_node = doc.create_text(&text);
                doc.insert_before(*sibling, text_node);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        let has_parent = self.document.borrow().parent(*element).is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let mut doc = self.document.borrow_mut();
        let root = doc.root();
        let doctype = doc.new_node(NodeData::DocumentType {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        });
        doc.append_child(root, doctype);
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut doc = self.document.borrow_mut();
        if let Some(NodeData::Element(elem)) = doc.get_mut(*target) {
            for attr in attrs {
                let exists = elem.attributes.iter().any(|(qname, _)| qname == &attr.name);
                if !exists {
                    elem.attributes.push((attr.name, attr.value.to_string()));
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.document.borrow_mut().remove_child(*target);
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let mut doc = self.document.borrow_mut();
        let children: Vec<NodeId> = doc.children(*node).collect();
        for child in children {
            doc.append_child(*new_parent, child);
        }
    }

    fn mark_script_already_started(&self, _node: &Self::Handle) {
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        let doc = self.document.borrow();
        match doc.get(*target) {
            Some(NodeData::Element(elem)) => elem.template_contents.unwrap_or(*target),
            _ => *target,
        }
    }
}
