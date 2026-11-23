//! Stands in for `crownos-portal-dialog` in the nested secret typing test, where no person can
//! click "Allow". It maps a consent layer, as the real dialog does, so the compositor counts its
//! answer, then exits with the status of the first button with every extra question at its first
//! value: "Allow". Name it in `CROWN_CONSENT_DIALOG` of a nested debug compositor only.

use std::fs::File;
use std::io::Write as _;
use std::os::fd::AsFd;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use crownos_config::consent_dialog::FIRST_ANSWER;
use rustix::fs::MemfdFlags;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_shm,
    wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

/// The layer namespace the compositor knows as a consent dialog.
const NAMESPACE: &str = "crownos-portal-dialog";
const SIZE: u32 = 64;
/// Long enough for the compositor to see the layer mapped.
const SHOWN_FOR: Duration = Duration::from_millis(800);

#[derive(Default)]
struct Stub {
    configured: bool,
}

impl Dispatch<WlRegistry, GlobalListContents> for Stub {
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

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Stub {
    fn event(
        state: &mut Self,
        surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

delegate_noop!(Stub: ignore WlCompositor);
delegate_noop!(Stub: ignore WlSurface);
delegate_noop!(Stub: ignore WlShm);
delegate_noop!(Stub: ignore WlShmPool);
delegate_noop!(Stub: ignore WlBuffer);
delegate_noop!(Stub: ignore ZwlrLayerShellV1);

fn main() -> ExitCode {
    match show() {
        Ok(()) => ExitCode::from(FIRST_ANSWER),
        Err(error) => {
            eprintln!("consent_dialog_stub: {error:#}");
            ExitCode::from(crownos_config::consent_dialog::MISUSED)
        }
    }
}

fn show() -> anyhow::Result<()> {
    let connection = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Stub>(&connection)?;
    let handle = queue.handle();
    let compositor: WlCompositor = globals.bind(&handle, 4..=6, ())?;
    let shm: WlShm = globals.bind(&handle, 1..=1, ())?;
    let layers: ZwlrLayerShellV1 = globals.bind(&handle, 1..=4, ())?;
    let surface = compositor.create_surface(&handle, ());
    let layer = layers.get_layer_surface(
        &surface,
        None,
        Layer::Overlay,
        NAMESPACE.to_owned(),
        &handle,
        (),
    );
    layer.set_size(SIZE, SIZE);
    surface.commit();
    let mut stub = Stub::default();
    while !stub.configured {
        queue.blocking_dispatch(&mut stub)?;
    }
    let pixels = vec![0x80_u8; (SIZE * SIZE * 4) as usize];
    let mut memfd = File::from(rustix::fs::memfd_create(
        "consent-stub",
        MemfdFlags::CLOEXEC,
    )?);
    memfd.write_all(&pixels)?;
    let pool = shm.create_pool(memfd.as_fd(), i32::try_from(pixels.len())?, &handle, ());
    let side = i32::try_from(SIZE)?;
    let buffer = pool.create_buffer(
        0,
        side,
        side,
        side * 4,
        wl_shm::Format::Argb8888,
        &handle,
        (),
    );
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    queue.roundtrip(&mut stub)?;
    std::thread::sleep(SHOWN_FOR);
    queue.roundtrip(&mut stub)?;
    Ok(())
}
