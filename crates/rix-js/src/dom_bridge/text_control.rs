use super::*;

pub(super) fn is_text_control(document: &Document, node: NodeId) -> bool {
    matches!(document.get(node), Some(NodeData::Element(element)) if
        &*element.name.local == "textarea" || (&*element.name.local == "input" &&
            matches!(element.get_attribute("type").unwrap_or("text").to_ascii_lowercase().as_str(),
                "text" | "search" | "url" | "tel" | "password")))
}

pub(super) fn text_control_value(state: &DomBridgeState, document: &Document, node: NodeId) -> String {
    if let Some(value) = state.input_values.get(&node) {
        return value.clone();
    }
    match document.get(node) {
        Some(NodeData::Element(element)) => element
            .get_attribute("value")
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if &*element.name.local == "textarea" {
                    document.inner_text(node)
                } else {
                    String::new()
                }
            }),
        _ => String::new(),
    }
}

pub(super) fn scalar_to_utf16(value: &str, offset: usize) -> usize {
    value.chars().take(offset).map(char::len_utf16).sum()
}

pub(super) fn utf16_to_scalar(value: &str, offset: usize) -> usize {
    let mut code_units = 0;
    value
        .chars()
        .take_while(|character| {
            code_units += character.len_utf16();
            code_units <= offset
        })
        .count()
}

pub(super) fn input_get_selection(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let (start, end, direction) = {
        let state = state.borrow();
        let document = state.document.borrow();
        if !is_text_control(&document, node) {
            return Ok(JsValue::null());
        }
        let value = text_control_value(&state, &document, node);
        let (anchor, cursor) = document
            .text_selection()
            .filter(|(target, _, _)| *target == node)
            .map(|(_, anchor, cursor)| (anchor, cursor))
            .unwrap_or((0, 0));
        (
            scalar_to_utf16(&value, anchor.min(cursor)),
            scalar_to_utf16(&value, anchor.max(cursor)),
            if anchor > cursor {
                "backward"
            } else if anchor < cursor {
                "forward"
            } else {
                "none"
            },
        )
    };
    Ok(boa_engine::object::builtins::JsArray::from_iter(
        [
            JsValue::new(start as f64),
            JsValue::new(end as f64),
            js_string!(direction).into(),
        ],
        context,
    )
    .into())
}

pub(super) fn input_set_selection_range(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let start = args.get_or_undefined(0).to_number(context)?.max(0.0) as usize;
    let end = args.get_or_undefined(1).to_number(context)?.max(0.0) as usize;
    let backward = args
        .get_or_undefined(2)
        .to_string(context)?
        .to_std_string_escaped()
        == "backward";
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let (start, end) = {
        let document = document.borrow();
        if !is_text_control(&document, node) {
            return Err(JsNativeError::typ()
                .with_message("InvalidStateError: input does not support text selection")
                .into());
        }
        let value = text_control_value(&state.borrow(), &document, node);
        (
            utf16_to_scalar(&value, start.min(end)),
            utf16_to_scalar(&value, end),
        )
    };
    let (anchor, cursor) = if backward { (end, start) } else { (start, end) };
    document
        .borrow_mut()
        .set_text_selection(Some((node, anchor, cursor)));
    Ok(JsValue::undefined())
}
