use super::*;
use rix_css::InternedStr;

pub(super) fn inline_style(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<Vec<rix_css::Declaration>> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let doc = state.borrow().document.clone();
    let doc = doc.borrow();
    let value = match doc.get(node) {
        Some(NodeData::Element(element)) => element.get_attribute("style").unwrap_or(""),
        _ => "",
    };
    Ok(rix_css::parse_inline_style(value))
}

pub(super) fn element_style_get(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let value = inline_style(this, context)?
        .into_iter()
        .rev()
        .find(|item| item.name == name)
        .map(|item| item.value)
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

pub(super) fn element_style_priority(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let important = inline_style(this, context)?
        .into_iter()
        .rev()
        .find(|item| item.name == name)
        .is_some_and(|item| item.important);
    Ok(js_string!(if important { "important" } else { "" }).into())
}

fn inline_property_names(this: &JsValue, context: &mut Context) -> JsResult<Vec<InternedStr>> {
    let mut seen = HashSet::new();
    Ok(inline_style(this, context)?
        .into_iter()
        .map(|item| item.name)
        .filter(|name| seen.insert(name.clone()))
        .collect())
}

pub(super) fn element_style_length(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let length = inline_property_names(this, context)?.len();
    let length = length.min(u32::MAX as usize) as u32;
    Ok(JsValue::new(length))
}

pub(super) fn element_style_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = args.get_or_undefined(0).to_number(context)? as u32 as usize;
    Ok(inline_property_names(this, context)?
        .get(index)
        .map_or(JsValue::from(js_string!("")), |name| {
            JsValue::from(js_string!(name.as_str()))
        }))
}

pub(super) fn write_inline_style(
    this: &JsValue,
    declarations: Vec<rix_css::Declaration>,
    context: &mut Context,
) -> JsResult<()> {
    let value = declarations
        .into_iter()
        .map(|item| {
            format!(
                "{}: {}{};",
                item.name,
                item.value,
                if item.important { " !important" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    element_set_attribute(
        this,
        &[js_string!("style").into(), js_string!(value).into()],
        context,
    )?;
    Ok(())
}

pub(super) fn element_style_remove(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let mut declarations = inline_style(this, context)?;
    let previous = declarations
        .iter()
        .rev()
        .find(|item| item.name == name)
        .map(|item| item.value.clone())
        .unwrap_or_default();
    declarations.retain(|item| item.name != name);
    write_inline_style(this, declarations, context)?;
    Ok(js_string!(previous).into())
}

pub(super) fn element_style_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let mut name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if !name.starts_with("--") {
        name.make_ascii_lowercase();
    }
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Ok(JsValue::undefined());
    }
    let value = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    if value.is_empty() {
        element_style_remove(this, &[js_string!(name).into()], context)?;
        return Ok(JsValue::undefined());
    }
    let priority = if args.get_or_undefined(2).is_undefined() {
        String::new()
    } else {
        args.get_or_undefined(2)
            .to_string(context)?
            .to_std_string_escaped()
    };
    if !priority.is_empty() && !priority.eq_ignore_ascii_case("important") {
        return Ok(JsValue::undefined());
    }
    let added = rix_css::parse_inline_style(&format!(
        "{name}: {value}{}",
        if priority.is_empty() {
            ""
        } else {
            " !important"
        }
    ));
    if added.len() != 1 || added[0].name != name {
        return Ok(JsValue::undefined());
    }
    let mut declarations = inline_style(this, context)?;
    declarations.retain(|item| item.name != name);
    declarations.extend(added);
    write_inline_style(this, declarations, context)?;
    Ok(JsValue::undefined())
}
