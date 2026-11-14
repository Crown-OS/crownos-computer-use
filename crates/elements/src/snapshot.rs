use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::element::{Element, Source, WindowHandle};
use crate::find::Query;
use crate::refs::{ElementRef, SnapshotId};

/// One window's elements at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub window: WindowHandle,
    pub source: Source,
    pub elements: Vec<Element>,
}

impl Snapshot {
    pub fn find(&self, query: &Query) -> Vec<&Element> {
        self.elements
            .iter()
            .filter(|element| query.matches(element))
            .collect()
    }

    /// The same snapshot keeping only what an agent operates.
    pub fn interactive(&self) -> Self {
        Self {
            id: self.id,
            window: self.window.clone(),
            source: self.source,
            elements: self
                .elements
                .iter()
                .filter(|element| element.is_interactive())
                .cloned()
                .collect(),
        }
    }

    pub fn element(&self, reference: ElementRef) -> Option<&Element> {
        (reference.snapshot == self.id)
            .then(|| {
                self.elements
                    .iter()
                    .find(|element| element.reference == reference)
            })
            .flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("{0} is stale: the window was snapshotted again as @s{1}; read it and retry")]
    Stale(ElementRef, u32),
    #[error("{0} names no snapshot this daemon took")]
    Unknown(ElementRef),
    #[error("{0} is not an element of its snapshot")]
    NoSuchElement(ElementRef),
}

/// The latest snapshot of every window. Taking a new one of a window retires the old one, so
/// its refs resolve to a stale-ref error that names the replacement.
#[derive(Debug, Default)]
pub struct SnapshotStore {
    next: u32,
    current: HashMap<WindowHandle, Snapshot>,
    /// Every snapshot ever taken, so a retired ref is told what replaced it.
    windows: HashMap<SnapshotId, WindowHandle>,
}

impl SnapshotStore {
    /// Numbers `elements` as a new snapshot of `window` and makes it the current one.
    pub fn take(
        &mut self,
        window: WindowHandle,
        source: Source,
        mut elements: Vec<Element>,
    ) -> &Snapshot {
        self.next += 1;
        let id = SnapshotId(self.next);
        for (index, element) in (0u32..).zip(&mut elements) {
            element.reference = ElementRef {
                snapshot: id,
                index,
            };
        }
        self.windows.insert(id, window.clone());
        let snapshot = Snapshot {
            id,
            window: window.clone(),
            source,
            elements,
        };
        self.current.entry(window).insert_entry(snapshot).into_mut()
    }

    pub fn current(&self, window: &WindowHandle) -> Option<&Snapshot> {
        self.current.get(window)
    }

    pub fn forget(&mut self, window: &WindowHandle) {
        self.current.remove(window);
        self.windows.retain(|_, owner| owner != window);
    }

    /// The element a ref names, if its snapshot is still the current one of its window.
    pub fn resolve(&self, reference: ElementRef) -> Result<(&Snapshot, &Element), ResolveError> {
        let window = self
            .windows
            .get(&reference.snapshot)
            .ok_or(ResolveError::Unknown(reference))?;
        let snapshot = self
            .current
            .get(window)
            .ok_or(ResolveError::Unknown(reference))?;
        if snapshot.id != reference.snapshot {
            return Err(ResolveError::Stale(reference, snapshot.id.0));
        }
        let element = snapshot
            .element(reference)
            .ok_or(ResolveError::NoSuchElement(reference))?;
        Ok((snapshot, element))
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
    use crate::element::Bounds;

    fn element(role: &str, name: &str, value: Option<&str>, actions: &[&str]) -> Element {
        Element {
            reference: ElementRef {
                snapshot: SnapshotId(0),
                index: 0,
            },
            role: role.to_owned(),
            name: name.to_owned(),
            value: value.map(str::to_owned),
            bounds: Bounds::new(0, 0, 10, 10),
            actions: actions.iter().map(|action| (*action).to_owned()).collect(),
            secure: false,
            source: Source::CrownUi,
            source_id: String::new(),
            depth: 0,
        }
    }

    fn window(name: &str) -> WindowHandle {
        WindowHandle(name.to_owned())
    }

    #[test]
    fn a_new_snapshot_retires_the_old_refs_of_the_same_window() {
        let mut store = SnapshotStore::default();
        let first = store
            .take(
                window("a"),
                Source::CrownUi,
                vec![element("button", "Save", None, &["click"])],
            )
            .elements[0]
            .reference;
        assert_eq!(first.to_string(), "@s1:e0");
        assert!(store.resolve(first).is_ok());
        let second = store
            .take(
                window("a"),
                Source::CrownUi,
                vec![element("button", "Save", None, &[])],
            )
            .id;
        assert_eq!(
            store.resolve(first),
            Err(ResolveError::Stale(first, second.0))
        );
        let other = store
            .take(
                window("b"),
                Source::Atspi,
                vec![element("link", "Docs", None, &[])],
            )
            .elements[0]
            .reference;
        assert!(store.resolve(other).is_ok());
        assert!(matches!(
            store.resolve(ElementRef {
                snapshot: second,
                index: 9
            }),
            Err(ResolveError::NoSuchElement(_))
        ));
        store.forget(&window("b"));
        assert_eq!(store.resolve(other), Err(ResolveError::Unknown(other)));
    }

    #[test]
    fn find_matches_role_name_and_text() {
        let mut store = SnapshotStore::default();
        let snapshot = store.take(
            window("a"),
            Source::Browser,
            vec![
                element("button", "Save changes", None, &["click"]),
                element(
                    "text_field",
                    "Email",
                    Some("ada@example.com"),
                    &["type_text"],
                ),
                element("text", "Welcome back", None, &[]),
            ],
        );
        let by_role = snapshot.find(&Query {
            role: Some("button".into()),
            ..Query::default()
        });
        assert_eq!(by_role.len(), 1);
        let by_name = snapshot.find(&Query {
            name: Some("email".into()),
            ..Query::default()
        });
        assert_eq!(by_name[0].role, "text_field");
        let by_text = snapshot.find(&Query {
            text: Some("EXAMPLE".into()),
            ..Query::default()
        });
        assert_eq!(by_text.len(), 1);
        assert_eq!(snapshot.interactive().elements.len(), 2);
        let text = snapshot.to_text();
        assert!(text.starts_with("@s1:e0 button \"Save changes\" [click]\n"));
        assert!(text.contains("= \"ada@example.com\""));
    }
}
