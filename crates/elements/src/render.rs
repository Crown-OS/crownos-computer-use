use std::fmt::Write as _;

use crate::snapshot::Snapshot;

impl Snapshot {
    /// The snapshot as an agent reads it: one element per line, indented by depth, with its
    /// ref first so it can be acted on.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for element in &self.elements {
            let indent = "  ".repeat(usize::from(element.depth));
            let _ = write!(text, "{indent}{} {}", element.reference, element.role);
            if !element.name.is_empty() {
                let _ = write!(text, " {:?}", element.name);
            }
            match (&element.value, element.secure) {
                (_, true) => text.push_str(" [secure]"),
                (Some(value), false) if !value.is_empty() => {
                    let _ = write!(text, " = {value:?}");
                }
                _ => {}
            }
            if !element.actions.is_empty() {
                let _ = write!(text, " [{}]", element.actions.join(", "));
            }
            text.push('\n');
        }
        text
    }
}
