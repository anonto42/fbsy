//! CLI boundary.
//!
//! This module should parse commands and call application use cases. It should
//! not implement device protocol, webhook forwarding, or sync policy.

mod args;
mod command;
mod dispatch;

use anyhow::Result;

pub use args::Cli;
pub use command::{Command, UninstallArgs};

/// Run the parsed CLI command.
pub fn run(cli: Cli) -> Result<()> {
    dispatch::run(cli)
}
