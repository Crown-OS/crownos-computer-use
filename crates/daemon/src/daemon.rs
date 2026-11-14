//! The daemon's state and what each request does to it.

use std::collections::HashMap;
use std::io::Write as _;
use std::os::fd::OwnedFd;
use std::process::Stdio;

use crownos_computer_use_protocol::{ActOutcome, Screenshot, WindowInfo, WindowState};
use crownos_ipc::PeerId;
use crownos_ui_elements::{
    Action, Bounds, Element, Query, Snapshot, SnapshotStore, Source, WindowHandle,
};
use crownos_ui_sources::atspi::AtspiSource;
use crownos_ui_sources::cdp::{Browser, Tab, default_profile_dir};
use crownos_ui_sources::crownui::{CrownUiApps, CrownUiSource};
use crownos_ui_sources::{ActResult, SourceError};
use crownos_window_layer::util::keymap;
use crownos_window_layer::{BTN_LEFT, BTN_RIGHT, Compositor, InputEvent, InputFrame};

use crate::agents::Agents;
use crate::resolve::{BROWSER_PREFIX, Route, route};

pub(crate) const AUDIT: &str = "crownos::audit::computer_use";
const BROWSER_APP_ID: &str = "chromium";

#[derive(Debug, thiserror::Error)]
pub(crate) enum DaemonError {
    #[error("say hello first")]
    NoHello,
    #[error("no window is called {0}")]
    NoSuchWindow(String),
    #[error("{0}")]
    Route(#[from] crate::resolve::RouteError),
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Compositor(#[from] crownos_window_layer::WaylandError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Resolve(#[from] crownos_ui_elements::ResolveError),
}

pub(crate) struct Daemon {
    pub(crate) compositor: Compositor,
    pub(crate) agents: Agents,
    snapshots: SnapshotStore,
    crownui: CrownUiApps,
    crownui_sources: HashMap<String, CrownUiSource>,
    atspi: Option<AtspiSource>,
    browser: Option<Browser>,
}

impl Daemon {
    pub(crate) fn new(compositor: Compositor) -> Self {
        Self {
            compositor,
            agents: Agents::default(),
            snapshots: SnapshotStore::default(),
            crownui: CrownUiApps::default(),
            crownui_sources: HashMap::new(),
            atspi: None,
            browser: None,
        }
    }

    pub(crate) async fn hello(&mut self, peer: PeerId, name: String, color: u32) -> u32 {
        let cursor = self.compositor.create_cursor(&name, color).ok();
        tracing::info!(target: AUDIT, agent = %name, cursor = ?cursor, "agent connected");
        self.agents.register(peer, name, cursor)
    }

    pub(crate) fn goodbye(&mut self, peer: PeerId) {
        if let Some(agent) = self.agents.remove(peer) {
            if let Some(cursor) = agent.cursor {
                let _ = self.compositor.destroy_cursor(cursor);
            }
            tracing::info!(target: AUDIT, agent = %agent.name, "agent disconnected");
        }
    }

    pub(crate) async fn list_windows(&mut self) -> Result<Vec<WindowInfo>, DaemonError> {
        let mut windows = self.compositor.windows()?;
        if let Some(browser) = &self.browser {
            windows.extend(browser.tabs().await?.into_iter().map(tab_window));
        }
        Ok(windows)
    }

    async fn window(&mut self, identifier: &str) -> Result<WindowInfo, DaemonError> {
        self.list_windows()
            .await?
            .into_iter()
            .find(|window| window.identifier == identifier)
            .ok_or_else(|| DaemonError::NoSuchWindow(identifier.to_owned()))
    }

    pub(crate) async fn snapshot(
        &mut self,
        identifier: &str,
        interactive_only: bool,
    ) -> Result<Snapshot, DaemonError> {
        let window = self.window(identifier).await?;
        let (source, elements) = self.elements_of(&window).await?;
        let snapshot = self
            .snapshots
            .take(WindowHandle(identifier.to_owned()), source, elements);
        Ok(if interactive_only {
            snapshot.interactive()
        } else {
            snapshot.clone()
        })
    }

    pub(crate) fn find(&self, identifier: &str, query: &Query) -> Vec<Element> {
        self.snapshots
            .current(&WindowHandle(identifier.to_owned()))
            .map(|snapshot| snapshot.find(query).into_iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) async fn act(
        &mut self,
        peer: PeerId,
        reference: &str,
        action: &Action,
    ) -> Result<ActOutcome, DaemonError> {
        let agent_name = self.agents.name(peer).to_owned();
        let Some(agent) = self.agents.get_mut(peer) else {
            return Err(DaemonError::NoHello);
        };
        if !agent.limiter.allow() {
            return Ok(ActOutcome::RateLimited);
        }
        let cursor = agent.cursor;
        let reference = match reference.parse::<crownos_ui_elements::ElementRef>() {
            Ok(reference) => reference,
            Err(error) => return Ok(ActOutcome::Failed(error.to_string())),
        };
        let (snapshot, element) = match self.snapshots.resolve(reference) {
            Ok(found) => found,
            Err(crownos_ui_elements::ResolveError::Stale(_, newer)) => {
                return Ok(ActOutcome::Stale(format!("@s{newer}")));
            }
            Err(error) => return Ok(ActOutcome::Failed(error.to_string())),
        };
        if element.secure && action.types() {
            tracing::warn!(target: AUDIT, agent = %agent_name, %reference, "refused to type into a secure element");
            return Ok(ActOutcome::Secure);
        }
        let element = element.clone();
        let identifier = snapshot.window.0.clone();
        tracing::info!(target: AUDIT, agent = %agent_name, window = %identifier, %reference, role = %element.role, name = %element.name, action = action.name(), "act");
        let window = self.window(&identifier).await?;
        let result = match self.perform(&window, &element, action).await {
            Ok(result) => result,
            Err(SourceError::Unsupported(_)) => fallback(&element),
            Err(error) => return Ok(ActOutcome::Failed(error.to_string())),
        };
        if let ActResult::NeedsPointer { x, y } = result {
            let frames = pointer_frames(&window, &element, action, x, y);
            for frame in frames {
                self.compositor.inject(frame, cursor)?;
            }
        }
        Ok(ActOutcome::Done)
    }

    async fn perform(
        &mut self,
        window: &WindowInfo,
        element: &Element,
        action: &Action,
    ) -> Result<ActResult, SourceError> {
        match element.source {
            Source::Browser => {
                let target = window
                    .identifier
                    .strip_prefix(BROWSER_PREFIX)
                    .unwrap_or_default()
                    .to_owned();
                self.browser().await?.act(&target, element, action).await
            }
            Source::CrownUi => match self.crownui_sources.get_mut(&window.identifier) {
                Some(source) => source.act(element, action).await,
                None => Err(SourceError::Gone),
            },
            Source::Atspi => match &mut self.atspi {
                Some(atspi) => atspi.act(element, action).await,
                None => Err(SourceError::Gone),
            },
            Source::Ocr => Ok(fallback(element)),
        }
    }

    async fn elements_of(
        &mut self,
        window: &WindowInfo,
    ) -> Result<(Source, Vec<Element>), DaemonError> {
        let crownui = self.crownui.clone();
        match route(window, |pid| crownui.endpoint_for_pid(pid).is_some())? {
            Route::Browser { target } => {
                let elements = self.browser().await?.elements(&target).await?;
                Ok((Source::Browser, elements))
            }
            Route::CrownUi { pid } => {
                let endpoint = crownui.endpoint_for_pid(pid).ok_or(SourceError::Gone)?;
                let token = std::env::var("CROWNMCP_TOKEN").ok();
                let stale = self
                    .crownui_sources
                    .get(&window.identifier)
                    .is_some_and(CrownUiSource::is_closed);
                if stale || !self.crownui_sources.contains_key(&window.identifier) {
                    let source = CrownUiSource::connect(&endpoint, token.as_deref()).await?;
                    self.crownui_sources
                        .insert(window.identifier.clone(), source);
                }
                let source = self
                    .crownui_sources
                    .get_mut(&window.identifier)
                    .ok_or(SourceError::Gone)?;
                Ok((Source::CrownUi, source.elements().await?))
            }
            Route::Atspi { pid } => {
                if self.atspi.is_none() {
                    self.atspi = Some(AtspiSource::connect().await?);
                }
                let atspi = self.atspi.as_mut().ok_or(SourceError::Gone)?;
                Ok((Source::Atspi, atspi.elements(pid).await?))
            }
        }
    }

    async fn browser(&mut self) -> Result<&Browser, SourceError> {
        if self
            .browser
            .as_mut()
            .is_none_or(|browser| !browser.is_alive())
        {
            self.browser = Some(Browser::launch(&default_profile_dir(), &[]).await?);
        }
        self.browser.as_ref().ok_or(SourceError::Gone)
    }

    pub(crate) async fn open_url(&mut self, url: &str) -> Result<WindowInfo, DaemonError> {
        let tab = self.browser().await?.open(url).await?;
        Ok(tab_window(tab))
    }

    pub(crate) async fn screenshot(
        &mut self,
        identifier: &str,
        sink: OwnedFd,
    ) -> Result<Screenshot, DaemonError> {
        let (png, width, height) = match identifier.strip_prefix(BROWSER_PREFIX) {
            Some(target) => {
                let png = self.browser().await?.screenshot(target).await?;
                let (width, height) = png_size(&png);
                (png, width, height)
            }
            None => {
                let picture = self.compositor.screenshot(identifier)?;
                (picture.to_png()?, picture.width, picture.height)
            }
        };
        let mut file = std::fs::File::from(sink);
        file.write_all(&png)?;
        Ok(Screenshot {
            width,
            height,
            png_bytes: u64::try_from(png.len()).unwrap_or(u64::MAX),
        })
    }

    pub(crate) async fn focus(&mut self, identifier: &str) -> Result<(), DaemonError> {
        if let Some(target) = identifier.strip_prefix(BROWSER_PREFIX) {
            let target = target.to_owned();
            self.browser()
                .await?
                .navigate(&target, "javascript:void(0)")
                .await
                .ok();
            return Ok(());
        }
        Ok(self.compositor.focus(identifier)?)
    }

    pub(crate) async fn close(&mut self, identifier: &str) -> Result<(), DaemonError> {
        if let Some(target) = identifier.strip_prefix(BROWSER_PREFIX) {
            let target = target.to_owned();
            self.browser().await?.close_tab(&target).await?;
            self.snapshots.forget(&WindowHandle(identifier.to_owned()));
            return Ok(());
        }
        Ok(self.compositor.close(identifier)?)
    }

    pub(crate) fn forget(&mut self, identifier: &str) {
        self.snapshots.forget(&WindowHandle(identifier.to_owned()));
        self.crownui_sources.remove(identifier);
    }

    pub(crate) fn launch(&self, command: &[String], headless: bool) -> Result<u32, DaemonError> {
        let (program, arguments) = command
            .split_first()
            .ok_or_else(|| std::io::Error::other("an empty command"))?;
        let mut child = std::process::Command::new(program);
        child
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if headless {
            child.env("WORKSPACE", "headless");
        }
        let child = child.spawn()?;
        tracing::info!(target: AUDIT, %program, headless, pid = child.id(), "launched");
        Ok(child.id())
    }
}

/// What the daemon does itself when the source cannot: press or type at the element.
const fn fallback(element: &Element) -> ActResult {
    let (x, y) = element.bounds.center();
    ActResult::NeedsPointer { x, y }
}

/// Injected frames for `action` at window point `(x, y)`: the window's placement turns it into
/// a point on its output.
fn pointer_frames(
    window: &WindowInfo,
    element: &Element,
    action: &Action,
    x: i32,
    y: i32,
) -> Vec<InputFrame> {
    let output = (!window.output.is_empty()).then(|| window.output.clone());
    let point = (
        f64::from(window.bounds.x + x),
        f64::from(window.bounds.y + y),
    );
    let click = |button| InputFrame::click_at(output.as_deref(), point.0, point.1, button);
    let move_to = InputFrame {
        events: vec![InputEvent::PointerTo {
            output: output.clone(),
            x: point.0,
            y: point.1,
        }],
        time_ms: 0,
    };
    match action {
        Action::Click | Action::Focus => vec![click(BTN_LEFT)],
        Action::DoubleClick => vec![click(BTN_LEFT), click(BTN_LEFT)],
        Action::Hover => vec![move_to],
        Action::Scroll { dx, dy } => vec![
            move_to,
            InputFrame {
                events: vec![
                    InputEvent::Scroll {
                        vertical: true,
                        value: *dy,
                    },
                    InputEvent::Scroll {
                        vertical: false,
                        value: *dx,
                    },
                ],
                time_ms: 0,
            },
        ],
        Action::TypeText { text } | Action::SetValue { value: text } => vec![
            click(BTN_LEFT),
            InputFrame {
                events: keymap::type_text(text),
                time_ms: 0,
            },
        ],
        Action::PressKey { keybind } => {
            let _ = element;
            keybind_frames(keybind)
                .into_iter()
                .map(|events| InputFrame { events, time_ms: 0 })
                .collect()
        }
    }
}

const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTMETA: u32 = 125;

fn keybind_frames(keybind: &str) -> Vec<Vec<InputEvent>> {
    let mut modifiers = Vec::new();
    let mut key = None;
    for part in keybind.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.push(KEY_LEFTCTRL),
            "alt" => modifiers.push(KEY_LEFTALT),
            "shift" => modifiers.push(KEY_LEFTSHIFT),
            "super" | "meta" => modifiers.push(KEY_LEFTMETA),
            "enter" | "return" => key = Some(28),
            "tab" => key = Some(15),
            "escape" | "esc" => key = Some(1),
            "backspace" => key = Some(14),
            "delete" => key = Some(111),
            "up" => key = Some(103),
            "down" => key = Some(108),
            "left" => key = Some(105),
            "right" => key = Some(106),
            other => {
                key = other
                    .chars()
                    .next()
                    .and_then(keymap::key_for)
                    .map(|(code, _)| code);
            }
        }
    }
    let Some(key) = key else {
        return Vec::new();
    };
    let press = |code| InputEvent::Key {
        code,
        pressed: true,
    };
    let release = |code| InputEvent::Key {
        code,
        pressed: false,
    };
    let mut events: Vec<InputEvent> = modifiers.iter().map(|code| press(*code)).collect();
    events.push(press(key));
    events.push(release(key));
    events.extend(modifiers.iter().rev().map(|code| release(*code)));
    vec![events]
}

fn tab_window(tab: Tab) -> WindowInfo {
    WindowInfo {
        identifier: format!("{BROWSER_PREFIX}{}", tab.target_id),
        app_id: BROWSER_APP_ID.to_owned(),
        title: if tab.title.is_empty() {
            tab.url
        } else {
            tab.title
        },
        pid: 0,
        bounds: Bounds::default(),
        output: String::new(),
        workspace: String::new(),
        state: WindowState::default(),
        browser_tab: true,
    }
}

/// Width and height from a PNG's IHDR chunk; zero when it is not a PNG.
fn png_size(png: &[u8]) -> (u32, u32) {
    let number = |offset: usize| {
        png.get(offset..offset + 4)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(u32::from_be_bytes)
            .unwrap_or_default()
    };
    (number(16), number(20))
}

#[allow(
    dead_code,
    reason = "kept for the right-button fallback agents will ask for"
)]
const _: u32 = BTN_RIGHT;
