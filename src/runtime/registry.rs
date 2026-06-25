//! Service registry.
//!
//! Each running service writes one JSON file under `run/{service}.json`
//! recording its pid, port, and start time. `show`/`close`/`status` read these
//! files; liveness is verified separately via [`crate::runtime::process`].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{services::ServiceKind, support::paths};

/// One service's recorded runtime state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    /// Service name (`ServiceKind::name`).
    pub service: String,
    /// OS process id of the detached service.
    pub pid: u32,
    /// Port the service listens on, if known.
    pub port: Option<u16>,
    /// Flags the service was started with (for inspection / restart).
    pub args: Vec<String>,
    /// RFC 3339 UTC timestamp when the service was started.
    pub started_at: String,
    /// Executable path used to spawn (guards against pid reuse).
    pub exe: String,
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

/// All present registry entries, ordered by [`ServiceKind::all`].
pub fn list() -> Result<Vec<RegistryEntry>> {
    let mut entries = Vec::new();
    for kind in ServiceKind::all() {
        if let Some(entry) = read(kind.name())? {
            entries.push(entry);
        }
    }
    Ok(entries)
}
