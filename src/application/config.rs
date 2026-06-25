//! Config-related use cases.
//!
//! These functions are called by CLI commands and later can be reused by setup
//! or HTTP diagnostics.

use std::path::PathBuf;

use anyhow::Result;
use console::style;

use crate::{
    adapters::config_file::JsonConfigStore, ports::config_store::ConfigStore,
    support::paths::default_config_path,
};

/// Validate the active config file.
pub fn validate(path: Option<PathBuf>) -> Result<()> {
    let path = path.unwrap_or_else(default_config_path);
    let store = JsonConfigStore;
    // Loading through the store also validates the config.
    let _cfg = store.load(&path)?;
    println!("{} Config is valid: {}", style("✔").green().bold(), style(path.display()).yellow());
    Ok(())
}

/// Show a redacted view of config for support/debugging.
pub fn show(path: Option<PathBuf>) -> Result<()> {
    let path = path.unwrap_or_else(default_config_path);
    let store = JsonConfigStore;
    let cfg = store.load(&path)?;
    println!("{}", serde_json::to_string_pretty(&cfg.redacted())?);
    Ok(())
}

/// Print the default config path.
pub fn path() -> Result<()> {
    println!("{}", default_config_path().display());
    Ok(())
}

/// Print configured devices without exposing secrets.
pub fn devices_list(path: Option<PathBuf>) -> Result<()> {
    let path = path.unwrap_or_else(default_config_path);
    let store = JsonConfigStore;
    let cfg = store.load(&path)?;
    let devices = cfg
        .redacted()
        .devices
        .into_iter()
        .map(|device| {
            serde_json::json!({
                "deviceCode": device.device_code,
                "deviceIp": device.device_ip,
                "devicePort": device.device_port,
                "organizationId": device.organization_id,
                "syncIntervalSeconds": device.sync_interval_seconds
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&devices)?);
    Ok(())
}
