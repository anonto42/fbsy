//! Binary entrypoint for the bridge executable.
//!
//! Keep this file tiny: process startup belongs here, while product behavior
//! belongs in the library modules under `src/`.

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    // Initialize structured logging before any command runs.
    tracing_subscriber::fmt::init();

    // `clap` converts terminal arguments into the typed `Cli` struct.
    let cli = zkteco_bridge::cli::Cli::parse();

    // The CLI layer dispatches to application use cases.
    zkteco_bridge::cli::run(cli)
}
