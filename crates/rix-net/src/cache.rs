use crate::client::HttpResponse;
use crate::dns_tls::NegotiatedProtocol;
use crate::error::{NetError, Result};
use crate::timing::NavigationTiming;
use bytes::Bytes;
use rix_core::url::RixUrl;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_MEM_ENTRIES: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheControlDirectives {
    pub no_store: bool,
    pub no_cache: bool,
    pub max_age: Option<u64>,
    pub s_maxage: Option<u64>,
    pub immutable: bool,
    pub must_revalidate: bool,
    pub public: bool,
    pub private: bool,
}

impl CacheControlDirectives {
    pub fn parse(header_value: &str) -> Self {
        let mut directives = Self::default();
        for item in header_value.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let (key, val) = match item.split_once('=') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), Some(v.trim())),
                None => (item.to_ascii_lowercase(), None),
            };
            match key.as_str() {
                "no-store" => directives.no_store = true,
                "no-cache" => directives.no_cache = true,
                "immutable" => directives.immutable = true,
                "must-revalidate" => directives.must_revalidate = true,
                "public" => directives.public = true,
                "private" => directives.private = true,
                "max-age" => {
                    if let Some(v) = val {
                        let clean = v.trim_matches('"');
                        if let Ok(secs) = clean.parse::<u64>() {
                            directives.max_age = Some(secs);
                        }
                    }
                }
                "s-maxage" => {
                    if let Some(v) = val {
                        let clean = v.trim_matches('"');
                        if let Ok(secs) = clean.parse::<u64>() {
                            directives.s_maxage = Some(secs);
                        }
                    }
                }
                _ => {}
            }
        }
        directives
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn parse_http_date(date_str: &str) -> Option<u64> {
    httpdate::parse_http_date(date_str.trim())
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

pub fn fmt_http_date(secs: u64) -> String {
    let st = UNIX_EPOCH + Duration::from_secs(secs);
    httpdate::fmt_http_date(st)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpCacheMeta {
    pub url: String,
    pub final_url: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub content_type: String,
    pub protocol: NegotiatedProtocol,
    pub fetched_at: u64,
    pub response_date: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub max_age: Option<u64>,
    pub expires: Option<u64>,
    pub immutable: bool,
    pub must_revalidate: bool,
    pub no_cache: bool,
    pub body_len: usize,
}

impl HttpCacheMeta {
    
    pub fn freshness_lifetime(&self) -> u64 {
        if let Some(max_age) = self.max_age {
            return max_age;
        }
        if let Some(expires) = self.expires {
            let ref_date = self.response_date.unwrap_or(self.fetched_at);
            return expires.saturating_sub(ref_date);
        }
        if let Some(ref lm) = self.last_modified {
            if let Some(lm_sec) = parse_http_date(lm) {
                let diff = self.fetched_at.saturating_sub(lm_sec);
                return (diff / 10).min(7 * 86400);
            }
        }
        // RFC 9111 §4.2.2: Heuristic freshness for static assets (images, SVGs, fonts, media) or responses with ETag
        let is_static_asset = self.content_type.starts_with("image/")
            || self.content_type.starts_with("font/")
            || self.content_type.contains("svg")
            || self.url.ends_with(".svg")
            || self.url.ends_with(".png")
            || self.url.ends_with(".jpg")
            || self.url.ends_with(".jpeg")
            || self.url.ends_with(".webp")
            || self.url.ends_with(".woff2")
            || self.url.ends_with(".woff")
            || self.url.ends_with(".ttf");
        if is_static_asset {
            return 86400; // 24 hours default for images/svg/fonts without explicit headers
        }
        if self.etag.is_some() {
            return 3600; // 1 hour for resources with ETag
        }
        0
    }

    pub fn current_age(&self, now: u64) -> u64 {
        let apparent_age = match self.response_date {
            Some(date) if self.fetched_at >= date => {
                
                (self.fetched_at - date).min(86400)
            }
            _ => 0,
        };
        let resident_time = now.saturating_sub(self.fetched_at);
        apparent_age.saturating_add(resident_time)
    }

    pub fn is_fresh(&self, now: u64) -> bool {
        if self.no_cache {
            return false;
        }
        let age = self.current_age(now);
        let lifetime = self.freshness_lifetime();
        age < lifetime
    }

    pub fn can_revalidate(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct HttpCacheEntry {
    pub meta: HttpCacheMeta,
    pub body: Bytes,
}

impl HttpCacheEntry {
    pub fn is_fresh(&self, now: u64) -> bool {
        self.meta.is_fresh(now)
    }

    pub fn can_revalidate(&self) -> bool {
        self.meta.can_revalidate()
    }
}

#[derive(Clone)]
pub struct HttpCache {
    dir: Option<PathBuf>,
    l1_mem: Arc<RwLock<HashMap<String, HttpCacheEntry>>>,
    max_mem_entries: usize,
}

impl HttpCache {
    
    pub fn new(dir: Option<PathBuf>) -> Self {
        if let Some(ref d) = dir {
            let _ = std::fs::create_dir_all(d);
        }
        Self {
            dir,
            l1_mem: Arc::new(RwLock::new(HashMap::new())),
            max_mem_entries: DEFAULT_MAX_MEM_ENTRIES,
        }
    }

    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| NetError::Cache(format!("Cannot create HTTP cache dir: {e}")))?;
        Ok(Self::new(Some(dir)))
    }

    pub fn default_cache_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("RIX_HTTP_CACHE_DIR") {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        if let Ok(runtime_dir) = std::env::var("XDG_CACHE_HOME") {
            if !runtime_dir.is_empty() {
                return PathBuf::from(runtime_dir).join("rixbrowser").join("http_cache");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home)
                    .join(".cache")
                    .join("rixbrowser")
                    .join("http_cache");
            }
        }
        std::env::temp_dir().join("rix-http-cache")
    }

    pub fn default_disk_cache() -> Self {
        let dir = Self::default_cache_dir();
        let _ = std::fs::create_dir_all(&dir);
        Self::new(Some(dir))
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    fn stem_for(url_str: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(url_str.as_bytes());
        let digest = hasher.finalize();
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(digest.len() * 2);
        for b in digest {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
        out
    }

    fn paths_for(&self, url_str: &str) -> Option<(PathBuf, PathBuf)> {
        let dir = self.dir.as_ref()?;
        let stem = Self::stem_for(url_str);
        Some((
            dir.join(format!("{stem}.meta.json")),
            dir.join(format!("{stem}.body")),
        ))
    }

    pub fn get(&self, url_str: &str) -> Option<HttpCacheEntry> {
        
        if let Ok(guard) = self.l1_mem.read() {
            if let Some(entry) = guard.get(url_str) {
                return Some(entry.clone());
            }
        }

        let (meta_path, body_path) = self.paths_for(url_str)?;
        let meta_bytes = std::fs::read(&meta_path).ok()?;
        let meta: HttpCacheMeta = serde_json::from_slice(&meta_bytes).ok()?;
        if meta.url != url_str {
            return None;
        }
        let body = std::fs::read(&body_path).ok()?;
        if body.len() != meta.body_len {
            return None;
        }

        let entry = HttpCacheEntry {
            meta,
            body: Bytes::from(body),
        };

        if let Ok(mut guard) = self.l1_mem.write() {
            if guard.len() >= self.max_mem_entries && !guard.contains_key(url_str) {
                if let Some(first_key) = guard.keys().next().cloned() {
                    guard.remove(&first_key);
                }
            }
            guard.insert(url_str.to_string(), entry.clone());
        }

        Some(entry)
    }

    pub fn put(&self, url_str: &str, response: &HttpResponse) -> Result<()> {
        
        if response.status != 200 && response.status != 301 && response.status != 308 {
            return Ok(());
        }

        let cc = response
            .headers
            .get("cache-control")
            .map(|s| CacheControlDirectives::parse(s))
            .unwrap_or_default();

        if cc.no_store {
            return Ok(());
        }

        let etag = response.headers.get("etag").cloned();
        let last_modified = response.headers.get("last-modified").cloned();
        let response_date = response
            .headers
            .get("date")
            .and_then(|d| parse_http_date(d));
        let expires = response
            .headers
            .get("expires")
            .and_then(|e| parse_http_date(e));
        let max_age = cc.max_age.or(cc.s_maxage);
        let now = now_secs();

        let meta = HttpCacheMeta {
            url: url_str.to_string(),
            final_url: response.final_url.as_str().to_string(),
            status: response.status,
            headers: response.headers.clone(),
            content_type: response.content_type.clone(),
            protocol: response.protocol,
            fetched_at: now,
            response_date,
            etag,
            last_modified,
            max_age,
            expires,
            immutable: cc.immutable,
            must_revalidate: cc.must_revalidate,
            no_cache: cc.no_cache,
            body_len: response.body.len(),
        };

        let entry = HttpCacheEntry {
            meta: meta.clone(),
            body: Bytes::from(response.body.clone()),
        };

        if let Ok(mut guard) = self.l1_mem.write() {
            if guard.len() >= self.max_mem_entries && !guard.contains_key(url_str) {
                if let Some(first_key) = guard.keys().next().cloned() {
                    guard.remove(&first_key);
                }
            }
            guard.insert(url_str.to_string(), entry);
        }

        if let Some((meta_path, body_path)) = self.paths_for(url_str) {
            let meta_json = serde_json::to_vec(&meta)
                .map_err(|e| NetError::Cache(format!("Failed to serialize cache metadata: {e}")))?;

            let pid = std::process::id();
            let tmp_body = body_path.with_extension(format!("tmp.{pid}.body"));
            let tmp_meta = meta_path.with_extension(format!("tmp.{pid}.json"));

            std::fs::write(&tmp_body, &response.body)
                .map_err(|e| NetError::Cache(format!("Failed to write cache body: {e}")))?;
            std::fs::write(&tmp_meta, &meta_json)
                .map_err(|e| NetError::Cache(format!("Failed to write cache meta: {e}")))?;

            let _ = std::fs::rename(&tmp_body, &body_path);
            let _ = std::fs::rename(&tmp_meta, &meta_path);
        }

        Ok(())
    }

    pub fn update_revalidated(
        &self,
        url_str: &str,
        new_headers: &HashMap<String, String>,
        now: u64,
    ) -> Option<HttpCacheEntry> {
        let existing = self.get(url_str)?;
        let mut meta = existing.meta.clone();

        meta.fetched_at = now;

        for (k, v) in new_headers {
            let lower = k.to_ascii_lowercase();
            if !matches!(
                lower.as_str(),
                "content-length" | "content-type" | "content-encoding" | "content-range"
            ) {
                meta.headers.insert(k.clone(), v.clone());
            }
        }

        if let Some(cc_val) = meta.headers.get("cache-control") {
            let cc = CacheControlDirectives::parse(cc_val);
            meta.max_age = cc.max_age.or(cc.s_maxage);
            meta.immutable = cc.immutable;
            meta.must_revalidate = cc.must_revalidate;
            meta.no_cache = cc.no_cache;
        }
        if let Some(date_val) = meta.headers.get("date") {
            meta.response_date = parse_http_date(date_val);
        }
        if let Some(expires_val) = meta.headers.get("expires") {
            meta.expires = parse_http_date(expires_val);
        }
        if let Some(etag_val) = meta.headers.get("etag") {
            meta.etag = Some(etag_val.clone());
        }

        let updated_entry = HttpCacheEntry {
            meta: meta.clone(),
            body: existing.body,
        };

        if let Ok(mut guard) = self.l1_mem.write() {
            guard.insert(url_str.to_string(), updated_entry.clone());
        }

        if let Some((meta_path, _)) = self.paths_for(url_str) {
            if let Ok(meta_json) = serde_json::to_vec(&meta) {
                let pid = std::process::id();
                let tmp_meta = meta_path.with_extension(format!("tmp.{pid}.reval.json"));
                if std::fs::write(&tmp_meta, &meta_json).is_ok() {
                    let _ = std::fs::rename(&tmp_meta, &meta_path);
                }
            }
        }

        Some(updated_entry)
    }

    pub fn to_http_response(&self, entry: &HttpCacheEntry, now: u64) -> HttpResponse {
        let mut headers = entry.meta.headers.clone();
        let current_age = entry.meta.current_age(now);
        headers.insert("age".to_string(), format!("{current_age}"));

        headers.remove("content-encoding");
        headers.insert("content-length".to_string(), format!("{}", entry.body.len()));

        let final_url = RixUrl::parse(&entry.meta.final_url)
            .or_else(|_| RixUrl::parse(&entry.meta.url))
            .unwrap_or_else(|_| RixUrl::parse("about:blank").unwrap());

        HttpResponse {
            status: entry.meta.status,
            headers,
            body: entry.body.to_vec(),
            final_url,
            content_type: entry.meta.content_type.clone(),
            protocol: entry.meta.protocol,
            navigation_timing: NavigationTiming::default(),
            navigation_started: Instant::now(),
            set_cookies: Vec::new(),
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.l1_mem.write() {
            guard.clear();
        }
        if let Some(ref dir) = self.dir {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_control_parsing() {
        let cc = CacheControlDirectives::parse("public, max-age=31536000, immutable");
        assert!(cc.public);
        assert!(!cc.private);
        assert!(!cc.no_store);
        assert!(!cc.no_cache);
        assert!(cc.immutable);
        assert_eq!(cc.max_age, Some(31536000));

        let cc2 = CacheControlDirectives::parse("no-cache, no-store, must-revalidate");
        assert!(cc2.no_store);
        assert!(cc2.no_cache);
        assert!(cc2.must_revalidate);

        let cc3 = CacheControlDirectives::parse("private, s-maxage=\"7200\", max-age=3600");
        assert!(cc3.private);
        assert_eq!(cc3.max_age, Some(3600));
        assert_eq!(cc3.s_maxage, Some(7200));
    }

    #[test]
    fn test_http_date_parsing_and_formatting() {
        let date_str = "Sun, 06 Nov 1994 08:49:37 GMT";
        let parsed = parse_http_date(date_str).expect("Valid HTTP date");
        let formatted = fmt_http_date(parsed);
        assert_eq!(date_str, formatted);
    }

    #[test]
    fn test_freshness_and_age_calculation() {
        let now = 1_000_000;
        let mut meta = HttpCacheMeta {
            url: "https://example.com/bundle.js".to_string(),
            final_url: "https://example.com/bundle.js".to_string(),
            status: 200,
            headers: HashMap::new(),
            content_type: "application/javascript".to_string(),
            protocol: NegotiatedProtocol::H2,
            fetched_at: now - 100,
            response_date: Some(now - 100),
            etag: Some("\"xyz\"".to_string()),
            last_modified: None,
            max_age: Some(300),
            expires: None,
            immutable: false,
            must_revalidate: false,
            no_cache: false,
            body_len: 1024,
        };

        assert_eq!(meta.current_age(now), 100);
        assert_eq!(meta.freshness_lifetime(), 300);
        assert!(meta.is_fresh(now));

        assert!(!meta.is_fresh(now + 250));
        assert!(meta.can_revalidate());

        meta.immutable = true;
        assert!(meta.is_fresh(now));

        meta.no_cache = true;
        assert!(!meta.is_fresh(now));
    }

    #[test]
    fn test_heuristic_freshness_from_last_modified() {
        let now = 1_000_000;
        let last_modified_sec = now - 10_000;
        let lm_str = fmt_http_date(last_modified_sec);

        let meta = HttpCacheMeta {
            url: "https://example.com/image.png".to_string(),
            final_url: "https://example.com/image.png".to_string(),
            status: 200,
            headers: HashMap::new(),
            content_type: "image/png".to_string(),
            protocol: NegotiatedProtocol::Http11,
            fetched_at: now,
            response_date: Some(now),
            etag: None,
            last_modified: Some(lm_str),
            max_age: None,
            expires: None,
            immutable: false,
            must_revalidate: false,
            no_cache: false,
            body_len: 512,
        };

        assert_eq!(meta.freshness_lifetime(), 1000);
        assert!(meta.is_fresh(now + 500));
        assert!(!meta.is_fresh(now + 1500));
    }

    #[test]
    fn test_heuristic_freshness_for_svg_and_static_assets() {
        let now = 1_000_000;
        let svg_meta = HttpCacheMeta {
            url: "https://www.youtube.com/s/desktop/icons.svg".to_string(),
            final_url: "https://www.youtube.com/s/desktop/icons.svg".to_string(),
            status: 200,
            headers: HashMap::new(),
            content_type: "image/svg+xml".to_string(),
            protocol: NegotiatedProtocol::Http11,
            fetched_at: now,
            response_date: Some(now),
            etag: None,
            last_modified: None,
            max_age: None,
            expires: None,
            immutable: false,
            must_revalidate: false,
            no_cache: false,
            body_len: 2048,
        };

        assert_eq!(svg_meta.freshness_lifetime(), 86400);
        assert!(svg_meta.is_fresh(now + 3600));
        assert!(svg_meta.is_fresh(now + 86399));
        assert!(!svg_meta.is_fresh(now + 86401));
    }

    #[test]
    fn test_disk_cache_put_get_roundtrip() {
        let temp = tempfile::tempdir().expect("temp dir");
        let cache = HttpCache::open(temp.path()).expect("open cache");

        let mut headers = HashMap::new();
        headers.insert("cache-control".to_string(), "max-age=3600, public".to_string());
        headers.insert("etag".to_string(), "\"abc123\"".to_string());
        headers.insert("content-type".to_string(), "text/html; charset=utf-8".to_string());

        let body = b"<!doctype html><html><body>Cached Content</body></html>".to_vec();
        let final_url = RixUrl::parse("https://example.com/test.html").unwrap();

        let response = HttpResponse {
            status: 200,
            headers: headers.clone(),
            body: body.clone(),
            final_url: final_url.clone(),
            content_type: "text/html; charset=utf-8".to_string(),
            protocol: NegotiatedProtocol::H2,
            navigation_timing: NavigationTiming::default(),
            navigation_started: Instant::now(),
            set_cookies: Vec::new(),
        };

        let url_str = "https://example.com/test.html";
        cache.put(url_str, &response).expect("put in cache");

        let entry = cache.get(url_str).expect("cache hit");
        assert_eq!(entry.meta.url, url_str);
        assert_eq!(entry.meta.status, 200);
        assert_eq!(&entry.body[..], &body[..]);
        assert_eq!(entry.meta.etag.as_deref(), Some("\"abc123\""));
        assert_eq!(entry.meta.max_age, Some(3600));

        let now = now_secs();
        let http_res = cache.to_http_response(&entry, now);
        assert_eq!(http_res.status, 200);
        assert_eq!(http_res.body, body);
        assert_eq!(http_res.headers.get("content-length").unwrap(), &format!("{}", body.len()));
        assert!(http_res.headers.contains_key("age"));
    }

    #[test]
    fn test_no_store_is_never_cached() {
        let temp = tempfile::tempdir().expect("temp dir");
        let cache = HttpCache::open(temp.path()).expect("open cache");

        let mut headers = HashMap::new();
        headers.insert("cache-control".to_string(), "no-store".to_string());

        let response = HttpResponse {
            status: 200,
            headers,
            body: b"sensitive data".to_vec(),
            final_url: RixUrl::parse("https://example.com/secret").unwrap(),
            content_type: "application/json".to_string(),
            protocol: NegotiatedProtocol::H2,
            navigation_timing: NavigationTiming::default(),
            navigation_started: Instant::now(),
            set_cookies: Vec::new(),
        };

        cache.put("https://example.com/secret", &response).unwrap();
        assert!(cache.get("https://example.com/secret").is_none());
    }

    #[test]
    fn test_304_revalidation_updates_headers_and_preserves_body() {
        let temp = tempfile::tempdir().expect("temp dir");
        let cache = HttpCache::open(temp.path()).expect("open cache");

        let mut headers = HashMap::new();
        headers.insert("cache-control".to_string(), "max-age=60".to_string());
        headers.insert("etag".to_string(), "\"v1\"".to_string());
        headers.insert("date".to_string(), "Sun, 06 Nov 1994 08:49:37 GMT".to_string());

        let body = b"script v1 body bytes".to_vec();
        let response = HttpResponse {
            status: 200,
            headers,
            body: body.clone(),
            final_url: RixUrl::parse("https://example.com/script.js").unwrap(),
            content_type: "application/javascript".to_string(),
            protocol: NegotiatedProtocol::H2,
            navigation_timing: NavigationTiming::default(),
            navigation_started: Instant::now(),
            set_cookies: Vec::new(),
        };

        let url_str = "https://example.com/script.js";
        cache.put(url_str, &response).unwrap();

        let mut new_304_headers = HashMap::new();
        new_304_headers.insert("cache-control".to_string(), "max-age=3600".to_string());
        new_304_headers.insert("date".to_string(), "Sun, 06 Nov 1994 09:49:37 GMT".to_string());
        new_304_headers.insert("etag".to_string(), "\"v1\"".to_string());

        let now = now_secs();
        let updated = cache.update_revalidated(url_str, &new_304_headers, now).expect("updated entry");

        assert_eq!(&updated.body[..], &body[..]);
        
        assert_eq!(updated.meta.max_age, Some(3600));
        assert_eq!(updated.meta.fetched_at, now);
        assert_eq!(updated.meta.headers.get("date").unwrap(), "Sun, 06 Nov 1994 09:49:37 GMT");
    }
}
