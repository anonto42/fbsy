//! Setup use case.
//!
//! The wizard collects a valid `config.json`, backs up any existing config, and
//! writes the new file atomically.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

use crate::{
    config::{BridgeConfig, BridgeDeviceConfig},
    support::paths::default_config_path,
};

/// Run the interactive setup wizard.
pub fn run() -> Result<()> {
    run_at(default_config_path())
}

/// Run setup for a specific config path. Kept separate for tests.
pub fn run_at(path: PathBuf) -> Result<()> {
    println!("ZKTeco Bridge setup");
    println!("Config path: {}", path.display());
    println!();

    if path.exists() && !ask_bool("Config already exists. Reconfigure?", false)? {
        println!("Setup cancelled. Existing config was left unchanged.");
        return Ok(());
    }

    let cfg = collect_config()?;
    cfg.validate()?;
    backup_existing_config(&path)?;
    save_config_atomically(&path, &cfg)?;

    println!();
    println!("Setup complete.");
    println!("Next:");
    println!("  zkteco-bridge config validate");
    println!("  zkteco-bridge doctor");
    println!("  zkteco-bridge once --device <DEVICE_CODE>");
    Ok(())
}

fn collect_config() -> Result<BridgeConfig> {
    let vps_webhook_url = ask_string("HRMS webhook URL", None)?;
    let bridge_port = ask_u16("Bridge HTTP port", 7431)?;

    let enable_jobs = ask_bool("Enable HRMS job polling?", false)?;
    let (hrms_base_url, hrms_api_token, job_poll_interval_seconds) = if enable_jobs {
        let base = ask_string("HRMS base URL", None)?;
        let token = ask_string("HRMS API token (blank = use first device apiKey)", Some(""))?;
        let token = if token.trim().is_empty() {
            None
        } else {
            Some(token)
        };
        let interval = ask_u64("Job poll interval seconds", 30)?;
        (Some(base), token, interval.max(5))
    } else {
        (None, None, 30)
    };

    let mut devices = Vec::new();
    loop {
        devices.push(collect_device(devices.len() + 1)?);
        if !ask_bool("Add another device?", false)? {
            break;
        }
    }

    Ok(BridgeConfig {
        vps_webhook_url,
        bridge_port,
        hrms_base_url,
        hrms_api_token,
        job_poll_interval_seconds,
        devices,
    })
}

fn collect_device(number: usize) -> Result<BridgeDeviceConfig> {
    println!();
    println!("Device {number}");
    Ok(BridgeDeviceConfig {
        device_ip: ask_string("Device IP", None)?,
        device_port: ask_u16("Device port", 4370)?,
        device_password: ask_i32("Device password", 0)?,
        device_timeout: ask_u64("Device timeout seconds", 15)?,
        device_force_udp: ask_bool("Force UDP?", false)?,
        device_omit_ping: ask_bool("Omit ping?", true)?,
        device_code: ask_string("Device code", None)?,
        api_key: ask_string("Device API key", None)?,
        organization_id: ask_u64("Organization ID", 1)?,
        sync_interval_seconds: ask_u64("Sync interval seconds", 300)?.max(5),
        clear_attendance_after_sync: ask_bool("Clear attendance after successful sync?", false)?,
    })
}

fn ask_string(label: &str, default: Option<&str>) -> Result<String> {
    loop {
        let answer = prompt(label, default)?;
        if !answer.trim().is_empty() || default.is_some() {
            return Ok(answer.trim().to_string());
        }
        println!("{label} is required.");
    }
}

fn ask_u16(label: &str, default: u16) -> Result<u16> {
    loop {
        let answer = prompt(label, Some(&default.to_string()))?;
        match answer.trim().parse::<u16>() {
            Ok(value) if value >= 1 => return Ok(value),
            _ => println!("{label} must be a number from 1 to 65535."),
        }
    }
}

fn ask_u64(label: &str, default: u64) -> Result<u64> {
    loop {
        let answer = prompt(label, Some(&default.to_string()))?;
        match answer.trim().parse::<u64>() {
            Ok(value) => return Ok(value),
            _ => println!("{label} must be a positive number."),
        }
    }
}

fn ask_i32(label: &str, default: i32) -> Result<i32> {
    loop {
        let answer = prompt(label, Some(&default.to_string()))?;
        match answer.trim().parse::<i32>() {
            Ok(value) => return Ok(value),
            _ => println!("{label} must be a number."),
        }
    }
}

fn ask_bool(label: &str, default: bool) -> Result<bool> {
    let default_text = if default { "y" } else { "n" };
    loop {
        let answer = prompt(&format!("{label} [y/n]"), Some(default_text))?;
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" | "true" | "1" => return Ok(true),
            "n" | "no" | "false" | "0" => return Ok(false),
            _ => println!("Please answer y or n."),
        }
    }
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(default) => print!("{label} [{default}]: "),
        None => print!("{label}: "),
    }
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim_end();
    if trimmed.is_empty() {
        Ok(default.unwrap_or_default().to_string())
    } else {
        Ok(trimmed.to_string())
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

    use super::{backup_existing_config, save_config_atomically};

    #[test]
    fn atomic_save_writes_pretty_json() {
        let dir =
            std::env::temp_dir().join(format!("zkteco-bridge-setup-test-{}", std::process::id()));
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
            std::env::temp_dir().join(format!("zkteco-bridge-backup-test-{}", std::process::id()));
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
}
