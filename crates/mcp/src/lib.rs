//! MCP on stdin and stdout, forwarding to the daemon. One instance per agent: the agent's name
//! comes from `CROWNOS_AGENT_NAME` (default `agent`) and its cursor colour from
//! `CROWNOS_AGENT_COLOR` as `RRGGBB`.

mod bridge;
mod tools;

use anyhow::{Context, Result};
use rmcp::ServiceExt;

pub use bridge::Bridge;

/// Serves until the agent closes stdin.
///
/// # Errors
///
/// Fails when the daemon is not running or stdio breaks.
pub async fn run_stdio() -> Result<()> {
    let name = std::env::var("CROWNOS_AGENT_NAME").unwrap_or_else(|_| "agent".to_owned());
    let color = std::env::var("CROWNOS_AGENT_COLOR")
        .ok()
        .and_then(|hex| u32::from_str_radix(hex.trim_start_matches('#'), 16).ok())
        .unwrap_or(0x7A_A2_FF);
    let bridge = Bridge::connect(&name, color)
        .await
        .context("reaching crownos-computer-use")?;
    let service = bridge
        .serve(rmcp::transport::stdio())
        .await
        .context("starting the MCP server")?;
    service.waiting().await.context("serving MCP")?;
    Ok(())
}
