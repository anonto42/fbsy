//! CLI dispatch: route a parsed command into the application layer.

use anyhow::Result;

use crate::application;

use super::{
    args::Cli,
    command::{
        AtBridgeCommand, Command, ConfigCommand, DevicesCommand, HrmsCommand, RunService,
        ScannerCommand, ScannerScanArgs, WebhookCommand, ZktecoCommand,
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
        Command::Update(args) => application::update::run(application::update::UpdateOpts {
            check_only: args.check,
            assume_yes: args.yes,
            auto: args.auto,
        }),

        Command::Run(args) => dispatch_run(args.service),
        Command::Enable(args) => application::autostart::enable(&args.name, args.config),
        Command::Disable(args) => application::autostart::disable(&args.name),
        Command::Dashboard => application::dashboard::run(),
        Command::Show => application::service::show(),
        Command::Close(sel) => application::service::close(&sel.service),
        Command::Status(sel) => application::service::status(&sel.service),
        Command::Logs(args) => application::service::logs(&args.service, args.lines, args.follow),

        Command::AtBridge(args) => dispatch_at_bridge(args.command),

        Command::Zkteco(args) => match args.command {
            ZktecoCommand::Run {
                name,
                port,
                records,
            } => application::service::run_zkteco(name, port, records),
        },
        Command::Hrms(args) => match args.command {
            HrmsCommand::Run { name, port } => application::service::run_hrms(name, port),
        },
        Command::Scanner(args) => match args.command {
            ScannerCommand::Scan(args) => application::scanner::run_scan(scan_options(&args)),
            ScannerCommand::Run(args) => application::service::run_scanner(
                args.name,
                args.interval,
                scan_options(&args.scan),
            ),
        },

        Command::ServiceRun(args) => application::service::exec_internal(&args.service, &args.rest),
        Command::ServiceSupervised(args) => {
            application::service::run_supervised(&args.service, &args.rest)
        }
    }
}

fn dispatch_run(service: RunService) -> Result<()> {
    match service {
        RunService::AtBridge {
            name,
            config,
            interval,
            no_poll,
        } => application::service::run_at_bridge(name, config, interval, no_poll),
        RunService::Zkteco {
            name,
            port,
            records,
        } => application::service::run_zkteco(name, port, records),
        RunService::Hrms { name, port } => application::service::run_hrms(name, port),
        RunService::Scanner {
            name,
            cidr,
            host,
            port,
            interval,
            timeout_ms,
            device_timeout,
            password,
            udp,
            include_open,
        } => application::service::run_scanner(
            name,
            interval,
            application::scanner::ScanOptions {
                cidr,
                hosts: host,
                port,
                scan_timeout_ms: timeout_ms,
                device_timeout_secs: device_timeout,
                device_password: password,
                force_udp: udp,
                include_open,
                json: false,
            },
        ),
    }
}

fn dispatch_at_bridge(command: AtBridgeCommand) -> Result<()> {
    match command {
        AtBridgeCommand::Run {
            name,
            config,
            interval,
            no_poll,
        } => application::service::run_at_bridge(name, config, interval, no_poll),
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
            DevicesCommand::Info { code, users, path } => {
                application::doctor::device_info(path, &code, users)
            }
        },
        AtBridgeCommand::Webhook { command } => match command {
            WebhookCommand::Test { code, path } => application::doctor::webhook_test(path, &code),
        },
    }
}

fn scan_options(args: &ScannerScanArgs) -> application::scanner::ScanOptions {
    application::scanner::ScanOptions {
        cidr: args.cidr.clone(),
        hosts: args.host.clone(),
        port: args.port,
        scan_timeout_ms: args.timeout_ms,
        device_timeout_secs: args.device_timeout,
        device_password: args.password,
        force_udp: args.udp,
        include_open: args.include_open,
        json: args.json,
    }
}
