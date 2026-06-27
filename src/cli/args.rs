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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::command::Command;

    use super::Cli;

    #[test]
    fn uninstall_accepts_common_typo_aliases() {
        for word in ["uninstall", "unistall", "unistaill"] {
            let cli = Cli::parse_from(["fbsy", word]);
            assert!(matches!(cli.command, Some(Command::Uninstall)));
        }
    }
}
