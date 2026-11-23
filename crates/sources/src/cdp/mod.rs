//! The agent browser: a Chromium-based browser of its own, with a profile of its own, driven over
//! the DevTools protocol through a pipe. Chromium 136 and later ignore remote debugging on the
//! default profile, so the user's browser is never touched from here.

#![allow(
    clippy::indexing_slicing,
    reason = "indexing a serde_json::Value yields Null for a missing key, it never panics"
)]

mod browser;
mod client;
mod focus_check;
mod keys;
mod pipe;
mod tree;

use base64::Engine as _;
use crownos_ui_elements::{Action, Element};
use serde::Deserialize;
use serde_json::{Value, json};

pub use browser::{Browser, DEFAULT_EXECUTABLE, default_profile_dir};
pub use client::CdpClient;

use crate::ActResult;
use crate::error::SourceError;

/// One open page.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Tab {
    #[serde(rename = "targetId")]
    pub target_id: String,
    pub title: String,
    pub url: String,
}

/// Where an action lands inside the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ViewportPoint {
    x: f64,
    y: f64,
}

impl Browser {
    /// Every page the browser has open.
    ///
    /// # Errors
    ///
    /// Fails when the browser is gone.
    pub async fn tabs(&self) -> Result<Vec<Tab>, SourceError> {
        #[derive(Deserialize)]
        struct Info {
            #[serde(rename = "type")]
            kind: String,
            #[serde(flatten)]
            tab: Tab,
        }
        let result = self
            .client()
            .send("Target.getTargets", json!({}), None)
            .await?;
        let infos: Vec<Info> = serde_json::from_value(result["targetInfos"].clone())?;
        Ok(infos
            .into_iter()
            .filter(|info| info.kind == "page")
            .map(|info| info.tab)
            .collect())
    }

    /// Opens `url` in a new tab.
    ///
    /// # Errors
    ///
    /// Fails when the browser refuses the URL.
    pub async fn open(&self, url: &str) -> Result<Tab, SourceError> {
        let created = self
            .client()
            .send("Target.createTarget", json!({ "url": url }), None)
            .await?;
        let target_id = created["targetId"]
            .as_str()
            .ok_or_else(|| SourceError::Browser("createTarget gave no targetId".to_owned()))?
            .to_owned();
        Ok(Tab {
            target_id,
            title: String::new(),
            url: url.to_owned(),
        })
    }

    /// # Errors
    ///
    /// Fails when the tab is gone.
    pub async fn navigate(&self, target_id: &str, url: &str) -> Result<(), SourceError> {
        let session = self.session(target_id).await?;
        self.client()
            .send("Page.navigate", json!({ "url": url }), Some(&session))
            .await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Fails when the tab is gone.
    pub async fn close_tab(&self, target_id: &str) -> Result<(), SourceError> {
        self.client()
            .send("Target.closeTarget", json!({ "targetId": target_id }), None)
            .await?;
        self.forget_session(target_id).await;
        Ok(())
    }

    /// The page's accessibility tree joined with its layout, as elements.
    ///
    /// # Errors
    ///
    /// Fails when the tab is gone.
    pub async fn elements(&self, target_id: &str) -> Result<Vec<Element>, SourceError> {
        let session = self.session(target_id).await?;
        let ax = self
            .client()
            .send("Accessibility.getFullAXTree", json!({}), Some(&session))
            .await?;
        let dom = self
            .client()
            .send(
                "DOMSnapshot.captureSnapshot",
                json!({ "computedStyles": [], "includeDOMRects": true }),
                Some(&session),
            )
            .await?;
        Ok(tree::elements(&ax, &dom))
    }

    /// # Errors
    ///
    /// Fails when the element is gone from the page or the browser refuses.
    pub async fn act(
        &self,
        target_id: &str,
        element: &Element,
        action: &Action,
    ) -> Result<ActResult, SourceError> {
        let session = self.session(target_id).await?;
        let node: u64 = element
            .source_id
            .parse()
            .map_err(|_| SourceError::Unsupported("act on"))?;
        match action {
            Action::Click => {
                let point = self.center(&session, node).await?;
                self.click(&session, point, 1).await?;
            }
            Action::DoubleClick => {
                let point = self.center(&session, node).await?;
                self.click(&session, point, 1).await?;
                self.click(&session, point, 2).await?;
            }
            Action::Hover => {
                let point = self.center(&session, node).await?;
                self.mouse(&session, "mouseMoved", point, 0, "none").await?;
            }
            Action::Focus => self.focus(&session, node).await?,
            Action::TypeText { text } => {
                self.focus(&session, node).await?;
                self.insert_text(&session, text).await?;
            }
            Action::SetValue { value } => {
                self.focus(&session, node).await?;
                self.press_key(&session, "Ctrl+A").await?;
                self.insert_text(&session, value).await?;
            }
            Action::PressKey { keybind } => {
                self.focus(&session, node).await?;
                self.press_key(&session, keybind).await?;
            }
            Action::Scroll { dx, dy } => {
                let point = self.center(&session, node).await?;
                self.client()
                    .send(
                        "Input.dispatchMouseEvent",
                        json!({ "type": "mouseWheel", "x": point.x, "y": point.y, "deltaX": dx, "deltaY": dy }),
                        Some(&session),
                    )
                    .await?;
            }
        }
        Ok(ActResult::Done)
    }

    /// A PNG of the page's viewport.
    ///
    /// # Errors
    ///
    /// Fails when the tab is gone.
    pub async fn screenshot(&self, target_id: &str) -> Result<Vec<u8>, SourceError> {
        let session = self.session(target_id).await?;
        let shot = self
            .client()
            .send(
                "Page.captureScreenshot",
                json!({ "format": "png" }),
                Some(&session),
            )
            .await?;
        let data = shot["data"]
            .as_str()
            .ok_or_else(|| SourceError::Browser("captureScreenshot gave no data".to_owned()))?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| SourceError::Browser(error.to_string()))
    }

    async fn center(&self, session: &str, node: u64) -> Result<ViewportPoint, SourceError> {
        let client = self.client();
        client
            .send(
                "DOM.scrollIntoViewIfNeeded",
                json!({ "backendNodeId": node }),
                Some(session),
            )
            .await?;
        let model = client
            .send(
                "DOM.getBoxModel",
                json!({ "backendNodeId": node }),
                Some(session),
            )
            .await?;
        quad_center(&model["model"]["content"])
            .ok_or_else(|| SourceError::Browser("the element has no box".to_owned()))
    }

    async fn focus(&self, session: &str, node: u64) -> Result<(), SourceError> {
        self.client()
            .send("DOM.focus", json!({ "backendNodeId": node }), Some(session))
            .await?;
        Ok(())
    }

    async fn insert_text(&self, session: &str, text: &str) -> Result<(), SourceError> {
        self.client()
            .send("Input.insertText", json!({ "text": text }), Some(session))
            .await?;
        Ok(())
    }

    async fn click(
        &self,
        session: &str,
        point: ViewportPoint,
        count: u32,
    ) -> Result<(), SourceError> {
        self.mouse(session, "mouseMoved", point, 0, "none").await?;
        self.mouse(session, "mousePressed", point, count, "left")
            .await?;
        self.mouse(session, "mouseReleased", point, count, "left")
            .await
    }

    async fn mouse(
        &self,
        session: &str,
        kind: &str,
        point: ViewportPoint,
        click_count: u32,
        button: &str,
    ) -> Result<(), SourceError> {
        self.client()
            .send(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": point.x, "y": point.y, "button": button, "clickCount": click_count }),
                Some(session),
            )
            .await?;
        Ok(())
    }

    async fn press_key(&self, session: &str, keybind: &str) -> Result<(), SourceError> {
        let key = keys::parse(keybind).ok_or(SourceError::Unsupported("press that key on"))?;
        for kind in ["keyDown", "keyUp"] {
            let mut params = key.params();
            params["type"] = Value::from(kind);
            self.client()
                .send("Input.dispatchKeyEvent", params, Some(session))
                .await?;
        }
        Ok(())
    }
}

/// The middle of a DOM.getBoxModel quad: eight numbers, four corners.
fn quad_center(quad: &Value) -> Option<ViewportPoint> {
    let numbers: Vec<f64> = quad.as_array()?.iter().filter_map(Value::as_f64).collect();
    if numbers.len() != 8 {
        return None;
    }
    let xs: f64 = numbers.iter().step_by(2).sum();
    let ys: f64 = numbers.iter().skip(1).step_by(2).sum();
    Some(ViewportPoint {
        x: xs / 4.0,
        y: ys / 4.0,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_has_a_center() {
        let quad = json!([10.0, 10.0, 30.0, 10.0, 30.0, 20.0, 10.0, 20.0]);
        assert_eq!(quad_center(&quad), Some(ViewportPoint { x: 20.0, y: 15.0 }));
        assert_eq!(quad_center(&json!([1, 2])), None);
    }
}
