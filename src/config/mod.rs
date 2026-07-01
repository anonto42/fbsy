//! Configuration model and validation.
//!
//! This module knows what a valid bridge config looks like. Reading from disk is
//! handled by an adapter so validation can be reused by setup, tests, and future
//! config sources.

mod error;
mod impls;
mod model;

pub use error::ConfigError;
pub use model::{
    BridgeConfig, BridgeDeviceConfig, BridgeMode, RedactedBridgeConfig, RedactedBridgeDeviceConfig,
    RedactedSenseFaceConfig, RedactedSenseFaceDeviceConfig, SenseFaceConfig, SenseFaceDeviceConfig,
};
