//! The browser the user chose at install time, which the agent browser launches.

use std::path::Path;

use crownos_config::system::{Browser, BrowserEngine, SetupConfig};

/// The browser in `/etc/crownos/setup.ron` as it can launch now.
pub(crate) fn chosen_browser() -> Browser {
    let choice = match SetupConfig::load() {
        Ok(Some(config)) => config.browser,
        Ok(None) => Browser::Chromium,
        Err(error) => {
            tracing::warn!(%error, "the agent browser falls back to Chromium");
            Browser::Chromium
        }
    };
    resolve(choice, |executable| executable.is_file())
}

/// Chromium while the chosen Chromium-based browser is not installed yet. Firefox stays, so the
/// caller can refuse it.
fn resolve(choice: Browser, is_installed: impl Fn(&Path) -> bool) -> Browser {
    match choice.engine() {
        BrowserEngine::Chromium if !is_installed(Path::new(choice.executable())) => {
            Browser::Chromium
        }
        _ => choice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_choice_is_kept() {
        assert_eq!(resolve(Browser::Brave, |_| true), Browser::Brave);
    }

    #[test]
    fn a_missing_choice_falls_back_to_chromium() {
        assert_eq!(resolve(Browser::Brave, |_| false), Browser::Chromium);
    }

    #[test]
    fn firefox_is_kept_for_the_refusal() {
        assert_eq!(resolve(Browser::Firefox, |_| false), Browser::Firefox);
    }

    #[test]
    fn a_missing_chromium_stays_chromium() {
        assert_eq!(resolve(Browser::Chromium, |_| false), Browser::Chromium);
    }
}
