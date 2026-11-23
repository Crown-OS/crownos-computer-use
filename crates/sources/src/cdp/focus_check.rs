//! Bringing a tab to the front and checking where its keyboard focus is, before the compositor
//! types into it.

use crownos_ui_elements::Element;
use serde_json::json;

use super::Browser;
use crate::error::SourceError;

const HOLDS_FOCUS: &str = "function() { return this.ownerDocument.hasFocus() && this.ownerDocument.activeElement === this; }";

impl Browser {
    /// Makes the tab the selected one of its window.
    ///
    /// # Errors
    ///
    /// Fails when the tab is gone.
    pub async fn bring_to_front(&self, target_id: &str) -> Result<(), SourceError> {
        let session = self.session(target_id).await?;
        self.client()
            .send("Page.bringToFront", json!({}), Some(&session))
            .await?;
        Ok(())
    }

    /// Focuses the element and tells whether it now has the keyboard focus of a focused page.
    ///
    /// # Errors
    ///
    /// Fails when the element is gone from the page.
    pub async fn focus_and_check(
        &self,
        target_id: &str,
        element: &Element,
    ) -> Result<bool, SourceError> {
        let session = self.session(target_id).await?;
        let node: u64 = element
            .source_id
            .parse()
            .map_err(|_| SourceError::Unsupported("focus"))?;
        self.client()
            .send(
                "DOM.focus",
                json!({ "backendNodeId": node }),
                Some(&session),
            )
            .await?;
        let resolved = self
            .client()
            .send(
                "DOM.resolveNode",
                json!({ "backendNodeId": node }),
                Some(&session),
            )
            .await?;
        let object_id = resolved["object"]["objectId"]
            .as_str()
            .ok_or_else(|| SourceError::Browser("resolveNode gave no objectId".to_owned()))?;
        let checked = self
            .client()
            .send(
                "Runtime.callFunctionOn",
                json!({ "objectId": object_id, "functionDeclaration": HOLDS_FOCUS, "returnByValue": true }),
                Some(&session),
            )
            .await?;
        Ok(checked["result"]["value"].as_bool() == Some(true))
    }
}
