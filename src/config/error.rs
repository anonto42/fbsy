//! Config error types.
//!
//! This file answers: "What can go wrong while loading or validating config?"

use thiserror::Error;

#[derive(Debug, Error)]
/// Errors returned while loading or validating configuration.
pub enum ConfigError {
    /// The expected config file does not exist.
    #[error("config file not found at {0}")]
    NotFound(String),
    /// The file exists but does not contain valid JSON.
    #[error("config file is invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// The JSON is valid, but one or more values are not acceptable.
    #[error("config value is invalid: {0}")]
    Invalid(String),
    /// Reading the config file failed at the filesystem level.
    #[error("could not read config file: {0}")]
    Io(#[from] std::io::Error),
}
