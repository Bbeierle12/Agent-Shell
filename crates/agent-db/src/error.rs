use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("CRC mismatch: expected {expected:#010x}, got {actual:#010x}")]
    CrcMismatch { expected: u32, actual: u32 },

    #[error("Corrupt entry: {0}")]
    Corrupt(String),

    #[error("Serialization error: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("Collection '{collection}' key {id} not found")]
    NotFound { collection: String, id: u64 },
}

impl DbError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, DbError>;
