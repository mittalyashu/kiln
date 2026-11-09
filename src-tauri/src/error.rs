use serde::ser::{Serialize, Serializer};

/// Errors surfaced to the frontend. Tauri serializes command errors, so the
/// display text is what the UI shows to the user.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid settings: {0}")]
    InvalidSettings(String),

    #[error("{0}")]
    InvalidRepository(String),

    #[error("git error: {0}")]
    Git(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("tauri error: {0}")]
    Tauri(#[from] tauri::Error),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
