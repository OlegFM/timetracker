use tt_storage::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
    #[error("not found")]
    NotFound,
    #[error("in use by {what}")]
    InUse { what: &'static str },
    #[error("overlaps another entry")]
    Overlap,
    #[error("invalid: {0}")]
    Invalid(&'static str),
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("update: {0}")]
    Update(String),
}

pub type Result<T> = std::result::Result<T, ServiceError>;
