//! Doctor and troubleshooting use cases.

use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Result};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;

use crate::{
    adapters::{
        config_file::JsonConfigStore, device_zkteco_tcp::ZktecoTcpConnector,
        hrms_reqwest::ReqwestHrmsClient,
    },
    config::{BridgeConfig, BridgeDeviceConfig},
    ports::{config_store::ConfigStore, device::DeviceConnector, hrms::HrmsClient},
    support::paths::{default_config_path, log_dir},
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
    println!("{}", log_dir().display());
    Ok(())
}

/// Test one configured device connection.
pub fn device_test(config: Option<PathBuf>, code: &str) -> Result<()> {
    let cfg = load_config(config.unwrap_or_else(default_config_path))?;
    let device = find_device(&cfg, code)?;

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.green} {msg}")?,
    );
    pb.set_message(format!(
        "Connecting to device {} ({}:{})...",
        style(code).cyan().bold(),
        device.device_ip,
        device.device_port
    ));
    pb.enable_steady_tick(Duration::from_millis(100));

    let res = ZktecoTcpConnector.connect(device);
    pb.finish_and_clear();

    match res {
        Ok(mut client) => {
            client.disconnect();
            println!(
                "{} Device connection OK: {}",
                style("✔").green().bold(),
                style(code).cyan().bold()
            );
            Ok(())
        }
        Err(err) => {
            bail!(
                "{} Device connection failed for {}: {}",
                style("✘").red().bold(),
                style(code).cyan().bold(),
                style(err).red()
            )
        }
    }
}

/// Connect to one device and print its live identity + storage data.
pub fn device_info(config: Option<PathBuf>, code: &str, with_users: bool) -> Result<()> {
    let cfg = load_config(config.unwrap_or_else(default_config_path))?;
    let device = find_device(&cfg, code)?;

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.green} {msg}")?,
    );
    pb.set_message(format!(
        "Reading data from {} ({}:{})...",
        style(code).cyan().bold(),
        device.device_ip,
        device.device_port
    ));
    pb.enable_steady_tick(Duration::from_millis(100));

    let result = (|| {
        let mut client = ZktecoTcpConnector.connect(device)?;
        let info = client.device_info()?;
        let users = if with_users {
            client.get_users().unwrap_or_default()
        } else {
            Vec::new()
        };
        client.disconnect();
        Ok::<_, crate::ports::device::DeviceError>((info, users))
    })();
    pb.finish_and_clear();

    let (info, users) = match result {
        Ok(v) => v,
        Err(err) => bail!(
            "{} Could not read device {}: {}",
            style("✘").red().bold(),
            style(code).cyan().bold(),
            style(err).red()
        ),
    };

    let field = |label: &str, value: &str| {
        let shown = if value.is_empty() { "-" } else { value };
        println!("  {:<12} {}", format!("{label}:"), shown);
    };
    println!("Device: {}", style(code).cyan().bold());
    field(
        "IP",
        &format!("{}:{}", device.device_ip, device.device_port),
    );
    field("Serial", &info.serial);
    field("Firmware", &info.firmware);
    field("Platform", &info.platform);
    field("Name", &info.name);
    println!(
        "  {:<12} {} users · {} fingerprints · {} attendance records",
        "Storage:",
        style(info.users).yellow(),
        style(info.fingers).yellow(),
        style(info.records).yellow()
    );

    if with_users {
        println!("\n  Users ({}):", users.len());
        for u in &users {
            println!("    uid {:<6} id {:<10} {}", u.uid, u.user_id, u.name);
        }
    }
    Ok(())
}

/// Send an empty webhook event list for one configured device.
pub fn webhook_test(config: Option<PathBuf>, code: &str) -> Result<()> {
    let cfg = load_config(config.unwrap_or_else(default_config_path))?;
    let device = find_device(&cfg, code)?;
    let client = ReqwestHrmsClient::default();

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.green} {msg}")?,
    );
    pb.set_message(format!(
        "Sending webhook test request for {} to {}...",
        style(code).cyan().bold(),
        cfg.vps_webhook_url
    ));
    pb.enable_steady_tick(Duration::from_millis(100));

    let res = client.forward_events(&cfg.vps_webhook_url, device, &[]);
    pb.finish_and_clear();

    match res {
        Ok(_) => {
            println!(
                "{} Webhook test OK: {}",
                style("✔").green().bold(),
                style(code).cyan().bold()
            );
            Ok(())
        }
        Err(err) => {
            bail!(
                "{} Webhook test failed for {}: {}",
                style("✘").red().bold(),
                style(code).cyan().bold(),
                style(err).red()
            )
        }
    }
}

fn build_report(config_path: &Path, deep: bool) -> DoctorReport {
    let executable_path = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|err| format!("unknown: {err}"));
    let log_dir = log_dir();
    let config_exists = config_path.exists();

    let mut report = DoctorReport {
        agent: "fingerbridge".to_string(),
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
        let code = &device.device_code;
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
                .template("{spinner:.green} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        pb.set_message(format!("Checking connection to device {code}..."));
        pb.enable_steady_tick(Duration::from_millis(100));

        report.device_connection = Some(match ZktecoTcpConnector.connect(device) {
            Ok(mut client) => {
                client.disconnect();
                CheckResult::ok()
            }
            Err(err) => CheckResult::fail(err.to_string()),
        });

        pb.set_message(format!("Checking HRMS webhook connection for {code}..."));

        let client = ReqwestHrmsClient::default();
        report.webhook = Some(
            match client.forward_events(&cfg.vps_webhook_url, device, &[]) {
                Ok(_) => CheckResult::ok(),
                Err(err) => CheckResult::fail(err.to_string()),
            },
        );

        pb.finish_and_clear();
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
        return vec!["fbsy setup".to_string()];
    }
    if !report.config_valid {
        return vec!["fix config.json and run fbsy config validate".to_string()];
    }
    vec![
        "fbsy doctor --deep".to_string(),
        "fbsy once --device <DEVICE_CODE>".to_string(),
        "fbsy serve".to_string(),
    ]
}

fn print_report(report: &DoctorReport) {
    println!("{}", style("FingerBridge Diagnostics").cyan().bold());
    println!();
    println!("Agent:      {}", report.agent);
    println!("Version:    {}", report.version);
    println!("Runtime:    {}", report.runtime);
    println!("Executable: {}", report.executable_path);
    println!("Config:     {}", status_styled(report.config_exists));
    println!("Config path: {}", report.config_path);
    println!("Config valid: {}", status_styled(report.config_valid));
    if let Some(error) = &report.config_error {
        println!("Config error: {}", style(error).red().bold());
    }
    println!("Logs:       {}", report.log_dir);
    println!("Log dir:    {}", status_styled(report.log_dir_exists));
    if let Some(available) = report.bridge_port_available {
        println!(
            "HTTP port:  {}",
            if available {
                style("available").green().bold()
            } else {
                style("busy").red().bold()
            }
        );
    }
    println!("Devices:    {}", report.device_count);
    println!(
        "Job polling: {}",
        enabled_styled(report.job_polling_enabled)
    );
    println!("Service:    {}", style(&report.service_status).yellow());

    for device in &report.devices {
        println!();
        println!("Device {}", style(&device.device_code).cyan().bold());
        println!("  IP:       {}:{}", device.device_ip, device.device_port);
        println!("  Interval: {}s", device.sync_interval_seconds);
        println!(
            "  Clear after sync: {}",
            enabled_styled(device.clear_attendance_after_sync)
        );
        if let Some(check) = &device.device_connection {
            println!("  Device connection: {}", check.display_styled());
        }
        if let Some(check) = &device.webhook {
            println!("  Webhook:           {}", check.display_styled());
        }
    }

    println!();
    println!("{}", style("Next:").underlined().bold());
    for step in &report.next_steps {
        println!("  {}", style(step).yellow());
    }
}

fn status_styled(value: bool) -> String {
    if value {
        style("ok").green().bold().to_string()
    } else {
        style("missing").red().bold().to_string()
    }
}

fn enabled_styled(value: bool) -> String {
    if value {
        style("enabled").green().to_string()
    } else {
        style("disabled").yellow().to_string()
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

    fn display_styled(&self) -> String {
        if self.ok {
            style("ok").green().bold().to_string()
        } else {
            format!(
                "{} {}",
                style("failed:").red().bold(),
                style(&self.message).red()
            )
        }
    }
}
