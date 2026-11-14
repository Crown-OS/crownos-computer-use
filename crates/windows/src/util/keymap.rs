//! ASCII to evdev key codes on the US layout, for typing where no toolkit takes text directly.

use crate::input::InputEvent;

const KEY_LEFTSHIFT: u32 = 42;
const KEY_ENTER: u32 = 28;
const KEY_TAB: u32 = 15;
const KEY_SPACE: u32 = 57;

/// The evdev code and whether Shift is held for one ASCII character on the US layout.
pub fn key_for(character: char) -> Option<(u32, bool)> {
    const ROWS: &[(&str, &str, u32)] = &[
        ("1234567890-=", "!@#$%^&*()_+", 2),
        ("qwertyuiop[]", "QWERTYUIOP{}", 16),
        ("asdfghjkl;'", "ASDFGHJKL:\"", 30),
        ("zxcvbnm,./", "ZXCVBNM<>?", 44),
    ];
    match character {
        '\n' => return Some((KEY_ENTER, false)),
        '\t' => return Some((KEY_TAB, false)),
        ' ' => return Some((KEY_SPACE, false)),
        '`' => return Some((41, false)),
        '~' => return Some((41, true)),
        '\\' => return Some((43, false)),
        '|' => return Some((43, true)),
        _ => {}
    }
    for (plain, shifted, first) in ROWS {
        if let Some(index) = plain.find(character) {
            return Some((first + u32::try_from(index).ok()?, false));
        }
        if let Some(index) = shifted.find(character) {
            return Some((first + u32::try_from(index).ok()?, true));
        }
    }
    None
}

/// Key events that type `text`; characters the layout cannot type are skipped.
pub fn type_text(text: &str) -> Vec<InputEvent> {
    let mut events = Vec::with_capacity(text.len() * 4);
    for (code, shift) in text.chars().filter_map(key_for) {
        if shift {
            events.push(InputEvent::Key {
                code: KEY_LEFTSHIFT,
                pressed: true,
            });
        }
        events.push(InputEvent::Key {
            code,
            pressed: true,
        });
        events.push(InputEvent::Key {
            code,
            pressed: false,
        });
        if shift {
            events.push(InputEvent::Key {
                code: KEY_LEFTSHIFT,
                pressed: false,
            });
        }
    }
    events
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
    fn letters_digits_and_shifted_symbols_map_to_evdev_codes() {
        assert_eq!(key_for('a'), Some((30, false)));
        assert_eq!(key_for('A'), Some((30, true)));
        assert_eq!(key_for('1'), Some((2, false)));
        assert_eq!(key_for('!'), Some((2, true)));
        assert_eq!(key_for('/'), Some((53, false)));
        assert_eq!(key_for('é'), None);
        let events = type_text("Hi");
        assert_eq!(events.len(), 6);
        assert!(matches!(
            events[0],
            InputEvent::Key {
                code: 42,
                pressed: true
            }
        ));
    }
}
