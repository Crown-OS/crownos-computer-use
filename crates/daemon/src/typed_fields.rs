//! Fields a secret was typed into. Their value never goes back to an agent.

use std::collections::{HashMap, HashSet};

use crownos_ui_elements::Element;

#[derive(Default)]
pub(crate) struct TypedFields {
    by_window: HashMap<String, HashSet<String>>,
}

impl TypedFields {
    pub(crate) fn remember(&mut self, window: &str, element: &Element) {
        self.by_window
            .entry(window.to_owned())
            .or_default()
            .insert(element.source_id.clone());
    }

    pub(crate) fn forget_window(&mut self, window: &str) {
        self.by_window.remove(window);
    }

    /// Drops the value of each element of `window` that a secret was typed into.
    pub(crate) fn hide_values(&self, window: &str, elements: &mut [Element]) {
        let Some(typed) = self.by_window.get(window) else {
            return;
        };
        for element in elements
            .iter_mut()
            .filter(|element| typed.contains(&element.source_id))
        {
            element.value = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use crownos_ui_elements::{Bounds, ElementRef, Source};

    use super::*;

    fn field(source_id: &str, value: &str) -> Element {
        Element {
            reference: "@s1:e1"
                .parse::<ElementRef>()
                .unwrap_or_else(|_| unreachable!()),
            role: "textbox".into(),
            name: "Email".into(),
            value: Some(value.into()),
            bounds: Bounds::default(),
            actions: Vec::new(),
            secure: false,
            source: Source::Browser,
            source_id: source_id.into(),
            depth: 0,
        }
    }

    #[test]
    fn a_snapshot_after_typing_shows_no_value_of_the_typed_field() {
        let mut typed = TypedFields::default();
        typed.remember("browser:1", &field("41", "alice@example.com"));
        let mut elements = vec![field("41", "alice@example.com"), field("42", "search")];
        typed.hide_values("browser:1", &mut elements);
        let values = |elements: &[Element]| -> Vec<Option<String>> {
            elements
                .iter()
                .map(|element| element.value.clone())
                .collect()
        };
        assert_eq!(values(&elements), [None, Some("search".into())]);
        let mut other_window = vec![field("41", "visible")];
        typed.hide_values("browser:2", &mut other_window);
        assert_eq!(values(&other_window), [Some("visible".into())]);
        typed.forget_window("browser:1");
        let mut after_close = vec![field("41", "new page")];
        typed.hide_values("browser:1", &mut after_close);
        assert_eq!(values(&after_close), [Some("new page".into())]);
    }
}
