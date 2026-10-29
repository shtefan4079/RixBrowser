use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceLoadKind {
    Document,
    Stylesheet,
    Script,
    Image,
    Fetch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLoadRecord {
    pub kind: ResourceLoadKind,
    pub method: String,
    pub request_url: String,
    pub response_url: Option<String>,
    pub status: Option<u16>,
    pub elapsed_ms: f64,
    pub response_bytes: Option<usize>,
    pub error: Option<String>,
}

impl ResourceLoadRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: ResourceLoadKind,
        method: &str,
        request_url: &str,
        response_url: Option<&str>,
        status: Option<u16>,
        elapsed: Duration,
        response_bytes: Option<usize>,
        error: Option<&str>,
    ) -> Self {
        Self {
            kind,
            method: method.chars().take(16).collect(),
            request_url: redact_resource_url(request_url),
            response_url: response_url.map(redact_resource_url),
            status,
            elapsed_ms: elapsed.as_secs_f64() * 1000.0,
            response_bytes,
            error: error.map(redact_error),
        }
    }
}

pub fn redact_resource_url(input: &str) -> String {
    redact_url(input)
}

pub fn redact_diagnostic(input: &str) -> String {
    redact_error(input)
}

fn redact_error(input: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let tail = &input[cursor..];
        let next = ["https://", "http://", "data:"]
            .iter()
            .filter_map(|scheme| tail.find(scheme).map(|offset| (offset, *scheme)))
            .min_by_key(|(offset, _)| *offset);
        let Some((offset, scheme)) = next else {
            output.push_str(tail);
            break;
        };
        let start = cursor + offset;
        output.push_str(&input[cursor..start]);
        let url_start = start;
        let mut end = url_start + scheme.len();
        while end < input.len() {
            let ch = input[end..].chars().next().unwrap_or_default();
            if ch.is_whitespace() || matches!(ch, ')' | ']' | '}' | '"' | '\'') {
                break;
            }
            end += ch.len_utf8();
        }
        let mut url_end = end;
        while url_end > url_start {
            let Some(ch) = input[..url_end].chars().next_back() else {
                break;
            };
            if matches!(ch, '.' | ',' | ';' | ':') {
                url_end -= ch.len_utf8();
            } else {
                break;
            }
        }
        if url_end == url_start {
            output.push_str(scheme);
            cursor = url_start + scheme.len();
        } else {
            output.push_str(&redact_diagnostic_url(&input[url_start..url_end]));
            output.push_str(&input[url_end..end]);
            cursor = end;
        }
    }
    output.chars().take(4096).collect()
}

fn redact_diagnostic_url(input: &str) -> String {
    let Some(hash) = input.rfind('#') else {
        return redact_resource_url(input);
    };
    let fragment_start = hash + 1;
    let fragment = &input[fragment_start..];
    let Some((before_column, column)) = fragment.rsplit_once(':') else {
        return redact_resource_url(input);
    };
    let Some((fragment_body, line)) = before_column.rsplit_once(':') else {
        return redact_resource_url(input);
    };
    if line.is_empty()
        || column.is_empty()
        || !line.bytes().all(|byte| byte.is_ascii_digit())
        || !column.bytes().all(|byte| byte.is_ascii_digit())
    {
        return redact_resource_url(input);
    }

    let position_start = fragment_start + fragment_body.len();
    format!(
        "{}{}",
        redact_resource_url(&input[..position_start]),
        &input[position_start..]
    )
}

fn redact_url(input: &str) -> String {
    if input
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
    {
        let data_url = &input[5..];
        let metadata = data_url
            .split_once(',')
            .map_or(data_url, |(metadata, _)| metadata);
        let media_type = metadata.split(';').next().unwrap_or_default();
        let safe_media_type = media_type
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '+' | '-' | '.'))
            .then_some(media_type)
            .unwrap_or_default();
        return format!("data:{safe_media_type},[redacted]");
    }
    let Ok(mut url) = ::url::Url::parse(input) else {
        return input
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .chars()
            .take(2048)
            .collect();
    };
    if (!url.username().is_empty() || url.password().is_some())
        && (url.set_username("").is_err() || url.set_password(None).is_err())
    {
        return "[redacted URL]".to_string();
    }
    if url.query().is_some() {
        url.set_query(Some("[redacted]"));
    }
    url.set_fragment(None);
    url.as_str().chars().take(2048).collect()
}
