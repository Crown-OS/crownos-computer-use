use crate::element::Element;

/// What an agent looks for: every given part must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Exact snake_case role.
    pub role: Option<String>,
    /// The accessible name, compared ignoring case.
    pub name: Option<String>,
    /// A substring of the name or the value, compared ignoring case.
    pub text: Option<String>,
}

impl Query {
    pub fn matches(&self, element: &Element) -> bool {
        self.role.as_deref().is_none_or(|role| element.role == role)
            && self
                .name
                .as_deref()
                .is_none_or(|name| element.name.eq_ignore_ascii_case(name))
            && self.text.as_deref().is_none_or(|text| {
                let text = text.to_lowercase();
                element.name.to_lowercase().contains(&text)
                    || element
                        .value
                        .as_deref()
                        .is_some_and(|value| value.to_lowercase().contains(&text))
            })
    }
}
