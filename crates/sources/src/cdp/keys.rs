//! `Ctrl+Shift+A` as `Input.dispatchKeyEvent` wants it.

use serde_json::{Value, json};

const ALT: u32 = 1;
const CTRL: u32 = 2;
const META: u32 = 4;
const SHIFT: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Key {
    modifiers: u32,
    key: String,
    code: String,
    virtual_key: u32,
    text: Option<String>,
}

impl Key {
    pub(crate) fn params(&self) -> Value {
        let mut params = json!({
            "modifiers": self.modifiers,
            "key": self.key,
            "code": self.code,
            "windowsVirtualKeyCode": self.virtual_key,
        });
        if let Some(text) = &self.text {
            params["text"] = Value::from(text.as_str());
        }
        params
    }
}

const NAMED: &[(&str, &str, &str, u32)] = &[
    ("enter", "Enter", "Enter", 13),
    ("return", "Enter", "Enter", 13),
    ("tab", "Tab", "Tab", 9),
    ("escape", "Escape", "Escape", 27),
    ("esc", "Escape", "Escape", 27),
    ("backspace", "Backspace", "Backspace", 8),
    ("delete", "Delete", "Delete", 46),
    ("space", " ", "Space", 32),
    ("up", "ArrowUp", "ArrowUp", 38),
    ("down", "ArrowDown", "ArrowDown", 40),
    ("left", "ArrowLeft", "ArrowLeft", 37),
    ("right", "ArrowRight", "ArrowRight", 39),
    ("home", "Home", "Home", 36),
    ("end", "End", "End", 35),
    ("pageup", "PageUp", "PageUp", 33),
    ("pagedown", "PageDown", "PageDown", 34),
];

pub(crate) fn parse(keybind: &str) -> Option<Key> {
    let mut modifiers = 0;
    let mut main = None;
    for part in keybind
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= CTRL,
            "shift" => modifiers |= SHIFT,
            "alt" => modifiers |= ALT,
            "super" | "meta" | "cmd" => modifiers |= META,
            _ => main = Some(part),
        }
    }
    let main = main?;
    if let Some((_, key, code, virtual_key)) = NAMED
        .iter()
        .find(|(name, ..)| main.eq_ignore_ascii_case(name))
    {
        let text = (*key == " ").then(|| " ".to_owned());
        return Some(Key {
            modifiers,
            key: (*key).to_owned(),
            code: (*code).to_owned(),
            virtual_key: *virtual_key,
            text,
        });
    }
    let mut characters = main.chars();
    let character = characters.next()?;
    if characters.next().is_some() || !character.is_ascii() {
        return None;
    }
    let upper = character.to_ascii_uppercase();
    let code = if character.is_ascii_alphabetic() {
        format!("Key{upper}")
    } else if character.is_ascii_digit() {
        format!("Digit{character}")
    } else {
        String::new()
    };
    let typed = (modifiers & (CTRL | ALT | META) == 0).then(|| character.to_string());
    Some(Key {
        modifiers,
        key: character.to_string(),
        code,
        virtual_key: u32::from(upper as u8),
        text: typed,
    })
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

    #[test]
    fn chords_and_named_keys_parse() {
        let select_all = parse("Ctrl+A").unwrap();
        assert_eq!(select_all.modifiers, CTRL);
        assert_eq!(select_all.code, "KeyA");
        assert_eq!(select_all.virtual_key, 65);
        assert_eq!(select_all.text, None);
        let enter = parse("Enter").unwrap();
        assert_eq!(enter.params()["windowsVirtualKeyCode"], 13);
        let letter = parse("x").unwrap();
        assert_eq!(letter.text.as_deref(), Some("x"));
        assert!(parse("Ctrl+").is_none());
        assert!(parse("Hyper+Fn").is_none());
    }
}
