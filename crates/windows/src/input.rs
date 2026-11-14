use serde::{Deserialize, Serialize};

/// One of the agent cursors this connection created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CursorId(pub u32);

/// One event of a frame. Positions are logical pixels; absolute ones name the output they are
/// on, or the first output.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    PointerMotion {
        dx: f64,
        dy: f64,
    },
    PointerTo {
        output: Option<String>,
        x: f64,
        y: f64,
    },
    Button {
        code: u32,
        pressed: bool,
    },
    Scroll {
        vertical: bool,
        value: f64,
    },
    Key {
        code: u32,
        pressed: bool,
    },
}

/// Events applied together as one hardware event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InputFrame {
    pub events: Vec<InputEvent>,
    pub time_ms: u32,
}

/// evdev `BTN_LEFT`.
pub const BTN_LEFT: u32 = 0x110;
/// evdev `BTN_RIGHT`.
pub const BTN_RIGHT: u32 = 0x111;

impl InputFrame {
    /// A press and release of `button` at `(x, y)` on `output`.
    pub fn click_at(output: Option<&str>, x: f64, y: f64, button: u32) -> Self {
        Self {
            events: vec![
                InputEvent::PointerTo {
                    output: output.map(str::to_owned),
                    x,
                    y,
                },
                InputEvent::Button {
                    code: button,
                    pressed: true,
                },
                InputEvent::Button {
                    code: button,
                    pressed: false,
                },
            ],
            time_ms: 0,
        }
    }
}
