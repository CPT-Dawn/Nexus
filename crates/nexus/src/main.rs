mod backend;
mod cli;
mod config;
mod daemon;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "nexus", version = VERSION, about = "A network manager for Wayland desktops.")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the overall network state and the primary Link.
    Status {
        /// Print the status object as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run the Daemon in the foreground (systemd socket activation also uses this).
    Daemon,
}

fn main() -> ExitCode {
    match Args::parse().command {
        Command::Status { json } => cli::status(json),
        Command::Daemon => daemon::run(),
    }
}
