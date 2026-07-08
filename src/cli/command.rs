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

    /// Internal entry point for detached service processes.
    #[command(name = "__service-run", hide = true)]
    ServiceRun(ServiceRunArgs),

    /// Internal foreground entry point execed by OS boot units (self-registers).
    #[command(name = "__service-supervised", hide = true)]
    ServiceSupervised(ServiceRunArgs),
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
