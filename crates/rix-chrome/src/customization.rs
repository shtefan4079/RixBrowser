
#![deny(unsafe_code)]

use rix_css::stylesheet::StyleSheet;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TabOrientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomizationSettings {
    pub tab_orientation: TabOrientation,
    pub theme_mode: ThemeMode,
}

impl Default for CustomizationSettings {
    fn default() -> Self {
        Self {
            tab_orientation: TabOrientation::Horizontal,
            theme_mode: ThemeMode::Dark,
        }
    }
}

pub struct CustomizationConfig {
    pub tab_orientation: TabOrientation,
    pub theme_mode: ThemeMode,
    pub custom_css_path: PathBuf,
}

impl Default for CustomizationConfig {
    fn default() -> Self {
        Self {
            tab_orientation: TabOrientation::Horizontal,
            theme_mode: ThemeMode::Dark,
            custom_css_path: Self::default_user_css_path(),
        }
    }
}

impl CustomizationConfig {
    pub fn default_user_css_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".config")
            .join("rixbrowser")
            .join("userCustom.css")
    }

    pub fn default_settings_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".config")
            .join("rixbrowser")
            .join("customization.toml")
    }
}

pub struct CustomizationManager {
    config: CustomizationConfig,
}

impl CustomizationManager {
    pub fn new(config: CustomizationConfig) -> Self {
        Self { config }
    }

    pub fn load_or_default() -> Self {
        let settings_path = CustomizationConfig::default_settings_path();
        if let Ok(content) = fs::read_to_string(&settings_path) {
            if let Ok(settings) = toml::from_str::<CustomizationSettings>(&content) {
                return Self::new(CustomizationConfig {
                    tab_orientation: settings.tab_orientation,
                    theme_mode: settings.theme_mode,
                    custom_css_path: CustomizationConfig::default_user_css_path(),
                });
            }
        }
        Self::new(CustomizationConfig::default())
    }

    pub fn config(&self) -> &CustomizationConfig {
        &self.config
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.config.theme_mode
    }

    pub fn tab_orientation(&self) -> TabOrientation {
        self.config.tab_orientation
    }

    pub fn set_tab_orientation(&mut self, orientation: TabOrientation) {
        self.config.tab_orientation = orientation;
    }

    pub fn set_theme_mode(&mut self, theme: ThemeMode) {
        self.config.theme_mode = theme;
    }

    pub fn set_custom_css_path(&mut self, path: PathBuf) {
        self.config.custom_css_path = path;
    }

    pub fn save_settings(&self) -> Result<(), String> {
        let path = CustomizationConfig::default_settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
        }
        let settings = CustomizationSettings {
            tab_orientation: self.config.tab_orientation,
            theme_mode: self.config.theme_mode,
        };
        let content =
            toml::to_string_pretty(&settings).map_err(|e| format!("serialize settings: {e}"))?;
        fs::write(&path, content).map_err(|e| format!("write {path:?}: {e}"))?;
        Ok(())
    }

    pub fn ensure_default_user_css() -> Result<PathBuf, String> {
        let path = CustomizationConfig::default_user_css_path();
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
            }
            fs::write(&path, Self::default_user_css_template())
                .map_err(|e| format!("write {path:?}: {e}"))?;
        }
        Ok(path)
    }

    pub fn default_user_css_template() -> &'static str {
        r#""#
    }

    pub fn read_user_css(&self) -> Option<String> {
        if self.config.custom_css_path.exists() {
            fs::read_to_string(&self.config.custom_css_path).ok()
        } else {
            None
        }
    }

    pub fn generate_override_css(&self) -> String {
        self.read_user_css()
            .filter(|css| !css.trim().is_empty())
            .unwrap_or_default()
    }

    pub fn load_override_stylesheet(&self) -> StyleSheet {
        let css = self.generate_override_css();
        (*StyleSheet::parse(&css)).clone()
    }
}
