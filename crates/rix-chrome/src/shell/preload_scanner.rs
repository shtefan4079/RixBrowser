
use memchr::memchr;
use rix_core::url::RixUrl;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PreloadHint {
    Preconnect(RixUrl),
    DnsPrefetch(String),
    FetchStylesheet(RixUrl),
    FetchScript(RixUrl),
}

pub(super) struct PreloadScanner {
    base_url: RixUrl,
    carry_over: Vec<u8>,
    head_closed: bool,
}

impl PreloadScanner {
    pub(super) fn new(base_url: &RixUrl) -> Self {
        Self {
            base_url: base_url.clone(),
            carry_over: Vec::new(),
            head_closed: false,
        }
    }

    pub(super) fn scan_chunk<F>(&mut self, chunk_bytes: &[u8], mut on_hint: F)
    where
        F: FnMut(PreloadHint),
    {
        if self.head_closed {
            return;
        }

        let combined_bytes;
        let slice: &[u8] = if self.carry_over.is_empty() {
            chunk_bytes
        } else {
            let mut buf = std::mem::take(&mut self.carry_over);
            buf.extend_from_slice(chunk_bytes);
            combined_bytes = buf;
            &combined_bytes
        };

        let mut cursor = 0;
        while cursor < slice.len() {
            let Some(tag_start_rel) = memchr(b'<', &slice[cursor..]) else {
                break;
            };
            let start = cursor + tag_start_rel;
            let Some(tag_end_rel) = memchr(b'>', &slice[start..]) else {
                
                if slice.len() - start < 2048 {
                    self.carry_over.clear();
                    self.carry_over.extend_from_slice(&slice[start..]);
                }
                return;
            };
            let end = start + tag_end_rel;
            cursor = end + 1;

            let tag_bytes = &slice[start + 1..end];
            let Ok(tag_content) = std::str::from_utf8(tag_bytes) else {
                continue;
            };
            let tag_content = tag_content.trim();
            if tag_content.is_empty() {
                continue;
            }

            let tag_name = tag_content
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            if tag_name == "/head" || tag_name == "body" {
                self.head_closed = true;
                return;
            }

            if tag_name == "base" {
                let attrs = extract_attributes(tag_content);
                for (k, v) in attrs {
                    if k.eq_ignore_ascii_case("href") {
                        if let Ok(new_base) = self.base_url.join(v) {
                            self.base_url = new_base;
                        }
                    }
                }
            } else if tag_name == "link" {
                let attrs = extract_attributes(tag_content);
                let mut rel = "";
                let mut href = "";
                let mut as_attr = "";
                for (k, v) in attrs {
                    if k.eq_ignore_ascii_case("rel") {
                        rel = v;
                    } else if k.eq_ignore_ascii_case("href") {
                        href = v;
                    } else if k.eq_ignore_ascii_case("as") {
                        as_attr = v;
                    }
                }

                if href.is_empty() {
                    continue;
                }

                let rel_lower = rel.to_ascii_lowercase();
                let tokens: Vec<&str> = rel_lower.split_ascii_whitespace().collect();

                if tokens.contains(&"preconnect") {
                    if let Ok(target) = self.base_url.join(href) {
                        on_hint(PreloadHint::Preconnect(target));
                    }
                } else if tokens.contains(&"dns-prefetch") {
                    let host = if href.starts_with("//") {
                        href.trim_start_matches('/').split('/').next().unwrap_or("").to_string()
                    } else if let Ok(target) = self.base_url.join(href) {
                        target.host_str().unwrap_or("").to_string()
                    } else {
                        String::new()
                    };
                    if !host.is_empty() {
                        on_hint(PreloadHint::DnsPrefetch(host));
                    }
                } else if tokens.contains(&"preload") {
                    if let Ok(target) = self.base_url.join(href) {
                        if as_attr.eq_ignore_ascii_case("style") {
                            on_hint(PreloadHint::FetchStylesheet(target));
                        } else if as_attr.eq_ignore_ascii_case("script") {
                            on_hint(PreloadHint::FetchScript(target));
                        }
                    }
                } else if tokens.contains(&"stylesheet") {
                    if let Ok(target) = self.base_url.join(href) {
                        on_hint(PreloadHint::FetchStylesheet(target));
                    }
                }
            } else if tag_name == "script" {
                let attrs = extract_attributes(tag_content);
                let mut src = "";
                let mut script_type = "";
                for (k, v) in attrs {
                    if k.eq_ignore_ascii_case("src") {
                        src = v;
                    } else if k.eq_ignore_ascii_case("type") {
                        script_type = v;
                    }
                }

                if !src.is_empty() {
                    let is_module = crate::page_worker::is_module_script_type(script_type);
                    let is_classic = crate::page_worker::is_classic_script_type(script_type);
                    if is_module || is_classic {
                        if let Ok(target) = self.base_url.join(src) {
                            on_hint(PreloadHint::FetchScript(target));
                        }
                    }
                }
            }
        }
    }
}

fn extract_attributes(mut s: &str) -> Vec<(&str, &str)> {
    let mut attrs = Vec::new();
    if let Some(pos) = s.find(|c: char| c.is_whitespace()) {
        s = &s[pos..];
    } else {
        return attrs;
    }

    while !s.is_empty() {
        s = s.trim_start();
        if s.is_empty() || s.starts_with('>') || s.starts_with("/>") {
            break;
        }
        let name_end = s
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/' || c == '>')
            .unwrap_or(s.len());
        let name = &s[..name_end];
        s = &s[name_end..].trim_start();
        if s.starts_with('=') {
            s = &s[1..].trim_start();
            if s.starts_with('"') {
                s = &s[1..];
                let val_end = s.find('"').unwrap_or(s.len());
                let val = &s[..val_end];
                attrs.push((name, val));
                s = if val_end < s.len() { &s[val_end + 1..] } else { "" };
            } else if s.starts_with('\'') {
                s = &s[1..];
                let val_end = s.find('\'').unwrap_or(s.len());
                let val = &s[..val_end];
                attrs.push((name, val));
                s = if val_end < s.len() { &s[val_end + 1..] } else { "" };
            } else {
                let val_end = s
                    .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
                    .unwrap_or(s.len());
                let val = &s[..val_end];
                attrs.push((name, val));
                s = &s[val_end..];
            }
        } else {
            attrs.push((name, ""));
        }
    }
    attrs
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn test_preload_scanner_hints_and_chunking() {
        let base = RixUrl::parse("https://example.com/page").unwrap();
        let mut scanner = PreloadScanner::new(&base);
        let mut hints = Vec::new();

        let chunk1 = b"<!doctype html><html><head><base href=\"https://example.com/assets/\">\
            <link rel=\"preconnect\" href=\"https://fonts.gstatic.com\">\
            <link rel=\"dns-prefetch\" href=\"//i.ytimg.com\">\
            <link rel=\"preload\" href=\"app.css\" as=\"style\">\
            <link rel=\"stylesheet\" hr";
        let chunk2 = b"ef=\"theme.css\">\
            <script src=\"bundle.js\"></script></head><body><h1>Hello</h1></body></html>";

        scanner.scan_chunk(chunk1, |h| hints.push(h));
        scanner.scan_chunk(chunk2, |h| hints.push(h));

        assert!(hints.contains(&PreloadHint::Preconnect(
            RixUrl::parse("https://fonts.gstatic.com").unwrap()
        )));
        assert!(hints.contains(&PreloadHint::DnsPrefetch("i.ytimg.com".to_string())));
        assert!(hints.contains(&PreloadHint::FetchStylesheet(
            RixUrl::parse("https://example.com/assets/app.css").unwrap()
        )));
        assert!(hints.contains(&PreloadHint::FetchStylesheet(
            RixUrl::parse("https://example.com/assets/theme.css").unwrap()
        )));
        assert!(hints.contains(&PreloadHint::FetchScript(
            RixUrl::parse("https://example.com/assets/bundle.js").unwrap()
        )));
    }
}
