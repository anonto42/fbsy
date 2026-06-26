//! Filesystem path helpers.
//!
//! Data now lives in a per-OS application directory instead of the current
//! working directory:
//!   - Linux:   ~/.config/fbsy
//!   - Windows: %APPDATA%\fbsy
//!   - macOS:   ~/Library/Application Support/fbsy
//!
//! Under that base: `config/` (config.json), `logs/` (per-instance log files),
//! and `run/` (per-instance registry + pid JSON).
//!
//! Migration: earlier versions kept `config.json` in the working directory.
//! [`migrate_legacy_config`] copies a CWD config into the new location on first
//! `install` or `bridge run` so existing setups keep working.

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

/// Per-OS application base directory (the `fbsy` root).
pub fn base_dir() -> PathBuf {
    if let Some(dirs) = ProjectDirs::from("", "", "fbsy") {
        return dirs.config_dir().to_path_buf();
    }
    // Fallback: $HOME/.config/fbsy, else CWD/.fbsy.
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("fbsy");
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".fbsy")
}

/// Directory holding `config.json` and future profile-specific configs.
pub fn config_dir() -> PathBuf {
    base_dir().join("config")
}

/// Directory holding per-instance log files.
pub fn log_dir() -> PathBuf {
    base_dir().join("logs")
}

/// Directory holding per-instance registry / pid files.
pub fn run_dir() -> PathBuf {
    base_dir().join("run")
}

/// Directory holding self-update working files (download + backup).
pub fn update_dir() -> PathBuf {
    base_dir().join("update")
}

/// Default config path used by CLI commands.
pub fn default_config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// Per-instance log file, e.g. `logs/bridge.log` or `logs/dev1.log`.
pub fn service_log_path(service: &str) -> PathBuf {
    log_dir().join(format!("{service}.log"))
}

/// Per-instance registry/pid file, e.g. `run/bridge.json` or `run/dev1.json`.
pub fn service_registry_path(service: &str) -> PathBuf {
    run_dir().join(format!("{service}.json"))
}

/// Create base/config/log/run directories (idempotent).
pub fn ensure_dirs() -> Result<()> {
    for dir in [config_dir(), log_dir(), run_dir()] {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create directory {}", dir.display()))?;
    }
    Ok(())
}

/// Legacy config location: `./config.json` in the working directory.
pub fn legacy_config_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("config.json")
}

/// If the new config is missing but a legacy CWD config exists, copy it over.
pub fn migrate_legacy_config() -> Result<()> {
    let new = default_config_path();
    if new.exists() {
        return Ok(());
    }
    let legacy = legacy_config_path();
    if legacy.exists() && legacy != new {
        ensure_dirs()?;
        std::fs::copy(&legacy, &new)
            .with_context(|| format!("migrate config {} -> {}", legacy.display(), new.display()))?;
    }
    Ok(())
}

/// Best-effort directory where the executable lives.
pub fn app_base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}
