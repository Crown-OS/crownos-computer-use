//! The worker's state: every global this side binds and what the compositor said on them.

use std::collections::HashMap;

use crownos_computer_use_protocol::{WindowInfo, WindowState};
use crownos_protocols::agent_cursor::v1::client::crownos_agent_cursor_manager_v1::CrownosAgentCursorManagerV1;
use crownos_protocols::agent_cursor::v1::client::crownos_agent_cursor_v1::{
    self, CrownosAgentCursorV1,
};
use crownos_protocols::input::v1::client::crownos_input_injector_v1::{
    self, CrownosInputInjectorV1,
};
use crownos_protocols::input::v1::client::crownos_input_manager_v1::CrownosInputManagerV1;
use crownos_protocols::window_control::v1::client::crownos_window_control_manager_v1::{
    self, CrownosWindowControlManagerV1,
};
use crownos_protocols::window_control::v1::client::crownos_window_v1::{
    self, CrownosWindowV1, State as WindowFlags,
};
use crownos_ui_elements::Bounds;
use tokio::sync::mpsc::UnboundedSender;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalList, GlobalListContents};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::ext::image_capture_source::v1::client::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1;

use crate::capture::Captures;
use crate::error::WaylandError;
use crate::input::{CursorId, InputEvent, InputFrame};
use crate::outputs::Outputs;
use crate::worker::WorkerState;
use crate::{Capabilities, Command, Reply, WindowEvent};

/// Handles live until the daemon stops; no object is destroyed before then.
const ANY_VERSION: std::ops::RangeInclusive<u32> = 1..=1;
const SEAT_VERSION: std::ops::RangeInclusive<u32> = 1..=9;

/// One toplevel as the foreign toplevel list describes it.
#[derive(Debug, Default)]
struct Toplevel {
    identifier: String,
    title: String,
    app_id: String,
    announced: bool,
}

/// One toplevel as window control describes it, built up until `done`.
#[derive(Debug)]
struct Controlled {
    info: WindowInfo,
    announced: bool,
}

pub(crate) struct Layer {
    outputs: Outputs,
    events: UnboundedSender<WindowEvent>,
    toplevels: HashMap<ObjectId, Toplevel>,
    toplevel_handles: HashMap<ObjectId, ExtForeignToplevelHandleV1>,
    control: Option<CrownosWindowControlManagerV1>,
    controlled: HashMap<ObjectId, Controlled>,
    controlled_objects: HashMap<String, CrownosWindowV1>,
    agent_workspace: Option<String>,
    agent_workspace_waiters: Vec<Reply<String>>,
    injector: Option<CrownosInputInjectorV1>,
    cursor_manager: Option<CrownosAgentCursorManagerV1>,
    cursors: HashMap<CursorId, CrownosAgentCursorV1>,
    cursor_waiters: HashMap<ObjectId, (CursorId, Reply<Result<CursorId, WaylandError>>)>,
    next_cursor: u32,
    pub(crate) captures: Captures,
}

impl Layer {
    pub(crate) fn bind(
        globals: &GlobalList,
        queue: &QueueHandle<Self>,
        events: UnboundedSender<WindowEvent>,
    ) -> Result<Self, WaylandError> {
        let control: Option<CrownosWindowControlManagerV1> =
            globals.bind(queue, ANY_VERSION, ()).ok();
        let list: Option<ExtForeignToplevelListV1> = globals.bind(queue, ANY_VERSION, ()).ok();
        if control.is_none() && list.is_none() {
            return Err(WaylandError::Missing("a window list"));
        }
        let injector = globals
            .bind::<CrownosInputManagerV1, _, _>(queue, ANY_VERSION, ())
            .ok()
            .zip(globals.bind::<WlSeat, _, _>(queue, SEAT_VERSION, ()).ok())
            .map(|(manager, seat)| manager.create_injector(&seat, queue, ()));
        let cursor_manager = globals.bind(queue, ANY_VERSION, ()).ok();
        let shm: Option<WlShm> = globals.bind(queue, ANY_VERSION, ()).ok();
        let copy: Option<ExtImageCopyCaptureManagerV1> = globals.bind(queue, ANY_VERSION, ()).ok();
        let sources: Option<ExtForeignToplevelImageCaptureSourceManagerV1> =
            globals.bind(queue, ANY_VERSION, ()).ok();
        let captures = Captures::new(shm, copy, sources);
        if let Some(control) = &control {
            control.get_agent_workspace();
        }
        Ok(Self {
            outputs: Outputs::bind_existing(globals, queue),
            events,
            toplevels: HashMap::new(),
            toplevel_handles: HashMap::new(),
            control,
            controlled: HashMap::new(),
            controlled_objects: HashMap::new(),
            agent_workspace: None,
            agent_workspace_waiters: Vec::new(),
            injector,
            cursor_manager,
            cursors: HashMap::new(),
            cursor_waiters: HashMap::new(),
            next_cursor: 1,
            captures,
        })
    }

    const fn capabilities(&self) -> Capabilities {
        Capabilities {
            window_control: self.control.is_some(),
            input: self.injector.is_some(),
            agent_cursor: self.cursor_manager.is_some(),
            capture: self.captures.is_available(),
        }
    }

    fn windows(&self) -> Vec<WindowInfo> {
        if self.control.is_some() {
            let mut windows: Vec<WindowInfo> = self
                .controlled
                .values()
                .filter(|window| window.announced)
                .map(|window| window.info.clone())
                .collect();
            windows.sort_by(|left, right| left.identifier.cmp(&right.identifier));
            return windows;
        }
        let mut windows: Vec<WindowInfo> = self
            .toplevels
            .values()
            .filter(|toplevel| toplevel.announced)
            .map(|toplevel| WindowInfo {
                identifier: toplevel.identifier.clone(),
                app_id: toplevel.app_id.clone(),
                title: toplevel.title.clone(),
                pid: 0,
                bounds: Bounds::default(),
                output: String::new(),
                workspace: String::new(),
                state: WindowState::default(),
                browser_tab: false,
            })
            .collect();
        windows.sort_by(|left, right| left.identifier.cmp(&right.identifier));
        windows
    }

    fn window_object(&self, identifier: &str) -> Option<&CrownosWindowV1> {
        self.controlled_objects.get(identifier)
    }

    fn toplevel_handle(&self, identifier: &str) -> Option<&ExtForeignToplevelHandleV1> {
        self.toplevels
            .iter()
            .find(|(_, toplevel)| toplevel.identifier == identifier)
            .and_then(|(id, _)| self.toplevel_handles.get(id))
    }

    fn inject(&self, frame: &InputFrame, cursor: Option<CursorId>) {
        if let Some(cursor) = cursor.and_then(|id| self.cursors.get(&id)) {
            for event in &frame.events {
                match event {
                    InputEvent::PointerMotion { dx, dy } => cursor.pointer_motion(*dx, *dy),
                    InputEvent::PointerTo { x, y, .. } => cursor.pointer_motion_absolute(*x, *y),
                    InputEvent::Button { code, pressed } => {
                        cursor.button(*code, cursor_button(*pressed));
                    }
                    InputEvent::Scroll { vertical, value } => cursor.axis(
                        if *vertical {
                            crownos_agent_cursor_v1::Axis::VerticalScroll
                        } else {
                            crownos_agent_cursor_v1::Axis::HorizontalScroll
                        },
                        *value,
                    ),
                    InputEvent::Key { code, pressed } => cursor.key(*code, cursor_key(*pressed)),
                }
            }
            cursor.frame(frame.time_ms);
            return;
        }
        let Some(injector) = &self.injector else {
            tracing::warn!(
                "no input path: the compositor offers neither agent cursors nor an injector"
            );
            return;
        };
        for event in &frame.events {
            match event {
                InputEvent::PointerMotion { dx, dy } => injector.pointer_motion(*dx, *dy),
                InputEvent::PointerTo { output, x, y } => {
                    if let Some(output) = self.outputs.find(output.as_deref()) {
                        injector.pointer_motion_absolute(&output.output, *x, *y);
                    }
                }
                InputEvent::Button { code, pressed } => {
                    injector.button(*code, injector_button(*pressed));
                }
                InputEvent::Scroll { vertical, value } => injector.axis(
                    if *vertical {
                        crownos_input_injector_v1::Axis::VerticalScroll
                    } else {
                        crownos_input_injector_v1::Axis::HorizontalScroll
                    },
                    *value,
                    0,
                ),
                InputEvent::Key { code, pressed } => injector.key(*code, injector_key(*pressed)),
            }
        }
        let micros = u64::from(frame.time_ms) * 1000;
        #[allow(clippy::cast_possible_truncation, reason = "the halves are the split")]
        injector.frame((micros >> 32) as u32, micros as u32);
    }

    fn create_cursor(
        &mut self,
        name: String,
        color: u32,
        reply: Reply<Result<CursorId, WaylandError>>,
        queue: &QueueHandle<Self>,
    ) {
        let Some(manager) = &self.cursor_manager else {
            let _ = reply.send(Err(WaylandError::Missing("crownos_agent_cursor_v1")));
            return;
        };
        let id = CursorId(self.next_cursor);
        self.next_cursor += 1;
        let cursor = manager.create_cursor(name, color, queue, ());
        self.cursor_waiters.insert(cursor.id(), (id, reply));
        self.cursors.insert(id, cursor);
    }

    fn emit(&self, event: WindowEvent) {
        let _ = self.events.send(event);
    }
}

const fn cursor_button(pressed: bool) -> crownos_agent_cursor_v1::ButtonState {
    if pressed {
        crownos_agent_cursor_v1::ButtonState::Pressed
    } else {
        crownos_agent_cursor_v1::ButtonState::Released
    }
}

const fn cursor_key(pressed: bool) -> crownos_agent_cursor_v1::KeyState {
    if pressed {
        crownos_agent_cursor_v1::KeyState::Pressed
    } else {
        crownos_agent_cursor_v1::KeyState::Released
    }
}

const fn injector_button(pressed: bool) -> crownos_input_injector_v1::ButtonState {
    if pressed {
        crownos_input_injector_v1::ButtonState::Pressed
    } else {
        crownos_input_injector_v1::ButtonState::Released
    }
}

const fn injector_key(pressed: bool) -> crownos_input_injector_v1::KeyState {
    if pressed {
        crownos_input_injector_v1::KeyState::Pressed
    } else {
        crownos_input_injector_v1::KeyState::Released
    }
}

impl WorkerState for Layer {
    type Command = Command;

    fn apply(&mut self, command: Command, queue: &QueueHandle<Self>) {
        match command {
            Command::Capabilities(reply) => {
                let _ = reply.send(self.capabilities());
            }
            Command::Windows(reply) => {
                let _ = reply.send(self.windows());
            }
            Command::Focus(identifier) => {
                if let Some(window) = self.window_object(&identifier) {
                    window.focus();
                }
            }
            Command::Close(identifier) => {
                if let Some(window) = self.window_object(&identifier) {
                    window.close();
                }
            }
            Command::MakeVisible(identifier) => {
                if let Some(window) = self.window_object(&identifier) {
                    window.make_visible();
                }
            }
            Command::MakeHidden(identifier) => {
                if let Some(window) = self.window_object(&identifier) {
                    window.make_hidden();
                }
            }
            Command::MoveToWorkspace(identifier, workspace) => {
                if let Some(window) = self.window_object(&identifier) {
                    window.move_to_workspace(workspace);
                }
            }
            Command::SetRectangle(identifier, (x, y, width, height)) => {
                if let Some(window) = self.window_object(&identifier) {
                    window.set_rectangle(x, y, width, height);
                }
            }
            Command::AgentWorkspace(reply) => match (&self.agent_workspace, &self.control) {
                (Some(workspace), _) => {
                    let _ = reply.send(workspace.clone());
                }
                (None, Some(control)) => {
                    control.get_agent_workspace();
                    self.agent_workspace_waiters.push(reply);
                }
                (None, None) => {
                    let _ = reply.send(String::new());
                }
            },
            Command::Inject(frame, cursor) => self.inject(&frame, cursor),
            Command::CreateCursor(name, color, reply) => {
                self.create_cursor(name, color, reply, queue);
            }
            Command::DestroyCursor(id) => {
                if let Some(cursor) = self.cursors.remove(&id) {
                    cursor.destroy();
                }
            }
            Command::Screenshot(identifier, reply) => {
                let handle = self.toplevel_handle(&identifier).cloned();
                match handle {
                    Some(handle) => self.captures.start(&handle, reply, queue),
                    None => {
                        let _ = reply.send(Err(WaylandError::NoSuchWindow(identifier)));
                    }
                }
            }
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Layer {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        state.outputs.on_registry_event(registry, event, queue);
    }
}

impl Dispatch<WlOutput, u32> for Layer {
    fn event(
        state: &mut Self,
        _output: &WlOutput,
        event: wl_output::Event,
        global: &u32,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        state.outputs.on_output_event(*global, event);
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Layer {
    fn event(
        state: &mut Self,
        _list: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.insert(toplevel.id(), Toplevel::default());
            state.toplevel_handles.insert(toplevel.id(), toplevel);
        }
    }

    event_created_child!(Layer, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Layer {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        let id = handle.id();
        let Some(toplevel) = state.toplevels.get_mut(&id) else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                toplevel.identifier = identifier;
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => toplevel.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => toplevel.app_id = app_id,
            ext_foreign_toplevel_handle_v1::Event::Done => {
                let first = !toplevel.announced;
                toplevel.announced = true;
                let identifier = toplevel.identifier.clone();
                if state.control.is_none() {
                    let info = state
                        .windows()
                        .into_iter()
                        .find(|window| window.identifier == identifier);
                    if let Some(info) = info {
                        state.emit(if first {
                            WindowEvent::Opened(info)
                        } else {
                            WindowEvent::Changed(info)
                        });
                    }
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                let identifier = toplevel.identifier.clone();
                state.toplevels.remove(&id);
                state.toplevel_handles.remove(&id);
                handle.destroy();
                if state.control.is_none() {
                    state.emit(WindowEvent::Closed(identifier));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<CrownosWindowControlManagerV1, ()> for Layer {
    fn event(
        state: &mut Self,
        _manager: &CrownosWindowControlManagerV1,
        event: crownos_window_control_manager_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        match event {
            crownos_window_control_manager_v1::Event::Window { window } => {
                state.controlled.insert(
                    window.id(),
                    Controlled {
                        info: WindowInfo {
                            identifier: String::new(),
                            app_id: String::new(),
                            title: String::new(),
                            pid: 0,
                            bounds: Bounds::default(),
                            output: String::new(),
                            workspace: String::new(),
                            state: WindowState::default(),
                            browser_tab: false,
                        },
                        announced: false,
                    },
                );
            }
            crownos_window_control_manager_v1::Event::Focus { window } => {
                let identifier = window
                    .and_then(|window| state.controlled.get(&window.id()))
                    .map(|window| window.info.identifier.clone());
                state.emit(WindowEvent::Focus(identifier));
            }
            crownos_window_control_manager_v1::Event::AgentWorkspace { workspace } => {
                state.agent_workspace = Some(workspace.clone());
                for waiter in state.agent_workspace_waiters.drain(..) {
                    let _ = waiter.send(workspace.clone());
                }
            }
            _ => {}
        }
    }

    event_created_child!(Layer, CrownosWindowControlManagerV1, [
        crownos_window_control_manager_v1::EVT_WINDOW_OPCODE => (CrownosWindowV1, ()),
    ]);
}

impl Dispatch<CrownosWindowV1, ()> for Layer {
    fn event(
        state: &mut Self,
        window: &CrownosWindowV1,
        event: crownos_window_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        let id = window.id();
        let Some(controlled) = state.controlled.get_mut(&id) else {
            return;
        };
        match event {
            crownos_window_v1::Event::Identifier { identifier } => {
                state
                    .controlled_objects
                    .insert(identifier.clone(), window.clone());
                controlled.info.identifier = identifier;
            }
            crownos_window_v1::Event::AppId { app_id } => controlled.info.app_id = app_id,
            crownos_window_v1::Event::Title { title } => controlled.info.title = title,
            crownos_window_v1::Event::Process { pid } => controlled.info.pid = pid,
            crownos_window_v1::Event::Placement {
                x,
                y,
                width,
                height,
                output,
                workspace,
            } => {
                controlled.info.bounds = Bounds::new(x, y, width, height);
                controlled.info.output = output;
                controlled.info.workspace = workspace;
            }
            crownos_window_v1::Event::State { state: flags } => {
                let flags = match flags {
                    WEnum::Value(flags) => flags,
                    WEnum::Unknown(bits) => WindowFlags::from_bits_truncate(bits),
                };
                controlled.info.state = WindowState {
                    maximized: flags.contains(WindowFlags::Maximized),
                    fullscreen: flags.contains(WindowFlags::Fullscreen),
                    minimized: flags.contains(WindowFlags::Minimized),
                    floating: flags.contains(WindowFlags::Floating),
                    activated: flags.contains(WindowFlags::Activated),
                    agent_workspace: flags.contains(WindowFlags::AgentWorkspace),
                };
            }
            crownos_window_v1::Event::Done => {
                let first = !controlled.announced;
                controlled.announced = true;
                let info = controlled.info.clone();
                state.emit(if first {
                    WindowEvent::Opened(info)
                } else {
                    WindowEvent::Changed(info)
                });
            }
            crownos_window_v1::Event::Closed => {
                let identifier = controlled.info.identifier.clone();
                state.controlled.remove(&id);
                state.controlled_objects.remove(&identifier);
                window.destroy();
                state.emit(WindowEvent::Closed(identifier));
            }
            _ => {}
        }
    }
}

impl Dispatch<CrownosAgentCursorV1, ()> for Layer {
    fn event(
        state: &mut Self,
        cursor: &CrownosAgentCursorV1,
        event: crownos_agent_cursor_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        match event {
            crownos_agent_cursor_v1::Event::Created { .. } => {
                if let Some((id, reply)) = state.cursor_waiters.remove(&cursor.id()) {
                    let _ = reply.send(Ok(id));
                }
            }
            crownos_agent_cursor_v1::Event::Revoked { .. } => {
                if let Some((_, reply)) = state.cursor_waiters.remove(&cursor.id()) {
                    let _ = reply.send(Err(WaylandError::CursorRefused));
                }
                state.cursors.retain(|_, held| held.id() != cursor.id());
            }
            _ => {}
        }
    }
}

impl Dispatch<CrownosInputInjectorV1, ()> for Layer {
    fn event(
        state: &mut Self,
        _injector: &CrownosInputInjectorV1,
        event: crownos_input_injector_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        if let crownos_input_injector_v1::Event::Revoked = event {
            tracing::warn!("the compositor revoked the input injector");
            state.injector = None;
        }
    }
}

delegate_noop!(Layer: ignore WlSeat);
delegate_noop!(Layer: ignore CrownosInputManagerV1);
delegate_noop!(Layer: ignore CrownosAgentCursorManagerV1);
