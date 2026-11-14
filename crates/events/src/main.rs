//! `crownos-events tail`: the daemon's window and focus events, one JSON object per line, until
//! interrupted. A hook script reads this to react to the desktop.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crownos_computer_use_protocol::computer_use::{
    self as proto, FocusChanged, NotificationPosted, WindowClosed, WindowOpened,
};
use crownos_ipc::adapter::tokio::AsyncClient;
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "crownos-events", about, version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print events as they happen.
    Tail,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let Cli {
        command: Command::Tail,
    } = Cli::parse();
    let mut client =
        AsyncClient::connect(proto::SERVICE).context("is crownos-computer-use running?")?;
    client.subscribe::<WindowOpened>().await?;
    client.subscribe::<WindowClosed>().await?;
    client.subscribe::<FocusChanged>().await?;
    client.subscribe::<NotificationPosted>().await?;
    loop {
        tokio::select! {
            pumped = client.pump() => pumped.context("the daemon went away")?,
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
        let inner = client.client_mut();
        while let Some(event) = inner.next_event::<WindowOpened>() {
            println!(
                "{}",
                json!({ "event": "window_opened", "window": event.window })
            );
        }
        while let Some(event) = inner.next_event::<WindowClosed>() {
            println!(
                "{}",
                json!({ "event": "window_closed", "identifier": event.identifier })
            );
        }
        while let Some(event) = inner.next_event::<FocusChanged>() {
            println!(
                "{}",
                json!({ "event": "focus_changed", "identifier": event.identifier })
            );
        }
        while let Some(event) = inner.next_event::<NotificationPosted>() {
            println!(
                "{}",
                json!({ "event": "notification_posted", "app": event.app, "summary": event.summary })
            );
        }
    }
}
