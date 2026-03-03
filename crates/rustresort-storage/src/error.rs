use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Storage error: {0}")]
    Storage(String),
}

impl StorageError {
    pub fn storage(error: impl std::fmt::Display) -> Self {
        Self::Storage(error.to_string())
    }

    pub fn database(error: impl std::fmt::Display) -> Self {
        Self::Database(error.to_string())
    }
}

pub type StorageResult<T> = std::result::Result<T, StorageError>;
