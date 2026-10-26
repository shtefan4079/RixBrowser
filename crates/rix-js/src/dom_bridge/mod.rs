
use boa_engine::{
    js_string,
    native_function::NativeFunction,
    object::{FunctionObjectBuilder, ObjectInitializer},
    property::{Attribute, PropertyDescriptor},
    Context, JsArgs, JsError, JsNativeError, JsObject, JsResult, JsValue,
};
use rix_dom::node::NodeData;
use rix_dom::tree::Document;
use rix_dom::NodeId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

mod attributes;
mod canvas;
mod class_list;
pub mod collections;
mod computed_style;
pub mod cssom;
mod custom_element_sequence;
mod custom_elements;
mod document;
mod element;
mod element_query;
mod event_handlers;
mod events;
mod font_loading;
mod html_elements;
mod html_image;
mod hyperlink;
mod intersection_observer;
mod layout;
pub mod legacy_key;
pub mod media;
mod media_source;
mod mutation;
mod range;
mod rendered_text;
pub mod prototypes;
mod resize_observer;
pub mod script;
mod shadow_dom;
mod style;

mod text_control;
mod traversal;
mod tree_ops;
mod wrap;

use class_list::*;
use document::*;
use element::*;
use element_query::*;
pub use events::*;
use font_loading::*;
pub use layout::{
    register_layout_api, run_layout_observers, ImageLoadState, LayoutHost, LayoutSnapshot,
};
use mutation::*;
use range::*;
use rendered_text::*;
use style::*;
use text_control::*;
use traversal::*;
use tree_ops::*;
pub mod iframe;
pub use wrap::*;

#[derive(Clone)]
pub struct JsListener {
    pub callback: JsObject,
    pub capture: bool,
    pub once: bool,
    pub passive: bool,
}

pub struct DomBridgeState {
    pub document: Rc<RefCell<Document>>,
    pub main_document: NodeId,
    pub page_visible: bool,
    pub current_script: Option<NodeId>,
    pub owner_documents: HashMap<NodeId, NodeId>,
    pub inert_documents: HashSet<NodeId>,
    pub template_contents_documents: HashMap<NodeId, NodeId>,
    pub listeners: HashMap<(NodeId, String), Vec<JsListener>>,
    pub element_wrappers: HashMap<NodeId, JsObject>,
    pub element_class_lists: HashMap<NodeId, JsObject>,
    pub wrapper_nodes: HashMap<JsObject, NodeId>,
    pub attribute_mutation_callback: Option<JsObject>,
    pub input_values: HashMap<NodeId, String>,
    pub element_scroll_positions: HashMap<NodeId, (f32, f32)>,
    pub script_force_async: HashMap<NodeId, bool>,
    pub canvas_contexts: HashMap<NodeId, canvas::CanvasContextState>,
    pub inline_style_factory: Option<JsObject>,
    pub canvas_bitmap_bytes: usize,
    ranges: HashMap<JsObject, range::RangeData>,
    pub font_face_sets: HashMap<NodeId, JsObject>,
    pub font_face_set_readies: HashMap<NodeId, JsObject>,
    custom_element_definitions: HashMap<String, custom_elements::CustomElementDefinition>,
    custom_element_instances: HashMap<NodeId, custom_elements::CustomElementInstance>,
    custom_element_candidates: HashMap<String, HashSet<NodeId>>,
    custom_element_waiters: HashMap<String, Vec<JsObject>>,
    custom_element_registry: Option<JsObject>,
    custom_element_definition_running: bool,
    custom_element_construction: Option<JsObject>,
    pub custom_element_push: Option<JsObject>,
    pub custom_element_pop: Option<JsObject>,
    pub(super) custom_element_reaction_stack: Vec<(Vec<NodeId>, HashSet<NodeId>)>,
    pub(super) custom_element_backup_queue: VecDeque<NodeId>,
    pub(super) custom_element_backup_set: HashSet<NodeId>,
    pub(super) custom_element_reaction_queues:
        HashMap<NodeId, VecDeque<custom_elements::ElementReaction>>,
    pub(super) custom_element_processing_backup: bool,
    pub document_ready_states: HashMap<NodeId, String>,
    pub script_already_started: HashSet<NodeId>,
    pub iframe_documents: HashMap<NodeId, NodeId>,
    pub document_iframes: HashMap<NodeId, NodeId>,
    pub iframe_windows: HashMap<NodeId, JsObject>,
    pub document_default_views: HashMap<NodeId, JsObject>,
    pub active_mutation_observers: usize,
    pub dom_token_list_prototype: Option<JsObject>,
    pub prototypes: Option<prototypes::DomPrototypes>,
    html_interface_lookup: Option<JsObject>,
}

impl DomBridgeState {
    pub fn with_shared_document(document: Rc<RefCell<Document>>) -> Self {
        let main_document = document.borrow().root();
        let mut document_ready_states = HashMap::new();
        document_ready_states.insert(main_document, "loading".to_string());
        let mut candidates: HashMap<String, HashSet<NodeId>> = HashMap::new();
        {
            let doc = document.borrow();
            let mut pending = vec![main_document];
            while let Some(node) = pending.pop() {
                if let Some(NodeData::Element(element)) = doc.get(node) {
                    if element.name.ns.as_str() == "http://www.w3.org/1999/xhtml"
                        && element.name.local.contains('-')
                        && custom_elements::is_valid_custom_element_name(&element.name.local)
                    {
                        candidates
                            .entry(element.name.local.to_string())
                            .or_default()
                            .insert(node);
                    }
                    if let Some(shadow_root) = element.shadow_root {
                        pending.push(shadow_root);
                    }
                }
                for child in doc.children(node) {
                    pending.push(child);
                }
            }
        }
        let mut state = Self {
            document,
            main_document,
            page_visible: true,
            current_script: None,
            owner_documents: HashMap::new(),
            inert_documents: HashSet::new(),
            template_contents_documents: HashMap::new(),
            listeners: HashMap::new(),
            element_wrappers: HashMap::new(),
            element_class_lists: HashMap::new(),
            wrapper_nodes: HashMap::new(),
            attribute_mutation_callback: None,
            input_values: HashMap::new(),
            element_scroll_positions: HashMap::new(),
            script_force_async: HashMap::new(),
            canvas_contexts: HashMap::new(),
            inline_style_factory: None,
            canvas_bitmap_bytes: 0,
            ranges: HashMap::new(),
            font_face_sets: HashMap::new(),
            font_face_set_readies: HashMap::new(),
            custom_element_definitions: HashMap::new(),
            custom_element_instances: HashMap::new(),
            custom_element_candidates: candidates,
            custom_element_waiters: HashMap::new(),
            custom_element_registry: None,
            custom_element_definition_running: false,
            custom_element_construction: None,
            custom_element_push: None,
            custom_element_pop: None,
            custom_element_reaction_stack: Vec::new(),
            custom_element_backup_queue: VecDeque::new(),
            custom_element_backup_set: HashSet::new(),
            custom_element_reaction_queues: HashMap::new(),
            custom_element_processing_backup: false,
            document_ready_states,
            script_already_started: HashSet::new(),
            iframe_documents: HashMap::new(),
            document_iframes: HashMap::new(),
            iframe_windows: HashMap::new(),
            document_default_views: HashMap::new(),
            active_mutation_observers: 0,
            dom_token_list_prototype: None,
            prototypes: None,
            html_interface_lookup: None,
        };
        state.set_subtree_owner_document(main_document, main_document);
        state
    }

    pub fn new(document: Document) -> Self {
        let main_document = document.root();
        let mut parser_script_nodes = Vec::new();
        let mut candidates: HashMap<String, HashSet<NodeId>> = HashMap::new();
        let mut pending = vec![main_document];
        while let Some(node) = pending.pop() {
            if let Some(NodeData::Element(element)) = document.get(node) {
                if element.name.ns.as_str() == "http://www.w3.org/1999/xhtml" {
                    if element.name.local.as_str() == "script" {
                        parser_script_nodes.push(node);
                    } else if element.name.local.contains('-')
                        && custom_elements::is_valid_custom_element_name(&element.name.local)
                    {
                        candidates
                            .entry(element.name.local.to_string())
                            .or_default()
                            .insert(node);
                    }
                }
                if let Some(shadow_root) = element.shadow_root {
                    pending.push(shadow_root);
                }
            }
            pending.extend(document.children(node).rev());
        }
        let mut state = Self::with_shared_document(Rc::new(RefCell::new(document)));
        state
            .script_already_started
            .extend(parser_script_nodes.iter().copied());
        state
            .script_force_async
            .extend(parser_script_nodes.into_iter().map(|node| (node, false)));
        state.custom_element_candidates.extend(candidates);
        state.set_subtree_owner_document(main_document, main_document);
        state
    }

    pub fn is_script_started(&self, node: NodeId) -> bool {
        self.script_already_started.contains(&node)
    }

    pub fn mark_script_started(&mut self, node: NodeId) {
        self.script_already_started.insert(node);
    }

    pub fn script_async(&self, node: NodeId) -> bool {
        let has_async_attribute = self
            .document
            .borrow()
            .get(node)
            .and_then(|data| match data {
                NodeData::Element(element) => Some(element.get_attribute("async").is_some()),
                _ => None,
            })
            .unwrap_or(false);
        has_async_attribute || self.script_force_async.get(&node).copied().unwrap_or(true)
    }

    pub fn owner_document(&self, node: NodeId) -> Option<NodeId> {
        if node == self.main_document {
            return None;
        }
        if let Some(owner) = self.owner_documents.get(&node).copied() {
            if owner == node && (node == self.main_document || self.inert_documents.contains(&node)) {
                return None;
            }
            return Some(owner);
        }
        if self.document.borrow().get(node).is_some() {
            Some(self.main_document)
        } else {
            None
        }
    }

    pub fn ensure_template_contents_document(&mut self, associated_document: NodeId) -> NodeId {
        if let Some(document) = self
            .template_contents_documents
            .get(&associated_document)
            .copied()
        {
            return document;
        }
        let document = self.document.borrow_mut().new_node(NodeData::Document);
        self.template_contents_documents
            .insert(associated_document, document);
        self.inert_documents.insert(document);
        self.owner_documents.insert(document, document);
        document
    }

    pub fn set_subtree_owner_document(&mut self, root: NodeId, owner: NodeId) {
        {
            let doc = self.document.borrow();
            let is_leaf = doc.children(root).next().is_none()
                && !matches!(doc.get(root), Some(NodeData::Element(el)) if el.template_contents.is_some() || el.shadow_root.is_some());
            if is_leaf {
                if self.owner_documents.get(&root).copied() == Some(owner) {
                    return;
                }
                self.owner_documents.insert(root, owner);
                return;
            }
        }
        let document = self.document.clone();
        let mut pending = vec![(root, owner)];
        let mut visited = HashSet::new();
        while let Some((node, node_owner)) = pending.pop() {
            if !visited.insert(node) {
                continue;
            }
            self.owner_documents.insert(node, node_owner);
            let (children, template_fragment, shadow_root) = {
                let document = document.borrow();
                let children = document.children(node).collect::<Vec<_>>();
                let (template_fragment, shadow_root) = match document.get(node) {
                    Some(NodeData::Element(element)) => {
                        (element.template_contents, element.shadow_root)
                    }
                    _ => (None, None),
                };
                (children, template_fragment, shadow_root)
            };
            pending.extend(children.into_iter().map(|child| (child, node_owner)));
            if let Some(fragment) = template_fragment {
                let template_document = self.ensure_template_contents_document(node_owner);
                pending.push((fragment, template_document));
            }
            if let Some(root) = shadow_root {
                pending.push((root, node_owner));
            }
        }
    }

    pub fn add_listener(
        &mut self,
        node: NodeId,
        event_type: String,
        callback: JsObject,
        capture: bool,
        once: bool,
        passive: bool,
    ) {
        let entry = self.listeners.entry((node, event_type)).or_default();
        if !entry
            .iter()
            .any(|l| JsObject::equals(&l.callback, &callback) && l.capture == capture)
        {
            entry.push(JsListener {
                callback,
                capture,
                once,
                passive,
            });
        }
    }

    pub fn add_bubble_listener(&mut self, node: NodeId, event_type: String, callback: JsObject) {
        self.add_listener(node, event_type, callback, false, false, false);
    }

    pub fn remove_listener(
        &mut self,
        node: NodeId,
        event_type: &str,
        callback: &JsObject,
        capture: Option<bool>,
    ) {
        if let Some(cbs) = self.listeners.get_mut(&(node, event_type.to_string())) {
            cbs.retain(|cb| {
                if !JsObject::equals(&cb.callback, callback) {
                    return true;
                }
                if let Some(cap) = capture {
                    return cb.capture != cap;
                }
                false
            });
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct ListenerOptions {
    pub capture: bool,
    pub once: bool,
    pub passive: bool,
    pub signal: Option<JsObject>,
}

fn parse_listener_options(value: &JsValue, context: &mut Context) -> JsResult<ListenerOptions> {
    if let Some(b) = value.as_boolean() {
        return Ok(ListenerOptions {
            capture: b,
            once: false,
            passive: false,
            signal: None,
        });
    }
    if let Some(obj) = value.as_object() {
        let capture = obj.get(js_string!("capture"), context)?.to_boolean();
        let once = obj.get(js_string!("once"), context)?.to_boolean();
        let passive = obj.get(js_string!("passive"), context)?.to_boolean();
        let signal_val = obj.get(js_string!("signal"), context)?;
        let signal = if signal_val.is_null() || signal_val.is_undefined() {
            None
        } else if let Some(sig_obj) = signal_val.as_object() {
            if !sig_obj.has_property(js_string!("aborted"), context)? {
                return Err(JsNativeError::typ()
                    .with_message("Failed to read the 'signal' property from 'AddEventListenerOptions': member is not of type AbortSignal.")
                    .into());
            }
            Some(sig_obj.clone())
        } else {
            return Err(JsNativeError::typ()
                .with_message("Failed to read the 'signal' property from 'AddEventListenerOptions': member is not of type AbortSignal.")
                .into());
        };
        return Ok(ListenerOptions {
            capture,
            once,
            passive,
            signal,
        });
    }
    Ok(ListenerOptions::default())
}

fn parse_remove_listener_options(value: &JsValue, context: &mut Context) -> JsResult<Option<bool>> {
    if value.is_undefined() {
        return Ok(None);
    }
    if let Some(b) = value.as_boolean() {
        return Ok(Some(b));
    }
    if let Some(obj) = value.as_object() {
        let capture = obj.get(js_string!("capture"), context)?.to_boolean();
        return Ok(Some(capture));
    }
    Ok(Some(false))
}

fn register_dom_interfaces(context: &mut Context) -> JsResult<()> {
    let construction = context.eval(boa_engine::Source::from_bytes(
        r#"
        (function() {
            const customElementConstructionStack = [];
            function illegal(name, Parent) {
                const Constructor = function() { throw new TypeError('Illegal constructor: ' + name); };
                Object.defineProperty(Constructor, 'name', {value: name});
                Constructor.prototype = Object.create(Parent.prototype, {
                    constructor: {value: Constructor, writable: true, configurable: true},
                    [Symbol.toStringTag]: {value: name, configurable: true},
                });
                Object.setPrototypeOf(Constructor, Parent);
                globalThis[name] = Constructor;
                return Constructor;
            }
            const Node = illegal('Node', EventTarget);
            const Element = illegal('Element', Node);
            const HTMLElement = function HTMLElement() {
                const entry = customElementConstructionStack[customElementConstructionStack.length - 1];
                if (!new.target || !entry || entry.constructor !== new.target || entry.constructed) {
                    throw new TypeError('Illegal constructor: HTMLElement');
                }
                entry.constructed = true;
                if (new.target.prototype && typeof new.target.prototype === 'object') {
                    Object.setPrototypeOf(entry.element, new.target.prototype);
                }
                return entry.element;
            };
            HTMLElement.prototype = Object.create(Element.prototype, {
                constructor: {value: HTMLElement, writable: true, configurable: true},
                [Symbol.toStringTag]: {value: 'HTMLElement', configurable: true},
            });
            Object.setPrototypeOf(HTMLElement, Element);
            globalThis.HTMLElement = HTMLElement;
            const SVGElement = illegal('SVGElement', Element);
            const SVGGraphicsElement = illegal('SVGGraphicsElement', SVGElement);
            illegal('SVGSVGElement', SVGGraphicsElement);
            illegal('SVGPathElement', SVGGraphicsElement);
            illegal('SVGGElement', SVGGraphicsElement);
            illegal('SVGCircleElement', SVGGraphicsElement);
            illegal('SVGRectElement', SVGGraphicsElement);
            illegal('SVGUseElement', SVGGraphicsElement);
            illegal('SVGDefsElement', SVGGraphicsElement);
            illegal('SVGClipPathElement', SVGGraphicsElement);
            illegal('SVGPolylineElement', SVGGraphicsElement);
            illegal('SVGPolygonElement', SVGGraphicsElement);
            illegal('SVGLineElement', SVGGraphicsElement);
            illegal('SVGEllipseElement', SVGGraphicsElement);
            illegal('SVGTextElement', SVGGraphicsElement);

            Object.defineProperty(SVGElement.prototype, 'ownerSVGElement', {
                get() {
                    let cur = this.parentNode;
                    while (cur && cur.nodeType === 1) {
                        if (cur.namespaceURI === 'http://www.w3.org/2000/svg' && cur.localName === 'svg') {
                            return cur;
                        }
                        cur = cur.parentNode;
                    }
                    return null;
                },
                enumerable: true,
                configurable: true,
            });

            Object.defineProperty(SVGElement.prototype, 'viewportElement', {
                get() {
                    return this.ownerSVGElement;
                },
                enumerable: true,
                configurable: true,
            });
            const CharacterData = illegal('CharacterData', Node);
            illegal('Document', Node);
            illegal('FontFaceSet', EventTarget);
            illegal('DocumentType', Node);
            illegal('DOMImplementation', Object);
            illegal('DOMTokenList', Object);
            illegal('HTMLCollection', Object);
            illegal('NodeList', Object);
            illegal('Range', Object);
            illegal('CustomElementRegistry', Object);
            illegal('HTMLTemplateElement', HTMLElement);
            illegal('HTMLInputElement', HTMLElement);
            illegal('HTMLTextAreaElement', HTMLElement);
            illegal('HTMLImageElement', HTMLElement);
            illegal('HTMLUnknownElement', HTMLElement);
            const htmlElementConstructors = new Map([
                ['HTMLElement', HTMLElement], ['HTMLTemplateElement', HTMLTemplateElement],
                ['HTMLInputElement', HTMLInputElement], ['HTMLTextAreaElement', HTMLTextAreaElement],
                ['HTMLImageElement', HTMLImageElement], ['HTMLUnknownElement', HTMLUnknownElement],
            ]);
            const htmlTagInterfaces = new Map();
            function factory(name, Parent, create) {
                const Constructor = function(value) {
                    if (!new.target) throw new TypeError(name + ' requires new');
                    return create(value === undefined ? '' : String(value));
                };
                Object.defineProperty(Constructor, 'name', {value: name});
                Constructor.prototype = Object.create(Parent.prototype, {
                    constructor: {value: Constructor, writable: true, configurable: true},
                    [Symbol.toStringTag]: {value: name, configurable: true},
                });
                Object.setPrototypeOf(Constructor, Parent);
                globalThis[name] = Constructor;
            }
            factory('DocumentFragment', Node, () => document.createDocumentFragment());
            illegal('ShadowRoot', DocumentFragment);
            factory('Text', CharacterData, value => document.createTextNode(value));
            factory('Comment', CharacterData, value => document.createComment(value));
            illegal('CDATASection', globalThis.Text);
            illegal('ProcessingInstruction', CharacterData);
            const constants = {
                ELEMENT_NODE: 1, TEXT_NODE: 3, CDATA_SECTION_NODE: 4, COMMENT_NODE: 8,
                DOCUMENT_NODE: 9, DOCUMENT_TYPE_NODE: 10, PROCESSING_INSTRUCTION_NODE: 7,
                DOCUMENT_FRAGMENT_NODE: 11,
            };
            for (const [name, value] of Object.entries(constants)) {
                Object.defineProperty(Node, name, {value, enumerable: true});
                Object.defineProperty(Node.prototype, name, {value, enumerable: true});
            }
            return {
                push(element, constructor) {
                    customElementConstructionStack.push({element, constructor, constructed: false});
                },
                pop() { return customElementConstructionStack.pop(); },
                defineHTMLInterface(name, parentName) {
                    let Constructor = globalThis[name];
                    if (typeof Constructor !== 'function') Constructor = illegal(name, globalThis[parentName]);
                    htmlElementConstructors.set(name, Constructor);
                    return Constructor;
                },
                registerHTMLTag(tagName, interfaceName) {
                    const Constructor = htmlElementConstructors.get(interfaceName);
                    if (!Constructor) throw new TypeError('Unknown HTML element interface: ' + interfaceName);
                    htmlTagInterfaces.set(String(tagName).toLowerCase(), Constructor);
                },
                interfaceForTag(tagName) {
                    const name = String(tagName).toLowerCase();
                    const registered = htmlTagInterfaces.get(name);
                    if (registered) return registered;
                    return name.includes('-')
                        ? htmlElementConstructors.get('HTMLElement')
                        : htmlElementConstructors.get('HTMLUnknownElement');
                },
            };
        })();
        "#,
    ))?;
    let construction = construction.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("custom element construction bridge missing")
    })?;
    let push = construction
        .get(js_string!("push"), context)?
        .as_object();
    let pop = construction
        .get(js_string!("pop"), context)?
        .as_object();
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        let mut borrowed = state.borrow_mut();
        borrowed.custom_element_construction = Some(construction);
        borrowed.custom_element_push = push;
        borrowed.custom_element_pop = pop;
    }
    html_elements::register_html_element_interfaces(context)?;
    hyperlink::register_hyperlink_interfaces(context)?;
    canvas::register_canvas_api(context)?;
    range::register_range_interface(context)?;

    type Method = (
        &'static str,
        usize,
        fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
    );
    fn methods(context: &mut Context, interface: &str, methods: &[Method]) -> JsResult<()> {
        let constructor = context
            .global_object()
            .get(js_string!(interface), context)?;
        let prototype = constructor
            .as_object()
            .expect("registered DOM constructor")
            .get(js_string!("prototype"), context)?
            .as_object()
            .expect("DOM interface prototype");
        for &(name, length, function) in methods {
            let function =
                FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(function))
                    .name(js_string!(name))
                    .length(length)
                    .build();
            prototype.define_property_or_throw(
                js_string!(name),
                PropertyDescriptor::builder()
                    .value(function)
                    .writable(true)
                    .configurable(true)
                    .build(),
                context,
            )?;
        }
        Ok(())
    }
    methods(
        context,
        "Node",
        &[
            ("addEventListener", 2, element_add_event_listener),
            ("removeEventListener", 2, element_remove_event_listener),
            ("dispatchEvent", 1, element_dispatch_event),
            ("appendChild", 1, element_append_child),
            ("removeChild", 1, element_remove_child),
            ("insertBefore", 2, element_insert_before),
            ("replaceChild", 2, element_replace_child),
            ("cloneNode", 1, element_clone_node),
            ("contains", 1, element_contains),
            ("__getParentNode", 0, element_get_parent_node),
            ("__getChildren", 0, element_get_children),
            ("__getChildNodes", 0, element_get_child_nodes),
            ("__getFirstChild", 0, element_get_first_child),
            ("__getLastChild", 0, element_get_last_child),
            ("__getNextSibling", 0, element_get_next_sibling),
            ("__getPrevSibling", 0, element_get_prev_sibling),
            ("__getFirstElementChild", 0, element_get_first_element_child),
            ("__getLastElementChild", 0, element_get_last_element_child),
            ("__getNextElementSibling", 0, element_get_next_element_sibling),
            ("__getPrevElementSibling", 0, element_get_prev_element_sibling),
            ("__getChildElementCount", 0, element_get_child_element_count),
            ("__getTextContent", 0, node_get_text_content),
            ("__setTextContent", 1, node_set_text_content),
            ("__getOwnerDocument", 0, node_get_owner_document),
            ("__isConnected", 0, node_is_connected),
            ("__getRootNode", 1, shadow_dom::node_get_root_node),
        ],
    )?;
    methods(
        context,
        "Element",
        &[
            ("getAttribute", 1, element_get_attribute),
            ("setAttribute", 2, element_set_attribute),
            ("removeAttribute", 1, element_remove_attribute),
            ("hasAttribute", 1, element_has_attribute),
            ("getAttributeNames", 0, element_get_attribute_names),
            ("attachShadow", 1, shadow_dom::element_attach_shadow),
            ("__getShadowRoot", 0, shadow_dom::element_get_shadow_root),
            (
                "__getAttributeRecords",
                0,
                attributes::element_get_attribute_records,
            ),
            ("__getAttributeNS", 2, attributes::element_get_attribute_ns),
            (
                "__setAttributeRecord",
                4,
                attributes::element_set_attribute_record,
            ),
            (
                "__removeAttributeNS",
                2,
                attributes::element_remove_attribute_ns,
            ),
            ("__hasAttributeNS", 2, attributes::element_has_attribute_ns),
            ("matches", 1, element_matches),
            ("closest", 1, element_closest),
            ("remove", 0, element_remove_self),
            ("__getElementsByTagName", 1, element_get_elements_by_tag),
            ("__getElementsByClassName", 1, element_get_elements_by_class),
            ("querySelector", 1, element_query_selector),
            ("__querySelectorAll", 1, element_query_selector_all),
            ("__styleGet", 1, element_style_get),
            ("__styleSet", 3, element_style_set),
            ("__styleRemove", 1, element_style_remove),
            ("__stylePriority", 1, element_style_priority),
            ("__styleLength", 0, element_style_length),
            ("__styleItem", 1, element_style_item),
            ("__getInnerHTML", 0, node_get_inner_html),
            ("__setInnerHTML", 1, node_set_inner_html),
            ("__getClassList", 0, element_get_class_list),
        ],
    )?;
    methods(
        context,
        "DOMTokenList",
        &[
            ("item", 1, class_list_item),
            ("contains", 1, class_list_contains),
            ("add", 1, class_list_add),
            ("remove", 1, class_list_remove),
            ("toggle", 1, class_list_toggle),
            ("replace", 2, class_list_replace),
            ("__getValue", 0, class_list_get_value),
            ("__setValue", 1, class_list_set_value),
            ("__getTokens", 0, class_list_get_tokens),
            ("__getLength", 0, class_list_length),
        ],
    )?;
    let dom_token_list_proto = context
        .global_object()
        .get(js_string!("DOMTokenList"), context)?
        .as_object()
        .and_then(|ctor| ctor.get(js_string!("prototype"), context).ok())
        .and_then(|proto| proto.as_object());
    if let (Some(state), Some(proto)) = (context.get_data::<Rc<RefCell<DomBridgeState>>>(), dom_token_list_proto) {
        state.borrow_mut().dom_token_list_prototype = Some(proto);
    }
    methods(
        context,
        "HTMLElement",
        &[
            ("blur", 0, element_blur),
            ("focus", 0, element_focus),
            ("click", 0, element_click),
            ("__getInnerText", 0, element_get_inner_text),
        ],
    )?;
    for interface in ["HTMLInputElement", "HTMLTextAreaElement"] {
        methods(
            context,
            interface,
            &[
                ("select", 0, element_select),
                ("setSelectionRange", 2, input_set_selection_range),
                ("__getSelection", 0, input_get_selection),
            ],
        )?;
    }
    methods(
        context,
        "HTMLTemplateElement",
        &[("__getContent", 0, template_get_content)],
    )?;
    methods(
        context,
        "CharacterData",
        &[("remove", 0, element_remove_self)],
    )?;
    methods(
        context,
        "DocumentFragment",
        &[
            ("querySelector", 1, element_query_selector),
            ("__querySelectorAll", 1, element_query_selector_all),
        ],
    )?;
    methods(
        context,
        "ShadowRoot",
        &[
            ("__getHost", 0, shadow_dom::shadow_root_get_host),
            ("__getMode", 0, shadow_dom::shadow_root_get_mode),
            (
                "__getDelegatesFocus",
                0,
                shadow_dom::shadow_root_get_delegates_focus,
            ),
            (
                "__getClonable",
                0,
                shadow_dom::shadow_root_get_clonable,
            ),
            ("__getInnerHTML", 0, node_get_inner_html),
            ("__setInnerHTML", 1, node_set_inner_html),
        ],
    )?;
    methods(
        context,
        "Document",
        &[
            ("getElementById", 1, document_get_element_by_id),
            ("querySelector", 1, document_query_selector),
            ("__querySelectorAll", 1, document_query_selector_all),
            ("createElement", 1, document_create_element),
            ("createElementNS", 2, document_create_element_ns),
            ("createTextNode", 1, document_create_text_node),
            ("createComment", 1, document_create_comment),
            ("createDocumentFragment", 0, document_create_fragment),
            ("createRange", 0, document_create_range),
            ("__getElementsByTagName", 1, element_get_elements_by_tag),
            ("__getElementsByClassName", 1, element_get_elements_by_class),
            ("importNode", 2, document_import_node),
            ("adoptNode", 1, document_adopt_node),
            ("__getActiveElement", 0, document_active_element),
            ("__getCurrentScript", 0, document_current_script),
            ("__getBaseURI", 0, document_base_uri),
            ("__getFontFaceSet", 0, document_get_font_face_set),
            ("__getDoctype", 0, document_doctype),
            ("__hasDefaultView", 0, document_has_default_view),
            ("__getDefaultView", 0, document_get_default_view),
            ("__getVisibilityState", 0, document_visibility_state),
            ("__getHidden", 0, document_hidden),
            ("__getReadyState", 0, document_ready_state),
            ("__getTitle", 0, document_get_title),
            ("__setTitle", 1, document_set_title),
            ("__getCookie", 0, document_get_cookie),
            ("__setCookie", 1, document_set_cookie),
            ("__getRevision", 0, document_get_revision),
        ],
    )?;
    methods(
        context,
        "DOMImplementation",
        &[("createHTMLDocument", 1, document_create_html_document)],
    )?;
    register_font_face_set_interface(context)?;

    context.eval(boa_engine::Source::from_bytes(r#"
        (function() {
            const documentImplementations = new WeakMap();
            function property(prototype, name, get, set) {
                Object.defineProperty(prototype, name, {get, set, configurable: true, enumerable: true});
            }
            property(Node.prototype, 'textContent', function() { return this.__getTextContent(); }, function(value) { this.__setTextContent(value == null ? '' : String(value)); });
            property(Node.prototype, 'nodeValue', function() { return this.nodeType === 3 || this.nodeType === 8 ? this.textContent : null; }, function(value) { if (this.nodeType === 3 || this.nodeType === 8) this.textContent = value; });
            property(Node.prototype, 'parentNode', function() { return this.__getParentNode(); });
            property(Node.prototype, 'parentElement', function() { const parent = this.__getParentNode(); return parent && parent.nodeType === 1 ? parent : null; });
            property(Node.prototype, 'firstChild', function() { return this.__getFirstChild(); });
            property(Node.prototype, 'lastChild', function() { return this.__getLastChild(); });
            property(Node.prototype, 'nextSibling', function() { return this.__getNextSibling(); });
            property(Node.prototype, 'previousSibling', function() { return this.__getPrevSibling(); });
            property(Node.prototype, 'ownerDocument', function() { return this.nodeType === 9 ? null : this.__getOwnerDocument(); });
            property(Node.prototype, 'isConnected', function() { return this.__isConnected(); });
            Node.prototype.hasChildNodes = function() { return this.childNodes.length !== 0; };
            Node.prototype.getRootNode = function(options) { return this.__getRootNode(options == null ? false : Boolean(options.composed)); };
            property(CharacterData.prototype, 'data', function() { return this.textContent; }, function(value) { this.textContent = value; });
            property(CharacterData.prototype, 'length', function() { return this.data.length; });
            property(DocumentType.prototype, 'name', function() { return this.nodeName; });
            property(DOMTokenList.prototype, 'value', function() { return this.__getValue(); }, function(value) { this.__setValue(value); });
            property(DOMTokenList.prototype, 'length', function() { return this.__getLength(); });
            DOMTokenList.prototype.keys = function() { return this.__getTokens().keys(); };
            DOMTokenList.prototype.values = function() { return this.__getTokens().values(); };
            DOMTokenList.prototype.entries = function() { return this.__getTokens().entries(); };
            DOMTokenList.prototype.forEach = function(callback, thisArg) {
                if (typeof callback !== 'function') throw new TypeError('DOMTokenList.forEach requires a callback');
                const values = this.__getTokens();
                values.forEach((value, index) => callback.call(thisArg, value, index, this));
            };
            DOMTokenList.prototype.toString = function() { return this.value; };
            DOMTokenList.prototype[Symbol.iterator] = DOMTokenList.prototype.values;
            for (const prototype of [Element.prototype, Document.prototype, DocumentFragment.prototype]) {
                property(prototype, 'firstElementChild', function() { return this.__getFirstElementChild(); });
                property(prototype, 'lastElementChild', function() { return this.__getLastElementChild(); });
                property(prototype, 'childElementCount', function() { return this.__getChildElementCount(); });
            }
            property(Element.prototype, 'nextElementSibling', function() { return this.__getNextElementSibling(); });
            property(Element.prototype, 'previousElementSibling', function() { return this.__getPrevElementSibling(); });
            property(CharacterData.prototype, 'nextElementSibling', function() { return this.__getNextElementSibling(); });
            property(CharacterData.prototype, 'previousElementSibling', function() { return this.__getPrevElementSibling(); });
            property(Element.prototype, 'innerHTML', function() { return this.__getInnerHTML(); }, function(value) { this.__setInnerHTML(value == null ? '' : String(value)); });
            property(Element.prototype, 'outerHTML', function() { return this.getAttribute('__outerHTML') || ''; });
            property(Element.prototype, 'id', function() { return this.getAttribute('id') || ''; }, function(value) { this.setAttribute('id', String(value)); });
            property(Element.prototype, 'className', function() { return this.getAttribute('class') || ''; }, function(value) { this.setAttribute('class', String(value)); });
            property(Element.prototype, 'classList', function() { return this.__getClassList(); });
            property(Element.prototype, 'shadowRoot', function() { return this.__getShadowRoot(); });
            property(HTMLElement.prototype, 'hidden',
                function() {
                    const value = this.getAttribute('hidden');
                    if (value === null) return false;
                    if (value.toLowerCase() === 'until-found') return 'until-found';
                    return true;
                },
                function(value) {
                    if (typeof value === 'string' && value.toLowerCase() === 'until-found') {
                        this.setAttribute('hidden', 'until-found');
                        return;
                    }
                    if (value === false || value === '' || value == null || value === 0 || Number.isNaN(value)) {
                        this.removeAttribute('hidden');
                        return;
                    }
                    this.setAttribute('hidden', '');
                });
            const renderedTextFragment = function(element, value) {
                const doc = element.ownerDocument;
                const fragment = doc.createDocumentFragment();
                const lines = value.split(/\r\n|\r|\n/);
                for (let i = 0; i < lines.length; i += 1) {
                    if (i > 0) fragment.appendChild(doc.createElement('br'));
                    if (lines[i] !== '') fragment.appendChild(doc.createTextNode(lines[i]));
                }
                return fragment;
            };
            property(HTMLElement.prototype, 'innerText',
                function() { return this.__getInnerText(); },
                function(value) {
                    const text = value == null ? '' : String(value);
                    if (this.__getInnerText() === text) return;
                    const fragment = renderedTextFragment(this, text);
                    while (this.firstChild) this.removeChild(this.firstChild);
                    this.appendChild(fragment);
                });
            property(HTMLElement.prototype, 'outerText',
                function() { return this.__getInnerText(); },
                function(value) {
                    const text = value == null ? '' : String(value);
                    if (this.__getInnerText() === text) return;
                    const fragment = renderedTextFragment(this, text);
                    const parent = this.parentNode;
                    if (parent) {
                        parent.insertBefore(fragment, this);
                        parent.removeChild(this);
                    }
                });
            property(ShadowRoot.prototype, 'host', function() { return this.__getHost(); });
            property(ShadowRoot.prototype, 'mode', function() { return this.__getMode(); });
            property(ShadowRoot.prototype, 'delegatesFocus', function() { return this.__getDelegatesFocus(); });
            property(ShadowRoot.prototype, 'clonable', function() { return this.__getClonable(); });
            property(ShadowRoot.prototype, 'innerHTML', function() { return this.__getInnerHTML(); }, function(value) { this.__setInnerHTML(value == null ? '' : String(value)); });
            Element.prototype.setHTMLUnsafe = function(html) { this.innerHTML = html == null ? '' : String(html); };
            ShadowRoot.prototype.setHTMLUnsafe = function(html) { this.innerHTML = html == null ? '' : String(html); };
            Document.parseHTMLUnsafe = function(html) {
                const doc = document.implementation.createHTMLDocument('');
                doc.body.innerHTML = html == null ? '' : String(html);
                return doc;
            };
            property(HTMLTemplateElement.prototype, 'content', function() { return this.__getContent(); });
            property(Document.prototype, 'activeElement', function() { return this.__getActiveElement() || this.body || this.documentElement; });
            property(Document.prototype, 'currentScript', function() { return this.__getCurrentScript(); });
            property(Document.prototype, 'baseURI', function() { return this.__getBaseURI(); });
            property(Document.prototype, 'cookie', function() { return this.__getCookie(); }, function(value) { this.__setCookie(value); });
            property(Document.prototype, 'fonts', function() { return this.__getFontFaceSet(); });
            property(Document.prototype, 'doctype', function() { return this.__getDoctype(); });
            property(Document.prototype, 'defaultView', function() { return this.__getDefaultView(); });
            property(Document.prototype, 'visibilityState', function() { return this.__getVisibilityState(); });
            property(Document.prototype, 'hidden', function() { return this.__getHidden(); });
            property(Document.prototype, 'readyState', function() { return this.__getReadyState(); });
            property(Document.prototype, 'title', function() { return this.__getTitle(); }, function(value) { this.__setTitle(value == null ? '' : String(value)); });
            property(Document.prototype, 'implementation', function() {
                let implementation = documentImplementations.get(this);
                if (!implementation) {
                    implementation = Object.create(DOMImplementation.prototype);
                    documentImplementations.set(this, implementation);
                }
                return implementation;
            });
            const pointerCaptures = new Map();
            Element.prototype.setPointerCapture = function(pointerId) {
                if (!this || !(this instanceof Element)) throw new TypeError('Illegal invocation');
                if (!this.isConnected) throw new DOMException('Element is not connected to a document', 'InvalidStateError');
                pointerCaptures.set(Number(pointerId) || 0, this);
            };
            Element.prototype.releasePointerCapture = function(pointerId) {
                if (!this || !(this instanceof Element)) throw new TypeError('Illegal invocation');
                const id = Number(pointerId) || 0;
                if (pointerCaptures.get(id) !== this) {
                    throw new DOMException('Element does not have pointer capture for this pointer', 'NotFoundError');
                }
                pointerCaptures.delete(id);
            };
            Element.prototype.hasPointerCapture = function(pointerId) {
                if (!this || !(this instanceof Element)) throw new TypeError('Illegal invocation');
                return pointerCaptures.get(Number(pointerId) || 0) === this;
            };
            const toNode = function(doc, arg) {
                if (arg && typeof arg === 'object' && typeof arg.nodeType === 'number') {
                    return arg;
                }
                return doc.createTextNode(arg == null ? '' : String(arg));
            };
            const toFragment = function(doc, args) {
                if (args.length === 1) {
                    return toNode(doc, args[0]);
                }
                const fragment = doc.createDocumentFragment();
                for (let i = 0; i < args.length; i += 1) {
                    fragment.appendChild(toNode(doc, args[i]));
                }
                return fragment;
            };
            for (const proto of [Element.prototype, Document.prototype, DocumentFragment.prototype]) {
                proto.prepend = function(...args) {
                    if (args.length === 0) return;
                    const doc = this.ownerDocument || this;
                    this.insertBefore(toFragment(doc, args), this.firstChild);
                };
                proto.append = function(...args) {
                    if (args.length === 0) return;
                    const doc = this.ownerDocument || this;
                    this.appendChild(toFragment(doc, args));
                };
                proto.replaceChildren = function(...args) {
                    while (this.firstChild) {
                        this.removeChild(this.firstChild);
                    }
                    if (args.length > 0) {
                        const doc = this.ownerDocument || this;
                        this.appendChild(toFragment(doc, args));
                    }
                };
            }
            for (const proto of [Element.prototype, CharacterData.prototype]) {
                proto.before = function(...args) {
                    const parent = this.parentNode;
                    if (!parent || args.length === 0) return;
                    const doc = this.ownerDocument || document;
                    parent.insertBefore(toFragment(doc, args), this);
                };
                proto.after = function(...args) {
                    const parent = this.parentNode;
                    if (!parent || args.length === 0) return;
                    const doc = this.ownerDocument || document;
                    parent.insertBefore(toFragment(doc, args), this.nextSibling);
                };
                proto.replaceWith = function(...args) {
                    const parent = this.parentNode;
                    if (!parent) return;
                    const doc = this.ownerDocument || document;
                    if (args.length > 0) {
                        parent.insertBefore(toFragment(doc, args), this);
                    }
                    parent.removeChild(this);
                };
            }
            Element.prototype.insertAdjacentElement = function(position, element) {
                if (!element || typeof element.nodeType !== 'number') {
                    throw new TypeError('Failed to execute insertAdjacentElement: parameter 2 is not of type Node');
                }
                const pos = String(position).toLowerCase();
                if (pos === 'beforebegin') {
                    return this.parentNode ? this.parentNode.insertBefore(element, this) : null;
                }
                if (pos === 'afterbegin') {
                    return this.insertBefore(element, this.firstChild);
                }
                if (pos === 'beforeend') {
                    return this.appendChild(element);
                }
                if (pos === 'afterend') {
                    return this.parentNode ? this.parentNode.insertBefore(element, this.nextSibling) : null;
                }
                throw new DOMException("The value provided ('" + position + "') is not one of 'beforebegin', 'afterbegin', 'beforeend', or 'afterend'", 'SyntaxError');
            };
            Element.prototype.insertAdjacentText = function(position, data) {
                const doc = this.ownerDocument || document;
                const textNode = doc.createTextNode(data == null ? '' : String(data));
                this.insertAdjacentElement(position, textNode);
            };
            Element.prototype.insertAdjacentHTML = function(position, text) {
                const pos = String(position).toLowerCase();
                if (pos !== 'beforebegin' && pos !== 'afterbegin' && pos !== 'beforeend' && pos !== 'afterend') {
                    throw new DOMException("The value provided ('" + position + "') is not one of 'beforebegin', 'afterbegin', 'beforeend', or 'afterend'", 'SyntaxError');
                }
                if ((pos === 'beforebegin' || pos === 'afterend') && (!this.parentNode || this.parentNode.nodeType === 9)) {
                    throw new DOMException('Cannot insert adjacent HTML when the target element has no parent or parent is a Document', 'NoModificationAllowedError');
                }
                const doc = this.ownerDocument || document;
                const template = doc.createElement('template');
                template.innerHTML = text == null ? '' : String(text);
                const fragment = template.content;
                if (pos === 'beforebegin') {
                    this.parentNode.insertBefore(fragment, this);
                } else if (pos === 'afterbegin') {
                    this.insertBefore(fragment, this.firstChild);
                } else if (pos === 'beforeend') {
                    this.appendChild(fragment);
                } else if (pos === 'afterend') {
                    this.parentNode.insertBefore(fragment, this.nextSibling);
                }
            };
            Document.prototype.createEvent = function(interfaceName) {
                if (!this || this.nodeType !== 9) throw new TypeError('Document.createEvent requires a Document receiver');
                const interfaces = {
                    event: Event, events: Event, htmlevents: Event,
                    uievent: UIEvent, uievents: UIEvent,
                    focusevent: FocusEvent, focusevents: FocusEvent,
                    mouseevent: MouseEvent, mouseevents: MouseEvent,
                    wheelevent: WheelEvent, wheelevents: WheelEvent,
                    keyboardevent: KeyboardEvent, keyevents: KeyboardEvent,
                    customevent: CustomEvent, popstateevent: PopStateEvent,
                    hashchangeevent: HashChangeEvent,
                    inputevent: InputEvent, pointerevent: PointerEvent,
                };
                const Constructor = interfaces[String(interfaceName).toLowerCase()];
                if (!Constructor) throw new DOMException('The requested event interface is not supported', 'NotSupportedError');
                return new Constructor('');
            };
            property(Document.prototype, 'body', function() { return this.querySelector('body'); });
            property(Document.prototype, 'head', function() { return this.querySelector('head'); });
            property(Document.prototype, 'documentElement', function() { return this.querySelector('html') || this.querySelector(':root'); });
            property(Document.prototype, 'location', function() { return this.defaultView ? this.defaultView.location : null; });
            for (const prototype of [HTMLInputElement.prototype, HTMLTextAreaElement.prototype]) {
                property(prototype, 'value', function() { return this.getAttribute('value') || ''; }, function(value) { this.setAttribute('value', String(value)); });
                property(prototype, 'selectionStart', function() { const range = this.__getSelection(); return range ? range[0] : null; }, function(value) { this.setSelectionRange(value, Math.max(value, this.selectionEnd), this.selectionDirection); });
                property(prototype, 'selectionEnd', function() { const range = this.__getSelection(); return range ? range[1] : null; }, function(value) { this.setSelectionRange(Math.min(value, this.selectionStart), value, this.selectionDirection); });
                property(prototype, 'selectionDirection', function() { const range = this.__getSelection(); return range ? range[2] : null; }, function(value) { this.setSelectionRange(this.selectionStart, this.selectionEnd, value); });
            }
        })();
    "#))?;
    hyperlink::register_script_src_property(context)?;
    hyperlink::register_script_async_property(context)?;
    hyperlink::register_script_defer_property(context)?;
    hyperlink::register_script_type_property(context)?;
    hyperlink::register_script_text_property(context)?;
    hyperlink::register_script_nomodule_property(context)?;
    html_image::register_html_image_properties(context)?;
    attributes::register_attribute_interfaces(context)?;
    iframe::register_iframe_interfaces(context)?;
    media::register_media_interfaces(context)?;
    legacy_key::register_legacy_key_interfaces(context)?;
    media_source::register_media_source_interfaces(context)?;
    Ok(())
}

pub fn register_dom_bridge(
    context: &mut Context,
    state: Rc<RefCell<DomBridgeState>>,
) -> Result<(), JsError> {
    context.insert_data(state.clone());
    crate::web_api::register_standard_web_platform_apis(context)?;
    register_dom_interfaces(context)?;
    collections::register_collections(context)?;
    event_handlers::register_event_handler_idl(context)?;
    register_dom_traversal(context)?;
    layout::register_geometry_interfaces(context)?;
    computed_style::register_get_computed_style(context)?;
    cssom::register_cssom(context)?;

    let dom_protos = prototypes::init_dom_prototypes(context)?;
    let inline_style_factory = state.borrow().inline_style_factory.clone();
    prototypes::install_prototype_accessors(context, inline_style_factory.as_ref())?;
    state.borrow_mut().dom_token_list_prototype = dom_protos.dom_token_list.clone();
    let doc_proto = dom_protos.document.clone();
    state.borrow_mut().prototypes = Some(dom_protos);

    let root = state.borrow().document.borrow().root();
    let mut doc_builder = ObjectInitializer::new(context);
    doc_builder.property(js_string!("nodeType"), JsValue::new(9), Attribute::READONLY);
    doc_builder.property(
        js_string!("nodeName"),
        js_string!("#document"),
        Attribute::READONLY,
    );
    let doc_obj = doc_builder.build();
    let _ = doc_obj.set_prototype(Some(doc_proto));
    context.register_global_property(
        js_string!("document"),
        doc_obj.clone(),
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;

    state
        .borrow_mut()
        .element_wrappers
        .insert(root, doc_obj.clone());
    state
        .borrow_mut()
        .wrapper_nodes
        .insert(doc_obj.clone(), root);

    let document_all = JsObject::from_proto_and_data(
        context.intrinsics().constructors().object().prototype(),
        boa_engine::builtins::is_html_dda::IsHTMLDDA,
    );
    doc_obj.define_property_or_throw(
        js_string!("all"),
        PropertyDescriptor::builder()
            .value(document_all)
            .writable(false)
            .enumerable(false)
            .configurable(true)
            .build(),
        context,
    )?;

    let doc_script = r#"
        (function(doc) {
            if (!doc) return;
            Object.setPrototypeOf(doc, Document.prototype);
            Object.defineProperty(Window.prototype, 'document', {
                get: function() {
                    if (this === globalThis) return doc;
                    if (this && this.__doc) return this.document;
                    throw new TypeError('Illegal invocation');
                },
                configurable: true,
                enumerable: true
            });
            Object.defineProperty(doc, "body", {
                get: function() { return doc.querySelector("body"); },
                configurable: true
            });
            Object.defineProperty(doc, "head", {
                get: function() { return doc.querySelector("head"); },
                configurable: true
            });
            Object.defineProperty(doc, "documentElement", {
                get: function() { return doc.querySelector("html") || doc.querySelector(":root"); },
                configurable: true
            });
            Object.defineProperty(doc, "location", {
                get: function() { return globalThis.location; },
                configurable: true
            });
            const documentAll = doc.all;
            Object.defineProperty(Document.prototype, "all", {
                get: function() { return documentAll; },
                configurable: true
            });
            Object.defineProperty(documentAll, "length", {
                get: function() { return doc.querySelectorAll("*").length; },
                configurable: true
            });
            Object.defineProperty(documentAll, "item", {
                value: function item(index) {
                    const elements = doc.querySelectorAll("*");
                    const position = Number(index);
                    return position >= 0 && position < elements.length ? elements[position] : null;
                },
                configurable: true,
                writable: true
            });
            Object.defineProperty(documentAll, "namedItem", {
                value: function namedItem(name) {
                    const key = String(name);
                    const byId = doc.getElementById(key);
                    if (byId) return byId;
                    const named = doc.querySelectorAll("[name]");
                    for (let index = 0; index < named.length; index += 1) {
                        if (named[index].getAttribute("name") === key) return named[index];
                    }
                    return null;
                },
                configurable: true,
                writable: true
            });
            if (!globalThis.Image) {
                globalThis.Image = function Image(width, height) {
                    const image = doc.createElement('img');
                    if (width !== undefined) image.setAttribute('width', String(width));
                    if (height !== undefined) image.setAttribute('height', String(height));
                    return image;
                };
                Image.prototype = HTMLImageElement.prototype;
            }

        })(document);
    "#;
    context.eval(boa_engine::Source::from_bytes(doc_script.as_bytes()))?;
    custom_elements::register_custom_elements(context)?;
    register_mutation_observer(context)?;

    Ok(())
}

pub fn set_document_ready_state(
    context: &mut Context,
    doc_node: NodeId,
    new_state: &str,
) -> JsResult<()> {
    let state_rc = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let Some(state) = state_rc else {
        return Ok(());
    };
    let (old_state, is_main) = {
        let state_ref = state.borrow();
        let is_main = doc_node == state_ref.main_document;
        let old = state_ref
            .document_ready_states
            .get(&doc_node)
            .cloned()
            .unwrap_or_else(|| {
                if is_main {
                    "loading".to_string()
                } else {
                    "complete".to_string()
                }
            });
        (old, is_main)
    };
    if old_state == new_state {
        return Ok(());
    }
    state
        .borrow_mut()
        .document_ready_states
        .insert(doc_node, new_state.to_string());

    if is_main {
        let props = [
            ("bubbles", JsValue::new(false)),
            ("cancelable", JsValue::new(false)),
        ];
        let _ = dispatch_dom_event_to_js(doc_node, "readystatechange", Some(&props), context)?;
    }
    Ok(())
}
