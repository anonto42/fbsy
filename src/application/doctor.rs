//! Doctor and troubleshooting use cases.

use std::{
    net::TcpListener,
    path::{Path, PathBuf},
};

use anyhow::{bail, Result};
use serde::Serialize;

use crate::{
    adapters::{
        config_file::JsonConfigStore, device_zkteco_tcp::ZktecoTcpConnector,
        hrms_reqwest::ReqwestHrmsClient,
    },
    config::{BridgeConfig, BridgeDeviceConfig},
    ports::{config_store::ConfigStore, device::DeviceConnector, hrms::HrmsClient},
    support::paths::{default_config_path, default_log_dir},
};

/// Print local readiness and suggested next commands.
pub fn run(config: Option<PathBuf>, json: bool, deep: bool) -> Result<()> {
    let config_path = config.unwrap_or_else(default_config_path);
    let report = build_report(&config_path, deep);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report);
    }
    Ok(())
}

/// Print the default log directory.
pub fn logs_path() -> Result<()> {
    println!("{}", default_log_dir().display());
    Ok(())
}

/// Test one configured device connection.
pub fn device_test(config: Option<PathBuf>, code: &str) -> Result<()> {
    let cfg = load_config(config.unwrap_or_else(default_config_path))?;
    let device = find_device(&cfg, code)?;
    match ZktecoTcpConnector.connect(device) {
        Ok(mut client) => {
            client.disconnect();
            println!("Device connection OK: {code}");
            Ok(())
        }
        Err(err) => bail!("Device connection failed for {code}: {err}"),
    }
}

/// Send an empty webhook event list for one configured device.
pub fn webhook_test(config: Option<PathBuf>, code: &str) -> Result<()> {
    let cfg = load_config(config.unwrap_or_else(default_config_path))?;
    let device = find_device(&cfg, code)?;
    let client = ReqwestHrmsClient::default();
    client.forward_events(&cfg.vps_webhook_url, device, &[])?;
    println!("Webhook test OK: {code}");
    Ok(())
}

fn build_report(config_path: &Path, deep: bool) -> DoctorReport {
    let executable_path = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|err| format!("unknown: {err}"));
    let log_dir = default_log_dir();
    let config_exists = config_path.exists();

    let mut report = DoctorReport {
        agent: "zkteco-bridge".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        runtime: "rust".to_string(),
        executable_path,
        config_path: config_path.display().to_string(),
        config_exists,
        config_valid: false,
        config_error: None,
        log_dir: log_dir.display().to_string(),
        log_dir_exists: log_dir.exists(),
        bridge_port_available: None,
        device_count: 0,
        job_polling_enabled: false,
        service_status: "not implemented".to_string(),
        devices: Vec::new(),
        next_steps: Vec::new(),
    };

    match load_config(config_path.to_path_buf()) {
        Ok(cfg) => {
            report.config_valid = true;
            report.device_count = cfg.devices.len();
            report.job_polling_enabled = cfg.hrms_base_url.is_some();
            report.bridge_port_available = Some(port_available(cfg.bridge_port));
            report.devices = cfg
                .devices
                .iter()
                .map(|device| build_device_report(&cfg, device, deep))
                .collect();
        }
        Err(err) => {
            report.config_error = Some(err.to_string());
        }
    }

    report.next_steps = next_steps(&report);
    report
}

fn build_device_report(
    cfg: &BridgeConfig,
    device: &BridgeDeviceConfig,
    deep: bool,
) -> DeviceDoctorReport {
    let mut report = DeviceDoctorReport {
        device_code: device.device_code.clone(),
        device_ip: device.device_ip.clone(),
        device_port: device.device_port,
        sync_interval_seconds: device.sync_interval_seconds,
        clear_attendance_after_sync: device.clear_attendance_after_sync,
        device_connection: None,
        webhook: None,
    };

    if deep {
        report.device_connection = Some(match ZktecoTcpConnector.connect(device) {
            Ok(mut client) => {
                client.disconnect();
                CheckResult::ok()
            }
            Err(err) => CheckResult::fail(err.to_string()),
        });

        let client = ReqwestHrmsClient::default();
        report.webhook = Some(
            match client.forward_events(&cfg.vps_webhook_url, device, &[]) {
                Ok(_) => CheckResult::ok(),
                Err(err) => CheckResult::fail(err.to_string()),
            },
        );
    }

    report
}

fn load_config(path: PathBuf) -> Result<BridgeConfig> {
    let store = JsonConfigStore;
    Ok(store.load(&path)?)
}

fn find_device<'a>(cfg: &'a BridgeConfig, code: &str) -> Result<&'a BridgeDeviceConfig> {
    cfg.devices
        .iter()
        .find(|device| device.device_code == code)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "device '{code}' not found. Available: {}",
                cfg.devices
                    .iter()
                    .map(|device| device.device_code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn next_steps(report: &DoctorReport) -> Vec<String> {
    if !report.config_exists {
        return vec!["zkteco-bridge setup".to_string()];
    }
    if !report.config_valid {
        return vec!["fix config.json and run zkteco-bridge config validate".to_string()];
    }
    vec![
        "zkteco-bridge doctor --deep".to_string(),
        "zkteco-bridge once --device <DEVICE_CODE>".to_string(),
        "zkteco-bridge serve".to_string(),
    ]
}

fn print_report(report: &DoctorReport) {
    println!("ZKTeco Bridge Rust");
    println!();
    println!("Agent:      {}", report.agent);
    println!("Version:    {}", report.version);
    println!("Runtime:    {}", report.runtime);
    println!("Executable: {}", report.executable_path);
    println!("Config:     {}", status(report.config_exists));
    println!("Config path: {}", report.config_path);
    println!("Config valid: {}", status(report.config_valid));
    if let Some(error) = &report.config_error {
        println!("Config error: {error}");
    }
    println!("Logs:       {}", report.log_dir);
    println!("Log dir:    {}", status(report.log_dir_exists));
    if let Some(available) = report.bridge_port_available {
        println!(
            "HTTP port:  {}",
            if available { "available" } else { "busy" }
        );
    }
    println!("Devices:    {}", report.device_count);
    println!("Job polling: {}", enabled(report.job_polling_enabled));
    println!("Service:    {}", report.service_status);

    for device in &report.devices {
        println!();
        println!("Device {}", device.device_code);
        println!("  IP:       {}:{}", device.device_ip, device.device_port);
        println!("  Interval: {}s", device.sync_interval_seconds);
        println!(
            "  Clear after sync: {}",
            enabled(device.clear_attendance_after_sync)
        );
        if let Some(check) = &device.device_connection {
            println!("  Device connection: {}", check.display());
        }
        if let Some(check) = &device.webhook {
            println!("  Webhook: {}", check.display());
        }
    }

    println!();
    println!("Next:");
    for step in &report.next_steps {
        println!("  {step}");
    }
}

fn status(value: bool) -> &'static str {
    if value {
        "ok"
    } else {
        "missing"
    }
}

fn enabled(value: bool) -> &'static str {
    if value {
        "enabled"
    } else {
        "disabled"
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorReport {
    agent: String,
    version: String,
    runtime: String,
    executable_path: String,
    config_path: String,
    config_exists: bool,
    config_valid: bool,
    config_error: Option<String>,
    log_dir: String,
    log_dir_exists: bool,
    bridge_port_available: Option<bool>,
    device_count: usize,
    job_polling_enabled: bool,
    service_status: String,
    devices: Vec<DeviceDoctorReport>,
    next_steps: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceDoctorReport {
    device_code: String,
    device_ip: String,
    device_port: u16,
    sync_interval_seconds: u64,
    clear_attendance_after_sync: bool,
    device_connection: Option<CheckResult>,
    webhook: Option<CheckResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckResult {
    ok: bool,
    message: String,
}

impl CheckResult {
    fn ok() -> Self {
        Self {
            ok: true,
            message: "ok".to_string(),
        }
    }

    fn fail(message: String) -> Self {
        Self { ok: false, message }
    }

    fn display(&self) -> String {
        if self.ok {
            "ok".to_string()
        } else {
            format!("failed: {}", self.message)
        }
    }
}
