
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("DNS resolution error: {0}")]
    Dns(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("QUIC error: {0}")]
    Quic(String),

    #[error("HTTP/3 error: {0}")]
    H3(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Hyper error: {0}")]
    Hyper(#[from] hyper::Error),

    #[error("URL error: {0}")]
    Url(#[from] rix_core::error::RixError),

    #[error("Too many redirects: limit {0} reached")]
    TooManyRedirects(usize),

    #[error("Decompression error: {0}")]
    Decompression(String),

    #[error("Header error: {0}")]
    Header(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Network request timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("Response exceeds the {0} byte limit")]
    ResponseTooLarge(usize),
}

pub type Result<T> = std::result::Result<T, NetError>;
