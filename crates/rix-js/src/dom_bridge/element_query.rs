use super::*;

pub(super) fn element_matches(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let node_id = get_node_id_from_this(this, context)?;
    let selector = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        if let Ok(selectors) = rix_dom::query::ComplexSelector::parse_shared(&selector) {
            for sel in selectors.as_ref() {
                if rix_dom::query::matches_complex(&doc_ref, node_id, sel) {
                    return Ok(JsValue::new(true));
                }
            }
        }
        return Ok(JsValue::new(false));
    }
    Ok(JsValue::new(false))
}

pub(super) fn element_closest(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let mut current = Some(get_node_id_from_this(this, context)?);
    let selector = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let Some(state) = state else {
        return Ok(JsValue::null());
    };
    let doc = state.borrow().document.clone();
    let Ok(selectors) = rix_dom::query::ComplexSelector::parse_shared(&selector) else {
        return Ok(JsValue::null());
    };
    while let Some(node) = current {
        {
            let doc_ref = doc.borrow();
            for sel in selectors.as_ref() {
                if rix_dom::query::matches_complex(&doc_ref, node, sel) {
                    drop(doc_ref);
                    return Ok(wrap_element(node, context).into());
                }
            }
            current = doc_ref.parent(node);
        }
    }
    Ok(JsValue::null())
}

pub(super) fn element_contains(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let node_id = get_node_id_from_this(this, context)?;
    let other_val = args.get_or_undefined(0);
    let Ok(other_id) = get_node_id_from_this(other_val, context) else {
        return Ok(JsValue::new(false));
    };
    if node_id == other_id {
        return Ok(JsValue::new(true));
    }
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        for desc in doc_ref.descendants(node_id) {
            if desc == other_id {
                return Ok(JsValue::new(true));
            }
        }
    }
    Ok(JsValue::new(false))
}

pub(super) fn element_get_attribute_names(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let mut names: Vec<JsValue> = Vec::new();
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        if let Some(NodeData::Element(elem)) = doc_ref.get(node_id) {
            for (qname, _) in &elem.attributes {
                let local_name = qname.local.to_string();
                let qualified_name = qname.prefix.as_ref().map_or_else(
                    || local_name.clone(),
                    |prefix| format!("{prefix}:{local_name}"),
                );
                names.push(js_string!(qualified_name).into());
            }
        }
    }
    Ok(boa_engine::object::builtins::JsArray::from_iter(names, context).into())
}

pub(super) fn element_get_elements_by_tag(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let node_id = get_node_id_from_this(this, context)?;
    let tag = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let found: Vec<NodeId> = if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        let is_wildcard = tag == "*";
        doc_ref
            .descendants(node_id)
            .filter(|desc| {
                if *desc == node_id {
                    return false;
                }
                if let Some(NodeData::Element(el)) = doc_ref.get(*desc) {
                    if is_wildcard {
                        return true;
                    }
                    if (&*el.name.local).eq_ignore_ascii_case(&tag) {
                        return true;
                    }
                }
                false
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut elements = Vec::with_capacity(found.len());
    for node in found {
        elements.push(wrap_element(node, context).into());
    }
    Ok(boa_engine::object::builtins::JsArray::from_iter(elements, context).into())
}

pub(super) fn element_get_elements_by_class(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let node_id = get_node_id_from_this(this, context)?;
    let class_str = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let tokens: Vec<&str> = class_str.split_whitespace().collect();
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let found: Vec<NodeId> = if let Some(state) = state {
        if tokens.is_empty() {
            Vec::new()
        } else {
            let doc = state.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref
                .descendants(node_id)
                .filter(|desc| {
                    if *desc == node_id {
                        return false;
                    }
                    if let Some(NodeData::Element(el)) = doc_ref.get(*desc) {
                        if let Some((_, class_attr)) = el
                            .attributes
                            .iter()
                            .find(|(attr, _)| &*attr.local == "class")
                        {
                            let el_classes: Vec<&str> = class_attr.split_whitespace().collect();
                            return tokens.iter().all(|t| el_classes.contains(t));
                        }
                    }
                    false
                })
                .collect()
        }
    } else {
        Vec::new()
    };
    let mut elements = Vec::with_capacity(found.len());
    for node in found {
        elements.push(wrap_element(node, context).into());
    }
    Ok(boa_engine::object::builtins::JsArray::from_iter(elements, context).into())
}

#[allow(dead_code)]
pub(super) fn return_selector_array(
    scope: NodeId,
    selector: &str,
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let found: Vec<NodeId> = if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        doc_ref.query_selector_all(scope, selector)
    } else {
        Vec::new()
    };
    let mut elements = Vec::with_capacity(found.len());
    for node in found {
        elements.push(wrap_element(node, context).into());
    }
    Ok(boa_engine::object::builtins::JsArray::from_iter(elements, context).into())
}

pub(super) fn wrap_parent_or_null(parent: Option<NodeId>, context: &mut Context) -> JsValue {
    match parent {
        Some(p) => wrap_element(p, context).into(),
        None => JsValue::null(),
    }
}

pub(super) fn element_get_parent_node(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let parent = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.parent(node_id)
        });
    Ok(wrap_parent_or_null(parent, context))
}

pub(super) fn collect_children(
    node_id: NodeId,
    context: &mut Context,
    elements_only: bool,
) -> Vec<JsValue> {
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let Some(state) = state else {
        return Vec::new();
    };
    let doc = state.borrow().document.clone();
    let kids: Vec<NodeId> = {
        let doc_ref = doc.borrow();
        doc_ref
            .children(node_id)
            .filter(|c| {
                if !elements_only {
                    return true;
                }
                matches!(doc_ref.get(*c), Some(NodeData::Element(_)))
            })
            .collect()
    };
    kids.into_iter()
        .map(|k| wrap_element(k, context).into())
        .collect()
}

pub(super) fn element_get_children(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let items = collect_children(node_id, context, true);
    Ok(boa_engine::object::builtins::JsArray::from_iter(items, context).into())
}

pub(super) fn element_get_child_nodes(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let items = collect_children(node_id, context, false);
    Ok(boa_engine::object::builtins::JsArray::from_iter(items, context).into())
}

pub(super) fn element_get_first_child(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let first = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.first_child(node_id)
        });
    Ok(wrap_parent_or_null(first, context))
}

pub(super) fn element_get_last_child(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let last = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.last_child(node_id)
        });
    Ok(wrap_parent_or_null(last, context))
}

pub(super) fn element_get_next_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let next = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.next_sibling(node_id)
        });
    Ok(wrap_parent_or_null(next, context))
}

pub(super) fn element_get_prev_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let prev = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.previous_sibling(node_id)
        });
    Ok(wrap_parent_or_null(prev, context))
}

pub(super) fn element_get_first_element_child(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let first = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.first_element_child(node_id)
        });
    Ok(wrap_parent_or_null(first, context))
}

pub(super) fn element_get_last_element_child(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let last = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.last_element_child(node_id)
        });
    Ok(wrap_parent_or_null(last, context))
}

pub(super) fn element_get_next_element_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let next = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.next_element_sibling(node_id)
        });
    Ok(wrap_parent_or_null(next, context))
}

pub(super) fn element_get_prev_element_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let prev = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.previous_element_sibling(node_id)
        });
    Ok(wrap_parent_or_null(prev, context))
}

pub(super) fn element_get_child_element_count(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let count = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .map(|s| {
            let doc = s.borrow().document.clone();
            let doc_ref = doc.borrow();
            doc_ref.child_element_count(node_id)
        })
        .unwrap_or(0);
    Ok(JsValue::new(count as f64))
}

pub(super) fn element_click(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    dispatch_dom_event_to_js(node_id, "click", None, context)
}

pub(super) fn element_query_selector(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let selector = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        if let Some(found_node) = doc_ref.query_selector(node_id, &selector) {
            drop(doc_ref);
            let wrapped = wrap_element(found_node, context);
            return Ok(wrapped.into());
        }
    }
    Ok(JsValue::null())
}

pub(super) fn element_query_selector_all(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let selector = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let mut elements = Vec::new();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        let found = doc_ref.query_selector_all(node_id, &selector);
        drop(doc_ref);
        for node in found {
            elements.push(wrap_element(node, context).into());
        }
    }
    let array = boa_engine::object::builtins::JsArray::from_iter(elements, context);
    Ok(array.into())
}
