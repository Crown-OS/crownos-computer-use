//! Typing stored secrets: the daemon focuses the element, names the window, app and page
//! origin, and asks crownos-secretsd to have the compositor type the value. The value never
//! passes through this process.

use crownos_computer_use_protocol::{ActOutcome, SecretTyping};
use crownos_ipc::PeerId;
use crownos_ipc::adapter::tokio::AsyncClient;
use crownos_ui_elements::Action;
use secrets_core::{SecretName, UseOutcome};
use secrets_protocol::{TypingTarget, error_code, secrets};

use crate::daemon::{AUDIT, Daemon, DaemonError};
use crate::resolve::BROWSER_PREFIX;
use crate::util::origin::origin_of;

impl Daemon {
    pub(crate) async fn type_secret(
        &mut self,
        peer: PeerId,
        reference: &str,
        name: &str,
    ) -> Result<ActOutcome, DaemonError> {
        let Ok(name) = SecretName::parse(name) else {
            return Ok(ActOutcome::Secret(SecretTyping::Unknown));
        };
        let target = match self.focus_for_typing(peer, reference).await? {
            Ok(target) => target,
            Err(refusal) => return Ok(refusal),
        };
        tracing::info!(target: AUDIT, agent = %self.agents.name(peer), %reference, secret = %name, window = %target.window, "type secret");
        Ok(ActOutcome::Secret(type_into(name, target).await))
    }

    pub(crate) async fn fill_login(
        &mut self,
        peer: PeerId,
        username_reference: &str,
        password_reference: &str,
        username_name: &str,
        password_name: &str,
    ) -> Result<ActOutcome, DaemonError> {
        let typed = self
            .type_secret(peer, username_reference, username_name)
            .await?;
        if typed != ActOutcome::Secret(SecretTyping::Typed) {
            return Ok(typed);
        }
        self.type_secret(peer, password_reference, password_name)
            .await
    }

    /// Focuses the element and finds where its text goes. `Err` is the outcome to answer at once.
    async fn focus_for_typing(
        &mut self,
        peer: PeerId,
        reference: &str,
    ) -> Result<Result<TypingTarget, ActOutcome>, DaemonError> {
        let focused = self.act(peer, reference, &Action::Focus).await?;
        if focused != ActOutcome::Done {
            return Ok(Err(focused));
        }
        let Ok(reference) = reference.parse::<crownos_ui_elements::ElementRef>() else {
            return Ok(Err(ActOutcome::Failed("not a ref".into())));
        };
        let (snapshot, _) = self.snapshots.resolve(reference)?;
        let identifier = snapshot.window.0.clone();
        let window = self.window(&identifier).await?;
        let Some(target_id) = identifier.strip_prefix(BROWSER_PREFIX) else {
            return Ok(Ok(TypingTarget {
                window: window.identifier,
                app_id: window.app_id,
                origin: None,
            }));
        };
        let origin = self
            .browser_tab_url(target_id)
            .await?
            .and_then(|url| origin_of(&url));
        let toplevel = self
            .compositor
            .windows()?
            .into_iter()
            .find(|candidate| candidate.app_id == window.app_id)
            .map(|candidate| candidate.identifier);
        let Some(toplevel) = toplevel else {
            return Ok(Err(ActOutcome::Secret(SecretTyping::FocusChanged)));
        };
        Ok(Ok(TypingTarget {
            window: toplevel,
            app_id: window.app_id,
            origin,
        }))
    }
}

/// One `type_into` call to crownos-secretsd on a fresh connection.
async fn type_into(name: SecretName, target: TypingTarget) -> SecretTyping {
    let Ok(mut client) = AsyncClient::connect(secrets::SERVICE) else {
        return SecretTyping::Unavailable;
    };
    match client
        .call::<secrets::type_into>(&secrets::type_into { name, target }, Vec::new())
        .await
    {
        Ok(outcome) => secret_typing(outcome),
        Err(crownos_ipc::Error::Remote(remote)) if remote.code == error_code::UNAVAILABLE => {
            SecretTyping::Unavailable
        }
        Err(crownos_ipc::Error::Remote(remote)) if remote.code == error_code::LOCKED => {
            SecretTyping::Locked
        }
        Err(_) => SecretTyping::Unavailable,
    }
}

const fn secret_typing(outcome: UseOutcome) -> SecretTyping {
    match outcome {
        UseOutcome::Done => SecretTyping::Typed,
        UseOutcome::Denied => SecretTyping::Denied,
        UseOutcome::NotPasswordField => SecretTyping::NotPasswordField,
        UseOutcome::OriginMismatch => SecretTyping::OriginMismatch,
        UseOutcome::RateLimited => SecretTyping::RateLimited,
        UseOutcome::Locked => SecretTyping::Locked,
        UseOutcome::Unknown => SecretTyping::Unknown,
        UseOutcome::NeedsConfirmation => SecretTyping::NeedsConfirmation,
        UseOutcome::FocusChanged => SecretTyping::FocusChanged,
        UseOutcome::Cancelled => SecretTyping::Cancelled,
    }
}
