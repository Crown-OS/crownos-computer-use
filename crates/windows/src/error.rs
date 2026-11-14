#[derive(Debug, thiserror::Error)]
pub enum WaylandError {
    #[error("cannot connect to the compositor: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    #[error("cannot read the compositor's globals: {0}")]
    Globals(#[from] wayland_client::globals::GlobalError),
    #[error("the compositor does not offer {0}")]
    Missing(&'static str),
    #[error("wayland dispatch failed: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    #[error("the wayland connection failed: {0}")]
    Backend(#[from] wayland_client::backend::WaylandError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the compositor connection thread is gone")]
    ThreadGone,
    #[error("no window is called {0}")]
    NoSuchWindow(String),
    #[error("the compositor refused the capture: {0}")]
    Capture(String),
    #[error("the compositor refused the agent cursor")]
    CursorRefused,
    #[error("cannot encode the picture: {0}")]
    Png(#[from] png::EncodingError),
}

impl From<rustix::io::Errno> for WaylandError {
    fn from(errno: rustix::io::Errno) -> Self {
        Self::Io(errno.into())
    }
}
