//! Boot persistence: register the bridge with the OS so it starts
//! automatically at login/boot and restarts on crash.
//!
//! The detached-process model (`fbsy start`) does not survive a reboot — the
//! OS kills the process on shutdown and nothing restarts it. `enable` installs
//! a **per-user** unit that runs the service in the foreground (so the init
//! system supervises it) via the hidden `__service-supervised` entrypoint,
//! which self-registers so `fbsy status`/`logs` keep working.
//!
//! Everything here is per-user and needs **no sudo / Administrator**:
//!   - macOS:   `~/Library/LaunchAgents/com.fbsy.<name>.plist` (RunAtLoad + KeepAlive)
//!   - Linux:   `~/.config/systemd/user/fbsy-<name>.service` (systemctl --user)
//!   - Windows: `schtasks /sc onlogon` task for the current user

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use console::style;

use crate::{
    adapters::config_file::JsonConfigStore,
    application::service,
    ports::config_store::ConfigStore,
    services::ServiceKind,
    support::paths,
};

/// Whether a boot unit is installed for an instance.
pub struct AutostartStatus {
    pub installed: bool,
}

/// Human label for `status` (`boot:on` / `-`).
pub fn status_label(name: &str) -> &'static str {
    if status(name).installed {
        "on"
    } else {
        "-"
    }
}

/// Install and activate a per-user boot unit for `name`, print-free (safe to
/// call from inside the TUI). Stops any manually-run detached instance first;
/// the supervised process takes over immediately and self-registers.
pub fn install_quiet(name: &str, config: Option<PathBuf>) -> Result<()> {
    let kind = ServiceKind::from_name(name)
        .with_context(|| format!("unknown service '{name}' (try: bridge)"))?;
    if kind != ServiceKind::AtBridge {
        bail!("only the bridge service can be enabled on boot, not '{name}'");
    }
    let exe = std::env::current_exe().context("locate current executable")?;

    paths::ensure_dirs()?;
    let _ = paths::migrate_legacy_config();
    let cfg = absolute(&config.unwrap_or_else(paths::default_config_path));
    ensure_bridge_config_ready(&cfg)?;

    let ctx = UnitCtx {
        name: name.to_string(),
        kind,
        exe,
        config: cfg,
        log: paths::service_log_path(name),
    };

    // A manually-run detached instance would hold the port; best-effort stop it.
    let _ = service::stop_instance(name);

    install_unit(&ctx)
}

/// Install and activate a per-user boot unit for `name` (default kind: bridge).
pub fn enable(name: &str, config: Option<PathBuf>) -> Result<()> {
    install_quiet(name, config)?;
    println!(
        "{} {} will now start automatically and restart if it crashes.",
        style("✔").green().bold(),
        style(name).cyan().bold()
    );
    println!("  Inspect: {}", inspect_hint(name));
    Ok(())
}

/// Remove the boot unit without printing (safe to call from inside the TUI).
pub fn remove_quiet(name: &str) -> Result<()> {
    remove_unit(name)
}

/// Ask the OS supervisor to (re)start the instance now. launchd (KeepAlive)
/// and systemd (Restart=always) respawn killed processes on their own, so
/// this is only needed on Windows, where an ONLOGON task must be re-fired.
pub fn kick(name: &str) {
    #[cfg(windows)]
    {
        let _ = run_cmd("schtasks", &["/run", "/tn", &task_name(name)]);
    }
    #[cfg(not(windows))]
    {
        let _ = name; // unix supervisors respawn automatically
    }
}

/// Stop, disable-at-boot, and remove the boot unit for `name`.
pub fn disable(name: &str) -> Result<()> {
    ServiceKind::from_name(name)
        .with_context(|| format!("unknown service '{name}' (try: bridge)"))?;
    remove_unit(name)?;
    println!(
        "{} {} will no longer start on boot.",
        style("✔").green().bold(),
        style(name).cyan().bold()
    );
    Ok(())
}

// ── Shared context + pure renderers (unit-test friendly, not cfg-gated) ────────

/// Everything needed to render and install one boot unit.
pub struct UnitCtx {
    pub name: String,
    pub kind: ServiceKind,
    pub exe: PathBuf,
    pub config: PathBuf,
    pub log: PathBuf,
}

/// systemd **user** unit contents.
pub fn systemd_unit(ctx: &UnitCtx) -> String {
    format!(
        "[Unit]\n\
         Description=fbsy {name} (fingerbridge attendance service)\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} __service-supervised {name} --config {config}\n\
         Restart=always\n\
         RestartSec=3\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        name = ctx.name,
        exe = ctx.exe.display(),
        config = ctx.config.display(),
        log = ctx.log.display(),
    )
}

/// launchd user LaunchAgent plist contents.
pub fn launchd_plist(ctx: &UnitCtx) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>Label</key>\n  <string>com.fbsy.{name}</string>\n\
         \x20 <key>ProgramArguments</key>\n  <array>\n\
         \x20   <string>{exe}</string>\n\
         \x20   <string>__service-supervised</string>\n\
         \x20   <string>{name}</string>\n\
         \x20   <string>--config</string>\n\
         \x20   <string>{config}</string>\n\
         \x20 </array>\n\
         \x20 <key>RunAtLoad</key>\n  <true/>\n\
         \x20 <key>KeepAlive</key>\n  <true/>\n\
         \x20 <key>StandardOutPath</key>\n  <string>{log}</string>\n\
         \x20 <key>StandardErrorPath</key>\n  <string>{log}</string>\n\
         </dict>\n\
         </plist>\n",
        name = ctx.name,
        exe = ctx.exe.display(),
        config = ctx.config.display(),
        log = ctx.log.display(),
    )
}

/// `schtasks /create` arguments for an ONLOGON task as the current user
/// (no Administrator shell needed).
pub fn schtasks_create_args(ctx: &UnitCtx) -> Vec<String> {
    let run = format!(
        "\"{}\" __service-supervised {} --config \"{}\"",
        ctx.exe.display(),
        ctx.name,
        ctx.config.display()
    );
    vec![
        "/create".into(),
        "/tn".into(),
        task_name(&ctx.name),
        "/tr".into(),
        run,
        "/sc".into(),
        "onlogon".into(),
        "/f".into(),
    ]
}

fn task_name(name: &str) -> String {
    format!("fbsy-{name}")
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn ensure_bridge_config_ready(path: &Path) -> Result<()> {
    JsonConfigStore.load(path).map(|_| ()).with_context(|| {
        format!(
            "bridge config at {} must exist and be valid before enabling boot auto-start \
             (run `fbsy setup`)",
            path.display()
        )
    })
}

// ── Platform: install, remove, status ─────────────────────────────────────────

#[cfg(target_os = "linux")]
fn install_unit(ctx: &UnitCtx) -> Result<()> {
    let path = unit_path(&ctx.name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&path, systemd_unit(ctx))
        .with_context(|| format!("write unit {}", path.display()))?;
    run_cmd("systemctl", &["--user", "daemon-reload"])?;
    run_cmd(
        "systemctl",
        &["--user", "enable", "--now", &service_unit(&ctx.name)],
    )?;
    // Best-effort: let the user service run even before login after boot.
    let _ = std::process::Command::new("loginctl")
        .args(["enable-linger"])
        .status();
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_unit(name: &str) -> Result<()> {
    let unit = service_unit(name);
    let _ = run_cmd("systemctl", &["--user", "disable", "--now", &unit]);
    let path = unit_path(name);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    }
    let _ = run_cmd("systemctl", &["--user", "daemon-reload"]);
    Ok(())
}

#[cfg(target_os = "linux")]
fn unit_path(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(format!(".config/systemd/user/fbsy-{name}.service"))
}

#[cfg(target_os = "linux")]
fn service_unit(name: &str) -> String {
    format!("fbsy-{name}.service")
}

#[cfg(target_os = "macos")]
fn install_unit(ctx: &UnitCtx) -> Result<()> {
    let path = unit_path(&ctx.name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&path, launchd_plist(ctx))
        .with_context(|| format!("write plist {}", path.display()))?;
    let _ = run_cmd("launchctl", &["unload", &path.to_string_lossy()]);
    run_cmd("launchctl", &["load", "-w", &path.to_string_lossy()])?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn remove_unit(name: &str) -> Result<()> {
    let path = unit_path(name);
    let _ = run_cmd("launchctl", &["unload", "-w", &path.to_string_lossy()]);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn unit_path(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(format!("Library/LaunchAgents/com.fbsy.{name}.plist"))
}

#[cfg(windows)]
fn install_unit(ctx: &UnitCtx) -> Result<()> {
    let args = schtasks_create_args(ctx);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_cmd("schtasks", &arg_refs)?;
    // Start it now too; the task itself only fires at the next logon.
    let _ = run_cmd("schtasks", &["/run", "/tn", &task_name(&ctx.name)]);
    Ok(())
}

#[cfg(windows)]
fn remove_unit(name: &str) -> Result<()> {
    let _ = run_cmd("schtasks", &["/end", "/tn", &task_name(name)]);
    run_cmd("schtasks", &["/delete", "/tn", &task_name(name), "/f"])
}

/// Whether a boot unit currently exists for `name` (fast, no subprocess).
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn status(name: &str) -> AutostartStatus {
    AutostartStatus {
        installed: unit_path(name).exists(),
    }
}

#[cfg(windows)]
pub fn status(name: &str) -> AutostartStatus {
    let installed = std::process::Command::new("schtasks")
        .args(["/query", "/tn", &task_name(name)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    AutostartStatus { installed }
}

// ── Command helpers + hints ───────────────────────────────────────────────────

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn run_cmd(program: &str, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("{program} {} failed: {}", args.join(" "), err.trim());
    }
    Ok(())
}

fn inspect_hint(name: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        format!("systemctl --user status fbsy-{name}")
    }
    #[cfg(target_os = "macos")]
    {
        format!("launchctl list | grep com.fbsy.{name}")
    }
    #[cfg(target_os = "windows")]
    {
        format!("schtasks /query /tn fbsy-{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> UnitCtx {
        UnitCtx {
            name: "bridge".to_string(),
            kind: ServiceKind::AtBridge,
            exe: PathBuf::from("/home/u/.local/bin/fbsy"),
            config: PathBuf::from("/home/u/.config/fbsy/config/config.json"),
            log: PathBuf::from("/home/u/.config/fbsy/logs/bridge.log"),
        }
    }

    #[test]
    fn systemd_unit_is_a_user_unit_with_restart() {
        let unit = systemd_unit(&ctx());
        assert!(unit.contains(
            "ExecStart=/home/u/.local/bin/fbsy __service-supervised bridge \
             --config /home/u/.config/fbsy/config/config.json"
        ));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("WantedBy=default.target"));
        assert!(!unit.contains("User="));
    }

    #[test]
    fn launchd_plist_has_runatload_and_program_args() {
        let plist = launchd_plist(&ctx());
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<string>__service-supervised</string>"));
        assert!(plist.contains("<string>/home/u/.config/fbsy/config/config.json</string>"));
        assert!(plist.contains("com.fbsy.bridge"));
        assert!(!plist.contains("UserName"));
    }

    #[test]
    fn schtasks_args_use_onlogon_current_user() {
        let args = schtasks_create_args(&ctx());
        assert!(args.contains(&"onlogon".to_string()));
        assert!(!args.contains(&"SYSTEM".to_string()));
        assert!(args.contains(&"fbsy-bridge".to_string()));
        let tr = args
            .iter()
            .find(|a| a.contains("__service-supervised"))
            .unwrap();
        assert!(tr.contains("--config"));
    }
}
