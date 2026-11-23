use crownos_computer_use_protocol::{SecretTyping, WindowInfo};

/// The agent browser, as its windows show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BrowserIdentity<'browser> {
    pub(crate) app_id: &'browser str,
    /// The suffix of each window title: `<tab title> - <name>`.
    pub(crate) name: &'browser str,
    pub(crate) pid: i32,
}

/// The one toplevel of the agent browser that shows the tab titled `tab_title`. Chromium-based
/// browsers title a window `<selected tab title> - <browser name>`, so the title must be exactly
/// that: a background tab, a tab whose title only starts the same, another browser process, or
/// two windows with the same title find nothing, and nothing is typed.
pub(crate) fn browser_toplevel<'windows>(
    windows: &'windows [WindowInfo],
    browser: BrowserIdentity<'_>,
    tab_title: &str,
) -> Result<&'windows WindowInfo, SecretTyping> {
    if tab_title.is_empty() {
        return Err(SecretTyping::FocusChanged);
    }
    let expected_title = format!("{tab_title} - {}", browser.name);
    let mut matching = windows.iter().filter(|window| {
        !window.browser_tab
            && window.app_id == browser.app_id
            && window.pid == browser.pid
            && window.title == expected_title
    });
    match (matching.next(), matching.next()) {
        (Some(window), None) => Ok(window),
        _ => Err(SecretTyping::FocusChanged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHROMIUM: BrowserIdentity<'static> = BrowserIdentity {
        app_id: "chromium",
        name: "Chromium",
        pid: 40,
    };

    fn toplevel(identifier: &str, title: &str, pid: i32) -> WindowInfo {
        WindowInfo {
            identifier: identifier.into(),
            app_id: "chromium".into(),
            title: title.into(),
            pid,
            bounds: Default::default(),
            output: String::new(),
            workspace: String::new(),
            state: Default::default(),
            browser_tab: false,
        }
    }

    fn found(windows: &[WindowInfo], tab_title: &str) -> Result<String, SecretTyping> {
        browser_toplevel(windows, CHROMIUM, tab_title).map(|window| window.identifier.clone())
    }

    #[test]
    fn of_two_browser_windows_the_one_showing_the_tab_is_found() {
        let windows = [
            toplevel("w1", "Inbox - Chromium", 40),
            toplevel("w2", "Sign in - Chromium", 40),
        ];
        assert_eq!(found(&windows, "Sign in"), Ok("w2".into()));
    }

    #[test]
    fn a_title_that_only_starts_the_same_is_another_tab() {
        let windows = [toplevel("w1", "Sign in to Bank - Chromium", 40)];
        assert_eq!(found(&windows, "Sign in"), Err(SecretTyping::FocusChanged));
    }

    #[test]
    fn a_background_tab_or_another_browser_process_has_no_window() {
        let windows = [
            toplevel("w1", "Inbox - Chromium", 40),
            toplevel("w2", "Sign in - Chromium", 99),
        ];
        assert_eq!(found(&windows, "Sign in"), Err(SecretTyping::FocusChanged));
    }

    #[test]
    fn two_windows_with_the_same_title_are_refused() {
        let windows = [
            toplevel("w1", "Sign in - Chromium", 40),
            toplevel("w2", "Sign in - Chromium", 40),
        ];
        assert_eq!(found(&windows, "Sign in"), Err(SecretTyping::FocusChanged));
        assert_eq!(found(&windows, ""), Err(SecretTyping::FocusChanged));
    }
}
