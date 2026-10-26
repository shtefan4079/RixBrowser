
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RixError {
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Unsupported URL scheme: {0}")]
    UnsupportedScheme(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("DOM error: {0}")]
    Dom(String),

    #[error("Layout error: {0}")]
    Layout(String),

    #[error("Render error: {0}")]
    Render(String),

    #[error("JavaScript runtime error: {0}")]
    JavaScript(String),

    #[error("IPC communication error: {0}")]
    Ipc(String),

    #[error("Internal browser error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, RixError>;
