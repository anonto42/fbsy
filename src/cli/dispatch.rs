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
        // Bare `fbsy` opens the dashboard. When there is no interactive
        // terminal (scripts, pipes), fall back to printing help instead.
        None => {
            if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
                return application::dashboard::run();
            }
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

        Command::ServiceRun(args) => application::service::exec_internal(&args.service, &args.rest),
        Command::ServiceSupervised(args) => {
            application::service::run_supervised(&args.service, &args.rest)
        }
    }
}
