
#![deny(unsafe_code)]

use thiserror::Error;

pub type MediaResult<T> = std::result::Result<T, MediaError>;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Unexpected end of media stream while parsing {0}")]
    UnexpectedEof(&'static str),

    #[error("Invalid container header in {0}: {1}")]
    InvalidHeader(&'static str, String),

    #[error("Corrupt or invalid box structure in ISO-BMFF: {0}")]
    InvalidBox(String),

    #[error("Corrupt or invalid EBML element in WebM/Matroska: {0}")]
    InvalidEbml(String),

    #[error("Unsupported media container: {0}")]
    UnsupportedContainer(String),

    #[error("Unsupported codec: {0}")]
    UnsupportedCodec(String),

    #[error("Codec decoding error in {0}: {1}")]
    DecodeError(&'static str, String),

    #[error("Track not found: {0}")]
    TrackNotFound(u32),

    #[error("Buffer allocation or quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("Pipeline backpressure: queue full ({0})")]
    Backpressure(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),
}
