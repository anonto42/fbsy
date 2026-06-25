//! File-backed config adapter.
//!
//! This is the concrete implementation of `ConfigStore` for `config.json`.

use std::{fs, path::Path};

use crate::{
    config::{BridgeConfig, ConfigError},
    ports::config_store::ConfigStore,
};

#[derive(Debug, Default)]
/// JSON implementation of the config storage port.
pub struct JsonConfigStore;

impl ConfigStore for JsonConfigStore {
    /// Read JSON from disk, deserialize it, then run domain validation.
    fn load(&self, path: &Path) -> Result<BridgeConfig, ConfigError> {
        if !path.exists() {
            return Err(ConfigError::NotFound(path.display().to_string()));
        }
        // Keep I/O and JSON parsing here so application code stays clean.
        let raw = fs::read_to_string(path)?;
        let value = serde_json::from_str(&raw)?;
        BridgeConfig::from_json_value(value)
    }
}
