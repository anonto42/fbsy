//! CLI dispatch.
//!
//! This file answers: "After parsing the command, which application use case
//! should run?"

use anyhow::Result;

use crate::application;

use super::{
    args::Cli,
    command::{
        AutostartCommand, Command, ConfigCommand, DevicesCommand, LogsCommand, WebhookCommand,
    },
};

/// Dispatch a parsed command into the application layer.
pub fn run(cli: Cli) -> Result<()> {
    // Compatibility flags are checked first so old client scripts continue to work.
    if cli.setup {
        return application::setup::run();
    }
    if cli.once {
        return application::sync_once::run(None, cli.device);
    }
    if cli.install_autostart {
        return application::autostart::install();
    }
    if cli.uninstall_autostart {
        return application::autostart::uninstall();
    }

    // If no command is given, behave like a friendly product shell.
    match cli.command.unwrap_or(Command::Doctor {
        json: false,
        deep: false,
        config: None,
    }) {
        Command::Doctor { json, deep, config } => application::doctor::run(config, json, deep),
        Command::Setup => application::setup::run(),
        Command::Once { device, config } => application::sync_once::run(config, device),
        Command::Serve { interval, config } => application::serve::run(interval, config),
        Command::Config { command } => match command {
            ConfigCommand::Validate { path } => application::config::validate(path),
            ConfigCommand::Show { path } => application::config::show(path),
            ConfigCommand::Path => application::config::path(),
        },
        Command::Devices { command } => match command {
            DevicesCommand::List { path } => application::config::devices_list(path),
            DevicesCommand::Test { code, path } => application::doctor::device_test(path, &code),
        },
        Command::Webhook { command } => match command {
            WebhookCommand::Test { code, path } => application::doctor::webhook_test(path, &code),
        },
        Command::Logs { command } => match command {
            LogsCommand::Path => application::doctor::logs_path(),
        },
        Command::Autostart { command } => match command {
            AutostartCommand::Install => application::autostart::install(),
            AutostartCommand::Uninstall => application::autostart::uninstall(),
        },
    }
}
