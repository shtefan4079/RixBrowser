
#![deny(unsafe_code)]

pub mod cli;
pub mod config;
pub mod process_manager;

pub use cli::CliArgs;
pub use config::{AppConfig, DEFAULT_HEIGHT, DEFAULT_WIDTH};
pub use process_manager::AppProcessManager;

pub const RIX_BROWSER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version_string() -> String {
    format!("RixBrowser {RIX_BROWSER_VERSION}")
}
