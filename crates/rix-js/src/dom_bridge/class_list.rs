use super::*;

pub(super) fn class_list_add(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let node_id = get_node_id_from_this(this, context)?;
    let mut tokens = Vec::new();
    for i in 0..args.len() {
        let t = args
            .get_or_undefined(i)
            .to_string(context)?
            .to_std_string_escaped();
        if t.is_empty() || t.split_ascii_whitespace().count() != 1 {
            return Err(JsNativeError::typ()
                .with_message("classList.add: token must be a non-empty single class")
                .into());
        }
        tokens.push(t);
    }
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let mut doc_mut = doc.borrow_mut();
        let already_has_all = match doc_mut.get(node_id) {
            Some(NodeData::Element(element)) => tokens.iter().all(|t| element.has_class(t)),
            _ => false,
        };
        if already_has_all {
            return Ok(JsValue::undefined());
        }
        let old_attribute = match doc_mut.get(node_id) {
            Some(NodeData::Element(element)) => element.get_attribute("class").map(str::to_owned),
            _ => None,
        };
        let new_value = if let Some(NodeData::Element(elem)) = doc_mut.get_mut_untracked(node_id) {
            let mut current = elem
                .get_attribute("class")
                .map(|c| {
                    c.split_ascii_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for t in tokens {
                if !current.contains(&t) {
                    current.push(t);
                }
            }
            let new_value = current.join(" ");
            if old_attribute.as_deref() != Some(&new_value) {
                elem.set_attribute("class", &new_value);
                if doc_mut.is_connected(node_id) {
                    doc_mut.bump_revision();
                    doc_mut.bump_style_revision();
                }
            }
            new_value
        } else {
            String::new()
        };
        drop(doc_mut);
        super::attributes::notify_attribute_mutation(this, "", "class", Some(&new_value), context)?;
        record_attribute_mutation(context, node_id, "class", old_attribute.as_deref())?;
    }
    Ok(JsValue::undefined())
}

pub(super) fn class_list_remove(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let node_id = get_node_id_from_this(this, context)?;
    let mut tokens = Vec::new();
    for i in 0..args.len() {
        tokens.push(
            args.get_or_undefined(i)
                .to_string(context)?
                .to_std_string_escaped(),
        );
    }
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let mut doc_mut = doc.borrow_mut();
        let already_missing_all = match doc_mut.get(node_id) {
            Some(NodeData::Element(element)) => tokens.iter().all(|t| !element.has_class(t)),
            _ => true,
        };
        if already_missing_all {
            return Ok(JsValue::undefined());
        }
        let old_attribute = match doc_mut.get(node_id) {
            Some(NodeData::Element(element)) => element.get_attribute("class").map(str::to_owned),
            _ => None,
        };
        let new_value = if let Some(NodeData::Element(elem)) = doc_mut.get_mut_untracked(node_id) {
            let mut current = elem
                .get_attribute("class")
                .map(|c| {
                    c.split_ascii_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            current.retain(|c| !tokens.contains(c));
            let new_value = current.join(" ");
            if old_attribute.as_deref() != Some(&new_value) {
                elem.set_attribute("class", &new_value);
                if doc_mut.is_connected(node_id) {
                    doc_mut.bump_revision();
                    doc_mut.bump_style_revision();
                }
            }
            new_value
        } else {
            String::new()
        };
        drop(doc_mut);
        super::attributes::notify_attribute_mutation(this, "", "class", Some(&new_value), context)?;
        record_attribute_mutation(context, node_id, "class", old_attribute.as_deref())?;
    }
    Ok(JsValue::undefined())
}

pub(super) fn class_list_contains(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let class_name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        if let Some(NodeData::Element(elem)) = doc_ref.get(node_id) {
            let contains = elem.has_class(&class_name);
            return Ok(JsValue::new(contains));
        }
    }
    Ok(JsValue::new(false))
}

pub(super) fn class_list_toggle(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let force = if args.len() > 1 && !args.get_or_undefined(1).is_undefined() {
        Some(args.get_or_undefined(1).to_boolean())
    } else {
        None
    };
    let should_add = match force {
        Some(f) => f,
        None => !class_list_contains(this, args, context)?
            .as_boolean()
            .unwrap_or(false),
    };
    if should_add {
        class_list_add(this, args, context)?;
        Ok(JsValue::new(true))
    } else {
        class_list_remove(this, args, context)?;
        Ok(JsValue::new(false))
    }
}

pub(super) fn class_list_replace(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let old = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let new = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let had = class_list_contains(this, &[JsValue::from(js_string!(old.clone()))], context)?
        .as_boolean()
        .unwrap_or(false);
    if !had {
        return Ok(JsValue::new(false));
    }
    let node_id = get_node_id_from_this(this, context)?;
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        let doc = state.borrow().document.clone();
        let mut doc_mut = doc.borrow_mut();
        let old_attribute = match doc_mut.get(node_id) {
            Some(NodeData::Element(element)) => element.get_attribute("class").map(str::to_owned),
            _ => None,
        };
        let new_value = if let Some(NodeData::Element(elem)) = doc_mut.get_mut_untracked(node_id) {
            let mut current = elem
                .get_attribute("class")
                .map(|c| {
                    c.split_ascii_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for c in current.iter_mut() {
                if *c == old {
                    *c = new.clone();
                }
            }
            let new_value = current.join(" ");
            if old_attribute.as_deref() != Some(&new_value) {
                elem.set_attribute("class", &new_value);
                if doc_mut.is_connected(node_id) {
                    doc_mut.bump_revision();
                    doc_mut.bump_style_revision();
                }
            }
            new_value
        } else {
            String::new()
        };
        drop(doc_mut);
        super::attributes::notify_attribute_mutation(this, "", "class", Some(&new_value), context)?;
        record_attribute_mutation(context, node_id, "class", old_attribute.as_deref())?;
    }
    Ok(JsValue::new(true))
}

pub(super) fn class_list_get_value(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(js_string!("").into());
    };
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let value = match document.get(node_id) {
        Some(NodeData::Element(element)) => element.get_attribute("class").unwrap_or_default(),
        _ => {
            return Err(JsNativeError::typ()
                .with_message("DOMTokenList requires an Element")
                .into())
        }
    };
    Ok(js_string!(value).into())
}

pub(super) fn class_list_set_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let value = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let element = wrap_element(node_id, context);
    super::element::element_set_attribute(
        &element.into(),
        &[js_string!("class").into(), js_string!(value).into()],
        context,
    )
}

pub(super) fn class_list_get_tokens(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(boa_engine::object::builtins::JsArray::from_iter(
            Vec::<JsValue>::new(),
            context,
        )
        .into());
    };
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let value = match document.get(node_id) {
        Some(NodeData::Element(element)) => element.get_attribute("class").unwrap_or_default(),
        _ => {
            return Err(JsNativeError::typ()
                .with_message("DOMTokenList requires an Element")
                .into())
        }
    };
    let mut tokens = Vec::new();
    for token in value.split_ascii_whitespace() {
        if !tokens.iter().any(|existing: &String| existing == token) {
            tokens.push(token.to_string());
        }
    }
    let values = tokens.into_iter().map(|token| js_string!(token).into());
    Ok(boa_engine::object::builtins::JsArray::from_iter(values, context).into())
}

pub(super) fn class_list_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let values = class_list_get_tokens(this, &[], context)?;
    let values = values.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("DOMTokenList token list unavailable")
    })?;
    let index = args.get_or_undefined(0).to_number(context)?;
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 {
        return Ok(JsValue::null());
    }
    let item = values.get(index as u32, context)?;
    Ok(if item.is_undefined() {
        JsValue::null()
    } else {
        item
    })
}

pub(super) fn class_list_length(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let values = class_list_get_tokens(this, &[], context)?;
    let array = values.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("DOMTokenList token list unavailable")
    })?;
    Ok(array.get(js_string!("length"), context)?)
}
