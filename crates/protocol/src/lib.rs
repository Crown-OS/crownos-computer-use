//! What agents' tools ask the daemon over `$XDG_RUNTIME_DIR/crownos/crownos-computer-use.sock`.
//!
//! Every connection introduces its agent with `hello` first; the daemon gives that agent its own
//! cursor when the compositor offers one and names it in the audit log. Windows are named by the
//! compositor's foreign-toplevel identifier, browser tabs by `browser:<target>`.

use crownos_ui_elements::{Action, Bounds, Element, Snapshot};
use serde::{Deserialize, Serialize};

/// Where the picture of a window goes: an `fd` the caller made with memfd, which the daemon
/// writes the PNG into and the caller reads back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub png_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WindowState {
    pub maximized: bool,
    pub fullscreen: bool,
    pub minimized: bool,
    pub floating: bool,
    pub activated: bool,
    pub agent_workspace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub identifier: String,
    pub app_id: String,
    pub title: String,
    /// 0 while the compositor does not say.
    pub pid: i32,
    /// In the output's logical coordinates; zero-sized while the compositor does not say.
    pub bounds: Bounds,
    pub output: String,
    pub workspace: String,
    pub state: WindowState,
    /// A browser tab rather than a compositor toplevel.
    pub browser_tab: bool,
}

/// What happened to a request to type a stored secret. It never holds the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretTyping {
    Typed,
    /// The rules of the secret do not allow this agent, this app or this page.
    Denied,
    OriginMismatch,
    NotPasswordField,
    FocusChanged,
    /// The user said no, or the compositor stopped the typing.
    Cancelled,
    /// The rules ask for a confirmation and no dialog could show.
    NeedsConfirmation,
    RateLimited,
    Locked,
    /// No stored secret has this name.
    Unknown,
    /// crownos-secretsd is not running, or the compositor cannot type secrets yet.
    Unavailable,
    /// A password goes only into a secure element, and a user name never does.
    NotSecure,
    /// The compositor knows no such window.
    UnknownWindow,
    /// Nobody answered the compositor's confirmation in time.
    TimedOut,
    /// The window's app is not the one that the request named.
    TargetMismatch,
    /// The session is locked.
    SessionLocked,
}

/// What happened to a `type_secret` or `fill_login`, and the page origin after it. It never
/// holds a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretReport {
    pub outcome: SecretTyping,
    /// The origin of the browser page when the request ended; `None` outside the browser.
    pub origin: Option<String>,
}

/// Why an action was not performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActOutcome {
    Done,
    /// The ref's snapshot was replaced; the message names the new one.
    Stale(String),
    /// Typing into a secure element is never done.
    Secure,
    /// More than the agent's share of actions this second.
    RateLimited,
    Failed(String),
    /// The outcome of `type_secret` or `fill_login`.
    Secret(SecretReport),
}

crownos_ipc::protocol! {
    /// The computer-use daemon.
    pub mod computer_use: "crownos-computer-use" {
        /// Who is on this connection. Must come first; every action is logged under this name.
        fn hello(agent: String, color: u32) -> u32;
        fn list_windows() -> Vec<WindowInfo>;
        /// A fresh snapshot of one window, retiring the previous one's refs.
        fn snapshot(window: String, interactive_only: bool) -> Snapshot;
        /// Elements of the current snapshot of `window` matching every given part.
        fn find(window: String, role: Option<String>, name: Option<String>, text: Option<String>) -> Vec<Element>;
        fn act(reference: String, action: Action) -> ActOutcome;
        /// Focuses the secure element and asks crownos-secretsd to type the stored secret `name`
        /// into it. The daemon never holds the value.
        fn type_secret(reference: String, name: String) -> ActOutcome;
        /// Types the stored user name into one element, a Tab, and the stored password into the
        /// secure element that follows, as one request with one confirmation.
        fn fill_login(username_reference: String, password_reference: String, username_name: String, password_name: String) -> ActOutcome;
        /// Writes a PNG of the window into `sink`. Refused while a secret is typed into the window
        /// and for 30 seconds after.
        fn screenshot(window: String) fds(sink) -> Screenshot;
        fn focus(window: String);
        fn close(window: String);
        fn make_visible(window: String);
        fn make_hidden(window: String);
        /// Starts a program; with headless it opens on the agent workspace.
        fn launch(command: Vec<String>, headless: bool) -> u32;
        /// The ext_workspace identifier of the agent workspace, empty when the compositor has none.
        fn agent_workspace() -> String;
        /// Opens a tab in the agent browser.
        fn open_url(url: String) -> WindowInfo;

        event WindowOpened { window: WindowInfo };
        event WindowClosed { identifier: String };
        event FocusChanged { identifier: Option<String> };
        event NotificationPosted { app: String, summary: String };
    }
}
