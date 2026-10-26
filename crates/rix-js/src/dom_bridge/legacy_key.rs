
use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsResult, JsValue, NativeFunction};

pub(super) fn register_legacy_key_interfaces(context: &mut Context) -> JsResult<()> {
    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixLegacyKeyCode"),
        3,
        NativeFunction::from_copy_closure(|_this, args, context| {
            let event_type = args
                .get(0)
                .map(|value| value.to_string(context))
                .transpose()?
                .map(|value| value.to_std_string_escaped())
                .unwrap_or_default();
            let key = args
                .get(1)
                .map(|value| value.to_string(context))
                .transpose()?
                .map(|value| value.to_std_string_escaped())
                .unwrap_or_default();
            let code = args
                .get(2)
                .map(|value| value.to_string(context))
                .transpose()?
                .map(|value| value.to_std_string_escaped())
                .unwrap_or_default();
            let (key_code, char_code, which) = legacy_key_values(&event_type, &key, &code);
            let values = [
                JsValue::new(key_code),
                JsValue::new(char_code),
                JsValue::new(which),
            ];
            Ok(JsArray::from_iter(values, context).into())
        }),
    )?;
    Ok(())
}

pub fn legacy_key_values(event_type: &str, key: &str, code: &str) -> (u32, u32, u32) {
    if event_type == "keypress" {
        let char_code = if key == "Enter" {
            13
        } else {
            character_code(key)
        };
        return (char_code, char_code, char_code);
    }
    let key_code = legacy_key_code(key, code);
    (key_code, 0, key_code)
}

fn character_code(key: &str) -> u32 {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if !ch.is_control() => {
            ch.encode_utf16(&mut [0; 2]).first().copied().unwrap_or(0) as u32
        }
        _ => 0,
    }
}

fn legacy_key_code(key: &str, code: &str) -> u32 {
    if let Some(value) = named_key_code(key) {
        return value;
    }
    if let Some(value) = printable_key_code(key) {
        return value;
    }
    code_key_code(code)
}

fn named_key_code(key: &str) -> Option<u32> {
    Some(match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        " " | "Spacebar" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Select" => 41,
        "PrintScreen" => 44,
        "Insert" => 45,
        "Delete" => 46,
        "Help" => 47,
        "Meta" => 91,
        "ContextMenu" => 93,
        "NumLock" => 144,
        "ScrollLock" => 145,
        "AudioVolumeMute" => 173,
        "AudioVolumeDown" => 174,
        "AudioVolumeUp" => 175,
        "MediaTrackNext" => 176,
        "MediaTrackPrevious" => 177,
        "MediaStop" => 178,
        "MediaPlayPause" => 179,
        "LaunchMediaPlayer" => 181,
        "AltGraph" => 225,
        _ => return function_key_code(key),
    })
}

fn function_key_code(name: &str) -> Option<u32> {
    let index: u32 = name.strip_prefix('F')?.parse().ok()?;
    (1..=24).contains(&index).then_some(111 + index)
}

fn printable_key_code(key: &str) -> Option<u32> {
    let mut chars = key.chars();
    let ch = match (chars.next(), chars.next()) {
        (Some(ch), None) => ch,
        _ => return None,
    };
    if ch.is_ascii_alphabetic() {
        return Some(ch.to_ascii_uppercase() as u32);
    }
    if ch.is_ascii_digit() {
        return Some(ch as u32);
    }
    Some(match ch {
        ';' | ':' => 186,
        '=' | '+' => 187,
        ',' | '<' => 188,
        '-' | '_' => 189,
        '.' | '>' => 190,
        '/' | '?' => 191,
        '`' | '~' => 192,
        '[' | '{' => 219,
        '\\' | '|' => 220,
        ']' | '}' => 221,
        '\'' | '"' => 222,
        '!' => 49,
        '@' => 50,
        '#' => 51,
        '$' => 52,
        '%' => 53,
        '^' => 54,
        '&' => 55,
        '*' => 56,
        '(' => 57,
        ')' => 48,
        _ => return None,
    })
}

fn code_key_code(code: &str) -> u32 {
    if let Some(rest) = code.strip_prefix("Key") {
        if let Some(ch) = single_ascii(rest) {
            if ch.is_ascii_alphabetic() {
                return ch.to_ascii_uppercase() as u32;
            }
        }
    }
    if let Some(rest) = code.strip_prefix("Digit") {
        if let Some(ch) = single_ascii(rest) {
            if ch.is_ascii_digit() {
                return ch as u32;
            }
        }
    }
    if let Some(rest) = code.strip_prefix("Numpad") {
        return match rest {
            "0" => 96,
            "1" => 97,
            "2" => 98,
            "3" => 99,
            "4" => 100,
            "5" => 101,
            "6" => 102,
            "7" => 103,
            "8" => 104,
            "9" => 105,
            "Multiply" => 106,
            "Add" => 107,
            "Enter" => 13,
            "Subtract" => 109,
            "Decimal" => 110,
            "Divide" => 111,
            _ => 0,
        };
    }
    match code {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "ShiftLeft" | "ShiftRight" => 16,
        "ControlLeft" | "ControlRight" => 17,
        "AltLeft" | "AltRight" => 18,
        "CapsLock" => 20,
        "Escape" => 27,
        "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "PrintScreen" => 44,
        "Insert" => 45,
        "Delete" => 46,
        "Pause" => 19,
        "MetaLeft" | "MetaRight" => 91,
        "ContextMenu" => 93,
        "NumLock" => 144,
        "ScrollLock" => 145,
        _ => function_key_code(code).unwrap_or(0),
    }
}

fn single_ascii(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}
