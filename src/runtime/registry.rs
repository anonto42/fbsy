//! Service registry.
//!
//! Each running service writes one JSON file under `run/{service}.json`
//! recording its pid, port, and start time. `show`/`close`/`status` read these
//! files; liveness is verified separately via [`crate::runtime::process`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{services::ServiceKind, support::paths};

/// One service instance's recorded runtime state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    /// Instance name (registry/log key; default = the kind's name).
    pub service: String,
    /// Service kind that decides which loop runs (`ServiceKind::name`).
    /// Defaulted for back-compat with single-instance files written earlier.
    #[serde(default)]
    pub kind: String,
    /// OS process id of the detached service.
    pub pid: u32,
    /// Port the service listens on, if known.
    pub port: Option<u16>,
    /// Human-facing address/URL where the service can be reached.
    #[serde(default)]
    pub url: Option<String>,
    /// Flags the service was started with (for inspection / restart).
    pub args: Vec<String>,
    /// RFC 3339 UTC timestamp when the service was started.
    pub started_at: String,
    /// Executable path used to spawn (guards against pid reuse).
    pub exe: String,
}

impl RegistryEntry {
    /// The service kind for this instance. Falls back to parsing the instance
    /// name when `kind` is absent (older single-instance registry files).
    pub fn kind(&self) -> Option<ServiceKind> {
        ServiceKind::from_name(&self.kind).or_else(|| ServiceKind::from_name(&self.service))
    }
}

/// Write (replace) the registry entry for a service atomically.
pub fn write(entry: &RegistryEntry) -> Result<()> {
    paths::ensure_dirs()?;
    let path = paths::service_registry_path(&entry.service);
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(entry)?;
    std::fs::write(&tmp, format!("{body}\n"))
        .with_context(|| format!("write registry {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("rename registry {}", path.display()))?;
    Ok(())
}

/// Read the registry entry for a service, if present and parseable.
pub fn read(service: &str) -> Result<Option<RegistryEntry>> {
    let path = paths::service_registry_path(service);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read registry {}", path.display()))?;
    match serde_json::from_str::<RegistryEntry>(&raw) {
        Ok(entry) => Ok(Some(entry)),
        // A corrupt registry file should not be fatal — treat as "no entry".
        Err(_) => Ok(None),
    }
}

/// Remove the registry file for a service (idempotent).
pub fn clear(service: &str) -> Result<()> {
    let path = paths::service_registry_path(service);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("remove registry {}", path.display()))?;
    }
    Ok(())
}

/// All present registry entries — every `*.json` instance file in the run dir.
pub fn list() -> Result<Vec<RegistryEntry>> {
    let dir = paths::run_dir();
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(&dir) else {
        return Ok(entries);
    };
    for item in read_dir.flatten() {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(entry) = serde_json::from_str::<RegistryEntry>(&raw) {
                entries.push(entry);
            }
        }
    }
    // Stable order: by kind, then instance name.
    entries.sort_by(|a, b| a.service.cmp(&b.service));
    entries.sort_by_key(|e| e.kind().map(|k| k as u8).unwrap_or(255));
    Ok(entries)
}
