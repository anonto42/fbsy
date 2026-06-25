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
                style("fbsy at-bridge config setup").cyan()
            );
            return Ok(());
        }
    }

    // Already running? Show status instead of double-starting.
    if let Some(entry) = registry::read(ServiceKind::AtBridge.name())? {
        if process::is_alive(entry.pid, Some(&entry.exe)) {
            println!(
                "{} at-bridge already running (pid {}).",
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

/// Common spawn path: refuse double-start, clear stale entry, spawn, record.
fn start_detached(kind: ServiceKind, port: Option<u16>, args: &[String]) -> Result<()> {
    paths::ensure_dirs()?;

    if let Some(entry) = registry::read(kind.name())? {
        if process::is_alive(entry.pid, Some(&entry.exe)) {
            bail!("{} is already running (pid {})", kind.name(), entry.pid);
        }
        // Stale entry from a process that has since died.
        registry::clear(kind.name())?;
    }

    let log = paths::service_log_path(kind.name());
    let pid = process::spawn_detached(kind.name(), args, &log)?;
    registry::write(&RegistryEntry {
        service: kind.name().to_string(),
        pid,
        port,
        args: args.to_vec(),
        started_at: Utc::now().to_rfc3339(),
        exe: std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    })?;

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

// ── Management: show / close / status / logs ──────────────────────────────────

/// Print a table of all services with liveness, pid, port, and uptime.
pub fn show() -> Result<()> {
    let entries = registry::list()?;
    if entries.is_empty() {
        println!(
            "No services running. Start one with {}.",
            style("fbsy run <service>").cyan()
        );
        return Ok(());
    }

    println!(
        "{:<12} {:<9} {:<8} {:<6} {}",
        style("SERVICE").bold(),
        style("STATUS").bold(),
        style("PID").bold(),
        style("PORT").bold(),
        style("UPTIME").bold()
    );
    for entry in entries {
        let alive = process::is_alive(entry.pid, Some(&entry.exe));
        if !alive {
            // Auto-clear stale entries as we surface them.
            let _ = registry::clear(&entry.service);
        }
        let status_cell = if alive {
            style("running").green()
        } else {
            style("stopped").red()
        };
        let port = entry
            .port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".into());
        let uptime = if alive {
            format_uptime(&entry.started_at)
        } else {
            "-".to_string()
        };
        println!(
            "{:<12} {:<9} {:<8} {:<6} {}",
            entry.service, status_cell, entry.pid, port, uptime
        );
    }
    Ok(())
}

/// Stop a running service and clear its registry entry.
pub fn close(service: &str) -> Result<()> {
    let kind = ServiceKind::from_name(service)
        .ok_or_else(|| anyhow::anyhow!("unknown service '{service}'"))?;
    let Some(entry) = registry::read(kind.name())? else {
        println!("{} is not running.", kind.name());
        return Ok(());
    };

    if process::is_alive(entry.pid, Some(&entry.exe)) {
        let stopped = process::terminate(entry.pid)?;
        registry::clear(kind.name())?;
        if stopped {
            println!(
                "{} {} stopped (pid {}).",
                style("✔").green().bold(),
                kind.name(),
                entry.pid
            );
        } else {
            println!("{} was not running; cleared registry.", kind.name());
        }
    } else {
        registry::clear(kind.name())?;
        println!("{} was not running; cleared stale registry.", kind.name());
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
            println!("Service: {}", style(kind.name()).cyan().bold());
            println!("  Status:  {}", style("running").green());
            println!("  PID:     {}", entry.pid);
            if let Some(port) = entry.port {
                println!("  Port:    {port}");
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

/// Print the last `lines` of a service's log file (optionally follow).
pub fn logs(service: &str, lines: usize, follow: bool) -> Result<()> {
    let kind = ServiceKind::from_name(service)
        .ok_or_else(|| anyhow::anyhow!("unknown service '{service}'"))?;
    let log = paths::service_log_path(kind.name());

    if !log.exists() {
        println!("No log file yet at {}", log.display());
        return Ok(());
    }

    let content = std::fs::read_to_string(&log)?;
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(lines);
    for line in &all[start..] {
        println!("{line}");
    }

    if follow {
        follow_log(&log, content.len())?;
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

fn format_uptime(started_at: &str) -> String {
    let Ok(started) = started_at.parse::<DateTime<Utc>>() else {
        return "-".to_string();
    };
    let secs = (Utc::now() - started).num_seconds().max(0);
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
