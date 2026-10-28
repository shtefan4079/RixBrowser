
#![deny(unsafe_code)]

use rix_chrome::customization::{TabOrientation, ThemeMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_CONTROL_PORT: u16 = 9222;
pub const DEFAULT_WIDTH: f32 = 1280.0;
pub const DEFAULT_HEIGHT: f32 = 900.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub window_width: f32,
    pub window_height: f32,
    pub control_port: u16,
    pub headless: bool,
    #[serde(default = "default_theme")]
    pub theme: AppTheme,
    #[serde(default = "default_orientation")]
    pub tab_orientation: AppTabOrientation,
    #[serde(default = "default_homepage")]
    pub homepage: String,
    #[serde(default = "default_search_engine_template")]
    pub search_engine_template: String,
}

fn default_theme() -> AppTheme {
    AppTheme::Dark
}

fn default_orientation() -> AppTabOrientation {
    AppTabOrientation::Horizontal
}

fn default_homepage() -> String {
    "rix://newtab".to_string()
}

fn default_search_engine_template() -> String {
    "https://www.google.com/search?q={query}".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AppTheme {
    #[default]
    Dark,
    Light,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AppTabOrientation {
    #[default]
    Horizontal,
    Vertical,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            window_width: DEFAULT_WIDTH,
            window_height: DEFAULT_HEIGHT,
            control_port: DEFAULT_CONTROL_PORT,
            headless: false,
            theme: AppTheme::Dark,
            tab_orientation: AppTabOrientation::Horizontal,
            homepage: default_homepage(),
            search_engine_template: default_search_engine_template(),
        }
    }
}

impl AppConfig {
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".config")
            .join("rixbrowser")
            .join("config.toml")
    }

    pub fn data_dir() -> PathBuf {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("rixbrowser");
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("rixbrowser")
    }

    pub fn load_or_default(path: &std::path::Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("read config {path:?}: {e}"))?;
        toml::from_str(&content).map_err(|e| format!("parse config {path:?}: {e}"))
    }

    pub fn save_to_path(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create config dir {parent:?}: {e}"))?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| format!("serialize config: {e}"))?;
        std::fs::write(path, content).map_err(|e| format!("write config {path:?}: {e}"))?;
        Ok(())
    }

    pub fn theme_mode(&self) -> ThemeMode {
        match self.theme {
            AppTheme::Dark => ThemeMode::Dark,
            AppTheme::Light => ThemeMode::Light,
            AppTheme::Custom => ThemeMode::Custom,
        }
    }

    pub fn tab_orientation(&self) -> TabOrientation {
        match self.tab_orientation {
            AppTabOrientation::Horizontal => TabOrientation::Horizontal,
            AppTabOrientation::Vertical => TabOrientation::Vertical,
        }
    }

    pub fn validate(&mut self) {
        self.window_width = self.window_width.clamp(640.0, 7680.0);
        self.window_height = self.window_height.clamp(480.0, 4320.0);
        if self.control_port == 0 {
            self.control_port = DEFAULT_CONTROL_PORT;
        }
        if self.homepage.trim().is_empty() {
            self.homepage = default_homepage();
        }
        let candidate = self.search_engine_template.replace("{query}", "test");
        let valid_search_template = self.search_engine_template.contains("{query}")
            && rix_core::url::RixUrl::parse(&candidate)
                .map(|url| {
                    matches!(
                        url.scheme(),
                        rix_core::url::RixScheme::Http | rix_core::url::RixScheme::Https
                    )
                })
                .unwrap_or(false);
        if !valid_search_template {
            self.search_engine_template = default_search_engine_template();
        }
    }
}
