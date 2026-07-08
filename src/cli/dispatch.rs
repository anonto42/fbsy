//! CLI dispatch: route a parsed command into the application layer.

use std::io::IsTerminal;

use anyhow::Result;
use clap::CommandFactory;

use crate::application;

use super::{args::Cli, command::Command};

/// Dispatch a parsed command into the application layer.
pub fn run(cli: Cli) -> Result<()> {
    let command = match cli.command {
        Some(command) => command,
        // Bare `fbsy` shows help text. Previously defaulted to dashboard in interactive
        // terminals, but changed to be consistent with CLI best practices.
        None => {
            let mut cmd = Cli::command();
            cmd.print_help()?;
            println!();
            return Ok(());
        }
    };

    match command {
        Command::Install => application::install::install(),
        Command::Uninstall(args) => application::install::uninstall(&args),
        Command::Update(args) => application::update::run(application::update::UpdateOpts {
            check_only: args.check,
            assume_yes: args.yes,
            auto: args.auto,
        }),
        Command::Dashboard => application::dashboard::run(),
        Command::Setup => application::setup::run(),
        Command::Start => {
            let pid = application::service::default_start(crate::services::ServiceKind::AtBridge)?;
            println!("started bridge (pid {pid})");
            Ok(())
        }
        Command::Stop => {
            if application::service::stop_instance("bridge")? {
                println!("stopped bridge");
            } else {
                println!("bridge was not running");
            }
            Ok(())
        }
        Command::Restart => {
            let pid = match application::service::restart_instance("bridge") {
                Ok(pid) => pid,
                // Not running yet: restart degrades to a plain start.
                Err(_) => {
                    application::service::default_start(crate::services::ServiceKind::AtBridge)?
                }
            };
            println!("bridge running (pid {pid})");
            Ok(())
        }
        Command::Sync(args) => application::sync_once::run(args.config, args.device),
        Command::Status => application::service::show(),
        Command::Logs(args) => application::service::logs("bridge", args.lines, args.follow),

        Command::ServiceRun(args) => application::service::exec_internal(&args.service, &args.rest),
        Command::ServiceSupervised(args) => {
            application::service::run_supervised(&args.service, &args.rest)
        }
    }
}
