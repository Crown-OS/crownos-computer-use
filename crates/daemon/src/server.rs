//! The crownos-ipc service: every request runs on the daemon with the caller's identity.

use std::os::fd::OwnedFd;
use std::task::Poll;

use anyhow::{Context, Result};
use crownos_computer_use_protocol::computer_use::{self, Handler};
use crownos_computer_use_protocol::{ActOutcome, Screenshot, WindowInfo};
use crownos_ipc::adapter::tokio::serve_with_outbox;
use crownos_ipc::{PeerId, RemoteError, Server, ServiceBuilder};
use crownos_ui_elements::{Action, Element, Query, Snapshot};
use crownos_window_layer::{Compositor, WindowEvent};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::daemon::Daemon;

pub(crate) async fn run() -> Result<()> {
    let (compositor, events) = Compositor::connect().context("connecting to the compositor")?;
    let capabilities = compositor.capabilities()?;
    tracing::info!(?capabilities, "connected to the compositor");
    let mut daemon = Daemon::new(compositor);
    let mut server = ServiceBuilder::new(computer_use::SERVICE)
        .build()
        .context("binding the daemon socket")?;
    let handle = Handle::current();
    let mut events = events;
    tokio::select! {
        served = serve_with_outbox(
            &mut server,
            |server, peer, message| {
                dispatch(&handle, &mut daemon, server, peer, message)
            },
            |cx| poll_event(&mut events, cx),
            emit,
        ) => served.context("serving"),
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}

fn poll_event(
    events: &mut UnboundedReceiver<WindowEvent>,
    cx: &mut std::task::Context<'_>,
) -> Poll<Option<WindowEvent>> {
    events.poll_recv(cx)
}

fn emit(server: &mut Server, event: WindowEvent) {
    let sent = match event {
        WindowEvent::Opened(window) => server.emit(&computer_use::WindowOpened { window }),
        WindowEvent::Changed(window) => server.emit(&computer_use::WindowOpened { window }),
        WindowEvent::Closed(identifier) => server.emit(&computer_use::WindowClosed { identifier }),
        WindowEvent::Focus(identifier) => server.emit(&computer_use::FocusChanged { identifier }),
    };
    if let Err(error) = sent {
        tracing::debug!(%error, "cannot broadcast a window event");
    }
}

fn dispatch(
    handle: &Handle,
    daemon: &mut Daemon,
    server: &mut Server,
    peer: PeerId,
    message: crownos_ipc::Message,
) -> Result<bool, crownos_ipc::Error> {
    let mut requests = Requests {
        handle,
        daemon,
        peer,
    };
    computer_use::dispatch(&mut requests, server, peer, message)?;
    if server.peer_fd(peer).is_none() {
        daemon.goodbye(peer);
    }
    Ok(true)
}

struct Requests<'a> {
    handle: &'a Handle,
    daemon: &'a mut Daemon,
    peer: PeerId,
}

impl Handler for Requests<'_> {
    fn hello(&mut self, agent: String, color: u32) -> Result<u32, RemoteError> {
        let peer = self.peer;
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        Ok(tokio::task::block_in_place(|| {
            handle.block_on(daemon.hello(peer, agent, color))
        }))
    }

    fn list_windows(&mut self) -> Result<Vec<WindowInfo>, RemoteError> {
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| handle.block_on(daemon.list_windows()))
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn snapshot(
        &mut self,
        window: String,
        interactive_only: bool,
    ) -> Result<Snapshot, RemoteError> {
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| handle.block_on(daemon.snapshot(&window, interactive_only)))
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn find(
        &mut self,
        window: String,
        role: Option<String>,
        name: Option<String>,
        text: Option<String>,
    ) -> Result<Vec<Element>, RemoteError> {
        Ok(self.daemon.find(&window, &Query { role, name, text }))
    }

    fn act(&mut self, reference: String, action: Action) -> Result<ActOutcome, RemoteError> {
        let peer = self.peer;
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| handle.block_on(daemon.act(peer, &reference, &action)))
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn type_secret(&mut self, reference: String, name: String) -> Result<ActOutcome, RemoteError> {
        let peer = self.peer;
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| handle.block_on(daemon.type_secret(peer, &reference, &name)))
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn fill_login(
        &mut self,
        username_reference: String,
        password_reference: String,
        username_name: String,
        password_name: String,
    ) -> Result<ActOutcome, RemoteError> {
        let peer = self.peer;
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| {
            handle.block_on(daemon.fill_login(
                peer,
                &username_reference,
                &password_reference,
                &username_name,
                &password_name,
            ))
        })
        .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn screenshot(&mut self, window: String, sink: OwnedFd) -> Result<Screenshot, RemoteError> {
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| handle.block_on(daemon.screenshot(&window, sink)))
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn focus(&mut self, window: String) -> Result<(), RemoteError> {
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| handle.block_on(daemon.focus(&window)))
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn close(&mut self, window: String) -> Result<(), RemoteError> {
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| handle.block_on(daemon.close(&window)))
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn make_visible(&mut self, window: String) -> Result<(), RemoteError> {
        self.daemon
            .compositor
            .make_visible(&window)
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn make_hidden(&mut self, window: String) -> Result<(), RemoteError> {
        self.daemon.forget(&window);
        self.daemon
            .compositor
            .make_hidden(&window)
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn launch(&mut self, command: Vec<String>, headless: bool) -> Result<u32, RemoteError> {
        self.daemon
            .launch(&command, headless)
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn agent_workspace(&mut self) -> Result<String, RemoteError> {
        self.daemon
            .compositor
            .agent_workspace()
            .map_err(|error| RemoteError::handler(error.to_string()))
    }

    fn open_url(&mut self, url: String) -> Result<WindowInfo, RemoteError> {
        let daemon = &mut *self.daemon;
        let handle = self.handle;
        tokio::task::block_in_place(|| handle.block_on(daemon.open_url(&url)))
            .map_err(|error| RemoteError::handler(error.to_string()))
    }
}
