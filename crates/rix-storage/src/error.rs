
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(#[from] redb::DatabaseError),

    #[error("Database transaction error: {0}")]
    Transaction(#[from] redb::TransactionError),

    #[error("Database table error: {0}")]
    Table(#[from] redb::TableError),

    #[error("Database commit error: {0}")]
    Commit(#[from] redb::CommitError),

    #[error("Database storage error: {0}")]
    Storage(#[from] redb::StorageError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protected cookie storage is unavailable or damaged")]
    CookieProtection,

    #[error("Entity not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;
