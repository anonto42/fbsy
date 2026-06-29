//! Boot persistence: register a service with the OS init system so it starts
//! automatically at boot and restarts on crash.
//!
//! The detached-process model (`fbsy run`) does not survive a reboot — the OS
//! kills the process on shutdown and nothing restarts it. `enable` installs a
//! per-OS unit that runs the service in the **foreground** (so the init system
//! supervises it) via the hidden `__service-supervised` entrypoint, which
//! self-registers so `fbsy show`/`logs`/`status` keep working.
//!
//! Confirmed scope: **system-boot** units (no login required), which need
//! elevation to install (sudo / Administrator).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use console::style;
use dialoguer::Confirm;

use crate::{
    adapters::config_file::JsonConfigStore,
    application::{self, service},
    config::ConfigError,
    ports::config_store::ConfigStore,
    services::ServiceKind,
    support::paths,
};

/// Whether a boot unit is installed for an instance.
pub struct AutostartStatus {
    pub installed: bool,
}

/// Human label for `show`/`status` (`boot:on` / `-`).
pub fn status_label(name: &str) -> &'static str {
    if status(name).installed {
        "on"
    } else {
        "-"
    }
}

/// Install and activate a boot unit for `name` (default kind: bridge).
pub fn enable(name: &str, config: Option<PathBuf>) -> Result<()> {
    let kind = ServiceKind::from_name(name)
        .with_context(|| format!("unknown service '{name}' (try: bridge)"))?;
    if kind != ServiceKind::AtBridge && kind != ServiceKind::Scanner {
        bail!("only production services (bridge, scanner) can be enabled on boot, not '{name}'");
    }
    let exe = std::env::current_exe().context("locate current executable")?;

    if !is_elevated() {
        // Resolve paths as the *real* user here (correct), and print the exact
        // elevated command with --config baked in so the privileged run is
        // unambiguous regardless of which account it lands in.
        paths::ensure_dirs()?;
        let _ = paths::migrate_legacy_config();
        let cfg = absolute(&config.unwrap_or_else(paths::default_config_path));
        if kind == ServiceKind::AtBridge {
            ensure_bridge_config_ready(&cfg)?;
        }
        print_elevation_hint(&exe, name, &cfg);
        bail!("administrator privileges are required to install a boot service");
    }

    // Elevated. On Unix `sudo` resets HOME to root, so the config path can't be
    // auto-derived — require it explicitly (the non-root hint above supplies it).
    let cfg = resolve_config_when_elevated(config)?;
    let log = log_path_for(&cfg, name);
    let ctx = UnitCtx {
        name: name.to_string(),
        kind,
        exe,
        config: cfg,
        log,
        user: invoking_user(),
    };

    // A manually-run detached instance would hold the port; best-effort stop it.
    let _ = service::stop_instance(name);

    install_unit(&ctx)?;
    println!(
        "{} {} will now start automatically on boot.",
        style("✔").green().bold(),
        style(name).cyan().bold()
    );
    println!("  Inspect:  {}", inspect_hint(name));
    println!("  Disable:  {} disable {name}", elevated_prefix());
    Ok(())
}

/// Stop, disable-at-boot, and remove the boot unit for `name`.
pub fn disable(name: &str) -> Result<()> {
    ServiceKind::from_name(name)
        .with_context(|| format!("unknown service '{name}' (try: bridge)"))?;
    if !is_elevated() {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("fbsy"));
        eprintln!(
            "{} Administrator privileges are required. Re-run:\n    {} {} disable {name}",
            style("!").yellow().bold(),
            sudo_word(),
            exe.display()
        );
        bail!("administrator privileges are required to remove a boot service");
    }
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
    pub user: Option<String>,
}

/// systemd system unit contents.
pub fn systemd_unit(ctx: &UnitCtx) -> String {
    let user_line = ctx
        .user
        .as_deref()
        .map(|u| format!("User={u}\n"))
        .unwrap_or_default();
    format!(
        "[Unit]\n\
         Description=fbsy {name} (fingerbridge attendance service)\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} __service-supervised {name} --config {config}\n\
         Restart=always\n\
         RestartSec=3\n\
         {user_line}\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        name = ctx.name,
        exe = ctx.exe.display(),
        config = ctx.config.display(),
        log = ctx.log.display(),
    )
}

/// launchd LaunchDaemon plist contents.
pub fn launchd_plist(ctx: &UnitCtx) -> String {
    let user_line = ctx
        .user
        .as_deref()
        .map(|u| format!("  <key>UserName</key>\n  <string>{u}</string>\n"))
        .unwrap_or_default();
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
         {user_line}\
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

/// `schtasks /create` arguments for a SYSTEM ONSTART task.
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
        "onstart".into(),
        "/ru".into(),
        "SYSTEM".into(),
        "/rl".into(),
        "highest".into(),
        "/f".into(),
    ]
}

fn task_name(name: &str) -> String {
    format!("fbsy-{name}")
}

/// `<base>/config/config.json` → `<base>/logs/<name>.log`, so the unit's log
/// path matches `fbsy logs <name>` regardless of which account runs the service.
fn log_path_for(config: &Path, name: &str) -> PathBuf {
    config
        .parent()
        .and_then(|p| p.parent())
        .map(|base| base.join("logs").join(format!("{name}.log")))
        .unwrap_or_else(|| paths::service_log_path(name))
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
    let store = JsonConfigStore;
    match store.load(path) {
        Ok(_) => {
            println!(
                "{} Config is valid: {}",
                style("✔").green().bold(),
                style(path.display()).yellow()
            );
            Ok(())
        }
        Err(ConfigError::NotFound(_)) => {
            println!(
                "{} No config found at {}",
                style("!").yellow().bold(),
                style(path.display()).cyan()
            );
            let run_setup = Confirm::new()
                .with_prompt("Run the setup wizard now?")
                .default(true)
                .interact()
                .unwrap_or(false);
            if !run_setup {
                println!(
                    "Run {} when ready, then enable boot auto-start again.",
                    style("fbsy bridge config setup").cyan()
                );
                bail!("bridge config is required before enabling boot auto-start");
            }

            application::setup::run_at(path.to_path_buf())?;
            store
                .load(path)
                .map(|_| ())
                .context("validate config after setup")
        }
        Err(err) => {
            eprintln!(
                "{} Config is not valid: {}",
                style("!").yellow().bold(),
                err
            );
            eprintln!(
                "Run {} after fixing it.",
                style("fbsy bridge config validate").cyan()
            );
            bail!("bridge config must be valid before enabling boot auto-start");
        }
    }
}

// ── Platform: elevation, install, remove, status ──────────────────────────────

#[cfg(unix)]
fn is_elevated() -> bool {
    // Safety: geteuid is always safe and has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(windows)]
fn is_elevated() -> bool {
    // Best-effort: `net session` only succeeds for elevated processes. If this
    // is wrong, the schtasks call below fails with a clear access-denied error.
    std::process::Command::new("net")
        .args(["session"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn invoking_user() -> Option<String> {
    // Under sudo this is the real user, so the service runs as them (and its
    // config/log dirs resolve to their home, not root's).
    std::env::var("SUDO_USER").ok().filter(|u| u != "root")
}

#[cfg(windows)]
fn invoking_user() -> Option<String> {
    None // the schtasks task runs as SYSTEM
}

#[cfg(unix)]
fn resolve_config_when_elevated(config: Option<PathBuf>) -> Result<PathBuf> {
    let cfg = config.context(
        "pass --config <abs path> when running with sudo \
         (tip: run `fbsy enable` WITHOUT sudo to print the exact command)",
    )?;
    Ok(absolute(&cfg))
}

#[cfg(windows)]
fn resolve_config_when_elevated(config: Option<PathBuf>) -> Result<PathBuf> {
    // No HOME reset on Windows: an elevated shell is still the same user, so the
    // default path resolves correctly.
    Ok(absolute(&config.unwrap_or_else(paths::default_config_path)))
}

#[cfg(target_os = "linux")]
fn install_unit(ctx: &UnitCtx) -> Result<()> {
    let path = unit_path(&ctx.name);
    std::fs::write(&path, systemd_unit(ctx))
        .with_context(|| format!("write unit {}", path.display()))?;
    run_cmd("systemctl", &["daemon-reload"])?;
    run_cmd("systemctl", &["enable", "--now", &service_unit(&ctx.name)])?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_unit(name: &str) -> Result<()> {
    let unit = service_unit(name);
    let _ = run_cmd("systemctl", &["disable", "--now", &unit]);
    let path = unit_path(name);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    }
    let _ = run_cmd("systemctl", &["daemon-reload"]);
    Ok(())
}

#[cfg(target_os = "linux")]
fn unit_path(name: &str) -> PathBuf {
    PathBuf::from(format!("/etc/systemd/system/fbsy-{name}.service"))
}

#[cfg(target_os = "linux")]
fn service_unit(name: &str) -> String {
    format!("fbsy-{name}.service")
}

#[cfg(target_os = "macos")]
fn install_unit(ctx: &UnitCtx) -> Result<()> {
    let path = unit_path(&ctx.name);
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
    PathBuf::from(format!("/Library/LaunchDaemons/com.fbsy.{name}.plist"))
}

#[cfg(windows)]
fn install_unit(ctx: &UnitCtx) -> Result<()> {
    let args = schtasks_create_args(ctx);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_cmd("schtasks", &arg_refs)
}

#[cfg(windows)]
fn remove_unit(name: &str) -> Result<()> {
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

fn print_elevation_hint(exe: &Path, name: &str, cfg: &Path) {
    println!(
        "{} Installing a boot service needs administrator privileges. Run:",
        style("!").yellow().bold()
    );
    #[cfg(windows)]
    println!(
        "    (from an Administrator PowerShell)  \"{}\" enable {name} --config \"{}\"",
        exe.display(),
        cfg.display()
    );
    #[cfg(not(windows))]
    println!(
        "    sudo \"{}\" enable {name} --config \"{}\"",
        exe.display(),
        cfg.display()
    );
}

fn sudo_word() -> &'static str {
    #[cfg(windows)]
    {
        "(Administrator)"
    }
    #[cfg(not(windows))]
    {
        "sudo"
    }
}

fn elevated_prefix() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "fbsy".to_string());
    format!("{} {exe}", sudo_word())
}

fn inspect_hint(name: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        format!("systemctl status fbsy-{name}   ·   journalctl -u fbsy-{name}")
    }
    #[cfg(target_os = "macos")]
    {
        format!("sudo launchctl list | grep com.fbsy.{name}")
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
            user: Some("u".to_string()),
        }
    }

    #[test]
    fn systemd_unit_has_supervised_execstart_and_restart() {
        let unit = systemd_unit(&ctx());
        assert!(unit.contains(
            "ExecStart=/home/u/.local/bin/fbsy __service-supervised bridge \
             --config /home/u/.config/fbsy/config/config.json"
        ));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("User=u"));
        assert!(unit.contains("StandardOutput=append:/home/u/.config/fbsy/logs/bridge.log"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn systemd_unit_omits_user_line_when_unknown() {
        let mut c = ctx();
        c.user = None;
        assert!(!systemd_unit(&c).contains("User="));
    }

    #[test]
    fn launchd_plist_has_runatload_and_program_args() {
        let plist = launchd_plist(&ctx());
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<string>__service-supervised</string>"));
        assert!(plist.contains("<string>/home/u/.config/fbsy/config/config.json</string>"));
        assert!(plist.contains("com.fbsy.bridge"));
    }

    #[test]
    fn schtasks_args_use_onstart_system_and_absolute_paths() {
        let args = schtasks_create_args(&ctx());
        assert!(args.contains(&"onstart".to_string()));
        assert!(args.contains(&"SYSTEM".to_string()));
        assert!(args.contains(&"fbsy-bridge".to_string()));
        let tr = args
            .iter()
            .find(|a| a.contains("__service-supervised"))
            .unwrap();
        assert!(tr.contains("--config"));
    }

    #[test]
    fn log_path_is_derived_from_config_base() {
        let log = log_path_for(
            &PathBuf::from("/home/u/.config/fbsy/config/config.json"),
            "bridge",
        );
        assert_eq!(log, PathBuf::from("/home/u/.config/fbsy/logs/bridge.log"));
    }
}
