use serde::{Deserialize, Serialize};

/// What an agent can do to an element. The names follow Testing Library and Playwright, as
/// CrownUI's own `ui_act` does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Click,
    DoubleClick,
    Hover,
    Focus,
    TypeText { text: String },
    PressKey { keybind: String },
    Scroll { dx: f64, dy: f64 },
    SetValue { value: String },
}

impl Action {
    /// Whether the action puts text into the element.
    pub const fn types(&self) -> bool {
        matches!(self, Self::TypeText { .. } | Self::SetValue { .. })
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::DoubleClick => "double_click",
            Self::Hover => "hover",
            Self::Focus => "focus",
            Self::TypeText { .. } => "type_text",
            Self::PressKey { .. } => "press_key",
            Self::Scroll { .. } => "scroll",
            Self::SetValue { .. } => "set_value",
        }
    }
}
