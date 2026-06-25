//! Config storage port.
//!
//! Today the config is loaded from `config.json`; later this trait can support
//! other sources without changing application use cases.

use std::path::Path;

use crate::config::{BridgeConfig, ConfigError};

/// Abstraction for loading bridge configuration.
pub trait ConfigStore {
    /// Load and validate config from the given path.
    fn load(&self, path: &Path) -> Result<BridgeConfig, ConfigError>;
}
