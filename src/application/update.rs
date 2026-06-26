//! Safe self-update.
//!
//! Checks GitHub for a newer release, then performs an atomic, reversible swap:
//! download → verify checksum → smoke-test → back up the current binary →
//! replace → restart the services that were running → health-check → roll back
//! automatically if anything fails.
//!
//! No data is lost: config/logs/registry live in the data dir (untouched), and
//! attendance stays buffered on the device until a successful HRMS upload. The
//! only cost is a few seconds of restart downtime — not literal 100% uptime.

use std::{path::Path, process::Command, time::Duration};

use anyhow::{bail, Context, Result};
use console::style;
use sha2::{Digest, Sha256};

use crate::{
    application::service,
    runtime::{process, registry},
    services::ServiceKind,
    support::paths,
};

const REPO: &str = "anonto42/fbsy";

/// Release asset name for the platform this binary was built for.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const ASSET: &str = "fbsy-linux-x86_64";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const ASSET: &str = "fbsy-linux-aarch64";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const ASSET: &str = "fbsy-windows-x86_64.exe";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const ASSET: &str = "fbsy-macos-intel";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const ASSET: &str = "fbsy-macos-arm64";
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
)))]
const ASSET: &str = "";

/// Options for [`run`].
#[derive(Debug, Default, Clone, Copy)]
pub struct UpdateOpts {
    /// Only report whether an update exists; do not install.
    pub check_only: bool,
    /// Skip the confirmation prompt.
    pub assume_yes: bool,
    /// Non-interactive (used by the auto-update trigger).
    pub auto: bool,
}

/// Result of a version check.
pub struct UpdateStatus {
    pub current: String,
    pub latest: String,
    pub newer: bool,
}

/// The version this binary was built as.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compare current vs. latest release.
pub fn check() -> Result<UpdateStatus> {
    let current = current_version().to_string();
    let latest = latest_version()?;
    let newer = version_newer(&latest, &current);
    Ok(UpdateStatus {
        current,
        latest,
        newer,
    })
}

/// Entry point for `fbsy update`.
pub fn run(opts: UpdateOpts) -> Result<()> {
    if ASSET.is_empty() {
        bail!("self-update is not supported on this platform/architecture");
    }

    let status = check()?;
    println!(
        "Current: {}   Latest: {}",
        style(&status.current).cyan(),
        style(&status.latest).cyan()
    );
    if !status.newer {
        println!("{} fbsy is up to date.", style("✔").green().bold());
        return Ok(());
    }
    println!(
        "{} Update available: {} → {}",
        style("!").yellow().bold(),
        status.current,
        style(&status.latest).green().bold()
    );
    if opts.check_only {
        println!("Run {} to install it.", style("fbsy update").cyan());
        return Ok(());
    }
    if !opts.assume_yes && !opts.auto {
        let ok = dialoguer::Confirm::new()
            .with_prompt("Download and install this update now?")
            .default(true)
            .interact()
            .unwrap_or(false);
        if !ok {
            return Ok(());
        }
    }

    perform_update(&status.latest)
}

// ── The swap ──────────────────────────────────────────────────────────────────

/// (kind, old pid, port, args) for each service running at update time.
type Running = Vec<(ServiceKind, u32, Option<u16>, Vec<String>)>;

fn perform_update(latest: &str) -> Result<()> {
    paths::ensure_dirs()?;
    let dir = paths::update_dir();
    std::fs::create_dir_all(&dir)?;

    // 1. Remember which services are running so they can be restarted identically.
    let running = capture_running();
    if running.is_empty() {
        diag("no services currently running");
    } else {
        diag(&format!(
            "services running: {}",
            running
                .iter()
                .map(|(k, ..)| k.name())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // 2. Download the new binary.
    let url = format!("https://github.com/{REPO}/releases/download/v{latest}/{ASSET}");
    diag(&format!("downloading {ASSET} (v{latest})"));
    let bytes = download(&url).context("download new binary")?;

    // 3. Verify its checksum.
    verify_checksum(latest, &bytes)?;
    diag("checksum verified");

    // 4. Stage + smoke-test the new binary before committing to it.
    let new_path = dir.join("fbsy-new");
    std::fs::write(&new_path, &bytes).context("write new binary")?;
    set_executable(&new_path);
    let reported = smoke_test(&new_path)?;
    if !reported.contains(latest) {
        bail!("smoke test failed: new binary reported '{reported}', expected v{latest}");
    }
    diag(&format!("smoke test ok ({reported})"));

    // 5. Back up the running binary (this exact file is what self_replace swaps).
    let target = std::env::current_exe().context("locate current executable")?;
    let backup = dir.join("fbsy-backup");
    std::fs::copy(&target, &backup).context("back up current binary")?;
    diag(&format!("backed up current binary → {}", backup.display()));

    // 6. Replace the running binary atomically (handles the running-exe case).
    self_replace::self_replace(&new_path).context("replace installed binary")?;
    diag("installed new binary");

    // 7. Restart the previously-running services from the new binary (now at `target`).
    let restart_ok = restart_services(&target, &running).is_ok();

    // 8. Health-check.
    if restart_ok && health_check(&running) {
        diag(&format!("health check ok — now running v{latest}"));
        let _ = std::fs::remove_file(&new_path);
        println!("{} Updated to v{latest}.", style("✔").green().bold());
        Ok(())
    } else {
        // 9. Roll back.
        eprintln!(
            "{} update failed health check — rolling back to {}",
            style("✘").red().bold(),
            current_version()
        );
        let _ = self_replace::self_replace(&backup);
        let _ = restart_services(&target, &running);
        bail!("update rolled back to v{}", current_version());
    }
}

fn capture_running() -> Running {
    let mut out = Running::new();
    for entry in registry::list().unwrap_or_default() {
        if let Some(kind) = ServiceKind::from_name(&entry.service) {
            if process::is_alive(entry.pid, Some(&entry.exe)) {
                out.push((kind, entry.pid, entry.port, entry.args));
            }
        }
    }
    out
}

fn restart_services(exe: &Path, running: &Running) -> Result<()> {
    for (kind, old_pid, port, args) in running {
        // Terminate the OLD process by its captured pid. We can't rely on
        // `stop_service` here: once the binary is replaced, the old process's
        // exe path reads as "fbsy (deleted)", so the registry's liveness check
        // would skip the kill and leave it holding the port.
        let _ = process::terminate(*old_pid);
        let _ = registry::clear(kind.name());
        wait_for_exit(*old_pid);

        service::spawn_service_with_exe(exe, *kind, *port, args)
            .with_context(|| format!("restart {}", kind.name()))?;
        diag(&format!("restarted {}", kind.name()));
    }
    Ok(())
}

/// Wait briefly for a pid to exit (so its port is freed) before respawning.
fn wait_for_exit(pid: u32) {
    for _ in 0..20 {
        if !process::is_alive(pid, None) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn health_check(running: &Running) -> bool {
    std::thread::sleep(Duration::from_millis(600));
    for (kind, _, port, _) in running {
        match registry::read(kind.name()) {
            Ok(Some(entry)) if process::is_alive(entry.pid, Some(&entry.exe)) => {}
            _ => return false,
        }
        if *kind == ServiceKind::AtBridge {
            if let Some(p) = port {
                if !bridge_health_ok(*p) {
                    return false;
                }
            }
        }
    }
    true
}

fn bridge_health_ok(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    for _ in 0..6 {
        if let Ok(resp) = client.get(&url).send() {
            if resp.status().is_success() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

// ── Version detection (no GitHub API → no rate limit) ─────────────────────────

/// Resolve the latest version by reading the `releases/latest/download/<asset>`
/// redirect target (`…/download/vX.Y.Z/<asset>`); falls back to the API.
fn latest_version() -> Result<String> {
    let url = format!("https://github.com/{REPO}/releases/latest/download/{ASSET}");
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?;
    if let Ok(resp) = client.head(&url).send() {
        if let Some(loc) = resp.headers().get(reqwest::header::LOCATION) {
            if let Some(v) = loc.to_str().ok().and_then(parse_version_from_url) {
                return Ok(v);
            }
        }
    }
    latest_version_via_api()
}

fn latest_version_via_api() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?;
    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, "fbsy-self-update")
        .send()
        .context("query GitHub releases API")?;
    let json: serde_json::Value = resp.json().context("parse releases API response")?;
    let tag = json
        .get("tag_name")
        .and_then(|t| t.as_str())
        .context("releases API response had no tag_name")?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// Extract `X.Y.Z` from `…/releases/download/vX.Y.Z/asset`.
fn parse_version_from_url(url: &str) -> Option<String> {
    const MARKER: &str = "/download/v";
    let start = url.find(MARKER)? + MARKER.len();
    let rest = &url[start..];
    let end = rest.find('/')?;
    let v = &rest[..end];
    if v.chars().all(|c| c.is_ascii_digit() || c == '.') && !v.is_empty() {
        Some(v.to_string())
    } else {
        None
    }
}

/// True if `latest` is a strictly higher dotted version than `current`.
fn version_newer(latest: &str, current: &str) -> bool {
    fn parts(s: &str) -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    let (l, c) = (parts(latest), parts(current));
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv != cv {
            return lv > cv;
        }
    }
    false
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn download(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()?;
    let resp = client.get(url).send()?.error_for_status()?;
    Ok(resp.bytes()?.to_vec())
}

fn verify_checksum(version: &str, bytes: &[u8]) -> Result<()> {
    let url = format!("https://github.com/{REPO}/releases/download/v{version}/checksums.txt");
    let Ok(raw) = download(&url) else {
        eprintln!("  (checksums.txt unavailable — skipping verification)");
        return Ok(());
    };
    let text = String::from_utf8_lossy(&raw);
    let expected = text.lines().find_map(|line| {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?;
        (name == ASSET).then(|| hash.to_lowercase())
    });
    let Some(expected) = expected else {
        eprintln!("  (no checksum entry for {ASSET} — skipping verification)");
        return Ok(());
    };
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        bail!("checksum mismatch (expected {expected}, got {actual})");
    }
    Ok(())
}

fn smoke_test(path: &Path) -> Result<String> {
    let out = Command::new(path)
        .arg("--version")
        .output()
        .context("run new binary --version")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn diag(msg: &str) {
    println!("  {} {msg}", style("→").cyan());
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(version_newer("0.2.10", "0.2.9"));
        assert!(version_newer("0.3.0", "0.2.99"));
        assert!(version_newer("1.0.0", "0.9.9"));
        assert!(!version_newer("0.2.9", "0.2.9"));
        assert!(!version_newer("0.2.8", "0.2.9"));
        assert!(version_newer("v0.2.10", "0.2.9")); // tolerates leading v
    }

    #[test]
    fn parses_version_from_redirect_url() {
        let url = "https://github.com/anonto42/fbsy/releases/download/v0.3.1/fbsy-linux-x86_64";
        assert_eq!(parse_version_from_url(url).as_deref(), Some("0.3.1"));
        assert_eq!(
            parse_version_from_url("https://example.com/no/version"),
            None
        );
    }
}
