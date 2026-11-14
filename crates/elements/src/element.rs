use serde::{Deserialize, Serialize};

use crate::refs::ElementRef;

/// Where elements came from, in the order the daemon prefers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    CrownUi,
    Atspi,
    Browser,
    Ocr,
}

/// A rectangle in the window's logical coordinates, origin at its top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Bounds {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    pub const fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }
}

/// The compositor's identifier of a toplevel, or a browser tab as `browser:<target>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WindowHandle(pub String);

/// One thing on screen an agent can read or act on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub reference: ElementRef,
    /// snake_case: `button`, `text_field`, `link`, `menu_item`.
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub bounds: Bounds,
    /// The actions the source can perform itself; empty elements are reached by pointer.
    pub actions: Vec<String>,
    /// Holds a secret: its value is never exposed and nothing is typed into it.
    pub secure: bool,
    pub source: Source,
    /// The source's own id for the element, opaque outside the source.
    pub source_id: String,
    pub depth: u8,
}

impl Element {
    /// Whether the element is something an agent operates, as opposed to reads.
    pub fn is_interactive(&self) -> bool {
        !self.actions.is_empty() || INTERACTIVE_ROLES.contains(&self.role.as_str())
    }
}

const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "text_field",
    "textbox",
    "searchbox",
    "checkbox",
    "radio",
    "radio_button",
    "switch",
    "slider",
    "combobox",
    "combo_box",
    "list_item",
    "menu_item",
    "tab",
    "option",
    "spin_button",
    "toggle_button",
    "push_button",
    "entry",
    "password_text",
    "check_box",
];
