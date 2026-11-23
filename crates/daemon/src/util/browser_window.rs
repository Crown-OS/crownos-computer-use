use crownos_computer_use_protocol::{SecretTyping, WindowInfo};

/// The one browser toplevel that shows the tab titled `tab_title`. Chromium-based browsers
/// title a window after its selected tab, so a background tab, a window of another tab or two
/// windows with the same title find nothing: the focus cannot be proven, and nothing is typed.
pub(crate) fn browser_toplevel<'windows>(
    windows: &'windows [WindowInfo],
    app_id: &str,
    tab_title: &str,
) -> Result<&'windows WindowInfo, SecretTyping> {
    if tab_title.is_empty() {
        return Err(SecretTyping::FocusChanged);
    }
    let mut matching = windows.iter().filter(|window| {
        !window.browser_tab && window.app_id == app_id && window.title.starts_with(tab_title)
    });
    match (matching.next(), matching.next()) {
        (Some(window), None) => Ok(window),
        _ => Err(SecretTyping::FocusChanged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toplevel(identifier: &str, app_id: &str, title: &str) -> WindowInfo {
        WindowInfo {
            identifier: identifier.into(),
            app_id: app_id.into(),
            title: title.into(),
            pid: 0,
            bounds: Default::default(),
            output: String::new(),
            workspace: String::new(),
            state: Default::default(),
            browser_tab: false,
        }
    }

    #[test]
    fn of_two_browser_windows_the_one_showing_the_tab_is_found() {
        let windows = [
            toplevel("w1", "chromium", "Inbox - Chromium"),
            toplevel("w2", "chromium", "Sign in - Chromium"),
            toplevel("w3", "org.gnome.Nautilus", "Sign in"),
        ];
        let found = browser_toplevel(&windows, "chromium", "Sign in");
        assert_eq!(found.map(|window| window.identifier.as_str()), Ok("w2"));
    }

    #[test]
    fn a_background_tab_has_no_window() {
        let windows = [toplevel("w1", "chromium", "Inbox - Chromium")];
        assert_eq!(
            browser_toplevel(&windows, "chromium", "Sign in").map(|_| ()),
            Err(SecretTyping::FocusChanged)
        );
    }

    #[test]
    fn two_windows_with_the_same_title_are_refused() {
        let windows = [
            toplevel("w1", "chromium", "Sign in - Chromium"),
            toplevel("w2", "chromium", "Sign in - Chromium"),
        ];
        assert_eq!(
            browser_toplevel(&windows, "chromium", "Sign in").map(|_| ()),
            Err(SecretTyping::FocusChanged)
        );
        assert_eq!(
            browser_toplevel(&windows, "chromium", "").map(|_| ()),
            Err(SecretTyping::FocusChanged)
        );
    }
}
