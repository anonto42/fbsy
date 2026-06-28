//! Serve use case.
//!
//! Starts the local HTTP API and per-device schedulers. This implementation is
//! intentionally blocking and dependency-light so mock-device testing works now.
//!
//! # Graceful shutdown
//!
//! A SIGTERM or SIGINT handler sets the `SHUTDOWN` flag. The accept loop polls
//! it every 200 ms (non-blocking accept). Schedulers break between sleep slices.
//! After the loop, the main thread waits up to 30 s for any in-flight syncs to
//! complete before returning (which causes the process to exit cleanly).

use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use serde::Serialize;

// ── Graceful-shutdown flag ────────────────────────────────────────────────────

/// Set by the SIGTERM/SIGINT handler. Checked by the accept loop and schedulers.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn shutting_down() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

/// Install SIGTERM and SIGINT handlers that flip `SHUTDOWN`.
/// No-op on non-Unix targets (Windows receives TerminateProcess which cannot
/// be caught here; the non-blocking loop still exits cleanly on process kill).
fn install_signal_handlers() {
    #[cfg(unix)]
    unsafe {
        extern "C" fn on_signal(_: libc::c_int) {
            SHUTDOWN.store(true, Ordering::SeqCst);
        }
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
}

use crate::{
    adapters::{
        config_file::JsonConfigStore, device_zkteco_tcp::ZktecoTcpConnector,
        hrms_reqwest::ReqwestHrmsClient,
    },
    config::BridgeConfig,
    ports::config_store::ConfigStore,
    runtime::{job_poller::start_job_poller, DeviceSyncState},
    support::{log, paths::default_config_path},
};

/// Prepare the bridge to run as a long-lived local service.
pub fn run(interval: Option<u64>, no_poll: bool, config: Option<PathBuf>) -> Result<()> {
    let path = config.unwrap_or_else(default_config_path);
    let store = JsonConfigStore;
    let mut cfg = store.load(&path)?;
    if let Some(interval) = interval {
        for device in &mut cfg.devices {
            device.sync_interval_seconds = interval.max(5);
        }
    }

    println!("FingerBridge service starting");
    println!("  Config: {}", path.display());
    println!("  Devices: {}", cfg.devices.len());
    for device in &cfg.devices {
        println!(
            "  Device {} -> {}:{} every {}s clearAfterSync={}",
            device.device_code,
            device.device_ip,
            device.device_port,
            device.sync_interval_seconds,
            device.clear_attendance_after_sync
        );
    }
    println!(
        "  Job polling: {}",
        if no_poll {
            "disabled by --no-poll"
        } else if cfg.hrms_base_url.is_some() {
            "enabled"
        } else {
            "not configured"
        }
    );
    println!(
        "  Auto update: {}",
        if cfg.auto_update {
            "enabled"
        } else {
            "disabled"
        }
    );

    let states = build_states(&cfg);
    start_boot_syncs(&states);
    start_schedulers(&states);
    if !no_poll {
        start_job_poller(&cfg);
    }
    if cfg.auto_update {
        start_auto_updater(cfg.update_check_interval_hours);
    }

    install_signal_handlers();

    let address = format!("127.0.0.1:{}", cfg.bridge_port);
    let listener = TcpListener::bind(&address)?;
    listener.set_nonblocking(true)?;

    println!("FingerBridge serving on http://{address}");
    println!("  GET  /health");
    println!("  POST /sync");
    println!("  POST /sync?device=CODE");
    log::info("http", format_args!("listening on http://{address}"));

    loop {
        if shutting_down() {
            log::info(
                "shutdown",
                format_args!("shutdown signal received — stopping HTTP loop"),
            );
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false); // handle_client uses blocking reads
                let states = Arc::clone(&states);
                let webhook_url = cfg.vps_webhook_url.clone();
                thread::spawn(move || {
                    let _ = handle_client(stream, states, webhook_url);
                });
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(err) => log::error("http", format_args!("accept failed: {err}")),
        }
    }

    // Wait for in-flight syncs to finish (safety invariant: never cut a sync
    // mid-upload, because the device-clear comes after a successful HRMS send).
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let any_syncing = states.iter().any(|s| s.syncing());
        if !any_syncing || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    log::info("shutdown", format_args!("shutdown complete"));
    Ok(())
}

fn build_states(cfg: &BridgeConfig) -> Arc<Vec<Arc<DeviceSyncState>>> {
    let connector = Arc::new(ZktecoTcpConnector);
    let hrms = Arc::new(ReqwestHrmsClient::default());
    Arc::new(
        cfg.devices
            .iter()
            .cloned()
            .map(|device| {
                Arc::new(
                    DeviceSyncState::new(
                        device,
                        cfg.vps_webhook_url.clone(),
                        connector.clone(),
                        hrms.clone(),
                    )
                    .with_progress_logging(),
                )
            })
            .collect(),
    )
}

fn start_boot_syncs(states: &Arc<Vec<Arc<DeviceSyncState>>>) {
    for state in states.iter() {
        let state = Arc::clone(state);
        log::info(
            "boot",
            format_args!("scheduling boot sync for {}", state.device_code()),
        );
        thread::spawn(move || {
            let _ = state.sync_once();
        });
    }
}

fn start_schedulers(states: &Arc<Vec<Arc<DeviceSyncState>>>) {
    for state in states.iter() {
        let state = Arc::clone(state);
        log::info(
            "sched",
            format_args!(
                "scheduler started for {} every {}s",
                state.device_code(),
                state.sync_interval_seconds()
            ),
        );
        thread::spawn(move || {
            loop {
                // Sleep in 1 s slices so the scheduler notices a shutdown signal
                // without waiting up to a full sync interval.
                let interval = state.sync_interval_seconds();
                for _ in 0..interval {
                    if shutting_down() {
                        log::info(
                            "sched",
                            format_args!("scheduler stopped for {}", state.device_code()),
                        );
                        return;
                    }
                    thread::sleep(Duration::from_secs(1));
                }
                if shutting_down() {
                    return;
                }
                let _ = state.sync_once();
            }
        });
    }
}

/// Periodically check for a newer release and, when one exists, launch a
/// detached `fbsy update --auto` process to perform the safe swap + restart.
/// The updater is a separate process, so it can stop and restart this bridge
/// without tearing itself down. Stops checking after it triggers once.
fn start_auto_updater(interval_hours: u64) {
    let interval = Duration::from_secs(interval_hours.max(1) * 3600);
    thread::spawn(move || {
        // A short initial delay so the bridge finishes booting first.
        thread::sleep(Duration::from_secs(30));
        loop {
            match crate::application::update::check() {
                Ok(status) if status.newer => {
                    log::info(
                        "update",
                        format_args!(
                            "{} -> {} available; launching updater",
                            status.current, status.latest
                        ),
                    );
                    if let Err(err) = launch_detached_updater() {
                        log::error("update", format_args!("could not launch updater: {err}"));
                    }
                    // The updater will restart this process; stop checking.
                    return;
                }
                Ok(_) => {}
                Err(err) => log::warn("update", format_args!("version check failed: {err}")),
            }
            thread::sleep(interval);
        }
    });
}

/// Spawn `<installed fbsy> update --auto` fully detached.
fn launch_detached_updater() -> Result<()> {
    let exe = crate::application::install::install_bin_path()?;
    let log = crate::support::paths::service_log_path("update");
    crate::runtime::process::spawn_detached_command(
        &exe,
        &["update".to_string(), "--auto".to_string()],
        &log,
    )
    .map(|_| ())
}

fn handle_client(
    mut stream: TcpStream,
    states: Arc<Vec<Arc<DeviceSyncState>>>,
    webhook_url: String,
) -> Result<()> {
    let mut buffer = [0u8; 8192];
    let read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let request_line = request.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    match (method, path) {
        ("GET", "/health") => write_json(&mut stream, 200, &health_body(&states, &webhook_url)),
        ("POST", "/sync") => handle_sync(&mut stream, &states, query),
        ("OPTIONS", _) => write_options(&mut stream),
        _ => write_json(
            &mut stream,
            404,
            &serde_json::json!({ "error": "not found" }),
        ),
    }
}

fn handle_sync(
    stream: &mut TcpStream,
    states: &Arc<Vec<Arc<DeviceSyncState>>>,
    query: &str,
) -> Result<()> {
    if let Some(code) = query.strip_prefix("device=") {
        log::info(
            "http",
            format_args!("manual sync requested for device {code}"),
        );
        let Some(state) = states.iter().find(|state| state.device_code() == code) else {
            log::warn(
                "http",
                format_args!("manual sync rejected; device {code} not found"),
            );
            return write_json(
                stream,
                404,
                &serde_json::json!({ "error": format!("device '{code}' not found in config") }),
            );
        };
        if state.syncing() {
            log::warn(
                "http",
                format_args!("manual sync rejected for {code}: already in progress"),
            );
            return write_json(
                stream,
                429,
                &serde_json::json!({ "error": format!("[{code}] sync already in progress") }),
            );
        }
        let state = Arc::clone(state);
        thread::spawn(move || {
            let _ = state.sync_once();
        });
        log::info(
            "http",
            format_args!("manual sync accepted for device {code}"),
        );
        return write_json(
            stream,
            202,
            &serde_json::json!({ "ok": true, "message": format!("sync started for {code}") }),
        );
    }

    log::info(
        "http",
        format_args!("manual sync requested for all devices"),
    );
    let mut started = Vec::new();
    let mut skipped = Vec::new();
    for state in states.iter() {
        if state.syncing() {
            skipped.push(state.device_code().to_string());
        } else {
            started.push(state.device_code().to_string());
            let state = Arc::clone(state);
            thread::spawn(move || {
                let _ = state.sync_once();
            });
        }
    }
    log::info(
        "http",
        format_args!(
            "manual sync accepted; started=[{}] skipped=[{}]",
            started.join(", "),
            skipped.join(", ")
        ),
    );
    write_json(
        stream,
        202,
        &serde_json::json!({
            "ok": true,
            "message": "sync started",
            "started": started,
            "skipped": skipped
        }),
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthBody {
    status: &'static str,
    agent: &'static str,
    version: &'static str,
    runtime: &'static str,
    vps_webhook_url: String,
    device_count: usize,
    devices: Vec<DeviceHealth>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceHealth {
    device_code: String,
    syncing: bool,
    last_result: Option<crate::domain::SyncResult>,
}

fn health_body(states: &Arc<Vec<Arc<DeviceSyncState>>>, webhook_url: &str) -> HealthBody {
    HealthBody {
        status: "ok",
        agent: "fingerbridge",
        version: env!("CARGO_PKG_VERSION"),
        runtime: "rust",
        vps_webhook_url: webhook_url.to_string(),
        device_count: states.len(),
        devices: states
            .iter()
            .map(|state| DeviceHealth {
                device_code: state.device_code().to_string(),
                syncing: state.syncing(),
                last_result: state.last_result(),
            })
            .collect(),
    }
}

fn write_options(stream: &mut TcpStream) -> Result<()> {
    stream.write_all(
        b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\n\r\n",
    )?;
    Ok(())
}

fn write_json<T: Serialize>(stream: &mut TcpStream, status: u16, body: &T) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "OK",
    };
    let body = serde_json::to_string_pretty(body)?;
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}
