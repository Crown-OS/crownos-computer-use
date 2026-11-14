//! Which source answers for a window: CrownUI if the app opened its socket, the browser for its
//! tabs, AT-SPI for everything else.

use crownos_computer_use_protocol::WindowInfo;

pub(crate) const BROWSER_PREFIX: &str = "browser:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Route {
    Browser { target: String },
    CrownUi { pid: u32 },
    Atspi { pid: u32 },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RouteError {
    #[error("{0} has no known process; the compositor offers no window control")]
    NoPid(String),
}

pub(crate) fn route(
    window: &WindowInfo,
    has_crownui_socket: impl Fn(u32) -> bool,
) -> Result<Route, RouteError> {
    if let Some(target) = window.identifier.strip_prefix(BROWSER_PREFIX) {
        return Ok(Route::Browser {
            target: target.to_owned(),
        });
    }
    let pid = u32::try_from(window.pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| RouteError::NoPid(window.identifier.clone()))?;
    if has_crownui_socket(pid) {
        Ok(Route::CrownUi { pid })
    } else {
        Ok(Route::Atspi { pid })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    fn window(identifier: &str, pid: i32) -> WindowInfo {
        WindowInfo {
            identifier: identifier.to_owned(),
            app_id: String::new(),
            title: String::new(),
            pid,
            bounds: Default::default(),
            output: String::new(),
            workspace: String::new(),
            state: Default::default(),
            browser_tab: identifier.starts_with(BROWSER_PREFIX),
        }
    }

    #[test]
    fn tabs_apps_and_the_rest_go_their_own_way() {
        assert_eq!(
            route(&window("browser:AB12", 0), |_| false).unwrap(),
            Route::Browser {
                target: "AB12".into()
            }
        );
        assert_eq!(
            route(&window("w1", 42), |pid| pid == 42).unwrap(),
            Route::CrownUi { pid: 42 }
        );
        assert_eq!(
            route(&window("w1", 42), |_| false).unwrap(),
            Route::Atspi { pid: 42 }
        );
        assert!(matches!(
            route(&window("w1", 0), |_| false),
            Err(RouteError::NoPid(_))
        ));
    }
}
