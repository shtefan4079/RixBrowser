
use rix_core::url::RixUrl;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroize;

pub fn current_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub visited_at: u64,
    pub visit_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SameSitePolicy {
    None,
    #[default]
    Lax,
    Strict,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires_at: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSitePolicy,
}

impl Drop for Cookie {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Cookie {
    pub fn is_expired(&self, now: u64) -> bool {
        match self.expires_at {
            Some(exp) => exp <= now,
            None => false,
        }
    }

    pub fn matches_request(&self, url: &RixUrl, is_secure: bool, now: u64) -> bool {
        if self.is_expired(now) {
            return false;
        }

        if self.secure && !is_secure {
            return false;
        }

        let host = match url.host_str() {
            Some(h) => h,
            None => return false,
        };

        let domain_match = if self.domain.starts_with('.') {
            let suffix = &self.domain[1..];
            host.eq_ignore_ascii_case(suffix)
                || (host.len() > self.domain.len()
                    && host.to_ascii_lowercase().ends_with(&self.domain.to_ascii_lowercase()))
        } else {
            host.eq_ignore_ascii_case(&self.domain)
        };

        if !domain_match {
            return false;
        }

        let request_path = url.path();
        request_path == self.path
            || (request_path.starts_with(&self.path)
                && (self.path.ends_with('/')
                    || request_path.as_bytes().get(self.path.len()) == Some(&b'/')))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub title: String,
    pub url: Option<String>,
    pub is_folder: bool,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabSession {
    pub url: String,
    pub title: String,
    pub scroll_y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub active_tab_index: usize,
    pub tabs: Vec<TabSession>,
}
