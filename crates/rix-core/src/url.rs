
use crate::error::{Result, RixError};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RixScheme {
    Http,
    Https,
    File,
    Rix,
    About,
    Data,
    Custom(String),
}

impl fmt::Display for RixScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http => write!(f, "http"),
            Self::Https => write!(f, "https"),
            Self::File => write!(f, "file"),
            Self::Rix => write!(f, "rix"),
            Self::About => write!(f, "about"),
            Self::Data => write!(f, "data"),
            Self::Custom(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RixUrl {
    inner: Url,
}

impl RixUrl {
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        let url = Url::parse(trimmed).map_err(|e| RixError::InvalidUrl(e.to_string()))?;
        Ok(Self { inner: url })
    }

    pub fn join(&self, input: &str) -> Result<Self> {
        let u = self.inner.join(input).map_err(|e| RixError::InvalidUrl(e.to_string()))?;
        Ok(Self { inner: u })
    }

    pub fn from_user_input(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(RixError::InvalidUrl("Empty URL input".into()));
        }

        if trimmed.starts_with("rix://") || trimmed.starts_with("about:") {
            return Self::parse(trimmed);
        }

        if trimmed.starts_with("rix:") {
            let fixed = format!("rix://{}", trimmed.trim_start_matches("rix:").trim_start_matches('/'));
            return Self::parse(&fixed);
        }

        if trimmed.contains("://") {
            return Self::parse(trimmed);
        }

        if trimmed.starts_with('/') || (cfg!(windows) && trimmed.len() > 2 && trimmed.as_bytes()[1] == b':') {
            let path_url = format!("file://{trimmed}");
            return Self::parse(&path_url);
        }

        let https_url = format!("https://{trimmed}");
        Self::parse(&https_url)
    }

    pub fn scheme(&self) -> RixScheme {
        match self.inner.scheme() {
            "http" => RixScheme::Http,
            "https" => RixScheme::Https,
            "file" => RixScheme::File,
            "rix" => RixScheme::Rix,
            "about" => RixScheme::About,
            "data" => RixScheme::Data,
            other => RixScheme::Custom(other.to_string()),
        }
    }

    pub fn is_privileged(&self) -> bool {
        if self.scheme() != RixScheme::Rix {
            return false;
        }
        let host = self.inner.host_str().unwrap_or("");
        matches!(host, "chrome" | "settings" | "history" | "bookmarks" | "devtools")
    }

    pub fn is_internal(&self) -> bool {
        matches!(
            self.scheme(),
            RixScheme::Rix | RixScheme::About
        )
    }

    pub fn as_str(&self) -> &str {
        self.inner.as_str()
    }

    pub fn inner(&self) -> &Url {
        &self.inner
    }

    pub fn host_str(&self) -> Option<&str> {
        self.inner.host_str()
    }

    pub fn origin(&self) -> String {
        self.inner.origin().ascii_serialization()
    }

    pub fn path(&self) -> &str {
        self.inner.path()
    }

    pub fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    pub fn query(&self) -> Option<&str> {
        self.inner.query()
    }

    pub fn default_new_tab() -> Self {
        Self::parse("rix://chrome/browser.html").expect("Default new tab URL is guaranteed valid")
    }
}

impl fmt::Display for RixUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl FromStr for RixUrl {
    type Err = RixError;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl AsRef<str> for RixUrl {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
