use std::fs::File;
use std::io::Read as _;

use crownos_computer_use_protocol::computer_use::{self as proto};
use crownos_computer_use_protocol::{ActOutcome, Screenshot, WindowInfo};
use crownos_ipc::adapter::tokio::AsyncClient;
use crownos_ui_elements::{Action, Element, Snapshot};
use rustix::fs::{MemfdFlags, memfd_create};
use tokio::sync::Mutex;

/// The daemon, as one agent's connection.
pub struct Bridge {
    client: Mutex<AsyncClient>,
    pub(crate) agent: String,
}

impl std::fmt::Debug for Bridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Bridge")
            .field("agent", &self.agent)
            .finish_non_exhaustive()
    }
}

type Reply<T> = Result<T, crownos_ipc::Error>;

impl Bridge {
    /// Connects and introduces the agent.
    ///
    /// # Errors
    ///
    /// Fails when the daemon is not running.
    pub async fn connect(agent: &str, color: u32) -> Reply<Self> {
        let mut client = AsyncClient::connect(proto::SERVICE)?;
        client
            .call::<proto::hello>(
                &proto::hello {
                    agent: agent.to_owned(),
                    color,
                },
                Vec::new(),
            )
            .await?;
        Ok(Self {
            client: Mutex::new(client),
            agent: agent.to_owned(),
        })
    }

    pub(crate) async fn list_windows(&self) -> Reply<Vec<WindowInfo>> {
        self.client
            .lock()
            .await
            .call::<proto::list_windows>(&proto::list_windows {}, Vec::new())
            .await
    }

    pub(crate) async fn snapshot(&self, window: String, interactive_only: bool) -> Reply<Snapshot> {
        self.client
            .lock()
            .await
            .call::<proto::snapshot>(
                &proto::snapshot {
                    window,
                    interactive_only,
                },
                Vec::new(),
            )
            .await
    }

    pub(crate) async fn find(
        &self,
        window: String,
        role: Option<String>,
        name: Option<String>,
        text: Option<String>,
    ) -> Reply<Vec<Element>> {
        self.client
            .lock()
            .await
            .call::<proto::find>(
                &proto::find {
                    window,
                    role,
                    name,
                    text,
                },
                Vec::new(),
            )
            .await
    }

    pub(crate) async fn act(&self, reference: String, action: Action) -> Reply<ActOutcome> {
        self.client
            .lock()
            .await
            .call::<proto::act>(&proto::act { reference, action }, Vec::new())
            .await
    }

    pub(crate) async fn type_secret(&self, reference: String, name: String) -> Reply<ActOutcome> {
        self.client
            .lock()
            .await
            .call::<proto::type_secret>(&proto::type_secret { reference, name }, Vec::new())
            .await
    }

    pub(crate) async fn fill_login(
        &self,
        username_reference: String,
        password_reference: String,
        username_name: String,
        password_name: String,
    ) -> Reply<ActOutcome> {
        self.client
            .lock()
            .await
            .call::<proto::fill_login>(
                &proto::fill_login {
                    username_reference,
                    password_reference,
                    username_name,
                    password_name,
                },
                Vec::new(),
            )
            .await
    }

    /// The window as PNG bytes with its size.
    pub(crate) async fn screenshot(&self, window: String) -> Reply<(Screenshot, Vec<u8>)> {
        let sink = memfd_create("crownos-computer-use-screenshot", MemfdFlags::CLOEXEC)
            .map_err(|errno| crownos_ipc::Error::Io(errno.into()))?;
        let mut file = File::from(sink.try_clone().map_err(crownos_ipc::Error::Io)?);
        let info = self
            .client
            .lock()
            .await
            .call::<proto::screenshot>(&proto::screenshot { window }, vec![sink])
            .await?;
        let mut png = Vec::with_capacity(usize::try_from(info.png_bytes).unwrap_or_default());
        file.read_to_end(&mut png).map_err(crownos_ipc::Error::Io)?;
        Ok((info, png))
    }

    pub(crate) async fn focus(&self, window: String) -> Reply<()> {
        self.client
            .lock()
            .await
            .call::<proto::focus>(&proto::focus { window }, Vec::new())
            .await
    }

    pub(crate) async fn close(&self, window: String) -> Reply<()> {
        self.client
            .lock()
            .await
            .call::<proto::close>(&proto::close { window }, Vec::new())
            .await
    }

    pub(crate) async fn make_visible(&self, window: String) -> Reply<()> {
        self.client
            .lock()
            .await
            .call::<proto::make_visible>(&proto::make_visible { window }, Vec::new())
            .await
    }

    pub(crate) async fn make_hidden(&self, window: String) -> Reply<()> {
        self.client
            .lock()
            .await
            .call::<proto::make_hidden>(&proto::make_hidden { window }, Vec::new())
            .await
    }

    pub(crate) async fn launch(&self, command: Vec<String>, headless: bool) -> Reply<u32> {
        self.client
            .lock()
            .await
            .call::<proto::launch>(&proto::launch { command, headless }, Vec::new())
            .await
    }

    pub(crate) async fn agent_workspace(&self) -> Reply<String> {
        self.client
            .lock()
            .await
            .call::<proto::agent_workspace>(&proto::agent_workspace {}, Vec::new())
            .await
    }

    pub(crate) async fn open_url(&self, url: String) -> Reply<WindowInfo> {
        self.client
            .lock()
            .await
            .call::<proto::open_url>(&proto::open_url { url }, Vec::new())
            .await
    }
}
