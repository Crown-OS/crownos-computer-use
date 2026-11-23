//! Fields a secret was typed into. Their value never goes back to an agent: a snapshot of the
//! view the field came from hides it and its text, and every other read of the window is
//! refused while the field is live. A browser field stays live until the page navigates or the
//! field leaves the document; another field until its window closes.

use std::collections::HashSet;

use crownos_ui_elements::Element;

/// One field, remembered under the view it came from and its toplevel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypedField {
    /// The snapshot window of the field: a `browser:` tab or a toplevel.
    pub(crate) view: String,
    /// The compositor toplevel that shows the field.
    pub(crate) toplevel: String,
    pub(crate) source_id: String,
    /// The main-frame document the field was in, for a browser tab.
    pub(crate) loader_id: Option<String>,
}

#[derive(Default)]
pub(crate) struct TypedFields {
    fields: Vec<TypedField>,
}

/// How a read of one window may go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadAccess {
    Open,
    /// Elements may be read with the typed fields hidden; a picture may not.
    HideTypedFields,
    Refused,
}

impl TypedFields {
    pub(crate) fn remember(&mut self, field: TypedField) {
        if !self.fields.contains(&field) {
            self.fields.push(field);
        }
    }

    pub(crate) fn fields(&self) -> &[TypedField] {
        &self.fields
    }

    pub(crate) fn retain(&mut self, keep: impl FnMut(&TypedField) -> bool) {
        self.fields.retain(keep);
    }

    pub(crate) fn forget_window(&mut self, window: &str) {
        self.fields
            .retain(|field| field.view != window && field.toplevel != window);
    }

    /// Element reads of the view itself hide the fields; reads of its toplevel through another
    /// view, where the field cannot be found, are refused.
    pub(crate) fn element_access(&self, window: &str) -> ReadAccess {
        if self
            .fields
            .iter()
            .any(|field| field.toplevel == window && field.view != window)
        {
            ReadAccess::Refused
        } else if self.fields.iter().any(|field| field.view == window) {
            ReadAccess::HideTypedFields
        } else {
            ReadAccess::Open
        }
    }

    /// A picture of the window shows the field, so it waits until no field is live there.
    pub(crate) fn picture_access(&self, window: &str) -> ReadAccess {
        if self
            .fields
            .iter()
            .any(|field| field.view == window || field.toplevel == window)
        {
            ReadAccess::Refused
        } else {
            ReadAccess::Open
        }
    }

    /// Drops the value of each typed field of `window`, and the text of its descendants, where
    /// a browser repeats the value. Elements come in tree order. `secure` does not matter: a
    /// page can reveal a password field and keep the node.
    pub(crate) fn hide_values(&self, window: &str, elements: &mut [Element]) {
        let typed: HashSet<&str> = self
            .fields
            .iter()
            .filter(|field| field.view == window)
            .map(|field| field.source_id.as_str())
            .collect();
        if typed.is_empty() {
            return;
        }
        let mut typed_field_depth = None;
        for element in elements.iter_mut() {
            if typed_field_depth.is_some_and(|depth| element.depth > depth) {
                element.name.clear();
                element.value = None;
                continue;
            }
            typed_field_depth = typed
                .contains(element.source_id.as_str())
                .then_some(element.depth);
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

    fn element(
        source_id: &str,
        name: &str,
        value: Option<&str>,
        depth: u8,
        secure: bool,
    ) -> Element {
        Element {
            reference: "@s1:e1"
                .parse::<ElementRef>()
                .unwrap_or_else(|_| unreachable!()),
            role: "textbox".into(),
            name: name.into(),
            value: value.map(str::to_owned),
            bounds: Bounds::default(),
            actions: Vec::new(),
            secure,
            source: Source::Browser,
            source_id: source_id.into(),
            depth,
        }
    }

    fn typed_in_tab() -> TypedFields {
        let mut typed = TypedFields::default();
        typed.remember(TypedField {
            view: "browser:1".into(),
            toplevel: "w7".into(),
            source_id: "41".into(),
            loader_id: Some("L1".into()),
        });
        typed
    }

    fn values(elements: &[Element]) -> Vec<(String, Option<String>)> {
        elements
            .iter()
            .map(|element| (element.name.clone(), element.value.clone()))
            .collect()
    }

    #[test]
    fn a_snapshot_after_a_reveal_toggle_still_hides_the_field_and_its_text() {
        let typed = typed_in_tab();
        let mut elements = vec![
            element("41", "Password", Some("hunter2"), 3, false),
            element("50", "", None, 4, false),
            element("51", "hunter2", None, 5, false),
            element("60", "Sign in", None, 3, false),
        ];
        typed.hide_values("browser:1", &mut elements);
        assert_eq!(
            values(&elements),
            [
                ("Password".into(), None),
                (String::new(), None),
                (String::new(), None),
                ("Sign in".into(), None),
            ]
        );
    }

    #[test]
    fn the_toplevel_view_is_refused_while_the_field_is_live() {
        let mut typed = typed_in_tab();
        assert_eq!(
            typed.element_access("browser:1"),
            ReadAccess::HideTypedFields
        );
        assert_eq!(typed.element_access("w7"), ReadAccess::Refused);
        assert_eq!(typed.picture_access("browser:1"), ReadAccess::Refused);
        assert_eq!(typed.picture_access("w7"), ReadAccess::Refused);
        assert_eq!(typed.element_access("w8"), ReadAccess::Open);
        typed.forget_window("browser:1");
        assert_eq!(typed.element_access("w7"), ReadAccess::Open);
        assert_eq!(typed.picture_access("w7"), ReadAccess::Open);
    }

    #[test]
    fn a_field_typed_in_its_own_window_is_hidden_there() {
        let mut typed = TypedFields::default();
        typed.remember(TypedField {
            view: "w3".into(),
            toplevel: "w3".into(),
            source_id: "/org/a11y/atspi/accessible/9".into(),
            loader_id: None,
        });
        assert_eq!(typed.element_access("w3"), ReadAccess::HideTypedFields);
        let mut elements = vec![element(
            "/org/a11y/atspi/accessible/9",
            "User",
            Some("alice"),
            2,
            false,
        )];
        typed.hide_values("w3", &mut elements);
        assert_eq!(values(&elements), [("User".into(), None)]);
    }
}
