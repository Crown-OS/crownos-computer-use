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

    /// Drops the value of each element of `window` that a secret was typed into, and the text
    /// of its descendants, where a browser repeats the value. Elements come in tree order.
    pub(crate) fn hide_values(&self, window: &str, elements: &mut [Element]) {
        let Some(typed) = self.by_window.get(window) else {
            return;
        };
        let mut typed_field_depth = None;
        for element in elements.iter_mut() {
            if typed_field_depth.is_some_and(|depth| element.depth > depth) {
                element.name.clear();
                element.value = None;
                continue;
            }
            typed_field_depth = typed.contains(&element.source_id).then_some(element.depth);
            if typed_field_depth.is_some() {
                element.value = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crownos_ui_elements::{Bounds, ElementRef, Source};

    use super::*;

    fn field(source_id: &str, value: &str) -> Element {
        element(source_id, "Email", Some(value), 0)
    }

    fn element(source_id: &str, name: &str, value: Option<&str>, depth: u8) -> Element {
        Element {
            reference: "@s1:e1"
                .parse::<ElementRef>()
                .unwrap_or_else(|_| unreachable!()),
            role: "textbox".into(),
            name: name.into(),
            value: value.map(str::to_owned),
            bounds: Bounds::default(),
            actions: Vec::new(),
            secure: false,
            source: Source::Browser,
            source_id: source_id.into(),
            depth,
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

    #[test]
    fn the_text_below_a_typed_field_is_hidden_too() {
        let mut typed = TypedFields::default();
        typed.remember("browser:1", &element("41", "Email", Some("alice"), 3));
        let mut elements = vec![
            element("41", "Email", Some("alice"), 3),
            element("50", "", None, 4),
            element("51", "alice", None, 5),
            element("60", "Password", None, 3),
        ];
        typed.hide_values("browser:1", &mut elements);
        let names: Vec<&str> = elements
            .iter()
            .map(|element| element.name.as_str())
            .collect();
        assert_eq!(names, ["Email", "", "", "Password"]);
        assert!(elements.iter().all(|element| element.value.is_none()));
    }
}
