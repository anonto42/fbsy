//! Service orchestration: start services as detached background processes,
//! and inspect/stop them via the registry.
//!
//! The parent (`fbsy run …`) spawns a detached child that re-enters the binary
//! through the hidden `__service-run` subcommand and calls the existing blocking
//! server loops in [`crate::application::serve`] and
//! [`crate::application::test_server`]. The blocking sync logic — including the
//! safety invariant — is reused unchanged.

use std::{net::TcpListener, path::PathBuf, time::Duration};

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
    support::{log_rotation, network, paths},
};

// ── Parent side: `fbsy run <service>` ─────────────────────────────────────────

/// Start the mock ZKTeco device server, detached.
pub fn run_zkteco(name: Option<String>, port: u16, records: usize) -> Result<()> {
    let name = name.unwrap_or_else(|| ServiceKind::Zkteco.name().to_string());
    let args = vec![
        "--port".to_string(),
        port.to_string(),
        "--records".to_string(),
        records.to_string(),
    ];
    start_detached(ServiceKind::Zkteco, &name, Some(port), &args)
}

/// Start the mock HRMS webhook server, detached.
pub fn run_hrms(name: Option<String>, port: u16) -> Result<()> {
    let name = name.unwrap_or_else(|| ServiceKind::Hrms.name().to_string());
    let args = vec!["--port".to_string(), port.to_string()];
    start_detached(ServiceKind::Hrms, &name, Some(port), &args)
}

/// Start the network scanner service, detached.
pub fn run_scanner(
    name: Option<String>,
    interval: u64,
    opts: application::scanner::ScanOptions,
) -> Result<()> {
    let name = name.unwrap_or_else(|| ServiceKind::Scanner.name().to_string());
    let args = scanner_args(interval, &opts);
    start_detached(ServiceKind::Scanner, &name, None, &args)
}

/// Start the real attendance bridge, detached, with an interactive first run.
pub fn run_at_bridge(
    name: Option<String>,
    config: Option<PathBuf>,
    interval: Option<u64>,
    no_poll: bool,
) -> Result<()> {
    let name = name.unwrap_or_else(|| ServiceKind::AtBridge.name().to_string());
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
    if let Some(entry) = registry::read(&name)? {
        if process::is_alive(entry.pid, Some(&entry.exe)) {
            println!(
                "{} {name} already running (pid {}).",
                style("✔").green().bold(),
                entry.pid
            );
            return status(&name);
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
    start_detached(ServiceKind::AtBridge, &name, port, &args)
}

/// Print-free spawn: refuse double-start, clear stale entry, spawn detached,
/// verify the child stayed alive, record the registry entry, and return the pid.
pub fn spawn_service(
    kind: ServiceKind,
    name: &str,
    port: Option<u16>,
    args: &[String],
) -> Result<u32> {
    let exe = std::env::current_exe().unwrap_or_default();
    spawn_service_with_exe(&exe, kind, name, port, args)
}

/// Like [`spawn_service`] but launches a specific executable. The self-update
/// flow uses this to restart services from the freshly-installed binary.
pub fn spawn_service_with_exe(
    exe: &std::path::Path,
    kind: ServiceKind,
    name: &str,
    port: Option<u16>,
    args: &[String],
) -> Result<u32> {
    paths::ensure_dirs()?;

    if let Some(entry) = registry::read(name)? {
        if process::is_alive(entry.pid, Some(&entry.exe)) {
            bail!("instance '{name}' is already running (pid {})", entry.pid);
        }
        // Stale entry from a process that has since died.
        registry::clear(name)?;
    }

    let log = paths::service_log_path(name);

    // Inject the log path so the child can self-manage log rotation.
    let mut augmented = args.to_vec();
    augmented.push("--log-path".to_string());
    augmented.push(log.display().to_string());

    // The child runs the KIND's loop; the registry/log are keyed by the instance.
    let pid = process::spawn_detached_with_exe(exe, kind.name(), &augmented, &log)?;

    // Confirm the child stayed alive (it would exit fast on, e.g., a port clash).
    std::thread::sleep(Duration::from_millis(300));
    if !process::is_alive(pid, None) {
        let _ = registry::clear(name);
        bail!("'{name}' failed to start (port in use? check `fbsy logs {name}`)");
    }

    registry::write(&RegistryEntry {
        service: name.to_string(),
        kind: kind.name().to_string(),
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
///   zkteco — `LAN_IP:PORT`           (TCP; use as the device IP in setup)
///   hrms   — `http://LAN_IP:PORT`    (HTTP; webhook is at `/webhook`)
///   bridge — `http://127.0.0.1:PORT` (local-only bridge HTTP API)
fn service_url(kind: ServiceKind, port: Option<u16>) -> Option<String> {
    if kind == ServiceKind::Scanner {
        return Some("local network scanner".to_string());
    }
    let port = port?;
    let lan_host = network::lan_host_or_loopback();
    Some(match kind {
        ServiceKind::Zkteco => format!("{lan_host}:{port}"),
        ServiceKind::Hrms => format!("http://{lan_host}:{port}"),
        ServiceKind::AtBridge => format!("http://127.0.0.1:{port}"),
        ServiceKind::Scanner => unreachable!("scanner has no listening port"),
    })
}

/// Start the default-named instance of a kind (used by the TUI dashboard). For
/// `bridge` this requires an existing config — the wizard cannot run inside
/// the dashboard's alternate screen.
pub fn default_start(kind: ServiceKind) -> Result<u32> {
    start_named(kind, None, None, None)
}

/// Start an instance with optional name/port/config overrides.
///
/// Powers the dashboard's `:start <kind> [--name N] [--port P] [--config C]`
/// command and named instances generally. With no overrides it behaves exactly
/// like [`default_start`] (default name, default port, default config).
pub fn start_named(
    kind: ServiceKind,
    name: Option<String>,
    port: Option<u16>,
    config: Option<PathBuf>,
) -> Result<u32> {
    let name = name.unwrap_or_else(|| kind.name().to_string());
    match kind {
        ServiceKind::Zkteco => {
            let port = port.unwrap_or(4370);
            spawn_service(
                kind,
                &name,
                Some(port),
                &[
                    "--port".to_string(),
                    port.to_string(),
                    "--records".to_string(),
                    "5".to_string(),
                ],
            )
        }
        ServiceKind::Hrms => {
            let port = port.unwrap_or(8800);
            spawn_service(
                kind,
                &name,
                Some(port),
                &["--port".to_string(), port.to_string()],
            )
        }
        ServiceKind::AtBridge => {
            let cfg_path = config.clone().unwrap_or_else(paths::default_config_path);
            if !cfg_path.exists() {
                bail!(
                    "bridge needs setup — run `fbsy bridge config setup` \
                     (or pass --config <path> to use an existing config)"
                );
            }
            let port = port.or_else(|| load_bridge_port(&cfg_path));
            let mut args = Vec::new();
            if let Some(cfg) = &config {
                args.push("--config".to_string());
                args.push(cfg.display().to_string());
            }
            spawn_service(kind, &name, port, &args)
        }
        ServiceKind::Scanner => spawn_service(
            kind,
            &name,
            None,
            &scanner_args(300, &application::scanner::ScanOptions::default()),
        ),
    }
}

/// CLI wrapper around [`spawn_service`] that prints the result.
fn start_detached(kind: ServiceKind, name: &str, port: Option<u16>, args: &[String]) -> Result<()> {
    let pid = spawn_service(kind, name, port, args)?;
    let log = paths::service_log_path(name);
    println!(
        "{} {} started (pid {}). Logs: {}",
        style("✔").green().bold(),
        style(name).cyan().bold(),
        pid,
        log.display()
    );
    Ok(())
}

// ── Child side: hidden `__service-run <service> [args]` ───────────────────────

/// Entry point of the detached child: run the actual blocking server loop.
pub fn exec_internal(service: &str, rest: &[String]) -> Result<()> {
    // Extract --log-path (injected by spawn_service_with_exe) and start rotation.
    let (log_path, rest) = extract_log_path(rest);
    if let Some(path) = log_path {
        log_rotation::spawn_rotation_thread(path);
    }

    match ServiceKind::from_name(service) {
        Some(ServiceKind::Zkteco) => {
            let (port, records) = parse_zkteco(&rest);
            application::test_server::run_device(port, records)
        }
        Some(ServiceKind::Hrms) => {
            let port = parse_hrms(&rest);
            application::test_server::run_hrms(port)
        }
        Some(ServiceKind::AtBridge) => {
            let (config, interval, no_poll) = parse_bridge(&rest);
            application::serve::run(interval, no_poll, config)
        }
        Some(ServiceKind::Scanner) => {
            let (interval, opts) = parse_scanner(&rest);
            application::scanner::run_service(opts, interval)
        }
        None => bail!("unknown service '{service}'"),
    }
}

/// Supervised foreground run, used by OS init units (`fbsy enable`).
///
/// Unlike [`exec_internal`] (whose *parent* `fbsy run` records the registry), an
/// init system execs this leaf directly — so it self-registers, then runs the
/// blocking loop in-process. On exit the entry is left behind but self-heals:
/// `snapshot`/`show` verify liveness with `is_alive` and report it stopped.
pub fn run_supervised(service: &str, rest: &[String]) -> Result<()> {
    let Some(kind) = ServiceKind::from_name(service) else {
        bail!("unknown service '{service}'");
    };
    let name = kind.name().to_string();
    paths::ensure_dirs()?;

    // Record this process so `show`/`logs`/`status`/`close` see the boot service.
    let port = match kind {
        ServiceKind::AtBridge => {
            let (config, _, _) = parse_bridge(rest);
            let cfg_path = config.unwrap_or_else(paths::default_config_path);
            let port = load_bridge_port(&cfg_path);
            preflight_bridge_port(port)?;
            port
        }
        _ => None,
    };
    let exe = std::env::current_exe().unwrap_or_default();
    registry::write(&RegistryEntry {
        service: name.clone(),
        kind: kind.name().to_string(),
        pid: std::process::id(),
        port,
        url: service_url(kind, port),
        args: rest.to_vec(),
        started_at: Utc::now().to_rfc3339(),
        exe: exe.display().to_string(),
    })?;

    exec_internal(service, rest)
}

// ── Print-free core (shared by CLI commands and the TUI dashboard) ────────────

/// Live status of one service instance.
pub struct ServiceStatus {
    /// Instance name (registry key).
    pub name: String,
    pub kind: ServiceKind,
    pub running: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub uptime_secs: Option<i64>,
}

/// Every running instance found in the registry. Auto-clears entries whose
/// process is no longer alive.
pub fn snapshot() -> Vec<ServiceStatus> {
    let mut out = Vec::new();
    for entry in registry::list().unwrap_or_default() {
        let Some(kind) = entry.kind() else {
            continue;
        };
        if process::is_alive(entry.pid, Some(&entry.exe)) {
            let url = entry.url.clone().or_else(|| service_url(kind, entry.port));
            out.push(ServiceStatus {
                name: entry.service.clone(),
                kind,
                running: true,
                pid: Some(entry.pid),
                port: entry.port,
                url,
                uptime_secs: uptime_secs(&entry.started_at),
            });
        } else {
            // Stale entry — clear it.
            let _ = registry::clear(&entry.service);
        }
    }
    out
}

/// Print-free stop of a named instance. Returns whether a process was signalled.
pub fn stop_instance(name: &str) -> Result<bool> {
    let Some(entry) = registry::read(name)? else {
        return Ok(false);
    };
    let was_alive = process::is_alive(entry.pid, Some(&entry.exe));
    if was_alive {
        process::terminate(entry.pid)?;
    }
    registry::clear(name)?;
    Ok(was_alive)
}

/// Stop the default-named instance of a kind (used by the dashboard's start/stop).
pub fn stop_service(kind: ServiceKind) -> Result<bool> {
    stop_instance(kind.name())
}

/// Restart a named instance with its recorded kind/port/args.
pub fn restart_instance(name: &str) -> Result<u32> {
    let Some(entry) = registry::read(name)? else {
        bail!("instance '{name}' not found");
    };
    let kind = entry
        .kind()
        .ok_or_else(|| anyhow::anyhow!("instance '{name}' has unknown kind"))?;
    stop_instance(name)?;
    std::thread::sleep(Duration::from_millis(150));
    spawn_service(kind, name, entry.port, &entry.args)
}

// ── Management: show / close / status / logs ──────────────────────────────────

/// Print a table of all running instances.
pub fn show() -> Result<()> {
    let rows = snapshot();
    if rows.is_empty() {
        println!(
            "No services running. Start one with {}.",
            style("fbsy run <service>").cyan()
        );
        return Ok(());
    }

    println!(
        "{:<12} {:<8} {:<9} {:<5} {:<8} {:<7} {:<7} {}",
        style("INSTANCE").bold(),
        style("KIND").bold(),
        style("STATUS").bold(),
        style("BOOT").bold(),
        style("PID").bold(),
        style("UPTIME").bold(),
        style("PORT").bold(),
        style("ADDRESS").bold()
    );
    for row in rows {
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
        let boot = application::autostart::status_label(&row.name);
        println!(
            "{:<12} {:<8} {:<9} {:<5} {:<8} {:<7} {:<7} {}",
            row.name,
            row.kind.name(),
            style("running").green(),
            boot,
            pid,
            uptime,
            port,
            style(address).cyan()
        );
    }
    Ok(())
}

/// Stop a running instance and clear its registry entry.
pub fn close(name: &str) -> Result<()> {
    if stop_instance(name)? {
        println!(
            "{} {} stopped.",
            style("✔").green().bold(),
            style(name).cyan().bold()
        );
    } else {
        println!("{name} is not running.");
    }
    Ok(())
}

/// `enabled (starts on boot)` / `not enabled` label for one instance.
fn boot_label(name: &str) -> String {
    if application::autostart::status(name).installed {
        format!("{}", style("enabled (starts on boot)").green())
    } else {
        "not enabled".to_string()
    }
}

/// Show one instance's status and where to find its logs.
pub fn status(name: &str) -> Result<()> {
    let log = paths::service_log_path(name);
    match registry::read(name)? {
        Some(entry) if process::is_alive(entry.pid, Some(&entry.exe)) => {
            let kind = entry.kind();
            let url = entry
                .url
                .clone()
                .or_else(|| kind.and_then(|k| service_url(k, entry.port)));
            println!("Instance: {}", style(name).cyan().bold());
            if let Some(k) = kind {
                println!("  Kind:    {}", k.name());
            }
            println!("  Status:  {}", style("running").green());
            println!("  PID:     {}", entry.pid);
            if let Some(port) = entry.port {
                println!("  Port:    {port}");
            }
            if let Some(url) = &url {
                println!("  Address: {}", style(url).cyan());
            }
            if let Some(k) = kind {
                for hint in address_hints(k, url.as_deref()) {
                    println!("           {hint}");
                }
            }
            println!("  Uptime:  {}", format_uptime(&entry.started_at));
            println!("  On boot: {}", boot_label(name));
            println!("  Logs:    {}", log.display());
        }
        _ => {
            println!("Instance: {}", style(name).cyan().bold());
            println!("  Status:  {}", style("stopped").red());
            println!("  On boot: {}", boot_label(name));
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
        ServiceKind::Scanner => vec![format!(
            "Use `fbsy logs scanner` to see discovered attendance devices"
        )],
    }
}

/// Print the last `lines` of an instance's log file (optionally follow).
pub fn logs(name: &str, lines: usize, follow: bool) -> Result<()> {
    let log = paths::service_log_path(name);

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

/// A merged, time-ordered view of every running instance's recent log lines,
/// each prefixed with `[instance]`. Lines are interleaved by their leading
/// RFC 3339 timestamp; lines without one (e.g. multi-line payloads or banners)
/// carry forward the previous line's time so they stay grouped. Used by the
/// dashboard's "all logs" view.
pub fn tail_all_running(per_instance: usize) -> Vec<String> {
    use crate::support::log::parse_timestamp;

    // (timestamp, sequence, instance, raw_line). The sequence is a stable
    // tiebreaker so same-millisecond lines keep their original file order.
    let mut rows: Vec<(DateTime<Utc>, usize, String, String)> = Vec::new();
    let mut seq = 0usize;
    for s in snapshot() {
        let log = paths::service_log_path(&s.name);
        // Carry-forward time starts at the epoch so untimestamped leading lines
        // sort before the first real event rather than to "now".
        let mut carry = DateTime::<Utc>::MIN_UTC;
        for line in tail_lines(&log, per_instance) {
            if let Some(ts) = parse_timestamp(&line) {
                carry = ts;
            }
            rows.push((carry, seq, s.name.clone(), line));
            seq += 1;
        }
    }
    // Stable sort by (timestamp, sequence) keeps within-instance order intact.
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    rows.into_iter()
        .map(|(_, _, name, line)| format!("[{name}] {line}"))
        .collect()
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

fn preflight_bridge_port(port: Option<u16>) -> Result<()> {
    let Some(port) = port else {
        return Ok(());
    };
    TcpListener::bind(("127.0.0.1", port))
        .map(drop)
        .map_err(|err| anyhow::anyhow!("bridge port {port} is already in use: {err}"))
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

fn scanner_args(interval: u64, opts: &application::scanner::ScanOptions) -> Vec<String> {
    let mut args = vec!["--interval".to_string(), interval.to_string()];
    if let Some(cidr) = &opts.cidr {
        args.push("--cidr".to_string());
        args.push(cidr.clone());
    }
    for host in &opts.hosts {
        args.push("--host".to_string());
        args.push(host.to_string());
    }
    args.push("--port".to_string());
    args.push(opts.port.to_string());
    args.push("--timeout-ms".to_string());
    args.push(opts.scan_timeout_ms.to_string());
    args.push("--device-timeout".to_string());
    args.push(opts.device_timeout_secs.to_string());
    args.push("--password".to_string());
    args.push(opts.device_password.to_string());
    if opts.force_udp {
        args.push("--udp".to_string());
    }
    if opts.include_open {
        args.push("--include-open".to_string());
    }
    args
}

fn parse_scanner(rest: &[String]) -> (u64, application::scanner::ScanOptions) {
    let mut interval = 300u64;
    let mut opts = application::scanner::ScanOptions::default();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--interval" => {
                if let Some(v) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                    interval = v;
                }
                i += 2;
            }
            "--cidr" => {
                opts.cidr = rest.get(i + 1).cloned();
                i += 2;
            }
            "--host" => {
                if let Some(host) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                    opts.hosts.push(host);
                }
                i += 2;
            }
            "--port" => {
                if let Some(v) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                    opts.port = v;
                }
                i += 2;
            }
            "--timeout-ms" => {
                if let Some(v) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                    opts.scan_timeout_ms = v;
                }
                i += 2;
            }
            "--device-timeout" => {
                if let Some(v) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                    opts.device_timeout_secs = v;
                }
                i += 2;
            }
            "--password" => {
                if let Some(v) = rest.get(i + 1).and_then(|s| s.parse().ok()) {
                    opts.device_password = v;
                }
                i += 2;
            }
            "--udp" => {
                opts.force_udp = true;
                i += 1;
            }
            "--include-open" => {
                opts.include_open = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    (interval, opts)
}

/// Extract and remove the `--log-path <value>` injected by [`spawn_service_with_exe`].
/// Returns `(path, remaining_args)`. Used by [`exec_internal`] to start log rotation.
fn extract_log_path(args: &[String]) -> (Option<std::path::PathBuf>, Vec<String>) {
    let mut path = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--log-path" {
            if let Some(val) = args.get(i + 1) {
                path = Some(std::path::PathBuf::from(val));
                i += 2;
                continue;
            }
        }
        rest.push(args[i].clone());
        i += 1;
    }
    (path, rest)
}
