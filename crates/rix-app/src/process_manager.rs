
#![deny(unsafe_code)]

use crate::config::AppConfig;
use rix_chrome::customization::{CustomizationConfig, CustomizationManager};
use rix_chrome::shell::BrowserShell;
use rix_ipc::process::{BrowserProcessManager, TabStatus};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Shell error: {0}")]
    Shell(String),
    #[error("IPC error: {0}")]
    Ipc(String),
    #[error("Configuration error: {0}")]
    Config(String),
}

pub struct AppProcessManager {
    pub config: AppConfig,
    pub tabs: BrowserProcessManager,
    pub watchdog_timeout: Duration,
}

impl AppProcessManager {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            tabs: BrowserProcessManager::new(),
            watchdog_timeout: Duration::from_millis(5000),
        }
    }

    pub fn boot_shell(&self) -> Result<BrowserShell, AppError> {
        let mut shell = BrowserShell::new(self.config.window_width, self.config.window_height)
            .map_err(|e| AppError::Shell(format!("{e}")))?;

        shell.customization = CustomizationManager::new(CustomizationConfig {
            tab_orientation: self.config.tab_orientation(),
            theme_mode: self.config.theme_mode(),
            custom_css_path: CustomizationConfig::default().custom_css_path,
        });
        shell
            .recompute_layout()
            .map_err(|e| AppError::Shell(format!("{e}")))?;

        Ok(shell)
    }

    pub async fn spawn_renderer(&mut self, url: &str) -> Result<u32, AppError> {
        let tab_id = self
            .tabs
            .spawn_tab(
                url,
                self.config.window_width as u32,
                self.config.window_height as u32,
            )
            .await
            .map_err(|e| AppError::Ipc(format!("{e}")))?;
        self.tabs
            .mark_tab_running(tab_id, url.to_string(), format!("Loading {url}..."));
        Ok(tab_id)
    }

    pub fn watchdog_sweep(&mut self) -> Vec<u32> {
        self.tabs.check_watchdog(self.watchdog_timeout)
    }

    pub async fn recover_tab(&mut self, tab_id: u32) -> Result<(), AppError> {
        self.tabs
            .reload_crashed_tab(
                tab_id,
                self.config.window_width as u32,
                self.config.window_height as u32,
            )
            .await
            .map(|_| ())
            .map_err(|e| AppError::Ipc(format!("{e}")))?;
        Ok(())
    }

    pub fn is_tab_alive(&self, tab_id: u32) -> bool {
        matches!(
            self.tabs.tab_status(tab_id),
            Some(TabStatus::Running { .. }) | Some(TabStatus::Initializing)
        )
    }

    pub fn isolate_crash(&mut self, tab_id: u32, reason: impl Into<String>) {
        self.tabs.mark_tab_crashed(tab_id, reason.into());
    }
}
