use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// One snapshot's number, unique for the daemon's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SnapshotId(pub u32);

/// `@s<snapshot>:e<index>`: names one element in one snapshot and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct ElementRef {
    pub snapshot: SnapshotId,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefParseError {
    #[error("{0:?} is not a ref: expected @s<snapshot>:e<index>")]
    Shape(String),
    #[error("{0:?} has a number that does not parse")]
    Number(String),
}

impl fmt::Display for ElementRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "@s{}:e{}", self.snapshot.0, self.index)
    }
}

impl FromStr for ElementRef {
    type Err = RefParseError;

    fn from_str(text: &str) -> Result<Self, RefParseError> {
        let shape = || RefParseError::Shape(text.to_owned());
        let body = text.strip_prefix("@s").ok_or_else(shape)?;
        let (snapshot, element) = body.split_once(":e").ok_or_else(shape)?;
        let number = |part: &str| {
            part.parse::<u32>()
                .map_err(|_| RefParseError::Number(text.to_owned()))
        };
        Ok(Self {
            snapshot: SnapshotId(number(snapshot)?),
            index: number(element)?,
        })
    }
}

impl From<ElementRef> for String {
    fn from(reference: ElementRef) -> Self {
        reference.to_string()
    }
}

impl TryFrom<String> for ElementRef {
    type Error = RefParseError;

    fn try_from(text: String) -> Result<Self, RefParseError> {
        text.parse()
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

    #[test]
    fn refs_round_trip_through_text() {
        let reference = ElementRef {
            snapshot: SnapshotId(7),
            index: 12,
        };
        assert_eq!(reference.to_string(), "@s7:e12");
        assert_eq!("@s7:e12".parse::<ElementRef>(), Ok(reference));
    }

    #[test]
    fn bare_or_malformed_refs_are_refused() {
        assert!(matches!(
            "e12".parse::<ElementRef>(),
            Err(RefParseError::Shape(_))
        ));
        assert!(matches!(
            "@s7".parse::<ElementRef>(),
            Err(RefParseError::Shape(_))
        ));
        assert!(matches!(
            "@sx:e1".parse::<ElementRef>(),
            Err(RefParseError::Number(_))
        ));
    }
}
