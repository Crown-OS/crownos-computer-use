//! Where an agent's view of a window comes from.
//!
//! Three sources yield the same [`Element`](crownos_ui_elements::Element) record: a CrownUI app's
//! own MCP socket, which has stable ids and real actions; AT-SPI, which every GTK, Qt, Electron
//! and Chromium window speaks; and the agent browser's DevTools protocol, which sees the DOM. The
//! daemon tries them in that order and falls back to the pointer when a source cannot act.

pub mod atspi;
pub mod cdp;
pub mod crownui;
mod error;
mod util;

pub use error::SourceError;

/// What a source did about an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActResult {
    Done,
    /// The source has no semantic action for this; the caller should press at this point in
    /// window coordinates.
    NeedsPointer {
        x: i32,
        y: i32,
    },
}
