//! `cdp_dump <profile-dir> <url>`: opens the agent browser on a profile, loads a page, prints
//! its elements, takes a screenshot and closes the browser again.

use std::path::PathBuf;

use anyhow::{Context, Result};
use crownos_ui_elements::Action;
use crownos_ui_sources::cdp::Browser;

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let profile: PathBuf = arguments
        .next()
        .context("usage: cdp_dump <profile-dir> <url>")?
        .into();
    let url = arguments
        .next()
        .context("usage: cdp_dump <profile-dir> <url>")?;
    let browser = Browser::launch(&profile, &["--headless=new".to_owned()]).await?;
    let tab = browser.open(&url).await?;
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let elements = browser.elements(&tab.target_id).await?;
    for element in &elements {
        println!(
            "{}{} {:?} {:?} {:?} secure={}",
            "  ".repeat(usize::from(element.depth)),
            element.role,
            element.name,
            element.bounds,
            element.actions,
            element.secure
        );
    }
    if let Some(field) = elements
        .iter()
        .find(|element| element.role == "textbox" && !element.secure)
    {
        browser
            .act(
                &tab.target_id,
                field,
                &Action::TypeText {
                    text: "hello".into(),
                },
            )
            .await?;
        println!("typed into {:?}", field.name);
    }
    let png = browser.screenshot(&tab.target_id).await?;
    println!(
        "screenshot: {} bytes, tabs: {}",
        png.len(),
        browser.tabs().await?.len()
    );
    browser.close().await;
    Ok(())
}
