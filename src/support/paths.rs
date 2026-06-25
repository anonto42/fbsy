//! Filesystem path helpers.
//!
//! The current migration keeps folder-local `config.json` behavior so existing
//! support/install flows remain familiar.

use std::path::PathBuf;

/// Best-effort directory where the executable lives.
pub fn app_base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Default config path used by CLI commands.
pub fn default_config_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| app_base_dir())
        .join("config.json")
}

/// Default log directory used by diagnostics and future file logging.
pub fn default_log_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| app_base_dir())
        .join("logs")
}
