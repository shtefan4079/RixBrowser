
use boa_engine::{
    js_string, Context, JsError, JsNativeError, JsObject, JsResult, JsString, JsSymbol, JsValue,
};

const MAX_SEQUENCE_ENTRIES: usize = 4096;
const MAX_SEQUENCE_UTF8_BYTES: usize = 1024 * 1024;

pub(super) fn convert_observed_attributes(
    value: &JsValue,
    context: &mut Context,
) -> JsResult<Vec<String>> {
    let object = value.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("observedAttributes must be an iterable object")
    })?;
    let method = object.get(JsSymbol::iterator(), context)?;
    let method = method
        .as_object()
        .filter(JsObject::is_callable)
        .ok_or_else(|| {
            JsNativeError::typ().with_message("observedAttributes must have a callable iterator")
        })?;
    let iterator = method.call(value, &[], context)?;
    let iterator = iterator.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("observedAttributes iterator must return an object")
    })?;
    let next_method = iterator.get(js_string!("next"), context)?;
    let next_method = next_method
        .as_object()
        .filter(JsObject::is_callable)
        .ok_or_else(|| {
            JsNativeError::typ().with_message("observedAttributes iterator next is not callable")
        })?;
    let receiver = JsValue::from(iterator.clone());
    let mut values = Vec::new();
    let mut utf8_bytes = 0;
    loop {
        let result = next_method.call(&receiver, &[], context)?;
        let result = result.as_object().ok_or_else(|| {
            JsNativeError::typ().with_message("observedAttributes iterator result is not an object")
        })?;
        if result.get(js_string!("done"), context)?.to_boolean() {
            return Ok(values);
        }
        let next_value = result.get(js_string!("value"), context)?;
        if values.len() >= MAX_SEQUENCE_ENTRIES {
            return Err(close_for_budget(
                &iterator,
                "observedAttributes exceeds the per-sequence entry budget",
                context,
            ));
        }
        let string = next_value.to_string(context)?;
        let Some(bytes) = stored_utf8_bytes(&string, MAX_SEQUENCE_UTF8_BYTES - utf8_bytes) else {
            return Err(close_for_budget(
                &iterator,
                "observedAttributes exceeds the per-sequence UTF-8 byte budget",
                context,
            ));
        };
        utf8_bytes += bytes;
        values.push(string.to_std_string_escaped());
    }
}

fn stored_utf8_bytes(string: &JsString, maximum: usize) -> Option<usize> {
    char::decode_utf16(string.iter()).try_fold(0_usize, |total, character| {
        total
            .checked_add(character.map_or(6, char::len_utf8))
            .filter(|bytes| *bytes <= maximum)
    })
}

fn close_for_budget(iterator: &JsObject, message: &'static str, context: &mut Context) -> JsError {
    let original: JsError = JsNativeError::range().with_message(message).into();
    let return_method = match iterator.get(js_string!("return"), context) {
        Ok(method) => method,
        Err(_) => return original,
    };
    if let Some(method) = return_method.as_object().filter(JsObject::is_callable) {
        return match method.call(&iterator.clone().into(), &[], context) {
            Ok(_) => original,
            Err(_closing_error) => original,
        };
    }
    original
}
