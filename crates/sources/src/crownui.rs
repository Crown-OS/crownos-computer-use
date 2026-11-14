//! A CrownUI app over its own MCP socket: the semantic tree it exposes to screen readers and
//! tests, with stable ids and actions that run through the app's real input path.

use std::path::PathBuf;

use crownos_ui_elements::{Action, Bounds, Element, ElementRef, SnapshotId, Source};
use mcp::{AppEndpoint, default_socket_dir};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ReadResourceRequestParams, ResourceContents,
};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ActResult;
use crate::error::SourceError;
use crate::util::role::snake_case;

const TREE_JSON: &str = "ui://tree.json";
const UI_ACT: &str = "ui_act";

/// The sockets of running CrownUI apps.
#[derive(Debug, Clone)]
pub struct CrownUiApps {
    socket_dir: PathBuf,
}

impl Default for CrownUiApps {
    fn default() -> Self {
        Self {
            socket_dir: default_socket_dir(),
        }
    }
}

impl CrownUiApps {
    pub fn at(socket_dir: impl Into<PathBuf>) -> Self {
        Self {
            socket_dir: socket_dir.into(),
        }
    }

    /// The MCP socket of the app running as `pid`, if it opened one.
    pub fn endpoint_for_pid(&self, pid: u32) -> Option<AppEndpoint> {
        std::fs::read_dir(&self.socket_dir)
            .ok()?
            .filter_map(|entry| AppEndpoint::from_path(entry.ok()?.path()))
            .find(|endpoint| endpoint.pid == pid)
    }
}

/// One connected app.
pub struct CrownUiSource {
    client: RunningService<RoleClient, ()>,
    revision: u64,
}

impl std::fmt::Debug for CrownUiSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CrownUiSource")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct Tree {
    revision: u64,
    root: Option<u64>,
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: u64,
    role: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    states: States,
    #[serde(default)]
    actions: Vec<String>,
    #[serde(default)]
    bounds: Value,
    #[serde(default)]
    children: Vec<u64>,
}

#[derive(Deserialize, Default)]
struct States {
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    offscreen: bool,
}

impl CrownUiSource {
    /// # Errors
    ///
    /// Fails when the socket is gone or the app refuses the token.
    pub async fn connect(endpoint: &AppEndpoint, token: Option<&str>) -> Result<Self, SourceError> {
        let stream = mcp::connect(&endpoint.path, token).await?;
        let client = ().serve(stream).await.map_err(|error| SourceError::Mcp(error.to_string()))?;
        Ok(Self {
            client,
            revision: 0,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    /// The app's semantic tree, in reading order with depth.
    ///
    /// # Errors
    ///
    /// Fails when the connection dropped or the tree does not parse.
    pub async fn elements(&mut self) -> Result<Vec<Element>, SourceError> {
        let params: ReadResourceRequestParams =
            serde_json::from_value(json!({ "uri": TREE_JSON }))?;
        let result = self
            .client
            .peer()
            .read_resource(params)
            .await
            .map_err(|error| SourceError::Mcp(error.to_string()))?;
        let text = result
            .contents
            .iter()
            .find_map(|contents| match contents {
                ResourceContents::TextResourceContents { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .ok_or_else(|| SourceError::Mcp("ui://tree.json has no text".to_owned()))?;
        let tree: Tree = serde_json::from_str(text)?;
        self.revision = tree.revision;
        Ok(flatten(&tree))
    }

    /// Runs `action` on the element through `ui_act`, at the revision the elements were read at.
    ///
    /// # Errors
    ///
    /// Fails when the app refuses, which includes a stale revision; the caller then snapshots
    /// again.
    pub async fn act(
        &mut self,
        element: &Element,
        action: &Action,
    ) -> Result<ActResult, SourceError> {
        let id: u64 = element
            .source_id
            .parse()
            .map_err(|_| SourceError::Unsupported("act on"))?;
        let params: CallToolRequestParams = serde_json::from_value(json!({
            "name": UI_ACT,
            "arguments": { "id": id, "revision": self.revision, "action": action_json(action) },
        }))?;
        let result: CallToolResult = self
            .client
            .peer()
            .call_tool(params)
            .await
            .map_err(|error| SourceError::Mcp(error.to_string()))?;
        if result.is_error == Some(true) {
            return Err(SourceError::Mcp(error_text(&result)));
        }
        if let Some(revision) = result
            .structured_content
            .as_ref()
            .and_then(|value| value.get("revision"))
            .and_then(Value::as_u64)
        {
            self.revision = revision;
        }
        Ok(ActResult::Done)
    }
}

fn error_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn action_json(action: &Action) -> Value {
    match action {
        Action::Scroll { dx, dy } => json!({ "action": "scroll", "delta": { "x": dx, "y": dy } }),
        Action::SetValue { value } => json!({ "action": "type_text", "text": value }),
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    }
}

fn flatten(tree: &Tree) -> Vec<Element> {
    let mut elements = Vec::with_capacity(tree.nodes.len());
    let by_id = |id: u64| tree.nodes.iter().find(|node| node.id == id);
    let mut stack: Vec<(u64, u8)> = tree.root.map(|root| (root, 0)).into_iter().collect();
    while let Some((id, depth)) = stack.pop() {
        let Some(node) = by_id(id) else {
            continue;
        };
        if !node.states.offscreen {
            elements.push(element(node, depth));
        }
        for child in node.children.iter().rev() {
            stack.push((*child, depth.saturating_add(1)));
        }
    }
    elements
}

fn element(node: &Node, depth: u8) -> Element {
    Element {
        reference: ElementRef {
            snapshot: SnapshotId(0),
            index: 0,
        },
        role: snake_case(&node.role),
        name: node.name.clone().unwrap_or_default(),
        value: if node.states.secure {
            None
        } else {
            node.value.clone()
        },
        bounds: bounds(&node.bounds),
        actions: node
            .actions
            .iter()
            .map(|action| snake_case(action))
            .collect(),
        secure: node.states.secure,
        source: Source::CrownUi,
        source_id: node.id.to_string(),
        depth,
    }
}

/// `{x, y, width, height}` or `{origin: {x, y}, size: {width, height}}`, whichever the app's
/// geometry crate serializes.
fn bounds(value: &Value) -> Bounds {
    let number = |value: Option<&Value>| value.and_then(Value::as_f64).unwrap_or_default();
    let (origin, size) = match (value.get("origin"), value.get("size")) {
        (Some(origin), Some(size)) => (origin, size),
        _ => (value, value),
    };
    let round = |value: f64| {
        value
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX))
    };
    #[allow(clippy::cast_possible_truncation, reason = "clamped to i32 first")]
    Bounds::new(
        round(number(origin.get("x"))) as i32,
        round(number(origin.get("y"))) as i32,
        round(number(size.get("width"))) as i32,
        round(number(size.get("height"))) as i32,
    )
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
    fn the_tree_flattens_in_reading_order_with_depth() {
        let tree: Tree = serde_json::from_value(json!({
            "revision": 4,
            "root": 1,
            "nodes": [
                {"id": 1, "role": "Window", "name": "Notes", "states": {}, "actions": [], "bounds": {"origin": {"x": 0, "y": 0}, "size": {"width": 800, "height": 600}}, "children": [2, 3]},
                {"id": 2, "role": "Button", "name": "Save", "states": {}, "actions": ["Click"], "bounds": {"x": 10, "y": 10, "width": 80, "height": 30}, "children": []},
                {"id": 3, "role": "TextField", "name": "Password", "value": "hunter2", "states": {"secure": true}, "actions": ["TypeText"], "bounds": {}, "children": []}
            ]
        }))
        .unwrap();
        let elements = flatten(&tree);
        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0].bounds, Bounds::new(0, 0, 800, 600));
        assert_eq!(elements[1].role, "button");
        assert_eq!(elements[1].actions, vec!["click"]);
        assert_eq!(elements[1].depth, 1);
        assert_eq!(elements[2].role, "text_field");
        assert!(elements[2].secure);
        assert_eq!(elements[2].value, None);
    }

    #[test]
    fn actions_take_crownui_spellings() {
        assert_eq!(
            action_json(&Action::Scroll { dx: 0.0, dy: 120.0 }),
            json!({"action": "scroll", "delta": {"x": 0.0, "y": 120.0}})
        );
        assert_eq!(
            action_json(&Action::PressKey {
                keybind: "Ctrl+A".into()
            }),
            json!({"action": "press_key", "keybind": "Ctrl+A"})
        );
    }
}
