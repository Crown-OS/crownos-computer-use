//! Whether a field that a secret went into is still on its page: the main frame has not
//! navigated.

use serde_json::json;

use super::Browser;
use crate::error::SourceError;

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
}
