//! CLI dispatch: route a parsed command into the application layer.

use anyhow::Result;

use crate::application;

use super::{
    args::Cli,
    command::{
        AtBridgeCommand, Command, ConfigCommand, DevicesCommand, HrmsCommand, RunService,
        WebhookCommand, ZktecoCommand,
    },
};

/// Dispatch a parsed command into the application layer.
pub fn run(cli: Cli) -> Result<()> {
    // No command -> show running services.
    let command = match cli.command {
        Some(command) => command,
        None => return application::service::show(),
    };

    match command {
        Command::Install => application::install::install(),
        Command::Uninstall => application::install::uninstall(),

        Command::Run(args) => dispatch_run(args.service),
        Command::Show => application::service::show(),
        Command::Close(sel) => application::service::close(&sel.service),
        Command::Status(sel) => application::service::status(&sel.service),
        Command::Logs(args) => application::service::logs(&args.service, args.lines, args.follow),

        Command::AtBridge(args) => dispatch_at_bridge(args.command),

        Command::Zkteco(args) => match args.command {
            ZktecoCommand::Run { port, records } => application::service::run_zkteco(port, records),
        },
        Command::Hrms(args) => match args.command {
            HrmsCommand::Run { port } => application::service::run_hrms(port),
        },

        Command::ServiceRun(args) => application::service::exec_internal(&args.service, &args.rest),
    }
}

fn dispatch_run(service: RunService) -> Result<()> {
    match service {
        RunService::AtBridge {
            config,
            interval,
            no_poll,
        } => application::service::run_at_bridge(config, interval, no_poll),
        RunService::Zkteco { port, records } => application::service::run_zkteco(port, records),
        RunService::Hrms { port } => application::service::run_hrms(port),
    }
}

fn dispatch_at_bridge(command: AtBridgeCommand) -> Result<()> {
    match command {
        AtBridgeCommand::Run {
            config,
            interval,
            no_poll,
        } => application::service::run_at_bridge(config, interval, no_poll),
        AtBridgeCommand::Sync { device, config, .. } => application::sync_once::run(config, device),
        AtBridgeCommand::Config { command } => match command {
            ConfigCommand::Validate { path } => application::config::validate(path),
            ConfigCommand::Show { path } => application::config::show(path),
            ConfigCommand::Path => application::config::path(),
            ConfigCommand::Setup { path } => match path {
                Some(path) => application::setup::run_at(path),
                None => application::setup::run(),
            },
        },
        AtBridgeCommand::Doctor { json, deep, config } => {
            application::doctor::run(config, json, deep)
        }
        AtBridgeCommand::Devices { command } => match command {
            DevicesCommand::List { path } => application::config::devices_list(path),
            DevicesCommand::Test { code, path } => application::doctor::device_test(path, &code),
        },
        AtBridgeCommand::Webhook { command } => match command {
            WebhookCommand::Test { code, path } => application::doctor::webhook_test(path, &code),
        },
    }
}
