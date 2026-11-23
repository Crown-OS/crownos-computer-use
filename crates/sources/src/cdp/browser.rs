use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::json;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::client::CdpClient;
use crate::error::SourceError;

/// The browser a caller can launch when it has no system browser choice.
pub const DEFAULT_EXECUTABLE: &str = "/usr/bin/chromium";
/// Our end of the pipe is the child's stdin and stdout; the shell moves them to the fds
/// `--remote-debugging-pipe` reads and writes, then detaches the standard streams.
const SHELL_ADAPTER: &str = r#"exec 3<&0 4>&1 </dev/null >/dev/null; exec "$@""#;

/// `$XDG_DATA_HOME/crownos/agent-browser`: the agent's profiles, which nobody else uses.
pub fn default_profile_dir() -> PathBuf {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    data_home.join("crownos/agent-browser")
}

/// A Chromium-based browser this crate launched and talks to.
#[derive(Debug)]
pub struct Browser {
    child: Child,
    client: CdpClient,
    sessions: Mutex<HashMap<String, String>>,
}

impl Browser {
    /// Starts the Chromium-based browser at `executable` on `profile_dir` and connects.
    ///
    /// # Errors
    ///
    /// Fails when the browser is not installed or dies before answering.
    pub async fn launch(
        executable: &Path,
        profile_dir: &Path,
        extra_args: &[String],
    ) -> Result<Self, SourceError> {
        if !executable.is_file() {
            return Err(SourceError::Browser(format!(
                "{} is not installed",
                executable.display()
            )));
        }
        std::fs::create_dir_all(profile_dir)?;
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(SHELL_ADAPTER)
            .arg("sh")
            .arg(executable)
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--remote-debugging-pipe")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .args(extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let (Some(stdout), Some(stdin)) = (child.stdout.take(), child.stdin.take()) else {
            return Err(SourceError::Browser("no pipe to the browser".to_owned()));
        };
        let client = CdpClient::new(stdout, stdin);
        client.send("Target.getTargets", json!({}), None).await?;
        Ok(Self {
            child,
            client,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub(super) const fn client(&self) -> &CdpClient {
        &self.client
    }

    /// The browser's process id. The launcher shell `exec`s the browser, so this is the
    /// process that connects to the compositor.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Whether the browser process is still there.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Closes the browser.
    pub async fn close(mut self) {
        let _ = self.client.send("Browser.close", json!({}), None).await;
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        let _ = self.child.kill().await;
    }

    /// The session attached to `target_id`, attaching on first use.
    pub(super) async fn session(&self, target_id: &str) -> Result<String, SourceError> {
        if let Some(session) = self.sessions.lock().await.get(target_id) {
            return Ok(session.clone());
        }
        let attached = self
            .client
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            )
            .await?;
        let session = attached["sessionId"]
            .as_str()
            .ok_or_else(|| SourceError::Browser("attachToTarget gave no sessionId".to_owned()))?
            .to_owned();
        self.sessions
            .lock()
            .await
            .insert(target_id.to_owned(), session.clone());
        Ok(session)
    }

    pub(super) async fn forget_session(&self, target_id: &str) {
        self.sessions.lock().await.remove(target_id);
    }
}
