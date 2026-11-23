//! Windows that a secret is typed into, which screenshots refuse until a while after.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a window stays sensitive after the typing ends, as the compositor keeps it.
pub(crate) const SENSITIVE_AFTER: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct SensitiveWindows {
    until: HashMap<String, Instant>,
}

impl SensitiveWindows {
    /// Keeps `identifier` sensitive from now for `duration`, replacing an earlier mark.
    pub(crate) fn mark(&mut self, identifier: &str, duration: Duration) {
        self.until
            .insert(identifier.to_owned(), Instant::now() + duration);
    }

    pub(crate) fn is_sensitive(&mut self, identifier: &str) -> bool {
        let now = Instant::now();
        self.until.retain(|_, until| *until > now);
        self.until.contains_key(identifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_lasts_its_time_only() {
        let mut windows = SensitiveWindows::default();
        windows.mark("browser:1", Duration::from_secs(60));
        windows.mark("w2", Duration::from_secs(60));
        windows.mark("w2", Duration::ZERO);
        assert!(windows.is_sensitive("browser:1"));
        assert!(!windows.is_sensitive("w2"));
        assert!(!windows.is_sensitive("w3"));
    }
}
