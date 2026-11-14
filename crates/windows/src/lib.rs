//! The compositor from the daemon's side: one Wayland connection on its own thread.
//!
//! With `crownos_window_control_v1` the list carries process, placement and state and the daemon
//! can move windows about; without it `ext_foreign_toplevel_list_v1` still names them. Input goes
//! through an agent's own `crownos_agent_cursor_v1` when the compositor offers one, else through
//! `crownos_input_injector_v1` and the user's pointer. Screenshots come from
//! `ext_image_copy_capture_v1` into shared memory and leave as PNG.

mod capture;
mod error;
mod input;
mod layer;
mod outputs;
pub mod util;
mod worker;

use std::sync::mpsc;

use crownos_computer_use_protocol::WindowInfo;
use tokio::sync::mpsc::UnboundedReceiver;

pub use error::WaylandError;
pub use input::{BTN_LEFT, BTN_RIGHT, CursorId, InputEvent, InputFrame};

use layer::Layer;
use worker::WorkerHandle;

/// What the compositor offered on connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    pub window_control: bool,
    pub input: bool,
    pub agent_cursor: bool,
    pub capture: bool,
}

/// What the compositor tells about windows, as it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowEvent {
    Opened(WindowInfo),
    Changed(WindowInfo),
    Closed(String),
    Focus(Option<String>),
}

/// A decoded picture of a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA.
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub(crate) enum Command {
    Capabilities(Reply<Capabilities>),
    Windows(Reply<Vec<WindowInfo>>),
    Focus(String),
    Close(String),
    MakeVisible(String),
    MakeHidden(String),
    MoveToWorkspace(String, String),
    SetRectangle(String, (i32, i32, i32, i32)),
    AgentWorkspace(Reply<String>),
    Inject(InputFrame, Option<CursorId>),
    CreateCursor(String, u32, Reply<Result<CursorId, WaylandError>>),
    DestroyCursor(CursorId),
    Screenshot(String, Reply<Result<Picture, WaylandError>>),
}

pub(crate) type Reply<T> = mpsc::SyncSender<T>;

/// The connection; cheap to clone, every method blocks for one roundtrip.
#[derive(Debug, Clone)]
pub struct Compositor {
    worker: std::sync::Arc<WorkerHandle<Command>>,
}

impl Compositor {
    /// Connects to `$WAYLAND_DISPLAY` and binds what the compositor offers.
    ///
    /// # Errors
    ///
    /// Fails without a compositor, or when it offers neither window list.
    pub fn connect() -> Result<(Self, UnboundedReceiver<WindowEvent>), WaylandError> {
        let (events, receiver) = tokio::sync::mpsc::unbounded_channel();
        let worker = WorkerHandle::spawn("crownos-window-layer", move |globals, queue| {
            Layer::bind(globals, queue, events)
        })?;
        Ok((
            Self {
                worker: std::sync::Arc::new(worker),
            },
            receiver,
        ))
    }

    fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T, WaylandError> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.worker.send(make(reply))?;
        answer.recv().map_err(|_| WaylandError::ThreadGone)
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn capabilities(&self) -> Result<Capabilities, WaylandError> {
        self.ask(Command::Capabilities)
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn windows(&self) -> Result<Vec<WindowInfo>, WaylandError> {
        self.ask(Command::Windows)
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn focus(&self, identifier: &str) -> Result<(), WaylandError> {
        self.worker.send(Command::Focus(identifier.to_owned()))
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn close(&self, identifier: &str) -> Result<(), WaylandError> {
        self.worker.send(Command::Close(identifier.to_owned()))
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn make_visible(&self, identifier: &str) -> Result<(), WaylandError> {
        self.worker
            .send(Command::MakeVisible(identifier.to_owned()))
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn make_hidden(&self, identifier: &str) -> Result<(), WaylandError> {
        self.worker.send(Command::MakeHidden(identifier.to_owned()))
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn move_to_workspace(&self, identifier: &str, workspace: &str) -> Result<(), WaylandError> {
        self.worker.send(Command::MoveToWorkspace(
            identifier.to_owned(),
            workspace.to_owned(),
        ))
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn set_rectangle(
        &self,
        identifier: &str,
        rectangle: (i32, i32, i32, i32),
    ) -> Result<(), WaylandError> {
        self.worker
            .send(Command::SetRectangle(identifier.to_owned(), rectangle))
    }

    /// The agent workspace's identifier, empty when the compositor has none.
    ///
    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn agent_workspace(&self) -> Result<String, WaylandError> {
        self.ask(Command::AgentWorkspace)
    }

    /// Delivers one frame of input through `cursor`, or the user's seat when `None`.
    ///
    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn inject(&self, frame: InputFrame, cursor: Option<CursorId>) -> Result<(), WaylandError> {
        self.worker.send(Command::Inject(frame, cursor))
    }

    /// A cursor of the agent's own, when the compositor offers them.
    ///
    /// # Errors
    ///
    /// Fails without the protocol or when the compositor refuses.
    pub fn create_cursor(&self, name: &str, color: u32) -> Result<CursorId, WaylandError> {
        self.ask(|reply| Command::CreateCursor(name.to_owned(), color, reply))?
    }

    /// # Errors
    ///
    /// Fails once the connection is gone.
    pub fn destroy_cursor(&self, cursor: CursorId) -> Result<(), WaylandError> {
        self.worker.send(Command::DestroyCursor(cursor))
    }

    /// One frame of the window, decoded.
    ///
    /// # Errors
    ///
    /// Fails without capture support, for an unknown window, or when the compositor refuses.
    pub fn screenshot(&self, identifier: &str) -> Result<Picture, WaylandError> {
        self.ask(|reply| Command::Screenshot(identifier.to_owned(), reply))?
    }
}
