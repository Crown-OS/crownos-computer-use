/// `Push Button`, `push button`, `StaticText` and `push-button` all become `push_button`.
pub(crate) fn snake_case(role: &str) -> String {
    let mut out = String::with_capacity(role.len() + 4);
    let mut previous_lower = false;
    for character in role.chars() {
        match character {
            ' ' | '-' | '_' => {
                if !out.ends_with('_') && !out.is_empty() {
                    out.push('_');
                }
                previous_lower = false;
            }
            upper if upper.is_ascii_uppercase() => {
                if previous_lower && !out.ends_with('_') {
                    out.push('_');
                }
                out.push(upper.to_ascii_lowercase());
                previous_lower = false;
            }
            other => {
                out.push(other.to_ascii_lowercase());
                previous_lower = other.is_ascii_lowercase() || other.is_ascii_digit();
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::snake_case;

    #[test]
    fn roles_from_every_source_share_one_spelling() {
        assert_eq!(snake_case("push button"), "push_button");
        assert_eq!(snake_case("StaticText"), "static_text");
        assert_eq!(snake_case("RootWebArea"), "root_web_area");
        assert_eq!(snake_case("text_field"), "text_field");
        assert_eq!(snake_case("combo-box"), "combo_box");
    }
}
