//! What the test reads from the nested compositor itself: the `ext_foreign_toplevel` identifier
//! of a window, for `type_into`, and when the compositor flagged a window sensitive, from
//! `crownos_context_v1` with the metadata scope.

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crownos_protocols::context::v1::client::crownos_context_manager_v1::{
    self, CrownosContextManagerV1,
};
use crownos_protocols::context::v1::client::crownos_context_window_v1::{
    self, CrownosContextWindowV1, Sensitivity,
};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};

#[derive(Default)]
struct Toplevels {
    /// Identifier and app id of each handle, as far as the compositor said.
    handles: HashMap<u32, (String, String)>,
}

impl Dispatch<WlRegistry, GlobalListContents> for Toplevels {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Toplevels {
    fn event(
        _: &mut Self,
        _: &ExtForeignToplevelListV1,
        _: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(Toplevels, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Toplevels {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let entry = state.handles.entry(handle.id().protocol_id()).or_default();
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                entry.0 = identifier
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => entry.1 = app_id,
            _ => {}
        }
    }
}

/// The `ext_foreign_toplevel` identifier of the window of `app_id`, once it shows.
pub(crate) fn toplevel_identifier(display: &Path, app_id: &str, timeout: Duration) -> String {
    let connection = Connection::from_socket(UnixStream::connect(display).unwrap()).unwrap();
    let (globals, mut queue) = registry_queue_init::<Toplevels>(&connection).unwrap();
    let _list: ExtForeignToplevelListV1 = globals.bind(&queue.handle(), 1..=1, ()).unwrap();
    let mut toplevels = Toplevels::default();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        queue.roundtrip(&mut toplevels).unwrap();
        if let Some((identifier, _)) = toplevels
            .handles
            .values()
            .find(|(identifier, found)| found == app_id && !identifier.is_empty())
        {
            return identifier.clone();
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("no toplevel of {app_id} showed");
}

/// When the compositor first marked the window of an app as flagged by the engine.
#[derive(Debug, Clone, Default)]
pub(crate) struct FlagTimes {
    shared: Arc<Mutex<HashMap<String, SystemTime>>>,
}

impl FlagTimes {
    pub(crate) fn first_flag_of(&self, app_id: &str) -> Option<SystemTime> {
        self.shared.lock().unwrap().get(app_id).copied()
    }
}

#[derive(Default)]
struct WindowFacts {
    app_id: String,
}

struct ContextWatch {
    times: FlagTimes,
}

impl Dispatch<WlRegistry, GlobalListContents> for ContextWatch {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<CrownosContextManagerV1, ()> for ContextWatch {
    fn event(
        _: &mut Self,
        _: &CrownosContextManagerV1,
        _: crownos_context_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(ContextWatch, CrownosContextManagerV1, [
        crownos_context_manager_v1::EVT_WINDOW_OPCODE => (CrownosContextWindowV1, Mutex::new(WindowFacts::default())),
    ]);
}

impl Dispatch<CrownosContextWindowV1, Mutex<WindowFacts>> for ContextWatch {
    fn event(
        state: &mut Self,
        _: &CrownosContextWindowV1,
        event: crownos_context_window_v1::Event,
        facts: &Mutex<WindowFacts>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut facts = facts.lock().unwrap();
        match event {
            crownos_context_window_v1::Event::AppId { app_id } => facts.app_id = app_id,
            crownos_context_window_v1::Event::Sensitivity {
                sensitivity: WEnum::Value(sensitivity),
            } if sensitivity.contains(Sensitivity::EngineFlagged) => {
                state
                    .times
                    .shared
                    .lock()
                    .unwrap()
                    .entry(facts.app_id.clone())
                    .or_insert_with(SystemTime::now);
            }
            _ => {}
        }
    }
}

/// Watches the context stream of the nested compositor on its own thread. The test binary
/// needs the metadata scope in `context_access.ron`.
pub(crate) fn watch_flags(display: &Path) -> FlagTimes {
    let times = FlagTimes::default();
    let connection = Connection::from_socket(UnixStream::connect(display).unwrap()).unwrap();
    let (globals, mut queue) = registry_queue_init::<ContextWatch>(&connection).unwrap();
    let _manager: CrownosContextManagerV1 = globals.bind(&queue.handle(), 2..=2, ()).unwrap();
    let mut watch = ContextWatch {
        times: times.clone(),
    };
    std::thread::spawn(move || while queue.blocking_dispatch(&mut watch).is_ok() {});
    times
}
