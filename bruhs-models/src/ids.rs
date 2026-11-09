//! Validated identifier newtypes, so a meeting id cannot be passed where a
//! person id belongs and no id can smuggle URL structure into a request path.
//!
//! Every id serializes as a bare JSON string (`#[serde(transparent)]` on the
//! wire) and validates on deserialize. The accepted set covers every format
//! the app generates today: UUIDs (meetings, people, threads, speakers),
//! `audio-<ms>` meeting placeholders, `p-001` / `p001` people, `local-<id>`
//! events, and the speaker grammar `me`, `them`, `them_N` and legacy
//! `name:<display name>` (spaces and Unicode allowed).
//!
//! An id is rejected when it is empty, longer than [`MAX_ID_LEN`] bytes,
//! contains a control character, contains `/`, `\`, `?`, `#` or `%` (each
//! changes how a URL containing the id is parsed), or is `.` / `..`. Callers
//! that build URLs should still push ids with `Url::path_segments_mut` rather
//! than `format!`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Longest id accepted, in bytes.
pub const MAX_ID_LEN: usize = 256;

/// Why a string is not a valid id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("id is empty")]
    Empty,
    #[error("id is longer than {MAX_ID_LEN} bytes")]
    TooLong,
    #[error("id contains a control character")]
    ControlCharacter,
    #[error("id contains {0:?}, which is not allowed in ids")]
    ReservedCharacter(char),
    #[error("id is a dot path segment")]
    DotSegment,
}

/// Validation shared by every id type.
pub fn validate_id(raw: &str) -> Result<(), IdError> {
    if raw.is_empty() {
        return Err(IdError::Empty);
    }
    if raw.len() > MAX_ID_LEN {
        return Err(IdError::TooLong);
    }
    if raw == "." || raw == ".." {
        return Err(IdError::DotSegment);
    }
    for c in raw.chars() {
        if c.is_control() {
            return Err(IdError::ControlCharacter);
        }
        if matches!(c, '/' | '\\' | '?' | '#' | '%') {
            return Err(IdError::ReservedCharacter(c));
        }
    }
    Ok(())
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// The id as it appears on the wire.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Unwrap into the raw string.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(raw: String) -> Result<Self, Self::Error> {
                validate_id(&raw)?;
                Ok(Self(raw))
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdError;

            fn try_from(raw: &str) -> Result<Self, Self::Error> {
                Self::try_from(raw.to_string())
            }
        }

        impl std::str::FromStr for $name {
            type Err = IdError;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Self::try_from(raw)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// A meeting (`Meeting::id`): a UUID, or `audio-<ms>` for audio-only
    /// placeholders.
    MeetingId
);
id_type!(
    /// A person (`PersonProfile::id`): a UUID, or `p-001`-style seeds.
    PersonId
);
id_type!(
    /// A chat thread (`Thread::id`): a UUID.
    ThreadId
);
id_type!(
    /// A meeting speaker row (`Speaker::id`, `TranscriptSegment::speaker_id`):
    /// `me`, `them`, `them_N`, a UUID, or legacy `name:<display name>`. The
    /// grammar is parsed by [`crate::SpeakerId`]; this type only guarantees the
    /// raw string is safe to carry.
    SpeakerId
);

impl ThreadId {
    /// A fresh random thread id.
    pub fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl MeetingId {
    /// A fresh random meeting id.
    pub fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl PersonId {
    /// A fresh random person id.
    pub fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generated_format_is_accepted() {
        for raw in [
            "0b6f2c1e-8f3a-4c5e-9d2b-7a1f0e3c4d5b",
            "audio-1700000000000",
            "p-001",
            "p001",
            "local-abc123",
            "me",
            "them",
            "them_12",
            "name:Alice Smith",
            "name:  Zoë 李  ",
            "cal-test-roundtrip-0b6f2c1e",
        ] {
            assert_eq!(validate_id(raw), Ok(()), "{raw}");
            assert_eq!(
                MeetingId::try_from(raw).map(|id| id.to_string()),
                Ok(raw.to_string())
            );
        }
        assert!(validate_id(&ThreadId::new_v4().into_inner()).is_ok());
        assert!(validate_id(&"a".repeat(MAX_ID_LEN)).is_ok());
    }

    #[test]
    fn url_structure_and_garbage_are_rejected() {
        for (raw, err) in [
            ("", IdError::Empty),
            (".", IdError::DotSegment),
            ("..", IdError::DotSegment),
            ("../capture/stop", IdError::ReservedCharacter('/')),
            ("a\\b", IdError::ReservedCharacter('\\')),
            ("a?x=1", IdError::ReservedCharacter('?')),
            ("a#frag", IdError::ReservedCharacter('#')),
            ("%2e%2e", IdError::ReservedCharacter('%')),
            ("a\nb", IdError::ControlCharacter),
            ("a\u{7f}", IdError::ControlCharacter),
        ] {
            assert_eq!(validate_id(raw), Err(err), "{raw:?}");
        }
        assert_eq!(
            validate_id(&"a".repeat(MAX_ID_LEN + 1)),
            Err(IdError::TooLong)
        );
    }

    #[test]
    fn serde_is_a_bare_validated_string() {
        let id = PersonId::try_from("p-001").unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"p-001\"");
        assert_eq!(serde_json::from_str::<PersonId>("\"p-001\"").unwrap(), id);
        assert!(serde_json::from_str::<PersonId>("\"../x\"").is_err());
        assert!(serde_json::from_str::<PersonId>("\"\"").is_err());
        assert_eq!(id.as_str(), "p-001");
        assert_eq!("p-001".parse::<PersonId>(), Ok(id));
    }
}
