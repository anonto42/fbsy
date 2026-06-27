//! Self-install: copy the binary to a per-user bin directory, ensure it is on
//! PATH, and create the data directories so `fbsy` works from any directory.
//!
//! Install is per-user (no root). Data lives under the per-OS app dir
//! (see [`crate::support::paths`]); the binary goes to:
//!   - Linux/macOS: ~/.local/bin/fbsy
//!   - Windows:     %LOCALAPPDATA%\Programs\fbsy\fbsy.exe

use std::path::{Path, PathBuf};

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

    println!();
    println!(
        "Next: open a new shell, then run {}",
        style("fbsy --help").cyan()
    );
    Ok(())
}

/// Remove the installed binary. Data directories are left intact.
pub fn uninstall() -> Result<()> {
    let dst = install_bin_path()?;
    if dst.exists() {
        remove_installed_binary(&dst)?;
    } else {
        println!("Nothing to remove at {}", dst.display());
    }
    println!(
        "Data dir left intact: {} (delete manually if desired)",
        paths::base_dir().display()
    );
    println!("If a PATH line was added to your shell rc, remove it manually.");
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
