//! Top-level CLI argument shape.
//!
//! This file answers: "What can the user type at the top level?"

use clap::Parser;

use super::command::Command;

#[derive(Debug, Parser)]
#[command(name = "fbsy")]
#[command(about = "Native biometric attendance bridge for HRMS webhook ingestion")]
#[command(version)]
/// Top-level CLI shape parsed from terminal arguments.
pub struct Cli {
    /// Modern subcommand style, such as `fbsy config validate`.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Backward-compatible alias for the Python bridge's `--once` flag.
    #[arg(long, help = "Compatibility alias: pull once and exit")]
    pub once: bool,

    /// Backward-compatible alias used with `--once` to target one device.
    #[arg(long, help = "Compatibility alias: with --once, sync one deviceCode")]
    pub device: Option<String>,

    /// Backward-compatible alias for the Python bridge's `--setup` flag.
    #[arg(long, help = "Compatibility alias: run setup wizard")]
    pub setup: bool,

    /// Backward-compatible alias for overriding the scheduler interval.
    #[arg(long, help = "Compatibility alias: override sync interval in seconds")]
    pub interval: Option<u64>,

    /// Backward-compatible alias for Windows startup registration.
    #[arg(long, help = "Compatibility alias: install Windows autostart")]
    pub install_autostart: bool,

    /// Backward-compatible alias for removing Windows startup registration.
    #[arg(long, help = "Compatibility alias: uninstall Windows autostart")]
    pub uninstall_autostart: bool,
}
