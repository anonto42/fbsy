//! Setup use case.
//!
//! The wizard collects a valid `config.json`, backs up any existing config, and
//! writes the new file atomically.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use console::style;
use dialoguer::{Confirm, Input};

use crate::{
    config::{BridgeConfig, BridgeDeviceConfig},
    support::{network, paths::default_config_path},
};

/// Run the interactive setup wizard.
pub fn run() -> Result<()> {
    run_at(default_config_path())
}

/// Run setup for a specific config path. Kept separate for tests.
pub fn run_at(path: PathBuf) -> Result<()> {
    println!("{}", style("FingerBridge Setup Wizard").cyan().bold());
    println!("Config path: {}", style(path.display()).yellow());
    println!();

    if path.exists() {
        let reconfigure = Confirm::new()
            .with_prompt("Config already exists. Reconfigure?")
            .default(false)
            .interact()?;
        if !reconfigure {
            println!(
                "{}",
                style("Setup cancelled. Existing config was left unchanged.").yellow()
            );
            return Ok(());
        }
    }

    let cfg = collect_config()?;
    cfg.validate()?;
    backup_existing_config(&path)?;
    save_config_atomically(&path, &cfg)?;

    println!();
    println!("{}", style("Setup completed successfully!").green().bold());
    println!("{}", style("Next steps:").underlined().bold());
    println!(
        "  1. Validate config: {}",
        style("fbsy bridge config validate").cyan()
    );
    println!(
        "  2. Run diagnostics: {}",
        style("fbsy bridge doctor").cyan()
    );
    println!(
        "  3. Sync once:       {}",
        style("fbsy bridge sync --device <DEVICE_CODE>").cyan()
    );
    println!("  4. Start service:   {}", style("fbsy run bridge").cyan());
    Ok(())
}

fn collect_config() -> Result<BridgeConfig> {
    let lan_host = network::lan_host_or_loopback();
    println!("{}", style("Tip for mock testing:").yellow().bold());
    println!(
        "  If you started {} and {}, the defaults below should work.",
        style("fbsy run hrms").cyan(),
        style("fbsy run zkteco").cyan()
    );
    println!(
        "  A device unique code is an HRMS/bridge identifier you choose, not the ZKTeco serial."
    );
    println!();

    let vps_webhook_url: String = Input::new()
        .with_prompt("HRMS Webhook URL")
        .default(format!("http://{lan_host}:8800/webhook"))
        .validate_with(|input: &String| {
            if input.trim().starts_with("http://") || input.trim().starts_with("https://") {
                Ok(())
            } else {
                Err("Webhook URL must start with http:// or https://")
            }
        })
        .interact_text()?;

    let bridge_port: u16 = Input::new()
        .with_prompt("Bridge HTTP port")
        .default(7431)
        .interact_text()?;

    let enable_jobs = Confirm::new()
        .with_prompt("Enable HRMS job polling?")
        .default(false)
        .interact()?;

    let (hrms_base_url, hrms_api_token, job_poll_interval_seconds) = if enable_jobs {
        let base: String = Input::new()
            .with_prompt("HRMS Base URL")
            .default(format!("http://{lan_host}:8800/api/v1"))
            .validate_with(|input: &String| {
                if input.trim().starts_with("http://") || input.trim().starts_with("https://") {
                    Ok(())
                } else {
                    Err("HRMS Base URL must start with http:// or https://")
                }
            })
            .interact_text()?;

        let token: String = Input::new()
            .with_prompt("HRMS API token (blank = use first device apiKey)")
            .default("".to_string())
            .show_default(false)
            .interact_text()?;
        let token = if token.trim().is_empty() {
            None
        } else {
            Some(token.trim().to_string())
        };

        let interval: u64 = Input::new()
            .with_prompt("Job poll interval seconds")
            .default(30)
            .validate_with(|input: &u64| {
                if *input >= 5 {
                    Ok(())
                } else {
                    Err("Job poll interval must be at least 5 seconds")
                }
            })
            .interact_text()?;

        (Some(base.trim().to_string()), token, interval)
    } else {
        (None, None, 30)
    };

    let mut devices = Vec::new();
    loop {
        devices.push(collect_device(devices.len() + 1)?);
        let add_another = Confirm::new()
            .with_prompt("Add another device?")
            .default(false)
            .interact()?;
        if !add_another {
            break;
        }
    }

    Ok(BridgeConfig {
        vps_webhook_url: vps_webhook_url.trim().to_string(),
        bridge_port,
        hrms_base_url,
        hrms_api_token,
        job_poll_interval_seconds,
        auto_update: false,
        update_check_interval_hours: 6,
        devices,
    })
}

fn collect_device(number: usize) -> Result<BridgeDeviceConfig> {
    println!();
    println!(
        "{}",
        style(format!("--- Device {number} ---")).bold().cyan()
    );

    let lan_host = network::lan_host_or_loopback();
    let default_code = mock_device_code(number);
    let default_key = mock_api_key(number);

    let device_ip: String = Input::new()
        .with_prompt("Device IP")
        .default(lan_host)
        .validate_with(|input: &String| {
            if input.trim().is_empty() {
                Err("Device IP is required")
            } else {
                Ok(())
            }
        })
        .interact_text()?;

    let device_port: u16 = Input::new()
        .with_prompt("Device port")
        .default(4370)
        .interact_text()?;

    let device_password: i32 = Input::new()
        .with_prompt("Device connection password")
        .default(0)
        .interact_text()?;

    let device_timeout: u64 = Input::new()
        .with_prompt("Device connection timeout seconds")
        .default(15)
        .validate_with(|input: &u64| {
            if (1..=120).contains(input) {
                Ok(())
            } else {
                Err("Timeout must be between 1 and 120 seconds")
            }
        })
        .interact_text()?;

    let device_force_udp = Confirm::new()
        .with_prompt("Force UDP connection?")
        .default(false)
        .interact()?;

    let device_omit_ping = Confirm::new()
        .with_prompt("Omit ICMP ping check?")
        .default(true)
        .interact()?;

    let device_timezone: String = Input::new()
        .with_prompt("Device timezone offset (UTC or e.g. +06:00; the device's clock zone)")
        .default("UTC".to_string())
        .validate_with(|input: &String| {
            if crate::domain::parse_utc_offset(input).is_some() {
                Ok(())
            } else {
                Err("Must be UTC or a fixed offset like +06:00")
            }
        })
        .interact_text()?;
    let device_timezone = {
        let trimmed = device_timezone.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("utc") {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    let device_code: String = Input::new()
        .with_prompt("Device unique code (choose this; mock default is OK)")
        .default(default_code)
        .validate_with(|input: &String| {
            if input.trim().is_empty() {
                Err("Device code is required")
            } else {
                Ok(())
            }
        })
        .interact_text()?;

    let api_key: String = Input::new()
        .with_prompt("Device HRMS API key (mock default is OK)")
        .default(default_key)
        .validate_with(|input: &String| {
            if input.trim().is_empty() {
                Err("API key is required")
            } else {
                Ok(())
            }
        })
        .interact_text()?;

    let organization_id: u64 = Input::new()
        .with_prompt("Organization ID")
        .default(1)
        .interact_text()?;

    let sync_interval_seconds: u64 = Input::new()
        .with_prompt("Sync interval seconds")
        .default(300)
        .validate_with(|input: &u64| {
            if *input >= 5 {
                Ok(())
            } else {
                Err("Sync interval must be at least 5 seconds")
            }
        })
        .interact_text()?;

    let clear_attendance_after_sync = Confirm::new()
        .with_prompt("Clear attendance logs on device after successful sync?")
        .default(false)
        .interact()?;

    Ok(BridgeDeviceConfig {
        device_ip: device_ip.trim().to_string(),
        device_port,
        device_password,
        device_timeout,
        device_force_udp,
        device_omit_ping,
        device_timezone,
        device_code: device_code.trim().to_string(),
        api_key: api_key.trim().to_string(),
        organization_id,
        sync_interval_seconds,
        clear_attendance_after_sync,
    })
}

fn mock_device_code(number: usize) -> String {
    format!("MOCK-GATE-{number:02}")
}

fn mock_api_key(number: usize) -> String {
    if number == 1 {
        "mock-key".to_string()
    } else {
        format!("mock-key-{number:02}")
    }
}

fn backup_existing_config(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before unix epoch")?
        .as_secs();
    let backup_path = path.with_extension(format!("json.bak-{timestamp}"));
    fs::copy(path, &backup_path).with_context(|| {
        format!(
            "could not create config backup {}",
            backup_path.to_string_lossy()
        )
    })?;
    println!("Backup written: {}", backup_path.display());
    Ok(Some(backup_path))
}

fn save_config_atomically(path: &Path, cfg: &BridgeConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(cfg)?;
    fs::write(&tmp, format!("{body}\n"))?;
    fs::rename(&tmp, path)?;
    println!("Config written: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{backup_existing_config, mock_api_key, mock_device_code, save_config_atomically};

    #[test]
    fn atomic_save_writes_pretty_json() {
        let dir =
            std::env::temp_dir().join(format!("fingerbridge-setup-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("config.json");
        let cfg = crate::config::BridgeConfig::from_json_value(serde_json::json!({
            "vpsWebhookUrl": "https://example.test/webhook",
            "devices": [{
                "deviceIp": "127.0.0.1",
                "deviceCode": "GATE-01",
                "apiKey": "secret"
            }]
        }))
        .expect("config");

        save_config_atomically(&path, &cfg).expect("save config");
        let saved = fs::read_to_string(&path).expect("read config");

        assert!(saved.contains("\"devices\""));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_existing_config_creates_backup_file() {
        let dir =
            std::env::temp_dir().join(format!("fingerbridge-backup-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("config.json");
        fs::write(&path, "{}").expect("write config");

        let backup = backup_existing_config(&path)
            .expect("backup")
            .expect("backup path");

        assert!(backup.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mock_defaults_are_predictable_for_setup() {
        assert_eq!(mock_device_code(1), "MOCK-GATE-01");
        assert_eq!(mock_device_code(2), "MOCK-GATE-02");
        assert_eq!(mock_api_key(1), "mock-key");
        assert_eq!(mock_api_key(2), "mock-key-02");
    }
}
