
#![deny(unsafe_code)]

pub mod db;
mod cookie_crypto;
pub mod secrets;
pub mod error;
pub mod models;

pub use db::StorageEngine;
pub use secrets::SecretStoreError;
pub use error::{Result, StorageError};
pub use models::{Bookmark, Cookie, HistoryEntry, SameSitePolicy, SessionState, TabSession};
