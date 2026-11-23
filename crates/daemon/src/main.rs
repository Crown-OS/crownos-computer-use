//! `crownos-computer-use`: the one process the compositor trusts to see and drive windows for
//! agents. `crownos-computer-use mcp` is the stdio bridge an agent launches to reach it.

mod agents;
mod browser_choice;
mod daemon;
mod resolve;
mod secret_typing;
mod server;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "crownos-computer-use", about, version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the daemon (the default).
    Serve,
    /// Speak MCP on stdin and stdout for one agent, forwarding to the daemon.
    Mcp,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
                )
                .init();
            server::run().await
        }
        Command::Mcp => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
                )
                .with_writer(std::io::stderr)
                .init();
            crownos_computer_use_mcp::run_stdio().await
        }
    }
}
