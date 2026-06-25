//! Binary entrypoint for the bridge executable.
//!
//! Keep this file tiny: process startup belongs here, while product behavior
//! belongs in the library modules under `src/`.

use clap::Parser;

fn main() {
    // Initialize structured logging before any command runs.
    tracing_subscriber::fmt::init();

    // `clap` converts terminal arguments into the typed `Cli` struct.
    let cli = zkteco_bridge::cli::Cli::parse();

    // The CLI layer dispatches to application use cases.
    if let Err(err) = zkteco_bridge::cli::run(cli) {
        eprintln!(
            "{} {}",
            console::style("Error:").red().bold(),
            console::style(err).red()
        );
        std::process::exit(1);
    }
}
