
use crate::client::HttpClient;
use crate::error::{NetError, Result};
use rix_core::url::{RixScheme, RixUrl};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock, Semaphore};

pub const DEFAULT_MAX_CONCURRENCY: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourcePriority {
    Document = 0,
    Style = 1,
    Font = 2,
    Script = 3,
    Media = 4,
    Low = 5,
}

impl ResourcePriority {
    pub fn infer_from_url(url: &RixUrl) -> Self {
        let path = url.path().to_lowercase();
        if path.ends_with(".css") {
            Self::Style
        } else if path.ends_with(".woff2")
            || path.ends_with(".woff")
            || path.ends_with(".ttf")
            || path.ends_with(".otf")
        {
            Self::Font
        } else if path.ends_with(".js") || path.ends_with(".mjs") {
            Self::Script
        } else if path.ends_with(".png")
            || path.ends_with(".jpg")
            || path.ends_with(".jpeg")
            || path.ends_with(".gif")
            || path.ends_with(".webp")
            || path.ends_with(".avif")
            || path.ends_with(".svg")
            || path.ends_with(".mp4")
            || path.ends_with(".webm")
            || path.ends_with(".mp3")
        {
            Self::Media
        } else if path.ends_with(".html") || path.ends_with(".htm") || path == "/" || path.is_empty() {
            Self::Document
        } else {
            Self::Low
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resource {
    pub url: RixUrl,
    pub mime_type: String,
    pub data: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub from_cache: bool,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub data: Vec<u8>,
    pub mime_type: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub fetched_at: u64,
    pub max_age: Option<u64>,
}

impl CacheEntry {
    pub fn is_fresh(&self, now: u64) -> bool {
        match self.max_age {
            Some(max_age) => now.saturating_sub(self.fetched_at) < max_age,
            None => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiskMeta {
    url: String,
    mime_type: String,
    etag: Option<String>,
    last_modified: Option<String>,
    fetched_at: u64,
    max_age: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DiskCache {
    dir: PathBuf,
}

impl DiskCache {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| NetError::Cache(format!("Cannot create cache dir: {e}")))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn stem_for(url_str: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(url_str.as_bytes());
        hex_encode(hasher.finalize().as_slice())
    }

    fn paths_for(&self, url_str: &str) -> (PathBuf, PathBuf) {
        let stem = Self::stem_for(url_str);
        (
            self.dir.join(format!("{stem}.body")),
            self.dir.join(format!("{stem}.meta.json")),
        )
    }

    pub fn get(&self, url_str: &str) -> Option<CacheEntry> {
        let (body_path, meta_path) = self.paths_for(url_str);
        let meta_bytes = std::fs::read(meta_path).ok()?;
        let meta: DiskMeta = serde_json::from_slice(&meta_bytes).ok()?;
        if meta.url != url_str {
            return None;
        }
        let data = std::fs::read(body_path).ok()?;
        Some(CacheEntry {
            data,
            mime_type: meta.mime_type,
            etag: meta.etag,
            last_modified: meta.last_modified,
            fetched_at: meta.fetched_at,
            max_age: meta.max_age,
        })
    }

    pub fn put(
        &self,
        url_str: &str,
        data: &[u8],
        mime_type: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        max_age: Option<u64>,
    ) -> Result<()> {
        let (body_path, meta_path) = self.paths_for(url_str);
        let meta = DiskMeta {
            url: url_str.to_string(),
            mime_type: mime_type.to_string(),
            etag: etag.map(str::to_string),
            last_modified: last_modified.map(str::to_string),
            fetched_at: now_secs(),
            max_age,
        };
        let meta_bytes = serde_json::to_vec(&meta)
            .map_err(|e| NetError::Cache(format!("Cache meta encode failed: {e}")))?;

        let tmp_body = body_path.with_extension("body.tmp");
        let tmp_meta = meta_path.with_extension("json.tmp");
        std::fs::write(&tmp_body, data)
            .map_err(|e| NetError::Cache(format!("Cache body write failed: {e}")))?;
        std::fs::write(&tmp_meta, &meta_bytes)
            .map_err(|e| NetError::Cache(format!("Cache meta write failed: {e}")))?;
        std::fs::rename(&tmp_body, &body_path)
            .map_err(|e| NetError::Cache(format!("Cache body commit failed: {e}")))?;
        std::fs::rename(&tmp_meta, &meta_path)
            .map_err(|e| NetError::Cache(format!("Cache meta commit failed: {e}")))?;
        Ok(())
    }

    pub fn refresh(&self, url_str: &str) {
        if let Some(entry) = self.get(url_str) {
            let _ = self.put(
                url_str,
                &entry.data,
                &entry.mime_type,
                entry.etag.as_deref(),
                entry.last_modified.as_deref(),
                entry.max_age,
            );
        }
    }
}

#[derive(Clone)]
pub struct ResourceManager {
    client: HttpClient,
    mem: Arc<RwLock<HashMap<String, CacheEntry>>>,
    disk: Option<DiskCache>,
    max_concurrency: usize,
}

impl ResourceManager {
    pub fn new(client: HttpClient) -> Self {
        Self {
            client,
            mem: Arc::new(RwLock::new(HashMap::new())),
            disk: None,
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
        }
    }

    pub fn with_disk_cache(mut self, dir: impl Into<PathBuf>) -> Result<Self> {
        self.disk = Some(DiskCache::open(dir)?);
        Ok(self)
    }

    pub fn with_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max.max(1);
        self
    }

    pub async fn load(&self, url: &RixUrl) -> Result<Resource> {
        let priority = ResourcePriority::infer_from_url(url);
        self.load_prioritized(url, priority).await
    }

    pub async fn load_prioritized(
        &self,
        url: &RixUrl,
        _priority: ResourcePriority,
    ) -> Result<Resource> {
        match url.scheme() {
            RixScheme::Rix => self.load_internal_rix_resource(url).await,
            RixScheme::About => load_about_resource(url),
            RixScheme::Data => load_data_url(url),
            RixScheme::Http | RixScheme::Https => self.load_http(url).await,
            RixScheme::File => Err(NetError::Http(format!(
                "file:// URLs are not fetched by the network stack: {}",
                url.as_str()
            ))),
            RixScheme::Custom(s) => Err(NetError::Http(format!(
                "Unsupported URL scheme '{s}': {}",
                url.as_str()
            ))),
        }
    }

    async fn load_http(&self, url: &RixUrl) -> Result<Resource> {
        let url_str = url.as_str().to_string();
        let now = now_secs();

        let mem_hit = {
            let guard = self.mem.read().await;
            guard.get(&url_str).cloned()
        };
        if let Some(entry) = mem_hit {
            if entry.is_fresh(now) {
                return Ok(cached_resource(url, &entry, true));
            }
            if entry.etag.is_none() && entry.last_modified.is_none() {
                if now.saturating_sub(entry.fetched_at) < 60 {
                    return Ok(cached_resource(url, &entry, true));
                }
            } else if let Ok(resource) = self
                .revalidate(url, &url_str, &entry, None)
                .await
            {
                return Ok(resource);
            }
        }

        if let Some(disk) = &self.disk {
            if let Some(entry) = disk.get(&url_str) {
                if entry.is_fresh(now_secs()) {
                    self.remember_mem(&url_str, &entry).await;
                    return Ok(cached_resource(url, &entry, true));
                }
                if entry.etag.is_some() || entry.last_modified.is_some() {
                    if let Ok(resource) = self
                        .revalidate(url, &url_str, &entry, Some(disk))
                        .await
                    {
                        return Ok(resource);
                    }
                } else if now_secs().saturating_sub(entry.fetched_at) < 60 {
                    self.remember_mem(&url_str, &entry).await;
                    return Ok(cached_resource(url, &entry, true));
                }
            }
        }

        let response = self.client.fetch(url).await?;
        if response.status == 304 {
            return Ok(Resource {
                url: response.final_url,
                mime_type: response.content_type.clone(),
                data: Vec::new(),
                etag: response.headers.get("etag").cloned(),
                last_modified: response.headers.get("last-modified").cloned(),
                from_cache: false,
            });
        }

        let sniffed = sniff_mime_type_with_name(
            &response.body,
            &response.content_type,
            response.final_url.path(),
        );
        let etag = response.headers.get("etag").cloned();
        let last_modified = response.headers.get("last-modified").cloned();
        let max_age = parse_max_age(&response.headers);
        let no_store = is_no_store(&response.headers);

        let resource = Resource {
            url: response.final_url.clone(),
            mime_type: sniffed.clone(),
            data: response.body.clone(),
            etag: etag.clone(),
            last_modified: last_modified.clone(),
            from_cache: false,
        };

        if !no_store {
            let entry = CacheEntry {
                data: response.body.clone(),
                mime_type: sniffed.clone(),
                etag: etag.clone(),
                last_modified: last_modified.clone(),
                fetched_at: now_secs(),
                max_age,
            };
            self.remember_mem(&url_str, &entry).await;
            if let Some(disk) = &self.disk {
                let _ = disk.put(
                    &url_str,
                    &response.body,
                    &sniffed,
                    etag.as_deref(),
                    last_modified.as_deref(),
                    max_age,
                );
            }
        }

        Ok(resource)
    }

    async fn revalidate(
        &self,
        url: &RixUrl,
        url_str: &str,
        stale: &CacheEntry,
        disk: Option<&DiskCache>,
    ) -> Result<Resource> {
        let response = self
            .client
            .fetch_conditional(
                url,
                stale.etag.as_deref(),
                stale.last_modified.as_deref(),
            )
            .await?;

        if response.status == 304 {
            let refreshed = CacheEntry {
                fetched_at: now_secs(),
                ..stale.clone()
            };
            self.remember_mem(url_str, &refreshed).await;
            if let Some(disk) = disk.or(self.disk.as_ref()) {
                disk.refresh(url_str);
            }
            return Ok(cached_resource(url, &refreshed, true));
        }

        let sniffed = sniff_mime_type_with_name(
            &response.body,
            &response.content_type,
            response.final_url.path(),
        );
        let etag = response.headers.get("etag").cloned();
        let last_modified = response.headers.get("last-modified").cloned();
        let max_age = parse_max_age(&response.headers);
        let entry = CacheEntry {
            data: response.body.clone(),
            mime_type: sniffed.clone(),
            etag: etag.clone(),
            last_modified: last_modified.clone(),
            fetched_at: now_secs(),
            max_age,
        };
        if !is_no_store(&response.headers) {
            self.remember_mem(url_str, &entry).await;
            if let Some(disk) = disk.or(self.disk.as_ref()) {
                let _ = disk.put(
                    url_str,
                    &response.body,
                    &sniffed,
                    etag.as_deref(),
                    last_modified.as_deref(),
                    max_age,
                );
            }
        }
        Ok(Resource {
            url: response.final_url,
            mime_type: sniffed,
            data: response.body,
            etag,
            last_modified,
            from_cache: false,
        })
    }

    async fn remember_mem(&self, url_str: &str, entry: &CacheEntry) {
        let mut guard = self.mem.write().await;
        if guard.len() >= 1024 && !guard.contains_key(url_str) {
            if let Some(first) = guard.keys().next().cloned() {
                guard.remove(&first);
            }
        }
        guard.insert(url_str.to_string(), entry.clone());
    }

    async fn load_internal_rix_resource(&self, url: &RixUrl) -> Result<Resource> {
        let host = url.host_str().unwrap_or("").to_string();
        let raw_path = url.path().to_string();
        let clean = raw_path.trim_start_matches('/');

        if clean.contains("..") || raw_path.contains('\\') {
            return Err(NetError::NotFound(format!(
                "Rejected internal path with traversal: {raw_path}"
            )));
        }

        if host == "chrome" {
            let lookup = if clean.is_empty() { "browser.html" } else { clean };
            if let Some((content, mime)) = Self::get_embedded_chrome_asset(lookup) {
                return Ok(Resource {
                    url: url.clone(),
                    mime_type: mime.to_string(),
                    data: content.to_vec(),
                    etag: Some("embedded_asset_v1".to_string()),
                    last_modified: None,
                    from_cache: false,
                });
            }
            return Err(NetError::NotFound(format!(
                "Internal asset not found: rix://chrome/{clean}"
            )));
        }

        if url.is_privileged() {
            if let Some((content, mime)) = Self::get_embedded_chrome_asset("browser.html") {
                return Ok(Resource {
                    url: url.clone(),
                    mime_type: mime.to_string(),
                    data: content.to_vec(),
                    etag: Some("embedded_asset_v1".to_string()),
                    last_modified: None,
                    from_cache: false,
                });
            }
        }

        Err(NetError::NotFound(format!(
            "Internal URL not found or not privileged: {}",
            url.as_str()
        )))
    }

    pub fn get_embedded_chrome_asset(path: &str) -> Option<(&'static [u8], &'static str)> {
        let clean = path.trim_start_matches('/');
        let clean = clean.split(['?', '#']).next().unwrap_or(clean);
        match clean {
            "browser.html" | "" | "index.html" => Some((
                include_bytes!("../../rix-chrome/assets/browser.html"),
                "text/html; charset=utf-8",
            )),
            "browser.css" => Some((
                include_bytes!("../../rix-chrome/assets/browser.css"),
                "text/css; charset=utf-8",
            )),
            "browser.js" => Some((
                include_bytes!("../../rix-chrome/assets/browser.js"),
                "application/javascript; charset=utf-8",
            )),
            _ => None,
        }
    }

    pub async fn load_all(&self, urls: &[RixUrl]) -> Vec<Result<Resource>> {
        let prioritized: Vec<(RixUrl, ResourcePriority)> = urls
            .iter()
            .map(|u| (u.clone(), ResourcePriority::infer_from_url(u)))
            .collect();
        self.load_all_with_priorities(&prioritized).await
    }

    pub async fn load_all_with_priorities(
        &self,
        urls: &[(RixUrl, ResourcePriority)],
    ) -> Vec<Result<Resource>> {
        if urls.is_empty() {
            return Vec::new();
        }
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let mut order: Vec<usize> = (0..urls.len()).collect();
        order.sort_by_key(|&i| (urls[i].1, i));

        let mut handles = Vec::with_capacity(urls.len());
        for &i in &order {
            let manager = self.clone();
            let url = urls[i].0.clone();
            let priority = urls[i].1;
            let permit_sem = semaphore.clone();
            handles.push((i, tokio::spawn(async move {
                let _permit = permit_sem.acquire_owned().await.map_err(|e| {
                    NetError::Http(format!("Fetch semaphore closed: {e}"))
                })?;
                tracing::debug!("Fetching {:?} ({:?})", url.as_str(), priority);
                manager.load_prioritized(&url, priority).await
            })));
        }

        let mut out: Vec<Option<Result<Resource>>> = (0..urls.len()).map(|_| None).collect();
        for (i, handle) in handles {
            out[i] = Some(match handle.await {
                Ok(res) => res,
                Err(e) => Err(NetError::Http(format!("Task panicked: {e}"))),
            });
        }
        out.into_iter().map(|o| o.expect("slot filled above")).collect()
    }
}

fn cached_resource(url: &RixUrl, entry: &CacheEntry, from_cache: bool) -> Resource {
    Resource {
        url: url.clone(),
        mime_type: entry.mime_type.clone(),
        data: entry.data.clone(),
        etag: entry.etag.clone(),
        last_modified: entry.last_modified.clone(),
        from_cache,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn is_no_store(headers: &HashMap<String, String>) -> bool {
    headers
        .get("cache-control")
        .map(|v| v.to_lowercase().contains("no-store"))
        .unwrap_or(false)
}

fn parse_max_age(headers: &HashMap<String, String>) -> Option<u64> {
    let value = headers.get("cache-control")?;
    for directive in value.split(',') {
        let directive = directive.trim().to_lowercase();
        if let Some(n) = directive.strip_prefix("max-age=") {
            if let Ok(secs) = n.trim().parse::<u64>() {
                return Some(secs);
            }
        }
    }
    None
}

fn load_about_resource(url: &RixUrl) -> Result<Resource> {
    let raw = url.as_str();
    let after = raw.split_once("about:").map(|(_, rest)| rest).unwrap_or("");
    let path = after.split(['?', '#']).next().unwrap_or(after);
    match path.to_lowercase().as_str() {
        "blank" | "" => Ok(Resource {
            url: url.clone(),
            mime_type: "text/html; charset=utf-8".to_string(),
            data: b"<!DOCTYPE html><html><head><title>Blank</title></head><body></body></html>"
                .to_vec(),
            etag: None,
            last_modified: None,
            from_cache: false,
        }),
        _ => Err(NetError::NotFound(format!("Unsupported about: page: {raw}"))),
    }
}

fn load_data_url(url: &RixUrl) -> Result<Resource> {
    let raw = url.as_str();
    let payload = raw.split_once("data:").map(|(_, rest)| rest).ok_or_else(|| {
        NetError::Http(format!("Malformed data: URL (missing prefix): {raw}"))
    })?;
    let (meta, data_part) = payload.split_once(',').ok_or_else(|| {
        NetError::Http(format!("Malformed data: URL (missing comma): {raw}"))
    })?;

    let is_base64 = meta.split(';').any(|p| p.eq_ignore_ascii_case("base64"));
    let mime = meta
        .split(';')
        .next()
        .filter(|m| !m.is_empty() && m.contains('/'))
        .unwrap_or("text/plain;charset=US-ASCII");

    let data = if is_base64 {
        decode_base64(data_part.trim()).map_err(|e| {
            NetError::Http(format!("Invalid base64 in data: URL: {e}"))
        })?
    } else {
        percent_decode(data_part)
    };

    Ok(Resource {
        url: url.clone(),
        mime_type: mime.to_string(),
        data,
        etag: None,
        last_modified: None,
        from_cache: false,
    })
}

fn percent_decode(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_base64(input: &str) -> std::result::Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.is_empty() {
        return Ok(Vec::new());
    }
    if !clean.len().is_multiple_of(4) {
        return Err("length is not a multiple of 4".to_string());
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return Err("excessive padding".to_string());
        }
        let mut n: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            if c == b'=' {
                if i < 4 - pad {
                    return Err("misplaced padding".to_string());
                }
                n <<= 6;
            } else {
                let v = val(c).ok_or_else(|| format!("invalid character '{c}'"))?;
                n = (n << 6) | u32::from(v);
            }
        }
        out.push(((n >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((n & 0xff) as u8);
        }
    }
    Ok(out)
}

pub fn sniff_mime_type(data: &[u8], declared_type: &str) -> String {
    sniff_mime_type_with_name(data, declared_type, "")
}

pub fn sniff_mime_type_with_name(data: &[u8], declared_type: &str, path_hint: &str) -> String {
    let declared = declared_type.split(';').next().unwrap_or("").trim().to_lowercase();

    if let Some(binary) = sniff_binary_signature(data) {
        return binary.to_string();
    }

    if data.is_empty() {
        return canonical_declared(&declared, declared_type, path_hint);
    }

    if let Some(text_kind) = sniff_text_prolog(data) {
        return match text_kind {
            TextKind::Html => {
                if declared_is_generic(&declared) || declared == "text/plain" || declared.ends_with("+xml") {
                    "text/html; charset=utf-8".to_string()
                } else if declared == "text/html" {
                    declared_type.to_string()
                } else {
                    "text/html; charset=utf-8".to_string()
                }
            }
            TextKind::Svg => "image/svg+xml".to_string(),
            TextKind::Xml => {
                if declared_is_generic(&declared) {
                    "application/xml".to_string()
                } else {
                    canonical_declared(&declared, declared_type, path_hint)
                }
            }
        };
    }

    let head = &data[..data.len().min(1024)];
    if head.contains(&0) {
        return "application/octet-stream".to_string();
    }

    canonical_declared(&declared, declared_type, path_hint)
}

enum TextKind {
    Html,
    Svg,
    Xml,
}

fn declared_is_generic(declared: &str) -> bool {
    matches!(
        declared,
        "" | "application/octet-stream"
            | "application/unknown"
            | "unknown/unknown"
            | "text/plain"
            | "text-or-binary"
            | "binary/octet-stream"
    )
}

fn canonical_declared(declared: &str, full_declared: &str, path_hint: &str) -> String {
    if !declared.is_empty() && !declared_is_generic(declared) {
        return full_declared.trim().to_string();
    }
    if !path_hint.is_empty() {
        if let Some(guessed) = mime_guess::from_path(path_hint).first_raw() {
            if !declared_is_generic(guessed) || guessed.contains('/') {
                if !declared_is_generic(guessed) {
                    return guessed.to_string();
                }
            }
        }
    }
    if declared.is_empty() {
        return "application/octet-stream".to_string();
    }
    full_declared.trim().to_string()
}

fn sniff_binary_signature(data: &[u8]) -> Option<&'static str> {
    let len = data.len();
    if len >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("image/png");
    }
    if len >= 3 && &data[0..3] == b"\xff\xd8\xff" {
        return Some("image/jpeg");
    }
    if len >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if len >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if len >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return Some("audio/wav");
    }
    if len >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"AVI " {
        return Some("video/x-msvideo");
    }
    if len >= 2 && &data[0..2] == b"BM" {
        return Some("image/bmp");
    }
    if len >= 4 && &data[0..4] == b"\x00\x00\x01\x00" {
        return Some("image/x-icon");
    }
    if len >= 4
        && ((&data[0..4] == b"II*\x00") || (&data[0..4] == b"MM\x00*"))
    {
        return Some("image/tiff");
    }
    if len >= 4 && &data[0..4] == b"fLaC" {
        return Some("audio/flac");
    }
    if len >= 4 && &data[0..4] == b"OggS" {
        return Some("audio/ogg");
    }
    if len >= 3 && &data[0..3] == b"ID3" {
        return Some("audio/mpeg");
    }
    if len >= 2 && data[0] == 0xff && (data[1] & 0xe0) == 0xe0 {
        return Some("audio/mpeg");
    }
    if len >= 4 && &data[0..4] == b"\x1a\x45\xdf\xa3" {
        return Some("video/webm");
    }
    if len >= 5 && &data[0..5] == b"%PDF-" {
        return Some("application/pdf");
    }
    if len >= 4 && &data[0..4] == b"PK\x03\x04" {
        return Some("application/zip");
    }
    if len >= 2 && &data[0..2] == b"\x1f\x8b" {
        return Some("application/gzip");
    }
    if len >= 4 && &data[0..4] == b"Rar!" {
        return Some("application/vnd.rar");
    }
    if len >= 6 && &data[0..6] == b"7z\xbc\xaf\x27\x1c" {
        return Some("application/x-7z-compressed");
    }
    if len >= 6 && &data[0..6] == b"\xfd7zXZ\x00" {
        return Some("application/x-xz");
    }
    if len >= 4 && &data[0..4] == b"wOFF" {
        return Some("font/woff");
    }
    if len >= 4 && &data[0..4] == b"wOF2" {
        return Some("font/woff2");
    }
    if len >= 4 && (&data[0..4] == b"\x00\x01\x00\x00" || &data[0..4] == b"OTTO") {
        return Some("font/sfnt");
    }
    if len >= 12 && &data[4..8] == b"ftyp" {
        return Some(sniff_ftyp(&data[8..12.min(len)]));
    }
    None
}

fn sniff_ftyp(brand: &[u8]) -> &'static str {
    match brand {
        b"avif" | b"avis" => "image/avif",
        b"heic" | b"heix" | b"hevc" | b"hevx" => "image/heic",
        b"mif1" | b"msf1" => "image/heif",
        _ => "video/mp4",
    }
}

fn sniff_text_prolog(data: &[u8]) -> Option<TextKind> {
    let head_len = data.len().min(1024);
    let mut head = &data[..head_len];
    if head.len() >= 3 && &head[0..3] == b"\xef\xbb\xbf" {
        head = &head[3..];
    }
    let mut i = 0;
    while i < head.len() && matches!(head[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let text = std::str::from_utf8(&head[i..head.len().min(i + 512)]).ok()?;
    let lower = text.trim_start().to_lowercase();

    if lower.starts_with("<?xml") {
        if lower.contains("<svg") {
            return Some(TextKind::Svg);
        }
        return Some(TextKind::Xml);
    }
    if lower.starts_with("<svg") {
        return Some(TextKind::Svg);
    }
    const HTML_PREFIXES: &[&str] = &[
        "<!doctype html",
        "<html",
        "<head",
        "<body",
        "<frameset",
        "<script",
        "<iframe",
        "<h1",
        "<h2",
        "<h3",
        "<h4",
        "<h5",
        "<h6",
        "<div",
        "<font",
        "<table",
        "<a ",
        "<a>",
        "<style",
        "<title",
        "<b>",
        "<br",
        "<p",
        "<!--",
    ];
    for prefix in HTML_PREFIXES {
        if lower.starts_with(prefix) {
            return Some(TextKind::Html);
        }
    }
    None
}
