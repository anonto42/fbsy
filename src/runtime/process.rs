//! Cross-platform background-process management.
//!
//! - [`spawn_detached`] re-executes the current binary's hidden `__service-run`
//!   subcommand as a fully detached process, with output redirected to a log file.
//! - [`is_alive`] / [`terminate`] use `sysinfo` so liveness and shutdown work the
//!   same on Linux, macOS, and Windows.

use std::{
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};
use sysinfo::{Pid, ProcessesToUpdate, Signal, System};

/// True if a process with `pid` exists. When `expect_exe` is given, the running
/// process's executable name must match it — this guards against pid reuse after
/// the original service died.
pub fn is_alive(pid: u32, expect_exe: Option<&str>) -> bool {
    let mut sys = System::new();
    let target = Pid::from_u32(pid);
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    let Some(proc_) = sys.process(target) else {
        return false;
    };
    match expect_exe {
        None => true,
        Some(expected) => {
            let expected_name = file_name_of(expected);
            // Compare by the executable's file name (full paths differ across
            // installs; the basename is stable, e.g. "fbsy").
            let running = proc_
                .exe()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string());
            match running {
                Some(name) => name == expected_name,
                // If we can't read the exe (permissions), fall back to "alive".
                None => true,
            }
        }
    }
}

/// Terminate a process by pid. Sends SIGTERM on Unix / TerminateProcess on
/// Windows. Returns `Ok(true)` if a signal was delivered, `Ok(false)` if no such
/// process exists.
pub fn terminate(pid: u32) -> Result<bool> {
    let mut sys = System::new();
    let target = Pid::from_u32(pid);
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    let Some(proc_) = sys.process(target) else {
        return Ok(false);
    };
    // Prefer a graceful SIGTERM; fall back to the platform default kill if the
    // signal is unsupported (e.g. some Windows configurations).
    match proc_.kill_with(Signal::Term) {
        Some(sent) => Ok(sent),
        None => Ok(proc_.kill()),
    }
}

/// Spawn `current_exe __service-run <service> [internal_args...]` fully detached,
/// redirecting stdout and stderr to `log_path`. Returns the child pid.
pub fn spawn_detached(service: &str, internal_args: &[String], log_path: &Path) -> Result<u32> {
    let exe = std::env::current_exe().context("locate current executable")?;

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("open log file {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .context("clone log file handle for stderr")?;

    let mut cmd = Command::new(exe);
    cmd.arg("__service-run").arg(service);
    cmd.args(internal_args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(stdout));
    cmd.stderr(Stdio::from(stderr));

    detach(&mut cmd);

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn detached service '{service}'"))?;
    Ok(child.id())
}

/// Put the child in its own session/process group so it survives the parent
/// shell. On Unix this is `setsid`; on Windows, detached + no console window.
#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Safety: setsid is async-signal-safe and the closure does nothing else.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, windows)))]
fn detach(_cmd: &mut Command) {}

fn file_name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}
