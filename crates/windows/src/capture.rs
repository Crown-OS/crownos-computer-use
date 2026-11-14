//! One window's pixels through `ext_image_copy_capture_v1`: a shared-memory buffer the
//! compositor renders into once, then decoded and handed back.

use std::collections::HashMap;
use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_shm::{self, Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    ExtImageCopyCaptureManagerV1, Options,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};

use crate::error::WaylandError;
use crate::layer::Layer;
use crate::util::picture::{ACCEPTED, to_rgba};
use crate::{Picture, Reply};

const BYTES_PER_PIXEL: u32 = 4;

struct Pending {
    reply: Reply<Result<Picture, WaylandError>>,
    source: ExtImageCaptureSourceV1,
    session: ExtImageCopyCaptureSessionV1,
    size: Option<(u32, u32)>,
    format: Option<Format>,
    buffer: Option<Shared>,
}

struct Shared {
    pool: WlShmPool,
    buffer: WlBuffer,
    file: File,
    stride: u32,
}

impl Shared {
    fn contents(&self, len: u64) -> Result<Vec<u8>, WaylandError> {
        let mut bytes = vec![0u8; usize::try_from(len).unwrap_or(usize::MAX)];
        self.file.read_exact_at(&mut bytes, 0)?;
        Ok(bytes)
    }
}

/// Captures in flight, keyed by the object their next event arrives on.
pub(crate) struct Captures {
    shm: Option<WlShm>,
    manager: Option<ExtImageCopyCaptureManagerV1>,
    sources: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    by_session: HashMap<ObjectId, Pending>,
    frames: HashMap<ObjectId, ObjectId>,
}

impl Captures {
    pub(crate) fn new(
        shm: Option<WlShm>,
        manager: Option<ExtImageCopyCaptureManagerV1>,
        sources: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    ) -> Self {
        Self {
            shm,
            manager,
            sources,
            by_session: HashMap::new(),
            frames: HashMap::new(),
        }
    }

    pub(crate) const fn is_available(&self) -> bool {
        self.shm.is_some() && self.manager.is_some() && self.sources.is_some()
    }

    pub(crate) fn start(
        &mut self,
        toplevel: &ExtForeignToplevelHandleV1,
        reply: Reply<Result<Picture, WaylandError>>,
        queue: &QueueHandle<Layer>,
    ) {
        let (Some(manager), Some(sources)) = (&self.manager, &self.sources) else {
            let _ = reply.send(Err(WaylandError::Missing("ext_image_copy_capture_v1")));
            return;
        };
        let source = sources.create_source(toplevel, queue, ());
        let session = manager.create_session(&source, Options::empty(), queue, ());
        self.by_session.insert(
            session.id(),
            Pending {
                reply,
                source,
                session,
                size: None,
                format: None,
                buffer: None,
            },
        );
    }

    fn fail(&mut self, session: &ObjectId, error: WaylandError) {
        if let Some(pending) = self.by_session.remove(session) {
            let _ = pending.reply.send(Err(error));
            pending.session.destroy();
            pending.source.destroy();
        }
    }

    fn on_session_event(
        &mut self,
        session: &ExtImageCopyCaptureSessionV1,
        event: &ext_image_copy_capture_session_v1::Event,
        queue: &QueueHandle<Layer>,
    ) {
        let id = session.id();
        let Some(pending) = self.by_session.get_mut(&id) else {
            return;
        };
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                pending.size = Some((*width, *height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { format } => {
                if let WEnum::Value(format) = *format
                    && ACCEPTED.contains(&format)
                    && pending
                        .format
                        .is_none_or(|current| rank(format) < rank(current))
                {
                    pending.format = Some(format);
                }
            }
            ext_image_copy_capture_session_v1::Event::Done => {
                let (Some((width, height)), Some(format), Some(shm)) =
                    (pending.size, pending.format, &self.shm)
                else {
                    return self.fail(
                        &id,
                        WaylandError::Capture("no usable buffer format offered".to_owned()),
                    );
                };
                match shared_buffer(shm, width, height, format, queue) {
                    Ok(shared) => {
                        let frame = pending.session.create_frame(queue, ());
                        frame.attach_buffer(&shared.buffer);
                        frame.damage_buffer(
                            0,
                            0,
                            i32::try_from(width).unwrap_or(i32::MAX),
                            i32::try_from(height).unwrap_or(i32::MAX),
                        );
                        frame.capture();
                        pending.buffer = Some(shared);
                        self.frames.insert(frame.id(), id);
                    }
                    Err(error) => self.fail(&id, error),
                }
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                self.fail(
                    &id,
                    WaylandError::Capture("the window went away".to_owned()),
                );
            }
            _ => {}
        }
    }

    fn on_frame_event(
        &mut self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: &ext_image_copy_capture_frame_v1::Event,
    ) {
        let Some(session) = self.frames.get(&frame.id()).cloned() else {
            return;
        };
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => {
                self.frames.remove(&frame.id());
                frame.destroy();
                let Some(pending) = self.by_session.remove(&session) else {
                    return;
                };
                let picture = match (pending.size, pending.format, &pending.buffer) {
                    (Some((width, height)), Some(format), Some(shared)) => shared
                        .contents(u64::from(shared.stride) * u64::from(height))
                        .map(|bytes| to_rgba(format, width, height, shared.stride, &bytes)),
                    _ => Err(WaylandError::Capture(
                        "frame ready without a buffer".to_owned(),
                    )),
                };
                let _ = pending.reply.send(picture);
                if let Some(shared) = pending.buffer {
                    shared.buffer.destroy();
                    shared.pool.destroy();
                }
                pending.session.destroy();
                pending.source.destroy();
            }
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                self.frames.remove(&frame.id());
                frame.destroy();
                self.fail(&session, WaylandError::Capture(format!("{reason:?}")));
            }
            _ => {}
        }
    }
}

/// Lower is preferred: formats that are already RGBA need no shuffling.
fn rank(format: Format) -> usize {
    ACCEPTED
        .iter()
        .position(|accepted| *accepted == format)
        .unwrap_or(usize::MAX)
}

fn shared_buffer(
    shm: &WlShm,
    width: u32,
    height: u32,
    format: Format,
    queue: &QueueHandle<Layer>,
) -> Result<Shared, WaylandError> {
    let stride = width * BYTES_PER_PIXEL;
    let len = u64::from(stride) * u64::from(height);
    let file: File = memfd_create("crownos-computer-use-capture", MemfdFlags::CLOEXEC)?.into();
    file.set_len(len)?;
    let pool = shm.create_pool(
        file.as_fd(),
        i32::try_from(len).unwrap_or(i32::MAX),
        queue,
        (),
    );
    let buffer = pool.create_buffer(
        0,
        i32::try_from(width).unwrap_or(i32::MAX),
        i32::try_from(height).unwrap_or(i32::MAX),
        i32::try_from(stride).unwrap_or(i32::MAX),
        format,
        queue,
        (),
    );
    Ok(Shared {
        pool,
        buffer,
        file,
        stride,
    })
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for Layer {
    fn event(
        state: &mut Self,
        session: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _data: &(),
        _connection: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        state.captures.on_session_event(session, &event, queue);
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for Layer {
    fn event(
        state: &mut Self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        state.captures.on_frame_event(frame, &event);
    }
}

impl Dispatch<WlBuffer, ()> for Layer {
    fn event(
        _state: &mut Self,
        _buffer: &WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, ()> for Layer {
    fn event(
        _state: &mut Self,
        _shm: &WlShm,
        _event: wl_shm::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Layer: ignore WlShmPool);
delegate_noop!(Layer: ignore ExtImageCopyCaptureManagerV1);
delegate_noop!(Layer: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(Layer: ignore ExtImageCaptureSourceV1);
