//! Top-level CLI argument shape.

use clap::Parser;

use super::command::Command;

#[derive(Debug, Parser)]
#[command(name = "fbsy")]
#[command(about = "Native biometric attendance bridge for HRMS webhook ingestion")]
#[command(version)]
/// Top-level CLI shape parsed from terminal arguments.
pub struct Cli {
    /// Service-manager subcommand. When omitted, shows running services.
    #[command(subcommand)]
    pub command: Option<Command>,
}
