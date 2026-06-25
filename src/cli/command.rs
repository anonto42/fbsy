//! CLI command enum definitions.
//!
//! This file answers: "Which commands and subcommands does the program
//! understand?"

use std::path::PathBuf;

use clap::Subcommand;

#[derive(Debug, Subcommand)]
/// Primary command groups for the Rust bridge.
pub enum Command {
    /// Show local readiness and next steps.
    Doctor {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Run network checks against device/HRMS.
        #[arg(long)]
        deep: bool,
        /// Optional config path. Defaults to `./config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Run the first-time setup wizard.
    Setup,
    /// Pull attendance once, forward it, then exit.
    Once {
        /// Optional device code. When omitted, all configured devices are synced.
        #[arg(long)]
        device: Option<String>,
        /// Optional config path for testing or custom installs.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Run the local HTTP API and background scheduler.
    Serve {
        /// Optional interval override in seconds.
        #[arg(long)]
        interval: Option<u64>,
        /// Disable HRMS job polling (attendance sync only).
        #[arg(long)]
        no_poll: bool,
        /// Optional config path for testing or custom installs.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Inspect or validate configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Inspect or test configured devices.
    Devices {
        #[command(subcommand)]
        command: DevicesCommand,
    },
    /// Test HRMS webhook connectivity.
    Webhook {
        #[command(subcommand)]
        command: WebhookCommand,
    },
    /// Inspect log paths.
    Logs {
        #[command(subcommand)]
        command: LogsCommand,
    },
    /// Manage OS startup integration.
    Autostart {
        #[command(subcommand)]
        command: AutostartCommand,
    },
    /// Manage mock/test servers for local diagnostics.
    TestServer {
        #[command(subcommand)]
        command: TestServerCommand,
    },
}

#[derive(Debug, Subcommand)]
/// Subcommands to run test/mock servers.
pub enum TestServerCommand {
    /// Start a mock ZKTeco biometric device server.
    Device {
        /// TCP port to bind to.
        #[arg(long, default_value = "4370")]
        port: u16,
        /// Number of mock check-in/out records to pre-populate.
        #[arg(long, default_value = "5")]
        records: usize,
    },
    /// Start a mock HRMS webhook API server.
    Hrms {
        /// HTTP port to bind to.
        #[arg(long, default_value = "8800")]
        port: u16,
    },
}

#[derive(Debug, Subcommand)]
/// Subcommands that operate on `config.json`.
pub enum ConfigCommand {
    /// Validate config and print a simple success message.
    Validate {
        /// Optional config path. Defaults to `./config.json`.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Print a redacted config view for support/debugging.
    Show {
        /// Optional config path. Defaults to `./config.json`.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Print the config path that this CLI will use by default.
    Path,
}

#[derive(Debug, Subcommand)]
/// Subcommands that operate on configured devices.
pub enum DevicesCommand {
    /// Print configured devices without showing secrets.
    List {
        /// Optional config path. Defaults to `./config.json`.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Test one configured device connection.
    Test {
        /// Device code to test.
        code: String,
        /// Optional config path. Defaults to `./config.json`.
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
/// HRMS webhook commands.
pub enum WebhookCommand {
    /// Send an empty event batch for one device.
    Test {
        /// Device code to test.
        code: String,
        /// Optional config path. Defaults to `./config.json`.
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
/// Log helper commands.
pub enum LogsCommand {
    /// Print the default log directory.
    Path,
}

#[derive(Debug, Subcommand)]
/// Startup integration commands.
pub enum AutostartCommand {
    /// Install OS startup integration.
    Install,
    /// Remove OS startup integration.
    Uninstall,
}
