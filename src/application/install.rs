//! Self-install: copy the binary to a per-user bin directory, ensure it is on
//! PATH, and create the data directories so `fbsy` works from any directory.
//!
//! Install is per-user (no root). Data lives under the per-OS app dir
//! (see [`crate::support::paths`]); the binary goes to:
//!   - Linux/macOS: ~/.local/bin/fbsy
//!   - Windows:     %LOCALAPPDATA%\Programs\fbsy\fbsy.exe

use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use console::style;

use crate::support::paths;

/// Install the running binary and set up directories + PATH.
pub fn install() -> Result<()> {
    paths::ensure_dirs()?;
    let _ = paths::migrate_legacy_config();

    let src = std::env::current_exe().context("locate current executable")?;
    let dst = install_bin_path()?;

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create bin dir {}", parent.display()))?;
    }

    if src != dst {
        std::fs::copy(&src, &dst)
            .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
        set_executable(&dst);
    }

    println!(
        "{} Installed fbsy to {}",
        style("✔").green().bold(),
        style(dst.display()).cyan()
    );
    println!("  Data dir:   {}", paths::base_dir().display());
    println!("  Config dir: {}", paths::config_dir().display());
    println!("  Log dir:    {}", paths::log_dir().display());

    ensure_on_path(dst.parent().unwrap_or(&dst));

    // First-time install: no config yet means the bridge can't do anything.
    // Offer the setup wizard right away so install → configured is one step.
    let config_path = paths::default_config_path();
    if !config_path.exists() && std::io::stdin().is_terminal() {
        println!();
        println!(
            "{} No bridge configuration found yet.",
            style("!").yellow().bold()
        );
        let run_setup = dialoguer::Confirm::new()
            .with_prompt("Run the setup wizard now to connect your device and HRMS?")
            .default(true)
            .interact()
            .unwrap_or(false);
        if run_setup {
            crate::application::setup::run()?;
        } else {
            println!(
                "You can configure later from the dashboard: run {} and type {}",
                style("fbsy dashboard").cyan(),
                style("setup").cyan()
            );
        }
    }

    println!();
    println!(
        "Next: open a new shell, then run {}",
        style("fbsy dashboard").cyan()
    );
    Ok(())
}

/// Remove the installed binary and/or data directories.
pub fn uninstall(args: &crate::cli::UninstallArgs) -> Result<()> {
    // 1. Stop all running services
    println!(
        "{} Checking for running services...",
        style("→").cyan().bold()
    );
    match crate::runtime::registry::list() {
        Ok(entries) => {
            for entry in entries {
                println!(
                    "  Stopping service {} (pid {})...",
                    style(&entry.service).cyan(),
                    entry.pid
                );
                if let Err(err) = crate::application::service::stop_instance(&entry.service) {
                    println!(
                        "    {} Failed to stop service {}: {}",
                        style("!").yellow().bold(),
                        entry.service,
                        err
                    );
                }
            }
        }
        Err(err) => {
            println!(
                "  {} Failed to list running services: {}",
                style("!").yellow().bold(),
                err
            );
        }
    }

    // 2. Decide if full delete
    let mut full_delete = args.full;
    if !args.full && !args.yes {
        // If interactive, prompt the user
        let options = [
            "Just the software (removes binary and PATH shortcut; keeps config/logs)",
            "Full delete (removes binary, PATH, and all config/logs/state directories)",
        ];
        if let Ok(choice) = dialoguer::Select::new()
            .with_prompt("Select uninstall type:")
            .items(&options)
            .default(0)
            .interact()
        {
            if choice == 1 {
                full_delete = true;
            }
        }
    }

    // 3. Clean up PATH
    let dst = install_bin_path()?;
    if let Some(_parent) = dst.parent() {
        #[cfg(not(windows))]
        {
            if let Some(rc_path) = remove_from_shell_rc() {
                println!(
                    "{} Removed fbsy PATH line from {}",
                    style("✔").green().bold(),
                    rc_path.display()
                );
            }
        }
        #[cfg(windows)]
        {
            if let Err(err) = remove_from_windows_path(_parent) {
                println!(
                    "{} Could not remove fbsy from PATH: {}",
                    style("!").yellow().bold(),
                    err
                );
            } else {
                println!("{} Removed fbsy from User PATH", style("✔").green().bold());
            }
        }
    }

    // 4. Remove binary
    if dst.exists() {
        remove_installed_binary(&dst)?;
    } else {
        println!("Nothing to remove at {}", dst.display());
    }

    // 5. Clean up data directories if Full Delete
    if full_delete {
        let base = paths::base_dir();
        if base.exists() {
            println!(
                "{} Removing data directory {}...",
                style("→").cyan().bold(),
                base.display()
            );
            match std::fs::remove_dir_all(&base) {
                Ok(()) => {
                    println!(
                        "{} Removed data directory {}",
                        style("✔").green().bold(),
                        style(base.display()).cyan()
                    );
                }
                Err(err) => {
                    println!(
                        "{} Failed to remove data directory: {err}",
                        style("!").yellow().bold()
                    );
                }
            }
        }
    } else {
        println!(
            "Data directory left intact: {} (delete manually if desired)",
            paths::base_dir().display()
        );
    }

    Ok(())
}

#[cfg(not(windows))]
fn remove_from_shell_rc() -> Option<PathBuf> {
    const SENTINEL: &str = "# added by fbsy install";
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell_name = std::path::Path::new(&shell)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let rc = if shell_name == "fish" {
        home.join(".config").join("fish").join("config.fish")
    } else if shell_name == "zsh" {
        home.join(".zshrc")
    } else {
        home.join(".bashrc")
    };

    if let Ok(content) = std::fs::read_to_string(&rc) {
        if content.contains(SENTINEL) {
            let lines: Vec<&str> = content
                .lines()
                .filter(|line| !line.contains(SENTINEL))
                .collect();
            let mut new_content = lines.join("\n");
            if !new_content.is_empty() && !new_content.ends_with('\n') {
                new_content.push('\n');
            }
            if std::fs::write(&rc, new_content).is_ok() {
                return Some(rc);
            }
        }
    }
    None
}

#[cfg(windows)]
fn remove_from_windows_path(bin_dir: &std::path::Path) -> Result<()> {
    use std::process::Command;
    let script = format!(
        "$installDir = '{}'; \
         $userPath = [Environment]::GetEnvironmentVariable('Path', 'User'); \
         if ($userPath) {{ \
             $entries = $userPath -split ';' | Where-Object {{ $_ -ne '' -and $_ -ne $installDir }}; \
             $newPath = $entries -join ';'; \
             [Environment]::SetEnvironmentVariable('Path', $newPath, 'User'); \
         }}",
        bin_dir.display().to_string().replace('\'', "''")
    );
    Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .context("remove from Windows PATH")?;
    Ok(())
}

#[cfg(not(windows))]
fn remove_installed_binary(dst: &Path) -> Result<()> {
    std::fs::remove_file(dst).with_context(|| format!("remove {}", dst.display()))?;
    println!(
        "{} Removed {}",
        style("✔").green().bold(),
        style(dst.display()).cyan()
    );
    Ok(())
}

#[cfg(windows)]
fn remove_installed_binary(dst: &Path) -> Result<()> {
    match std::fs::remove_file(dst) {
        Ok(()) => {
            println!(
                "{} Removed {}",
                style("✔").green().bold(),
                style(dst.display()).cyan()
            );
            Ok(())
        }
        Err(_err) if current_exe_matches(dst) => {
            schedule_windows_self_delete(dst)?;
            println!(
                "{} Scheduled removal of {}",
                style("✔").green().bold(),
                style(dst.display()).cyan()
            );
            println!(
                "Windows locks the running .exe, so fbsy will delete it after this command exits."
            );
            Ok(())
        }
        Err(err) => Err(err).with_context(|| format!("remove {}", dst.display())),
    }
}

#[cfg(windows)]
fn current_exe_matches(dst: &Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    paths_match(&current, dst)
}

#[cfg(windows)]
fn paths_match(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    a.to_string_lossy()
        .eq_ignore_ascii_case(b.to_string_lossy().as_ref())
}

#[cfg(windows)]
fn schedule_windows_self_delete(dst: &Path) -> Result<()> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let literal = powershell_literal(dst);
    let script = format!(
        "$pidToWait = {}; \
         try {{ Wait-Process -Id $pidToWait -ErrorAction SilentlyContinue }} catch {{}}; \
         Start-Sleep -Milliseconds 250; \
         Remove-Item -LiteralPath {literal} -Force -ErrorAction SilentlyContinue; \
         $dir = Split-Path -LiteralPath {literal}; \
         if ($dir -and (Test-Path -LiteralPath $dir)) {{ \
             $children = @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue); \
             if ($children.Count -eq 0) {{ \
                 Remove-Item -LiteralPath $dir -Force -ErrorAction SilentlyContinue \
             }} \
         }}",
        std::process::id()
    );

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .context("schedule Windows self-removal")?;

    Ok(())
}

#[cfg(windows)]
fn powershell_literal(path: &Path) -> String {
    let escaped = path.display().to_string().replace('\'', "''");
    format!("'{escaped}'")
}

/// Where the installed binary should live.
pub fn install_bin_path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .context("LOCALAPPDATA not set")?;
        Ok(base.join("Programs").join("fbsy").join("fbsy.exe"))
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME not set")?;
        Ok(home.join(".local").join("bin").join("fbsy"))
    }
}

#[cfg(unix)]
fn set_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &std::path::Path) {}

/// Ensure `bin_dir` is on PATH; edit the user's shell rc on Unix, or print
/// instructions otherwise.
fn ensure_on_path(bin_dir: &std::path::Path) {
    if path_contains(bin_dir) {
        return;
    }

    #[cfg(not(windows))]
    {
        match add_to_shell_rc(bin_dir) {
            Some(rc) => println!(
                "{} Added {} to PATH in {}",
                style("✔").green().bold(),
                bin_dir.display(),
                rc.display()
            ),
            None => print_manual_path(bin_dir),
        }
    }
    #[cfg(windows)]
    {
        print_manual_path(bin_dir);
    }
}

fn path_contains(bin_dir: &std::path::Path) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|p| p == bin_dir)
}

fn print_manual_path(bin_dir: &std::path::Path) {
    println!(
        "{} Add this directory to your PATH: {}",
        style("!").yellow().bold(),
        style(bin_dir.display()).cyan()
    );
}

/// Append an idempotent, sentinel-guarded PATH line to the user's shell rc.
/// Returns the rc file edited, or `None` if it could not be determined.
#[cfg(not(windows))]
fn add_to_shell_rc(bin_dir: &std::path::Path) -> Option<PathBuf> {
    const SENTINEL: &str = "# added by fbsy install";
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell_name = std::path::Path::new(&shell)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let (rc, line) = if shell_name == "fish" {
        (
            home.join(".config").join("fish").join("config.fish"),
            format!("fish_add_path {} {SENTINEL}", bin_dir.display()),
        )
    } else if shell_name == "zsh" {
        (
            home.join(".zshrc"),
            format!("export PATH=\"{}:$PATH\" {SENTINEL}", bin_dir.display()),
        )
    } else {
        (
            home.join(".bashrc"),
            format!("export PATH=\"{}:$PATH\" {SENTINEL}", bin_dir.display()),
        )
    };

    // Idempotent: skip if our sentinel is already present.
    if let Ok(existing) = std::fs::read_to_string(&rc) {
        if existing.contains(SENTINEL) {
            return Some(rc);
        }
    }
    if let Some(parent) = rc.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&rc)
        .ok()?;
    writeln!(file, "\n{line}").ok()?;
    Some(rc)
}
