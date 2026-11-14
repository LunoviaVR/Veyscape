//! Typed application errors. Commands return `AppResult<T>`; errors serialize to a
//! plain message string for the UI. Never put URLs with credentials or user data in messages.

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("file system error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("not supported on this platform: {0}")]
    Unsupported(&'static str),
    #[error("network error: {0}")]
    Network(String),
    /// The user declined or could not be verified (e.g. Windows Hello cancelled).
    #[error("{0}")]
    Denied(String),
    #[error("browser engine error: {0}")]
    Engine(String),
    #[error("{0}")]
    Tauri(#[from] tauri::Error),
}

#[cfg(windows)]
impl From<windows::core::Error> for AppError {
    fn from(e: windows::core::Error) -> Self {
        AppError::Engine(e.message())
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
