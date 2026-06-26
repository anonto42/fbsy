//! CLI command definitions.
//!
//! `fbsy` is a service manager. Top-level verbs (`install`, `run`, `show`,
//! `close`, `status`, `logs`) manage background services by name; each service
//! also has its own command group (`bridge`, `zkteco`, `hrms`) exposing its
//! specific flags. A hidden `__service-run` subcommand is the entry point the
//! detached child process re-enters through.

use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install fbsy to a per-user bin dir and create data directories.
    Install,
    /// Remove the installed binary (data directories are left intact).
    Uninstall,
    /// Check for and install a newer release (restarts running services).
    Update(UpdateArgs),

    /// Start a service as a detached background process.
    Run(RunArgs),
    /// Live full-screen dashboard to monitor and control services.
    Dashboard,
    /// List all services with status, pid, port, and uptime.
    Show,
    /// Stop a running service.
    Close(ServiceSelector),
    /// Show one service's status.
    Status(ServiceSelector),
    /// Show a service's log output.
    Logs(LogsArgs),

    /// Attendance bridge: configure, run, sync, and diagnose.
    #[command(name = "bridge", visible_alias = "at-bridge")]
    AtBridge(AtBridgeArgs),
    /// Mock ZKTeco device server.
    Zkteco(ZktecoArgs),
    /// Mock HRMS webhook server.
    Hrms(HrmsArgs),

    /// Internal entry point for detached service processes.
    #[command(name = "__service-run", hide = true)]
    ServiceRun(ServiceRunArgs),
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

// ── run / selectors ───────────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct RunArgs {
    #[command(subcommand)]
    pub service: RunService,
}

#[derive(Debug, Subcommand)]
pub enum RunService {
    /// Start the attendance bridge (interactive first run if unconfigured).
    #[command(name = "bridge", visible_alias = "at-bridge")]
    AtBridge {
        /// Instance name (default: bridge). Use to run more than one.
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        interval: Option<u64>,
        #[arg(long)]
        no_poll: bool,
    },
    /// Start the mock ZKTeco device server.
    Zkteco {
        /// Instance name (default: zkteco). Use to run more than one.
        #[arg(long)]
        name: Option<String>,
        #[arg(short = 'p', long, default_value = "4370")]
        port: u16,
        #[arg(long, default_value = "5")]
        records: usize,
    },
    /// Start the mock HRMS webhook server.
    Hrms {
        /// Instance name (default: hrms). Use to run more than one.
        #[arg(long)]
        name: Option<String>,
        #[arg(short = 'p', long, default_value = "8800")]
        port: u16,
    },
}

#[derive(Debug, Args)]
pub struct ServiceSelector {
    /// Instance name, for example: bridge | zkteco | hrms | dev1
    pub service: String,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Instance name, for example: bridge | zkteco | hrms | dev1
    pub service: String,
    /// Number of trailing lines to print.
    #[arg(short = 'n', long, default_value = "50")]
    pub lines: usize,
    /// Keep printing new log output as it arrives.
    #[arg(short, long)]
    pub follow: bool,
}

// ── bridge service group ───────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct AtBridgeArgs {
    #[command(subcommand)]
    pub command: AtBridgeCommand,
}

#[derive(Debug, Subcommand)]
pub enum AtBridgeCommand {
    /// Run the bridge detached (same as `fbsy run bridge`).
    Run {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        interval: Option<u64>,
        #[arg(long)]
        no_poll: bool,
    },
    /// Pull attendance once and exit.
    Sync {
        /// Accepted for clarity; sync is always one-shot.
        #[arg(long)]
        once: bool,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Inspect or validate configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Local readiness and optional deep connectivity checks.
    Doctor {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        deep: bool,
        #[arg(long)]
        config: Option<PathBuf>,
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
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Validate config and report success/failure.
    Validate {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Print the config with secrets redacted.
    Show {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Print the config path that fbsy uses.
    Path,
    /// Run the interactive setup wizard.
    Setup {
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum DevicesCommand {
    /// Print configured devices without showing secrets.
    List {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Test one configured device connection.
    Test {
        code: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Read live data from a device (serial, firmware, user/finger/record counts).
    Info {
        code: String,
        /// Also list enrolled users (uid / id / name).
        #[arg(long)]
        users: bool,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum WebhookCommand {
    /// Send an empty event batch for one device.
    Test {
        code: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

// ── zkteco / hrms service groups ──────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct ZktecoArgs {
    #[command(subcommand)]
    pub command: ZktecoCommand,
}

#[derive(Debug, Subcommand)]
pub enum ZktecoCommand {
    /// Start the mock device server detached.
    Run {
        #[arg(long)]
        name: Option<String>,
        #[arg(short = 'p', long, default_value = "4370")]
        port: u16,
        #[arg(long, default_value = "5")]
        records: usize,
    },
}

#[derive(Debug, Args)]
pub struct HrmsArgs {
    #[command(subcommand)]
    pub command: HrmsCommand,
}

#[derive(Debug, Subcommand)]
pub enum HrmsCommand {
    /// Start the mock HRMS server detached.
    Run {
        #[arg(long)]
        name: Option<String>,
        #[arg(short = 'p', long, default_value = "8800")]
        port: u16,
    },
}

// ── hidden internal child entry point ─────────────────────────────────────────

#[derive(Debug, Args)]
pub struct ServiceRunArgs {
    /// Service kind: bridge | zkteco | hrms
    pub service: String,
    /// Remaining service-specific flags, parsed by the service itself.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub rest: Vec<String>,
}
