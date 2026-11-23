//! Whether a field that a secret went into is still on the page: the main frame has not
//! navigated, and the node is still in the document.

use serde_json::json;

use super::Browser;
use crate::error::SourceError;

const IS_CONNECTED: &str = "function() { return this.isConnected; }";

impl Browser {
    /// The id of the document that the tab's main frame shows; a navigation changes it.
    ///
    /// # Errors
    ///
    /// Fails when the tab is gone.
    pub async fn main_frame_loader_id(&self, target_id: &str) -> Result<String, SourceError> {
        let session = self.session(target_id).await?;
        let tree = self
            .client()
            .send("Page.getFrameTree", json!({}), Some(&session))
            .await?;
        tree["frameTree"]["frame"]["loaderId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| SourceError::Browser("getFrameTree gave no loaderId".to_owned()))
    }

    /// Whether the node with this backend id is still in its document. A node that cannot be
    /// found counts as removed.
    ///
    /// # Errors
    ///
    /// Fails when the tab is gone.
    pub async fn node_is_connected(
        &self,
        target_id: &str,
        backend_node_id: &str,
    ) -> Result<bool, SourceError> {
        let session = self.session(target_id).await?;
        let Ok(node) = backend_node_id.parse::<u64>() else {
            return Ok(false);
        };
        let Ok(resolved) = self
            .client()
            .send(
                "DOM.resolveNode",
                json!({ "backendNodeId": node }),
                Some(&session),
            )
            .await
        else {
            return Ok(false);
        };
        let Some(object_id) = resolved["object"]["objectId"].as_str() else {
            return Ok(false);
        };
        let checked = self
            .client()
            .send(
                "Runtime.callFunctionOn",
                json!({ "objectId": object_id, "functionDeclaration": IS_CONNECTED, "returnByValue": true }),
                Some(&session),
            )
            .await?;
        Ok(checked["result"]["value"].as_bool() == Some(true))
    }
}
