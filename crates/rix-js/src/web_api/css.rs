use super::*;

#[derive(Clone, Copy)]
struct CssNamespaceRegistered;

pub(super) fn register_css_namespace(context: &mut Context) -> JsResult<()> {
    if context.get_data::<CssNamespaceRegistered>().is_some() {
        return Ok(());
    }

    let supports =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(css_supports))
            .name(js_string!("supports"))
            .length(1)
            .constructor(false)
            .build();
    let escape =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(css_escape))
            .name(js_string!("escape"))
            .length(1)
            .constructor(false)
            .build();
    let css = ObjectInitializer::new(context)
        .property(
            js_string!("supports"),
            supports,
            Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE,
        )
        .property(
            js_string!("escape"),
            escape,
            Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE,
        )
        .build();
    context.register_global_property(
        js_string!("CSS"),
        css,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;
    context.insert_data(CssNamespaceRegistered);
    Ok(())
}

fn css_supports(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(condition_or_property) = args.first() else {
        return Err(JsNativeError::typ()
            .with_message("CSS.supports requires a condition or property")
            .into());
    };
    let first = condition_or_property
        .to_string(context)?
        .to_std_string_escaped();
    let supported = if let Some(value) = args.get(1) {
        let value = value.to_string(context)?.to_std_string_escaped();
        rix_css::supports_declaration(&first, &value)
    } else {
        rix_css::matches_supports_query(&first)
    };
    Ok(JsValue::new(supported))
}

fn css_escape(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(input) = args.first() else {
        return Err(JsNativeError::typ()
            .with_message("CSS.escape requires an identifier")
            .into());
    };
    let input = input.to_string(context)?;
    let escaped = escape_css_identifier(&input);
    Ok(JsString::from(escaped.as_slice()).into())
}

fn escape_css_identifier(input: &JsString) -> Vec<u16> {
    let units: Vec<u16> = input.iter().collect();
    let mut code_points = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let first = units[index];
        if (0xD800..=0xDBFF).contains(&first)
            && units
                .get(index + 1)
                .is_some_and(|second| (0xDC00..=0xDFFF).contains(second))
        {
            let second = units[index + 1];
            let point =
                0x10000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00);
            code_points.push(point);
            index += 2;
        } else {
            code_points.push(u32::from(first));
            index += 1;
        }
    }

    let mut output = Vec::with_capacity(units.len());
    for (index, point) in code_points.iter().copied().enumerate() {
        if point == 0 {
            push_css_code_point(&mut output, 0xFFFD);
            continue;
        }
        let is_ascii_digit = (0x30..=0x39).contains(&point);
        let is_control = (0x01..=0x1F).contains(&point) || point == 0x7F;
        let begins_with_digit = index == 0 && is_ascii_digit;
        let digit_after_leading_hyphen =
            index == 1 && code_points.first() == Some(&u32::from(b'-')) && is_ascii_digit;
        if is_control || begins_with_digit || digit_after_leading_hyphen {
            push_css_hex_escape(&mut output, point);
        } else if index == 0 && point == u32::from(b'-') && code_points.len() == 1 {
            output.extend([b'\\' as u16, b'-' as u16]);
        } else if point >= 0x80
            || point == u32::from(b'-')
            || point == u32::from(b'_')
            || is_ascii_digit
            || (0x41..=0x5A).contains(&point)
            || (0x61..=0x7A).contains(&point)
        {
            push_css_code_point(&mut output, point);
        } else {
            output.push(b'\\' as u16);
            push_css_code_point(&mut output, point);
        }
    }
    output
}

fn push_css_hex_escape(output: &mut Vec<u16>, code_point: u32) {
    output.push(b'\\' as u16);
    for digit in format!("{code_point:x}").bytes() {
        output.push(u16::from(digit));
    }
    output.push(b' ' as u16);
}

fn push_css_code_point(output: &mut Vec<u16>, code_point: u32) {
    if code_point <= 0xFFFF {
        output.push(code_point as u16);
    } else {
        let supplementary = code_point - 0x10000;
        output.push(((supplementary >> 10) + 0xD800) as u16);
        output.push(((supplementary & 0x3FF) + 0xDC00) as u16);
    }
}
