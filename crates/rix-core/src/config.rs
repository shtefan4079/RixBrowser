
use crate::error::{Result, RixError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RixConfig {
    pub window: WindowConfig,
    pub network: NetworkConfig,
    pub security: SecurityConfig,
    pub storage: StorageConfig,
}

impl Default for RixConfig {
    fn default() -> Self {
        Self {
            window: WindowConfig::default(),
            network: NetworkConfig::default(),
            security: SecurityConfig::default(),
            storage: StorageConfig::default(),
        }
    }
}

impl RixConfig {
    pub fn from_toml_str(toml_str: &str) -> Result<Self> {
        toml::from_str(toml_str).map_err(|e| RixError::Config(e.to_string()))
    }

    pub fn to_toml_string(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| RixError::Config(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path).map_err(RixError::Io)?;
        Self::from_toml_str(&content)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(RixError::Io)?;
        }
        let content = self.to_toml_string()?;
        fs::write(path, content).map_err(RixError::Io)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub min_width: u32,
    pub min_height: u32,
    pub title: String,
    pub vsync: bool,
    pub decorations: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            min_width: 640,
            min_height: 480,
            title: "RixBrowser".to_string(),
            vsync: true,
            decorations: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub user_agent: String,
    pub max_connections_per_host: usize,
    pub timeout_seconds: u64,
    pub enable_http3: bool,
    pub enable_cache: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            user_agent: crate::BrowserProfile::system_default()
                .user_agent()
                .to_owned(),
            max_connections_per_host: 6,
            timeout_seconds: 30,
            enable_http3: true,
            enable_cache: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub block_third_party_cookies: bool,
    pub enforce_https: bool,
    pub enable_javascript: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            block_third_party_cookies: true,
            enforce_https: false,
            enable_javascript: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageConfig {
    pub data_dir: PathBuf,
    pub max_history_entries: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        let base_dir = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let mut home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                home.push(".local");
                home.push("share");
                home
            });

        Self {
            data_dir: base_dir.join("rixbrowser"),
            max_history_entries: 50_000,
        }
    }
}
