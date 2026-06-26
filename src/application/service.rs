//! Service orchestration: start services as detached background processes,
//! and inspect/stop them via the registry.
//!
//! The parent (`fbsy run …`) spawns a detached child that re-enters the binary
//! through the hidden `__service-run` subcommand and calls the existing blocking
//! server loops in [`crate::application::serve`] and
//! [`crate::application::test_server`]. The blocking sync logic — including the
//! safety invariant — is reused unchanged.

use std::path::PathBuf;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use console::style;
use dialoguer::Confirm;

use crate::{
    adapters::config_file::JsonConfigStore,
    application,
    ports::config_store::ConfigStore,
    runtime::{
        process,
        registry::{self, RegistryEntry},
    },
    services::ServiceKind,
    support::paths,
};

// ── Parent side: `fbsy run <service>` ─────────────────────────────────────────

/// Start the mock ZKTeco device server, detached.
pub fn run_zkteco(port: u16, records: usize) -> Result<()> {
    let args = vec![
        "--port".to_string(),
        port.to_string(),
        "--records".to_string(),
        records.to_string(),
    ];
    start_detached(ServiceKind::Zkteco, Some(port), &args)
}

/// Start the mock HRMS webhook server, detached.
pub fn run_hrms(port: u16) -> Result<()> {
    let args = vec!["--port".to_string(), port.to_string()];
    start_detached(ServiceKind::Hrms, Some(port), &args)
}

/// Start the real attendance bridge, detached, with an interactive first run.
pub fn run_at_bridge(config: Option<PathBuf>, interval: Option<u64>, no_poll: bool) -> Result<()> {
    paths::ensure_dirs()?;
    let _ = paths::migrate_legacy_config();
    let cfg_path = config.clone().unwrap_or_else(paths::default_config_path);

    // First run: no config yet — offer the setup wizard.
    if !cfg_path.exists() {
        println!(
            "{} No config found at {}",
            style("!").yellow().bold(),
            style(cfg_path.display()).cyan()
        );
        let run_setup = Confirm::new()
            .with_prompt("Run the setup wizard now?")
            .default(true)
            .interact()
            .unwrap_or(false);
        if run_setup {
            application::setup::run_at(cfg_path.clone())?;
        } else {
            println!(
                "Run {} when ready, then start the bridge again.",
                style("fbsy bridge config setup").cyan()
            );
            return Ok(());
        }
    }

    // Already running? Show status instead of double-starting.
    if let Some(entry) = registry::read(ServiceKind::AtBridge.name())? {
        if process::is_alive(entry.pid, Some(&entry.exe)) {
            println!(
                "{} bridge already running (pid {}).",
                style("✔").green().bold(),
                entry.pid
            );
            return status(ServiceKind::AtBridge.name());
        }
    }

    // Configured and stopped — start it. Record the bridge port from config.
    let port = load_bridge_port(&cfg_path);
    let mut args = Vec::new();
    if let Some(c) = config {
        args.push("--config".to_string());
        args.push(c.display().to_string());
    }
    if let Some(i) = interval {
        args.push("--interval".to_string());
        args.push(i.to_string());
    }
    if no_poll {
        args.push("--no-poll".to_string());
    }
    start_detached(ServiceKind::AtBridge, port, &args)
}

/// Print-free spawn: refuse double-start, clear stale entry, spawn detached,
/// record the registry entry, and return the new pid. Shared by the CLI `run`
/// commands and the TUI dashboard.
pub fn spawn_service(kind: ServiceKind, port: Option<u16>, args: &[String]) -> Result<u32> {
    let exe = std::env::current_exe().unwrap_or_default();
    spawn_service_with_exe(&exe, kind, port, args)
}

/// Like [`spawn_service`] but launches a specific executable. The self-update
/// flow uses this to restart services from the freshly-installed binary.
pub fn spawn_service_with_exe(
    exe: &std::path::Path,
    kind: ServiceKind,
    port: Option<u16>,
    args: &[String],
) -> Result<u32> {
    paths::ensure_dirs()?;

    if let Some(entry) = registry::read(kind.name())? {
        if process::is_alive(entry.pid, Some(&entry.exe)) {
            bail!("{} is already running (pid {})", kind.name(), entry.pid);
        }
        // Stale entry from a process that has since died.
        registry::clear(kind.name())?;
    }

    let log = paths::service_log_path(kind.name());
    let pid = process::spawn_detached_with_exe(exe, kind.name(), args, &log)?;
    registry::write(&RegistryEntry {
        service: kind.name().to_string(),
        pid,
        port,
        url: service_url(kind, port),
        args: args.to_vec(),
        started_at: Utc::now().to_rfc3339(),
        exe: exe.display().to_string(),
    })?;
    Ok(pid)
}

/// The address/URL where a running service can be reached.
///   zkteco — `127.0.0.1:PORT`        (TCP; use as the device IP in setup)
///   hrms   — `http://127.0.0.1:PORT` (HTTP; webhook is at `/webhook`)
///   bridge — `http://127.0.0.1:PORT` (the bridge's local HTTP API)
fn service_url(kind: ServiceKind, port: Option<u16>) -> Option<String> {
    let port = port?;
    Some(match kind {
        ServiceKind::Zkteco => format!("127.0.0.1:{port}"),
        ServiceKind::Hrms | ServiceKind::AtBridge => format!("http://127.0.0.1:{port}"),
    })
}

/// Start a service with its default flags (used by the TUI dashboard). For
/// `bridge` this requires an existing config — the wizard cannot run inside
/// the dashboard's alternate screen.
pub fn default_start(kind: ServiceKind) -> Result<u32> {
    match kind {
        ServiceKind::Zkteco => spawn_service(
            kind,
            Some(4370),
            &[
                "--port".to_string(),
                "4370".to_string(),
                "--records".to_string(),
                "5".to_string(),
            ],
        ),
        ServiceKind::Hrms => spawn_service(
            kind,
            Some(8800),
            &["--port".to_string(), "8800".to_string()],
        ),
        ServiceKind::AtBridge => {
            if !at_bridge_configured() {
                bail!("bridge needs setup — run `fbsy bridge config setup`");
            }
            let port = load_bridge_port(&paths::default_config_path());
            spawn_service(kind, port, &[])
        }
    }
}

/// CLI wrapper around [`spawn_service`] that prints the result.
fn start_detached(kind: ServiceKind, port: Option<u16>, args: &[String]) -> Result<()> {
    let pid = spawn_service(kind, port, args)?;
    let log = paths::service_log_path(kind.name());
    println!(
        "{} {} started (pid {}). Logs: {}",
        style("✔").green().bold(),
        style(kind.name()).cyan().bold(),
        pid,
        log.display()
    );
    Ok(())
}

// ── Child side: hidden `__service-run <service> [args]` ───────────────────────

/// Entry point of the detached child: run the actual blocking server loop.
pub fn exec_internal(service: &str, rest: &[String]) -> Result<()> {
    match ServiceKind::from_name(service) {
        Some(ServiceKind::Zkteco) => {
            let (port, records) = parse_zkteco(rest);
            application::test_server::run_device(port, records)
        }
        Some(ServiceKind::Hrms) => {
            let port = parse_hrms(rest);
            application::test_server::run_hrms(port)
        }
        Some(ServiceKind::AtBridge) => {
            let (config, interval, no_poll) = parse_bridge(rest);
            application::serve::run(interval, no_poll, config)
        }
        None => bail!("unknown service '{service}'"),
    }
}

// ── Print-free core (shared by CLI commands and the TUI dashboard) ────────────

/// Live status of one service.
pub struct ServiceStatus {
    pub kind: ServiceKind,
    pub running: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub uptime_secs: Option<i64>,
}

/// Status of every known service, in [`ServiceKind::all`] order. Auto-clears
/// registry entries whose process is no longer alive.
pub fn snapshot() -> Vec<ServiceStatus> {
    let mut out = Vec::new();
    for kind in ServiceKind::all() {
        let entry = registry::read(kind.name()).ok().flatten();
        match entry {
            Some(e) if process::is_alive(e.pid, Some(&e.exe)) => {
                // Older registry files may predate the `url` field — fall back
                // to deriving it from the recorded port.
                let url = e.url.clone().or_else(|| service_url(kind, e.port));
                out.push(ServiceStatus {
                    kind,
                    running: true,
                    pid: Some(e.pid),
                    port: e.port,
                    url,
                    uptime_secs: uptime_secs(&e.started_at),
                });
            }
            Some(_) => {
                // Stale entry — clear it and report stopped.
                let _ = registry::clear(kind.name());
                out.push(ServiceStatus {
                    kind,
                    running: false,
                    pid: None,
                    port: None,
                    url: None,
                    uptime_secs: None,
                });
            }
            None => out.push(ServiceStatus {
                kind,
                running: false,
                pid: None,
                port: None,
                url: None,
                uptime_secs: None,
            }),
        }
    }
    out
}

/// Print-free stop: terminate the service (if alive) and clear its registry
/// entry. Returns whether a running process was signalled.
pub fn stop_service(kind: ServiceKind) -> Result<bool> {
    let Some(entry) = registry::read(kind.name())? else {
        return Ok(false);
    };
    let was_alive = process::is_alive(entry.pid, Some(&entry.exe));
    if was_alive {
        process::terminate(entry.pid)?;
    }
    registry::clear(kind.name())?;
    Ok(was_alive)
}

/// Whether the attendance bridge has a config file (the wizard can't run inside
/// the TUI, so the dashboard checks this before offering to start `bridge`).
pub fn at_bridge_configured() -> bool {
    paths::default_config_path().exists()
}

// ── Management: show / close / status / logs ──────────────────────────────────

/// Print a table of all services with liveness, pid, port, and uptime.
pub fn show() -> Result<()> {
    let rows = snapshot();
    if !rows.iter().any(|r| r.running) {
        println!(
            "No services running. Start one with {}.",
            style("fbsy run <service>").cyan()
        );
        return Ok(());
    }

    println!(
        "{:<10} {:<9} {:<8} {:<7} {:<8} {}",
        style("SERVICE").bold(),
        style("STATUS").bold(),
        style("PID").bold(),
        style("UPTIME").bold(),
        style("PORT").bold(),
        style("ADDRESS").bold()
    );
    for row in rows {
        if !row.running {
            continue;
        }
        let status_cell = style("running").green();
        let pid = row.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
        let port = row
            .port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".into());
        let uptime = row
            .uptime_secs
            .map(format_uptime_secs)
            .unwrap_or_else(|| "-".into());
        let address = row.url.clone().unwrap_or_else(|| "-".into());
        println!(
            "{:<10} {:<9} {:<8} {:<7} {:<8} {}",
            row.kind.name(),
            status_cell,
            pid,
            uptime,
            port,
            style(address).cyan()
        );
    }
    Ok(())
}

/// Stop a running service and clear its registry entry.
pub fn close(service: &str) -> Result<()> {
    let kind = ServiceKind::from_name(service)
        .ok_or_else(|| anyhow::anyhow!("unknown service '{service}'"))?;
    if stop_service(kind)? {
        println!(
            "{} {} stopped.",
            style("✔").green().bold(),
            style(kind.name()).cyan().bold()
        );
    } else {
        println!("{} is not running.", kind.name());
    }
    Ok(())
}

/// Show one service's status and where to find its logs.
pub fn status(service: &str) -> Result<()> {
    let kind = ServiceKind::from_name(service)
        .ok_or_else(|| anyhow::anyhow!("unknown service '{service}'"))?;
    let log = paths::service_log_path(kind.name());

    match registry::read(kind.name())? {
        Some(entry) if process::is_alive(entry.pid, Some(&entry.exe)) => {
            let url = entry.url.clone().or_else(|| service_url(kind, entry.port));
            println!("Service: {}", style(kind.name()).cyan().bold());
            println!("  Status:  {}", style("running").green());
            println!("  PID:     {}", entry.pid);
            if let Some(port) = entry.port {
                println!("  Port:    {port}");
            }
            if let Some(url) = &url {
                println!("  Address: {}", style(url).cyan());
            }
            for hint in address_hints(kind, url.as_deref()) {
                println!("           {hint}");
            }
            println!("  Uptime:  {}", format_uptime(&entry.started_at));
            println!("  Logs:    {}", log.display());
        }
        _ => {
            println!("Service: {}", style(kind.name()).cyan().bold());
            println!("  Status:  {}", style("stopped").red());
            println!("  Logs:    {}", log.display());
        }
    }
    Ok(())
}

/// Service-specific hints about how to use a running service's address.
fn address_hints(kind: ServiceKind, url: Option<&str>) -> Vec<String> {
    let Some(url) = url else {
        return Vec::new();
    };
    match kind {
        ServiceKind::Zkteco => vec![format!(
            "(use this host/port as a device in `fbsy bridge config setup`)"
        )],
        ServiceKind::Hrms => vec![
            format!("POST {url}/webhook   ← attendance events"),
            format!("GET  {url}/events    ← inspect received events"),
        ],
        ServiceKind::AtBridge => vec![
            format!("GET  {url}/health    ← status + last sync result"),
            format!("POST {url}/sync      ← trigger a sync"),
        ],
    }
}

/// Print the last `lines` of a service's log file (optionally follow).
pub fn logs(service: &str, lines: usize, follow: bool) -> Result<()> {
    let kind = ServiceKind::from_name(service)
        .ok_or_else(|| anyhow::anyhow!("unknown service '{service}'"))?;
    let log = paths::service_log_path(kind.name());

    if !log.exists() {
        println!("No log file yet at {}", log.display());
        return Ok(());
    }

    for line in tail_lines(&log, lines) {
        println!("{line}");
    }

    if follow {
        let offset = std::fs::metadata(&log)
            .map(|m| m.len() as usize)
            .unwrap_or(0);
        follow_log(&log, offset)?;
    }
    Ok(())
}

fn follow_log(log: &std::path::Path, mut offset: usize) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    println!("{}", style("-- following (Ctrl-C to stop) --").dim());
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut file = std::fs::File::open(log)?;
        let len = file.metadata()?.len() as usize;
        if len > offset {
            file.seek(SeekFrom::Start(offset as u64))?;
            let mut buf = String::new();
            file.read_to_string(&mut buf)?;
            print!("{buf}");
            offset = len;
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_bridge_port(cfg_path: &std::path::Path) -> Option<u16> {
    JsonConfigStore
        .load(cfg_path)
        .ok()
        .map(|cfg| cfg.bridge_port)
}

/// Seconds since an RFC 3339 start timestamp, or `None` if unparseable.
fn uptime_secs(started_at: &str) -> Option<i64> {
    started_at
        .parse::<DateTime<Utc>>()
        .ok()
        .map(|started| (Utc::now() - started).num_seconds().max(0))
}

/// Format an RFC 3339 start timestamp as a compact uptime string.
fn format_uptime(started_at: &str) -> String {
    uptime_secs(started_at)
        .map(format_uptime_secs)
        .unwrap_or_else(|| "-".to_string())
}

/// Format a duration in seconds as a compact uptime string (e.g. `2h5m`).
pub fn format_uptime_secs(secs: i64) -> String {
    let (d, h, m, s) = (
        secs / 86400,
        (secs % 86400) / 3600,
        (secs % 3600) / 60,
        secs % 60,
    );
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m}m")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

/// Last `n` lines of a file, oldest-first. Empty if the file is missing.
pub fn tail_lines(path: &std::path::Path, n: usize) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(n);
    all[start..].iter().map(|s| s.to_string()).collect()
}

// Manual parsers for the controlled arg vectors we generate in `start_detached`.

fn parse_zkteco(rest: &[String]) -> (u16, usize) {
    let mut port = 4370u16;
    let mut records = 5usize;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--port" => {
                if let Some(v) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                    port = v;
                }
                i += 2;
            }
            "--records" => {
                if let Some(v) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                    records = v;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    (port, records)
}

fn parse_hrms(rest: &[String]) -> u16 {
    let mut port = 8800u16;
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--port" {
            if let Some(v) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                port = v;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    port
}

fn parse_bridge(rest: &[String]) -> (Option<PathBuf>, Option<u64>, bool) {
    let mut config = None;
    let mut interval = None;
    let mut no_poll = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--config" => {
                config = rest.get(i + 1).map(PathBuf::from);
                i += 2;
            }
            "--interval" => {
                interval = rest.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "--no-poll" => {
                no_poll = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    (config, interval, no_poll)
}
