use super::*;

pub fn wrap_element(node_id: NodeId, context: &mut Context) -> JsObject {
    let state_rc = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(ref state) = state_rc {
        if let Some(existing) = state.borrow().element_wrappers.get(&node_id) {
            return existing.clone();
        }
    }
    let (tag_name, local_name, ns, prefix, node_type, is_shadow_root) = if let Some(ref state) = state_rc {
        let (name, local_name, ns, prefix, node_type, is_shadow_root) = {
            let doc = state.borrow().document.clone();
            let doc_ref = doc.borrow();
            if let Some(node) = doc_ref.get(node_id) {
                match node {
                    NodeData::Element(elem) => {
                        let name = elem.name.prefix.as_ref().map_or_else(
                            || elem.name.local.to_string(),
                            |prefix| format!("{prefix}:{}", elem.name.local),
                        );
                        (
                            if &*elem.name.ns == "http://www.w3.org/1999/xhtml" {
                                name.to_ascii_uppercase()
                            } else {
                                name
                            },
                            elem.name.local.to_string(),
                            elem.name.ns.to_string(),
                            elem.name.prefix.as_ref().map(|p| p.to_string()),
                            1,
                            false,
                        )
                    }
                    NodeData::Document => ("#document".into(), "".into(), "".into(), None, 9, false),
                    NodeData::DocumentFragment => ("#document-fragment".into(), "".into(), "".into(), None, 11, false),
                    NodeData::ShadowRoot { .. } => ("#document-fragment".into(), "".into(), "".into(), None, 11, true),
                    NodeData::Text(_) => ("#text".into(), "".into(), "".into(), None, 3, false),
                    NodeData::Comment(_) => ("#comment".into(), "".into(), "".into(), None, 8, false),
                    NodeData::DocumentType { name, .. } => (name.clone(), "".into(), "".into(), None, 10, false),
                }
            } else {
                ("".to_string(), "".to_string(), "".to_string(), None, 0, false)
            }
        };
        (name, local_name, ns, prefix, node_type, is_shadow_root)
    } else {
        ("".to_string(), "".to_string(), "".to_string(), None, 0, false)
    };

    let mut el_builder = ObjectInitializer::new(context);

    el_builder.property(
        js_string!("tagName"),
        js_string!(tag_name.clone()),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    el_builder.property(
        js_string!("nodeName"),
        js_string!(tag_name),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    el_builder.property(
        js_string!("nodeType"),
        JsValue::new(node_type),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    if node_type == 1 {
        el_builder.property(
            js_string!("namespaceURI"),
            if ns.is_empty() {
                JsValue::null()
            } else {
                js_string!(ns.clone()).into()
            },
            Attribute::READONLY,
        );
        el_builder.property(
            js_string!("localName"),
            js_string!(local_name.clone()),
            Attribute::READONLY,
        );
        el_builder.property(
            js_string!("prefix"),
            prefix.map_or(JsValue::null(), |p| js_string!(p).into()),
            Attribute::READONLY,
        );
    }

    let el_obj = el_builder.build();

    if let Some(ref state) = state_rc {
        let state = state.borrow();
        if let Some(ref protos) = state.prototypes {
            let proto = match node_type {
                1 => {
                    if ns == "http://www.w3.org/2000/svg" {
                        protos.svg_tags.get(&local_name)
                            .or(protos.svg_element.as_ref())
                            .or(Some(&protos.element))
                            .cloned()
                    } else if ns == "http://www.w3.org/1999/xhtml" || ns.is_empty() {
                        if let Some(proto) = protos.html_tags.get(&local_name) {
                            Some(proto.clone())
                        } else if local_name.contains('-') {
                            let custom_proto = state.custom_element_definitions.get(&local_name)
                                .and_then(|def| def.constructor.get(boa_engine::js_string!("prototype"), context).ok())
                                .and_then(|proto_val| proto_val.as_object());
                            custom_proto.or(Some(protos.html_element.clone()))
                        } else {
                            Some(protos.html_unknown.clone())
                        }
                    } else {
                        Some(protos.element.clone())
                    }
                }
                3 => Some(protos.text.clone()),
                8 => Some(protos.comment.clone()),
                9 => Some(protos.document.clone()),
                10 => Some(protos.document_type.clone()),
                11 => if is_shadow_root {
                    protos.shadow_root.clone().or_else(|| Some(protos.document_fragment.clone()))
                } else {
                    Some(protos.document_fragment.clone())
                },
                _ => Some(protos.node.clone()),
            };
            if let Some(proto) = proto {
                let _ = el_obj.set_prototype(Some(proto));
            }
        }
    }

    if let Some(state) = &state_rc {
        let mut borrowed = state.borrow_mut();
        borrowed.wrapper_nodes.insert(el_obj.clone(), node_id);
        borrowed.element_wrappers.insert(node_id, el_obj.clone());
    }

    el_obj
}

pub(super) fn get_node_id_from_this(this: &JsValue, context: &mut Context) -> JsResult<NodeId> {
    let obj = this.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("Method must be called on an HTMLElement")
    })?;
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    state
        .and_then(|state| state.borrow().wrapper_nodes.get(&obj).copied())
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("Illegal DOM wrapper receiver")
                .into()
        })
}
