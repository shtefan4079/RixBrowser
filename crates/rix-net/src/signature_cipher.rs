
pub const PLAYER_VERSION: &str = "0229cd64";

pub fn decipher(s: &str) -> String {
    let decoded = decode_uri_component(s);
    let mut chars: Vec<char> = decoded.chars().collect();

    chars.reverse();

    splice_remove(&mut chars, 0, 1);

    swap_first(&mut chars, 53);

    splice_remove(&mut chars, 0, 3);

    swap_first(&mut chars, 18);

    swap_first(&mut chars, 61);

    chars.into_iter().collect()
}

pub fn decipher_signature(cipher: &str, _player_js: &str) -> Option<String> {
    let query = cipher.split_once('?').map(|(_, q)| q).unwrap_or(cipher);
    let mut s_value: Option<String> = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut kv = pair.splitn(2, '=');
        let key = kv.next()?;
        let value = kv.next().unwrap_or("");
        if key == "s" {
            s_value = Some(value.to_string());
        }
    }
    s_value.map(|s| decipher(&s))
}

fn splice_remove(chars: &mut Vec<char>, start: usize, count: usize) {
    if start >= chars.len() {
        return;
    }
    let end = (start + count).min(chars.len());
    chars.drain(start..end);
}

fn swap_first(chars: &mut Vec<char>, l: usize) {
    if chars.is_empty() {
        return;
    }
    let idx = l % chars.len();
    chars.swap(0, idx);
}

fn decode_uri_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
