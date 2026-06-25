//! One-shot sync use case.
//!
//! This is where the main safety invariant is exercised: never clear device
//! attendance unless the HRMS upload succeeded.

use std::{path::PathBuf, sync::Arc};

use anyhow::{bail, Result};

use crate::{
    adapters::{
        config_file::JsonConfigStore, device_zkteco_tcp::ZktecoTcpConnector,
        hrms_reqwest::ReqwestHrmsClient,
    },
    config::BridgeDeviceConfig,
    domain::SyncResult,
    ports::config_store::ConfigStore,
    runtime::DeviceSyncState,
    support::paths::default_config_path,
};

/// Load config and run one sync attempt for all devices or one selected device.
pub fn run(config: Option<PathBuf>, device: Option<String>) -> Result<()> {
    let path = config.unwrap_or_else(default_config_path);
    let store = JsonConfigStore;
    let cfg = store.load(&path)?;
    let targets = select_targets(&cfg.devices, device.as_deref())?;

    let connector = Arc::new(ZktecoTcpConnector);
    let hrms = Arc::new(ReqwestHrmsClient::default());
    let results = targets
        .into_iter()
        .map(|device| {
            let state = DeviceSyncState::new(
                device.clone(),
                cfg.vps_webhook_url.clone(),
                connector.clone(),
                hrms.clone(),
            );
            state.sync_once()
        })
        .collect::<Vec<_>>();

    print_results(&results)?;
    if results.iter().all(|result| result.ok) {
        Ok(())
    } else {
        bail!("one or more device syncs failed")
    }
}

fn select_targets(
    devices: &[BridgeDeviceConfig],
    selected: Option<&str>,
) -> Result<Vec<BridgeDeviceConfig>> {
    match selected {
        Some(code) => {
            let matches = devices
                .iter()
                .filter(|device| device.device_code == code)
                .cloned()
                .collect::<Vec<_>>();
            if matches.is_empty() {
                let available = devices
                    .iter()
                    .map(|device| device.device_code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("device '{code}' not found in config. Available: {available}");
            }
            Ok(matches)
        }
        None => Ok(devices.to_vec()),
    }
}

fn print_results(results: &[SyncResult]) -> Result<()> {
    if results.len() == 1 {
        println!("{}", serde_json::to_string_pretty(&results[0])?);
    } else {
        println!("{}", serde_json::to_string_pretty(results)?);
    }
    Ok(())
}
