//! Error results that can be returned from the engine
use thiserror::Error;

/// Errors returned by the config crate
#[derive(Debug, Error)]
pub enum Error {
    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    IO(#[from] std::io::Error),

    #[error("json parsing error: {0}")]
    JsonSerde(#[from] serde_json::Error),

    #[cfg(not(target_arch = "wasm32"))]
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("there was a problem: {0}")]
    Generic(String),
}
