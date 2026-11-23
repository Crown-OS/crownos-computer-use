//! Typing stored secrets: the daemon proves where the keyboard focus is, names the window, app
//! and page origin, and asks crownos-secretsd to have the compositor type the value. The value
//! never passes through this process.

use std::time::{Duration, Instant};

use crownos_computer_use_protocol::{ActOutcome, SecretReport, SecretTyping};
use crownos_ipc::PeerId;
use crownos_ipc::adapter::tokio::AsyncClient;
use crownos_ui_elements::{Action, Element, ElementRef, ResolveError};
use secrets_core::{SecretName, UseOutcome};
use secrets_protocol::{TypingTarget, error_code, secrets};

use crate::daemon::{AUDIT, Daemon, DaemonError};
use crate::resolve::BROWSER_PREFIX;
use crate::sensitive_windows::SENSITIVE_AFTER;
use crate::util::browser_window::browser_toplevel;
use crate::util::origin::origin_of;

/// How long the browser window has to come to the front and take the focus.
const FOCUS_WAIT: Duration = Duration::from_millis(1500);
const FOCUS_POLL: Duration = Duration::from_millis(50);
/// Room for the confirmation and the typing, before the time after it.
const TYPING_TIME: Duration = Duration::from_secs(150);

/// An element of a snapshot that a secret may go into.
struct SecretField {
    window: String,
    element: Element,
}

/// What one request types: one secret, or a login.
enum Typing {
    Secret(SecretName),
    Login {
        username: SecretName,
        password: SecretName,
    },
}

impl Daemon {
    pub(crate) async fn type_secret(
        &mut self,
        peer: PeerId,
        reference: &str,
        name: &str,
    ) -> Result<ActOutcome, DaemonError> {
        let Ok(name) = SecretName::parse(name) else {
            return Ok(report(SecretTyping::Unknown, None));
        };
        let field = match self.secret_field(peer, reference)? {
            Ok(field) => field,
            Err(refusal) => return Ok(refusal),
        };
        if !field.element.secure {
            return Ok(report(SecretTyping::NotSecure, None));
        }
        tracing::info!(target: AUDIT, agent = %self.agents.name(peer), %reference, secret = %name, window = %field.window, "type secret");
        self.type_into_field(&field, Typing::Secret(name)).await
    }

    pub(crate) async fn fill_login(
        &mut self,
        peer: PeerId,
        username_reference: &str,
        password_reference: &str,
        username_name: &str,
        password_name: &str,
    ) -> Result<ActOutcome, DaemonError> {
        let (Ok(username), Ok(password)) = (
            SecretName::parse(username_name),
            SecretName::parse(password_name),
        ) else {
            return Ok(report(SecretTyping::Unknown, None));
        };
        let username_field = match self.secret_field(peer, username_reference)? {
            Ok(field) => field,
            Err(refusal) => return Ok(refusal),
        };
        let password_field = match self.secret_field(peer, password_reference)? {
            Ok(field) => field,
            Err(refusal) => return Ok(refusal),
        };
        if !is_login_pair(&username_field, &password_field) {
            return Ok(report(SecretTyping::NotSecure, None));
        }
        tracing::info!(target: AUDIT, agent = %self.agents.name(peer), %username_reference, %password_reference, username = %username, password = %password, window = %username_field.window, "fill login");
        let outcome = self
            .type_into_field(&username_field, Typing::Login { username, password })
            .await?;
        self.typed_fields
            .remember(&username_field.window, &username_field.element);
        Ok(outcome)
    }

    /// Resolves a ref for typing, under the agent's rate limit. `Err` is the outcome to answer.
    fn secret_field(
        &mut self,
        peer: PeerId,
        reference: &str,
    ) -> Result<Result<SecretField, ActOutcome>, DaemonError> {
        let agent = self.agents.get_mut(peer).ok_or(DaemonError::NoHello)?;
        if !agent.limiter.allow() {
            return Ok(Err(ActOutcome::RateLimited));
        }
        let reference = match reference.parse::<ElementRef>() {
            Ok(reference) => reference,
            Err(error) => return Ok(Err(ActOutcome::Failed(error.to_string()))),
        };
        match self.snapshots.resolve(reference) {
            Ok((snapshot, element)) => Ok(Ok(SecretField {
                window: snapshot.window.0.clone(),
                element: element.clone(),
            })),
            Err(ResolveError::Stale(_, newer)) => Ok(Err(ActOutcome::Stale(format!("@s{newer}")))),
            Err(error) => Ok(Err(ActOutcome::Failed(error.to_string()))),
        }
    }

    async fn type_into_field(
        &mut self,
        field: &SecretField,
        typing: Typing,
    ) -> Result<ActOutcome, DaemonError> {
        let target = match self.typing_target(field).await? {
            Ok(target) => target,
            Err(outcome) => return Ok(report(outcome, None)),
        };
        let windows = [field.window.clone(), target.window.clone()];
        for window in &windows {
            self.sensitive.mark(window, TYPING_TIME + SENSITIVE_AFTER);
        }
        let outcome = ask_secretsd(typing, target).await;
        for window in &windows {
            self.sensitive.mark(window, SENSITIVE_AFTER);
        }
        let origin = match field.window.strip_prefix(BROWSER_PREFIX) {
            Some(target_id) => self
                .browser_tab_url(target_id)
                .await?
                .and_then(|url| origin_of(&url)),
            None => None,
        };
        Ok(report(outcome, origin))
    }

    /// Puts the keyboard focus on the field and names the window it is in. For a browser tab it
    /// brings the tab to the front, focuses the one toplevel that shows it, and checks in the
    /// page that the element holds the focus. `Err` is the outcome to answer.
    async fn typing_target(
        &mut self,
        field: &SecretField,
    ) -> Result<Result<TypingTarget, SecretTyping>, DaemonError> {
        let window = self.window(&field.window).await?;
        let Some(target_id) = field.window.strip_prefix(BROWSER_PREFIX) else {
            if self
                .perform(&window, &field.element, &Action::Focus)
                .await
                .is_err()
            {
                return Ok(Err(SecretTyping::FocusChanged));
            }
            return Ok(Ok(TypingTarget {
                window: window.identifier,
                app_id: window.app_id,
                origin: None,
            }));
        };
        self.browser().await?.bring_to_front(target_id).await?;
        let Some(toplevel) = self.browser_toplevel_of(target_id, &window.app_id).await? else {
            return Ok(Err(SecretTyping::FocusChanged));
        };
        self.compositor.focus(&toplevel)?;
        let activated = self
            .wait_for(|daemon| {
                Ok(daemon
                    .compositor
                    .windows()?
                    .iter()
                    .any(|candidate| candidate.identifier == toplevel && candidate.state.activated))
            })
            .await?;
        if !activated
            || !self
                .browser()
                .await?
                .focus_and_check(target_id, &field.element)
                .await?
        {
            return Ok(Err(SecretTyping::FocusChanged));
        }
        let origin = self
            .browser_tab_url(target_id)
            .await?
            .and_then(|url| origin_of(&url));
        Ok(Ok(TypingTarget {
            window: toplevel,
            app_id: window.app_id,
            origin,
        }))
    }

    /// The toplevel whose title shows the tab, once the browser has retitled it.
    async fn browser_toplevel_of(
        &mut self,
        target_id: &str,
        app_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let deadline = Instant::now() + FOCUS_WAIT;
        loop {
            let title = self
                .browser()
                .await?
                .tabs()
                .await?
                .into_iter()
                .find(|tab| tab.target_id == target_id)
                .map(|tab| tab.title)
                .unwrap_or_default();
            let windows = self.compositor.windows()?;
            if let Ok(toplevel) = browser_toplevel(&windows, app_id, &title) {
                return Ok(Some(toplevel.identifier.clone()));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(FOCUS_POLL).await;
        }
    }

    async fn wait_for(
        &mut self,
        mut reached: impl FnMut(&mut Self) -> Result<bool, DaemonError>,
    ) -> Result<bool, DaemonError> {
        let deadline = Instant::now() + FOCUS_WAIT;
        loop {
            if reached(self)? {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(FOCUS_POLL).await;
        }
    }
}

/// A user name field and a password field of the same window.
fn is_login_pair(username: &SecretField, password: &SecretField) -> bool {
    !username.element.secure && password.element.secure && username.window == password.window
}

const fn report(outcome: SecretTyping, origin: Option<String>) -> ActOutcome {
    ActOutcome::Secret(SecretReport { outcome, origin })
}

/// One `type_into` or `fill_login` call to crownos-secretsd on a fresh connection.
async fn ask_secretsd(typing: Typing, target: TypingTarget) -> SecretTyping {
    let Ok(mut client) = AsyncClient::connect(secrets::SERVICE) else {
        return SecretTyping::Unavailable;
    };
    let answer = match typing {
        Typing::Secret(name) => {
            client
                .call::<secrets::type_into>(&secrets::type_into { name, target }, Vec::new())
                .await
        }
        Typing::Login { username, password } => {
            client
                .call::<secrets::fill_login>(
                    &secrets::fill_login {
                        username,
                        password,
                        target,
                    },
                    Vec::new(),
                )
                .await
        }
    };
    match answer {
        Ok(outcome) => secret_typing(outcome),
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
        UseOutcome::UnknownWindow => SecretTyping::UnknownWindow,
        UseOutcome::TimedOut => SecretTyping::TimedOut,
        UseOutcome::TargetMismatch => SecretTyping::TargetMismatch,
        UseOutcome::SessionLocked => SecretTyping::SessionLocked,
    }
}

#[cfg(test)]
mod tests {
    use crownos_ui_elements::{Bounds, Source};

    use super::*;

    fn field(window: &str, secure: bool) -> SecretField {
        SecretField {
            window: window.into(),
            element: Element {
                reference: "@s1:e1"
                    .parse::<ElementRef>()
                    .unwrap_or_else(|_| unreachable!()),
                role: "textbox".into(),
                name: String::new(),
                value: None,
                bounds: Bounds::default(),
                actions: Vec::new(),
                secure,
                source: Source::Browser,
                source_id: "7".into(),
                depth: 0,
            },
        }
    }

    #[test]
    fn a_login_needs_a_plain_field_then_a_secure_one_in_one_window() {
        assert!(is_login_pair(
            &field("browser:1", false),
            &field("browser:1", true)
        ));
        assert!(!is_login_pair(
            &field("browser:1", false),
            &field("browser:1", false)
        ));
        assert!(!is_login_pair(
            &field("browser:1", true),
            &field("browser:1", true)
        ));
        assert!(!is_login_pair(
            &field("browser:1", false),
            &field("browser:2", true)
        ));
    }
}
