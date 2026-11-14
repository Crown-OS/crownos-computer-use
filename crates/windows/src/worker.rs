use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::thread::JoinHandle;

use rustix::event::{EventfdFlags, PollFd, PollFlags, eventfd, poll};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};

use crate::error::WaylandError;

/// The state a worker thread owns: its protocol objects and what commands do to them.
pub(crate) trait WorkerState:
    Dispatch<WlRegistry, GlobalListContents> + Sized + 'static
{
    type Command: Send + 'static;

    fn apply(&mut self, command: Self::Command, queue: &QueueHandle<Self>);
}

/// A Wayland connection and event queue on a dedicated thread, driven by commands. Dropping the
/// handle stops the thread and destroys everything it created.
#[derive(Debug)]
pub(crate) struct WorkerHandle<C> {
    commands: Option<mpsc::Sender<C>>,
    wake: OwnedFd,
    thread: Option<JoinHandle<()>>,
}

impl<C: Send + 'static> WorkerHandle<C> {
    /// Connects, lets `init` bind what it needs, and returns once the first roundtrip is done.
    pub(crate) fn spawn<S, Init>(name: &str, init: Init) -> Result<Self, WaylandError>
    where
        S: WorkerState<Command = C>,
        Init: FnOnce(&GlobalList, &QueueHandle<S>) -> Result<S, WaylandError> + Send + 'static,
    {
        let wake = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        let thread_wake = wake.try_clone()?;
        let (commands, receiver) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let mut ready = Some(ready);
                if let Err(error) = run(init, &receiver, &thread_wake, &mut ready) {
                    match ready.take() {
                        Some(ready) => {
                            let _ = ready.send(Err(error));
                        }
                        None => tracing::warn!(%error, "wayland worker stopped"),
                    }
                }
            })?;
        let handle = Self {
            commands: Some(commands),
            wake,
            thread: Some(thread),
        };
        started.recv().map_err(|_| WaylandError::ThreadGone)??;
        Ok(handle)
    }

    /// # Errors
    ///
    /// Fails once the thread has exited.
    pub(crate) fn send(&self, command: C) -> Result<(), WaylandError> {
        self.commands
            .as_ref()
            .ok_or(WaylandError::ThreadGone)?
            .send(command)
            .map_err(|_| WaylandError::ThreadGone)?;
        wake(&self.wake);
        Ok(())
    }
}

impl<C> Drop for WorkerHandle<C> {
    fn drop(&mut self) {
        self.commands = None;
        wake(&self.wake);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn wake(eventfd: &OwnedFd) {
    let _ = rustix::io::write(eventfd, &1_u64.to_ne_bytes());
}

fn run<S, Init>(
    init: Init,
    commands: &mpsc::Receiver<S::Command>,
    wake: &OwnedFd,
    ready: &mut Option<mpsc::SyncSender<Result<(), WaylandError>>>,
) -> Result<(), WaylandError>
where
    S: WorkerState,
    Init: FnOnce(&GlobalList, &QueueHandle<S>) -> Result<S, WaylandError>,
{
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<S>(&connection)?;
    let handle = queue.handle();
    let mut state = init(&globals, &handle)?;
    queue.roundtrip(&mut state)?;
    if let Some(ready) = ready.take() {
        let _ = ready.send(Ok(()));
    }
    loop {
        loop {
            match commands.try_recv() {
                Ok(command) => state.apply(command, &handle),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        wait(&mut queue, &mut state, wake)?;
    }
}

fn wait<S: WorkerState>(
    queue: &mut EventQueue<S>,
    state: &mut S,
    wake: &OwnedFd,
) -> Result<(), WaylandError> {
    queue.flush()?;
    queue.dispatch_pending(state)?;
    queue.flush()?;
    let Some(guard) = queue.prepare_read() else {
        return Ok(());
    };
    let (wayland_ready, woken) = {
        let wayland_fd = guard.connection_fd();
        let mut fds = [
            PollFd::new(&wayland_fd, PollFlags::IN),
            PollFd::new(wake, PollFlags::IN),
        ];
        match poll(&mut fds, None) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(errno) => return Err(errno.into()),
        }
        let [wayland, woken] = fds.map(|fd| !fd.revents().is_empty());
        (wayland, woken)
    };
    if woken {
        let _ = rustix::io::read(wake, &mut [0_u8; 8]);
    }
    if wayland_ready {
        guard.read()?;
    } else {
        drop(guard);
    }
    queue.dispatch_pending(state)?;
    Ok(())
}
