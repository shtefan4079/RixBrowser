use super::*;

pub(super) fn document_create_text_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let text = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let root = document_root(this, context)?;
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let node_id = doc.borrow_mut().create_text(&text);
        state.borrow_mut().set_subtree_owner_document(node_id, root);
        let wrapped = wrap_element(node_id, context);
        return Ok(wrapped.into());
    }
    Ok(JsValue::null())
}

#[inline]
pub(crate) fn fast_js_string(s: &str) -> boa_engine::JsString {
    match s {
        "" => js_string!(""),
        "true" => js_string!("true"),
        "false" => js_string!("false"),
        "0" => js_string!("0"),
        "1" => js_string!("1"),
        "auto" => js_string!("auto"),
        "none" => js_string!("none"),
        "inherit" => js_string!("inherit"),
        "initial" => js_string!("initial"),
        _ => js_string!(s),
    }
}

pub(super) fn element_get_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let attr_name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();

    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        if attr_name == "__textContent" {
            let doc = state.borrow().document.clone();
            let text = doc.borrow().inner_text(node_id);
            return Ok(js_string!(text).into());
        }

        if attr_name == "__innerHTML" {
            return node_get_inner_html(this, args, context);
        }

        if attr_name == "__outerHTML" {
            let doc = state.borrow().document.clone();
            let html = doc.borrow().to_html_string(node_id);
            return Ok(js_string!(html).into());
        }

        if attr_name == "value" {
            if let Some(val) = state.borrow().input_values.get(&node_id) {
                return Ok(fast_js_string(val).into());
            }
            let document = state.borrow().document.clone();
            let document = document.borrow();
            if let Some(NodeData::Element(element)) = document.get(node_id) {
                if &*element.name.local == "textarea" && element.get_attribute("value").is_none() {
                    return Ok(js_string!(document.inner_text(node_id)).into());
                }
            }
        }

        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        if let Some(NodeData::Element(elem)) = doc_ref.get(node_id) {
            if let Some(val) = elem.get_attribute(&attr_name) {
                return Ok(fast_js_string(val).into());
            }
        }
    }
    Ok(JsValue::null())
}

pub(super) fn element_set_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let node_id = get_node_id_from_this(this, context)?;
        let attr_name = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        let attr_val = args
            .get_or_undefined(1)
            .to_string(context)?
            .to_std_string_escaped();

        let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
        if let Some(state) = state {
            if attr_name == "value" {
                state
                    .borrow_mut()
                    .input_values
                    .insert(node_id, attr_val.clone());
            }

            if attr_name == "__textContent" {
                return node_set_text_content(this, &[js_string!(attr_val).into()], context);
            }

            if attr_name == "__innerHTML" {
                return node_set_inner_html(this, &[js_string!(attr_val).into()], context);
            }

            if attr_name == "__outerHTML" {
                return Ok(JsValue::undefined());
            }

            let doc = state.borrow().document.clone();
            let mut doc_mut = doc.borrow_mut();
            let mut old = None;
            let mut changed = false;
            if let Some(NodeData::Element(elem)) = doc_mut.get_mut_untracked(node_id) {
                if let Some(current) = elem.get_attribute(&attr_name) {
                    if current == attr_val {
                        return Ok(JsValue::undefined());
                    }
                    old = Some(current.to_owned());
                }
                changed = true;
                elem.set_attribute(&attr_name, &attr_val);
                if doc_mut.is_connected(node_id) {
                    doc_mut.bump_revision();
                    doc_mut.bump_style_revision();
                }
            }
            drop(doc_mut);
            if changed {
                super::canvas::canvas_dimension_attribute_mutated(context, node_id, "", &attr_name)?;
                super::attributes::notify_attribute_mutation(
                    this,
                    "",
                    &attr_name,
                    Some(&attr_val),
                    context,
                )?;
                record_attribute_mutation(context, node_id, &attr_name, old.as_deref())?;
                super::custom_elements::notify_attribute_changed(
                    node_id,
                    &attr_name,
                    old.as_deref(),
                    Some(&attr_val),
                    context,
                );
            }
        }
        Ok(JsValue::undefined())
    })
}

pub(super) fn element_remove_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let node_id = get_node_id_from_this(this, context)?;
        let attr_name = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();

        let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
        if let Some(state) = state {
            let doc = state.borrow().document.clone();
            let mut doc_mut = doc.borrow_mut();
            let mut old = None;
            if let Some(NodeData::Element(elem)) = doc_mut.get_mut_untracked(node_id) {
                old = elem.get_attribute(&attr_name).map(str::to_owned);
                if old.is_some() {
                    elem.remove_attribute(&attr_name);
                    if doc_mut.is_connected(node_id) {
                        doc_mut.bump_revision();
                        doc_mut.bump_style_revision();
                    }
                }
            }
            drop(doc_mut);
            if let Some(old) = old {
                super::canvas::canvas_dimension_attribute_mutated(context, node_id, "", &attr_name)?;
                super::attributes::notify_attribute_mutation(this, "", &attr_name, None, context)?;
                record_attribute_mutation(context, node_id, &attr_name, Some(&old))?;
                super::custom_elements::notify_attribute_changed(
                    node_id,
                    &attr_name,
                    Some(&old),
                    None,
                    context,
                );
            }
        }
        Ok(JsValue::undefined())
    })
}

pub(super) fn element_has_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let attr_name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();

    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        if let Some(NodeData::Element(elem)) = doc_ref.get(node_id) {
            let has = elem.get_attribute(&attr_name).is_some();
            return Ok(JsValue::new(has));
        }
    }
    Ok(JsValue::new(false))
}

pub(super) fn element_add_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let event_type = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let options = parse_listener_options(args.get_or_undefined(2), context)?;
    let callback_value = args.get_or_undefined(1);
    if callback_value.is_null() || callback_value.is_undefined() {
        return Ok(JsValue::undefined());
    }
    let callback = callback_value
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("addEventListener: callback must be a function or object")
        })?
        .clone();

    if let Some(ref signal_obj) = options.signal {
        let aborted = signal_obj
            .get(js_string!("aborted"), context)?
            .to_boolean();
        if aborted {
            return Ok(JsValue::undefined());
        }
    }

    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(ref state) = state {
        state.borrow_mut().add_listener(
            node_id,
            event_type.clone(),
            callback.clone(),
            options.capture,
            options.once,
            options.passive,
        );
    }

    if let Some(signal_obj) = options.signal {
        let add_event_listener_fn = signal_obj.get(js_string!("addEventListener"), context)?;
        if let Some(add_fn) = add_event_listener_fn.as_callable() {
            let remover_code = r#"
                (function(node, eventType, callback, capture) {
                    return function() {
                        node.removeEventListener(eventType, callback, { capture: capture });
                    };
                })
            "#;
            let maker = context.eval(boa_engine::Source::from_bytes(remover_code))?;
            let abort_remover = maker.as_callable().unwrap().call(
                &JsValue::undefined(),
                &[
                    this.clone(),
                    js_string!(event_type).into(),
                    callback.into(),
                    options.capture.into(),
                ],
                context,
            )?;
            let once_options = ObjectInitializer::new(context)
                .property(js_string!("once"), true, Attribute::all())
                .build();
            add_fn.call(
                &signal_obj.into(),
                &[
                    js_string!("abort").into(),
                    abort_remover,
                    once_options.into(),
                ],
                context,
            )?;
        }
    }

    Ok(JsValue::undefined())
}

pub(super) fn element_remove_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let event_type = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let callback_value = args.get_or_undefined(1);
    let capture_opt = parse_remove_listener_options(args.get_or_undefined(2), context)?;
    if callback_value.is_null() || callback_value.is_undefined() {
        return Ok(JsValue::undefined());
    }
    let callback = callback_value.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("removeEventListener: callback must be a function or object")
    })?;

    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        state
            .borrow_mut()
            .remove_listener(node_id, &event_type, &callback, capture_opt);
    }
    Ok(JsValue::undefined())
}

pub(super) fn element_append_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let parent_id = get_node_id_from_this(this, context)?;
        let child_val = args.get_or_undefined(0);
        let child_id = get_node_id_from_this(child_val, context)?;

        let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
        if let Some(state) = state {
            let doc = state.borrow().document.clone();
            let mut document = doc.borrow_mut();
            let nodes = insertion_nodes(&document, parent_id, child_id)?;
            let has_observers = state.borrow().active_mutation_observers > 0;
            let (removals, disconnected) = if has_observers {
                let removals = capture_removals(&document, &nodes);
                (Some(removals), Vec::new())
            } else {
                let mut disconnected = Vec::new();
                for &child in &nodes {
                    if document.parent(child).is_some() {
                        disconnected.push(child);
                    }
                }
                (None, disconnected)
            };
            for &child in &nodes {
                document.append_child(parent_id, child);
            }
            let previous = if has_observers {
                nodes
                    .first()
                    .and_then(|first| node_neighbors(&document, *first).0)
            } else {
                None
            };
            drop(document);
            if let Some(removals) = &removals {
                for removal in removals {
                    for &child in &removal.nodes {
                        super::custom_elements::notify_disconnected_subtree(child, context);
                    }
                }
            } else {
                for child in disconnected {
                    super::custom_elements::notify_disconnected_subtree(child, context);
                }
            }
            let owner = {
                let state_ref = state.borrow();
                state_ref
                    .owner_document(parent_id)
                    .unwrap_or(state_ref.main_document)
            };
            {
                let mut state_mut = state.borrow_mut();
                for &node in &nodes {
                    state_mut.set_subtree_owner_document(node, owner);
                }
            }
            if let Some(removals) = removals {
                record_removals(context, removals)?;
                record_child_mutation(context, parent_id, &nodes, &[], previous, None)?;
            }
            for &child in &nodes {
                super::custom_elements::notify_connected_subtree(child, context)?;
            }
            super::script::prepare_connected_scripts(&nodes, context)?;
            super::iframe::prepare_connected_iframes(&nodes, context)?;
        }
        Ok(child_val.clone())
    })
}

pub(super) fn element_remove_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let parent_id = get_node_id_from_this(this, context)?;
        let child_val = args.get_or_undefined(0);
        let child_id = get_node_id_from_this(child_val, context)?;

        let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
        if let Some(state) = state {
            let doc = state.borrow().document.clone();
            let mut document = doc.borrow_mut();
            if document.parent(child_id) != Some(parent_id) {
                return Err(JsNativeError::typ()
                    .with_message("NotFoundError: node is not a child")
                    .into());
            }
            let has_observers = state.borrow().active_mutation_observers > 0;
            let removals = if has_observers {
                Some(capture_removals(&document, &[child_id]))
            } else {
                None
            };
            document.remove_child(child_id);
            drop(document);
            super::custom_elements::notify_disconnected_subtree(child_id, context);
            if let Some(removals) = removals {
                record_removals(context, removals)?;
            }
        }
        Ok(child_val.clone())
    })
}

pub(super) fn element_focus(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        let document = state.borrow().document.clone();
        let old = document.borrow().focused_node();
        if old != Some(node) {
            document.borrow_mut().set_focused(Some(node), true);
            if let Some(old) = old {
                dispatch_dom_event_to_js(old, "blur", None, context)?;
            }
            dispatch_dom_event_to_js(node, "focus", None, context)?;
        }
    }
    Ok(JsValue::undefined())
}

pub(super) fn element_blur(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        let document = state.borrow().document.clone();
        if document.borrow().focused_node() == Some(node) {
            document.borrow_mut().set_focused(None, false);
            document.borrow_mut().set_text_selection(None);
            dispatch_dom_event_to_js(node, "blur", None, context)?;
        }
    }
    Ok(JsValue::undefined())
}

pub(super) fn element_select(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        let document = state.borrow().document.clone();
        let length = text_control_value(&state.borrow(), &document.borrow(), node)
            .chars()
            .count();
        document
            .borrow_mut()
            .set_text_selection(Some((node, 0, length)));
    }
    Ok(JsValue::undefined())
}

pub(super) fn node_get_inner_html(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let doc = state.borrow().document.clone();
    let doc_ref = doc.borrow();
    let mut html = String::new();
    let container = template_contents_or_self(&doc_ref, node_id);
    for child in doc_ref.children(container) {
        html.push_str(&doc_ref.to_html_string(child));
    }
    Ok(js_string!(html).into())
}

pub(super) fn node_set_inner_html(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        use boa_engine::JsArgs;
        let node_id = get_node_id_from_this(this, context)?;
        let val_str = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        let state = context
            .get_data::<Rc<RefCell<DomBridgeState>>>()
            .cloned()
            .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;

        let (parsed_doc, source_children) = if val_str.is_empty() {
            (rix_dom::tree::Document::new(), Vec::new())
        } else {
            let doc_ref = state.borrow().document.clone();
            let doc_b = doc_ref.borrow();
            let context_name = match doc_b.get(node_id) {
                Some(NodeData::Element(elem)) => Some(elem.name.clone()),
                _ => None,
            };
            rix_html::parse_html_fragment(&val_str, context_name.as_ref())
        };

        let doc = state.borrow().document.clone();
        let (container, removed, added) = {
            let mut doc_mut = doc.borrow_mut();
            let container = template_contents_or_self(&doc_mut, node_id);
            let existing: Vec<NodeId> = doc_mut.children(container).collect();
            for child in &existing {
                doc_mut.remove_child(*child);
            }
            let mut added = Vec::new();
            for src_child in source_children {
                let cloned = clone_subtree_into(&parsed_doc, src_child, &mut doc_mut);
                doc_mut.append_child(container, cloned);
                added.push(cloned);
            }
            (container, existing, added)
        };

        let container_owner = {
            let state_ref = state.borrow();
            state_ref
                .owner_document(container)
                .unwrap_or(state_ref.main_document)
        };
        {
            let mut state_mut = state.borrow_mut();
            for &node in &added {
                state_mut.set_subtree_owner_document(node, container_owner);
            }
        }
        {
            let mut state_mut = state.borrow_mut();
            let doc_ref = state_mut.document.clone();
            let doc_ref = doc_ref.borrow();
            for root_node in &added {
                for n in doc_ref.descendants(*root_node) {
                    if let Some(NodeData::Element(el)) = doc_ref.get(n) {
                        if el.name.ns.as_str() == "http://www.w3.org/1999/xhtml"
                            && el.name.local.as_str() == "script"
                        {
                            state_mut.mark_script_started(n);
                        }
                    }
                }
            }
        }
        record_child_mutation(context, container, &added, &removed, None, None)?;
        for node in &removed {
            super::custom_elements::notify_disconnected_subtree(*node, context);
        }
        for node in &added {
            super::custom_elements::notify_connected_subtree(*node, context)?;
        }
        super::script::prepare_connected_scripts(&added, context)?;
        super::iframe::prepare_connected_iframes(&added, context)?;
        Ok(JsValue::undefined())
    })
}

pub(super) fn element_get_class_list(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let state_rc = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let Some(ref state) = state_rc else {
        return Ok(JsValue::undefined());
    };

    if let Some(list_obj) = state.borrow().element_class_lists.get(&node_id) {
        return Ok(list_obj.clone().into());
    }

    let class_list_obj = ObjectInitializer::new(context).build();
    if let Some(ref proto) = state
        .borrow()
        .prototypes
        .as_ref()
        .and_then(|p| p.dom_token_list.clone())
    {
        let _ = class_list_obj.set_prototype(Some(proto.clone()));
    }
    {
        let mut borrowed = state.borrow_mut();
        borrowed
            .wrapper_nodes
            .insert(class_list_obj.clone(), node_id);
        borrowed
            .element_class_lists
            .insert(node_id, class_list_obj.clone());
    }

    Ok(class_list_obj.into())
}
