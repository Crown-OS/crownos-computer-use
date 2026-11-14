//! Chromium's `--remote-debugging-pipe` framing: one JSON message, then a NUL byte.

use serde_json::Value;

const DELIMITER: u8 = 0;

/// `message` as the bytes to write.
pub(crate) fn frame(message: &Value) -> Vec<u8> {
    let mut bytes = message.to_string().into_bytes();
    bytes.push(DELIMITER);
    bytes
}

/// Reassembles messages from whatever the pipe delivers.
#[derive(Debug, Default)]
pub(crate) struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    /// Every complete message `bytes` finished; unparsable ones are dropped.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<Value> {
        self.buffer.extend_from_slice(bytes);
        let mut messages = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == DELIMITER) {
            let message: Vec<u8> = self.buffer.drain(..=end).take(end).collect();
            if let Ok(value) = serde_json::from_slice(&message) {
                messages.push(value);
            }
        }
        messages
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
    use serde_json::json;

    use super::*;

    #[test]
    fn frames_end_with_a_nul_and_decode_across_chunks() {
        let bytes = frame(&json!({"id": 1, "method": "Target.getTargets"}));
        assert_eq!(bytes.last(), Some(&0));
        let mut decoder = Decoder::default();
        let (head, tail) = bytes.split_at(10);
        assert!(decoder.push(head).is_empty());
        let mut more = tail.to_vec();
        more.extend(frame(&json!({"id": 2})));
        let messages = decoder.push(&more);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["method"], "Target.getTargets");
        assert_eq!(messages[1]["id"], 2);
    }

    #[test]
    fn garbage_between_delimiters_is_dropped() {
        let mut decoder = Decoder::default();
        let mut bytes = b"not json".to_vec();
        bytes.push(0);
        bytes.extend(frame(&json!({"id": 3})));
        let messages = decoder.push(&bytes);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], 3);
    }
}
