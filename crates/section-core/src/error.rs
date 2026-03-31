use thiserror::Error;

#[derive(Error, Debug)]
pub enum SectionError {
    #[error("account not found: {0}")]
    AccountNotFound(String),

    #[error("source not found: {0}")]
    SourceNotFound(String),

    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("opendal error: {0}")]
    OpenDal(#[from] opendal::Error),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl SectionError {
    /// Wrap an `opendal::Error` with a user-friendly message when the path context is known.
    /// Converts NotFound into `FileNotFound`, PermissionDenied into `PermissionDenied`,
    /// and passes other errors through as `OpenDal`.
    pub fn from_opendal(err: opendal::Error, path: &str) -> Self {
        match err.kind() {
            opendal::ErrorKind::NotFound => SectionError::FileNotFound(path.to_string()),
            opendal::ErrorKind::PermissionDenied => SectionError::PermissionDenied(path.to_string()),
            _ => SectionError::OpenDal(err),
        }
    }
}

pub type Result<T> = std::result::Result<T, SectionError>;
