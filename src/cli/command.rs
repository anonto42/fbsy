//! CLI command definitions.
//!
//! `fbsy` is a service manager. Top-level verbs (`install`, `uninstall`, `update`,
//! `dashboard`) manage services, while hidden entrypoints re-enter the binary
//! for background process execution.

use clap::{Args, Subcommand};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install fbsy to a per-user bin dir and create data directories.
    Install,
    /// Remove the installed binary and PATH configurations.
    #[command(visible_aliases = ["unistall", "unistaill"])]
    Uninstall(UninstallArgs),
    /// Check for and install a newer release (restarts running services).
    Update(UpdateArgs),
    /// Live full-screen dashboard to monitor and control services.
    Dashboard,
    /// Interactive setup wizard: configure HRMS connection and devices.
    Setup,
    /// Check the config: validate it and print a redacted view.
    Config,
    /// Start the bridge service in the background.
    Start,
    /// Stop the running bridge service.
    Stop,
    /// Restart the bridge service.
    Restart,
    /// Pull attendance from devices and forward to HRMS once, then exit.
    Sync(SyncArgs),
    /// Show the bridge service status and last sync results.
    Status,
    /// Print the bridge service log (optionally follow).
    Logs(LogsArgs),

    /// Internal entry point for detached service processes.
    #[command(name = "__service-run", hide = true)]
    ServiceRun(ServiceRunArgs),

    /// Internal foreground entry point execed by OS boot units (self-registers).
    #[command(name = "__service-supervised", hide = true)]
    ServiceSupervised(ServiceRunArgs),
}

// ── sync ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Sync only this device code (default: all configured devices).
    #[arg(long)]
    pub device: Option<String>,
    /// Use a specific config file instead of the default location.
    #[arg(long)]
    pub config: Option<std::path::PathBuf>,
}

// ── logs ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Number of lines to print from the end of the log.
    #[arg(short = 'n', long, default_value_t = 100)]
    pub lines: usize,
    /// Keep printing new lines as they are written.
    #[arg(short, long)]
    pub follow: bool,
}

// ── uninstall ─────────────────────────────────────────────────────────────────

#[derive(Debug, Args, Clone)]
pub struct UninstallArgs {
    /// Fully delete configuration, logs, and all data directories.
    #[arg(short, long)]
    pub full: bool,
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

// ── update ────────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Only report whether a newer release exists; do not install.
    #[arg(long)]
    pub check: bool,
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// Non-interactive (used by the auto-update trigger).
    #[arg(long, hide = true)]
    pub auto: bool,
}

// ── hidden internal child entry point ─────────────────────────────────────────

#[derive(Debug, Args)]
pub struct ServiceRunArgs {
    /// Service kind: bridge | zkteco | hrms | scanner
    pub service: String,
    /// Remaining service-specific flags, parsed by the service itself.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub rest: Vec<String>,
}
