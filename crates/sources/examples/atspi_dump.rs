//! `atspi_dump <pid>`: the accessible tree of one process, as elements.

use anyhow::{Context, Result};
use crownos_ui_sources::atspi::AtspiSource;

#[tokio::main]
async fn main() -> Result<()> {
    let pid: u32 = std::env::args()
        .nth(1)
        .context("usage: atspi_dump <pid>")?
        .parse()?;
    let mut source = AtspiSource::connect().await?;
    for element in source.elements(pid).await? {
        println!(
            "{}{} {:?} {:?} {:?}",
            "  ".repeat(usize::from(element.depth)),
            element.role,
            element.name,
            element.bounds,
            element.actions
        );
    }
    Ok(())
}
