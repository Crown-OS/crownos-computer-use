//! The accessibility tree joined with the DOM snapshot: roles and names from the first, boxes
//! and password fields from the second, matched on the backend node id both carry.

use std::collections::HashMap;

use crownos_ui_elements::{Bounds, Element, ElementRef, SnapshotId, Source};
use serde_json::Value;

use crate::util::role::snake_case;

const CLICKABLE: &[&str] = &[
    "button",
    "link",
    "checkbox",
    "radio",
    "switch",
    "tab",
    "menu_item",
    "menuitem",
    "option",
    "combobox",
    "slider",
    "menu_item_checkbox",
    "menu_item_radio",
];
const EDITABLE: &[&str] = &[
    "textbox",
    "searchbox",
    "text_field",
    "combobox",
    "spinbutton",
];

struct DomFacts {
    bounds: Bounds,
    secure: bool,
}

pub(super) fn elements(ax: &Value, dom: &Value) -> Vec<Element> {
    let facts = dom_facts(dom);
    let nodes: Vec<&Value> = ax["nodes"]
        .as_array()
        .map(|nodes| nodes.iter().collect())
        .unwrap_or_default();
    let by_id: HashMap<&str, &Value> = nodes
        .iter()
        .filter_map(|node| Some((node["nodeId"].as_str()?, *node)))
        .collect();
    let Some(root) = nodes.first() else {
        return Vec::new();
    };
    let mut elements = Vec::new();
    let mut stack = vec![(*root, 0u8)];
    while let Some((node, depth)) = stack.pop() {
        let ignored = node["ignored"].as_bool().unwrap_or(false);
        let child_depth = if ignored {
            depth
        } else {
            depth.saturating_add(1)
        };
        if !ignored {
            elements.push(element(node, depth, &facts));
        }
        let children = node["childIds"].as_array();
        for child in children.into_iter().flatten().rev() {
            if let Some(child) = child.as_str().and_then(|id| by_id.get(id)) {
                stack.push((child, child_depth));
            }
        }
    }
    elements
}

fn element(node: &Value, depth: u8, facts: &HashMap<u64, DomFacts>) -> Element {
    let role = snake_case(node["role"]["value"].as_str().unwrap_or("generic"));
    let backend = node["backendDOMNodeId"].as_u64();
    let dom = backend.and_then(|id| facts.get(&id));
    let secure = dom.is_some_and(|facts| facts.secure);
    let mut actions = Vec::new();
    if CLICKABLE.contains(&role.as_str()) {
        actions.push("click".to_owned());
    }
    if EDITABLE.contains(&role.as_str()) {
        actions.extend(["focus".to_owned(), "type_text".to_owned()]);
    }
    Element {
        reference: ElementRef {
            snapshot: SnapshotId(0),
            index: 0,
        },
        role,
        name: node["name"]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        value: if secure {
            None
        } else {
            node["value"]["value"].as_str().map(str::to_owned)
        },
        bounds: dom.map(|facts| facts.bounds).unwrap_or_default(),
        actions,
        secure,
        source: Source::Browser,
        source_id: backend.map(|id| id.to_string()).unwrap_or_default(),
        depth,
    }
}

/// Per backend node id: its layout box in viewport coordinates, and whether it is a password
/// field.
fn dom_facts(dom: &Value) -> HashMap<u64, DomFacts> {
    let mut facts = HashMap::new();
    let strings: Vec<&str> = dom["strings"]
        .as_array()
        .map(|strings| strings.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let string = |index: &Value| {
        index
            .as_i64()
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| strings.get(index).copied())
    };
    for document in dom["documents"].as_array().into_iter().flatten() {
        let nodes = &document["nodes"];
        let backend_ids: Vec<Option<u64>> = nodes["backendNodeId"]
            .as_array()
            .map(|ids| ids.iter().map(Value::as_u64).collect())
            .unwrap_or_default();
        let names: Vec<Option<&str>> = nodes["nodeName"]
            .as_array()
            .map(|names| names.iter().map(string).collect())
            .unwrap_or_default();
        let attributes: Vec<Option<&Vec<Value>>> = nodes["attributes"]
            .as_array()
            .map(|all| all.iter().map(Value::as_array).collect())
            .unwrap_or_default();
        let scroll_x = document["scrollOffsetX"].as_f64().unwrap_or_default();
        let scroll_y = document["scrollOffsetY"].as_f64().unwrap_or_default();
        let layout = &document["layout"];
        let indices = layout["nodeIndex"].as_array().cloned().unwrap_or_default();
        let boxes = layout["bounds"].as_array().cloned().unwrap_or_default();
        for (index, rect) in indices.iter().zip(&boxes) {
            let Some(node_index) = index.as_u64().and_then(|index| usize::try_from(index).ok())
            else {
                continue;
            };
            let Some(backend) = backend_ids.get(node_index).copied().flatten() else {
                continue;
            };
            let numbers: Vec<f64> = rect
                .as_array()
                .map(|rect| rect.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            let [x, y, width, height] = numbers.as_slice() else {
                continue;
            };
            let secure = names.get(node_index).copied().flatten() == Some("INPUT")
                && attributes
                    .get(node_index)
                    .copied()
                    .flatten()
                    .is_some_and(|attributes| is_password(attributes, &string));
            #[allow(clippy::cast_possible_truncation, reason = "layout boxes fit a screen")]
            let bounds = Bounds::new(
                (x - scroll_x).round() as i32,
                (y - scroll_y).round() as i32,
                width.round() as i32,
                height.round() as i32,
            );
            facts.insert(backend, DomFacts { bounds, secure });
        }
    }
    facts
}

fn is_password<'a>(attributes: &[Value], string: &impl Fn(&Value) -> Option<&'a str>) -> bool {
    attributes.chunks_exact(2).any(|pair| {
        let (Some(name), Some(value)) = (pair.first(), pair.get(1)) else {
            return false;
        };
        string(name) == Some("type")
            && string(value).is_some_and(|value| value.eq_ignore_ascii_case("password"))
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
    use serde_json::json;

    use super::*;

    #[test]
    fn ax_nodes_take_boxes_and_secrecy_from_the_dom_snapshot() {
        let ax = json!({ "nodes": [
            {"nodeId": "1", "role": {"value": "RootWebArea"}, "name": {"value": "Login"}, "childIds": ["2", "3"], "backendDOMNodeId": 1},
            {"nodeId": "2", "role": {"value": "button"}, "name": {"value": "Sign in"}, "childIds": [], "backendDOMNodeId": 7},
            {"nodeId": "3", "role": {"value": "textbox"}, "name": {"value": "Password"}, "value": {"value": "hunter2"}, "childIds": [], "backendDOMNodeId": 9}
        ]});
        let dom = json!({
            "strings": ["INPUT", "type", "password", "BUTTON"],
            "documents": [{
                "scrollOffsetX": 0, "scrollOffsetY": 100,
                "nodes": { "backendNodeId": [1, 7, 9], "nodeName": [3, 3, 0], "attributes": [[], [], [1, 2]] },
                "layout": { "nodeIndex": [1, 2], "bounds": [[10, 120, 80, 30], [10, 200, 200, 24]] }
            }]
        });
        let elements = elements(&ax, &dom);
        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0].role, "root_web_area");
        assert_eq!(elements[1].bounds, Bounds::new(10, 20, 80, 30));
        assert_eq!(elements[1].actions, vec!["click"]);
        assert_eq!(elements[1].depth, 1);
        assert!(elements[2].secure);
        assert_eq!(elements[2].value, None);
        assert_eq!(elements[2].source_id, "9");
    }
}
