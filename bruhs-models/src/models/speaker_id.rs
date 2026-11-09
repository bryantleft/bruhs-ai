//! Typed parse of the `TranscriptSegment.speaker_id` string.
//!
//! The grammar is closed — these four shapes are the only ones the
//! storage accepts:
//! - `"me"` — mic-only segments (channel-based diarization)
//! - `"them"` — sys-only segments before sub-diarization
//! - `"them_N"` — sherpa-onnx sub-speaker cluster
//! - `"name:X"` — legacy named identity (not proof of who spoke)
//!
//! Centralizing the grammar here means writers (`subdiarize`)
//! and readers (`speaker_label`,
//! `speaker_colors`, the link hook) all share one source of truth.

use crate::Speaker;

/// Closed enum over the supported persisted speaker_id grammar.
///
/// To add a new identity source (e.g. `"calendar:alice@example.com"`),
/// add one variant + one match arm in [`Self::parse`] / [`Self::to_raw`]
/// — every consumer picks up the new variant automatically via
/// [`SpeakerIdentity::resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeakerId {
    /// "me" — the user's microphone channel.
    Me,
    /// "them" — system audio before sub-speaker clustering ran.
    Them,
    /// "them_N" — sub-speaker cluster ordinal from sherpa-onnx.
    Cluster(u32),
    /// "name:X" — legacy named identity; new OCR hints never produce this.
    Named(String),
}

impl SpeakerId {
    /// Parse a raw `speaker_id` string. Returns `None` if the string
    /// doesn't match any known grammar — callers should treat this as
    /// "Unknown" rather than silently aliasing.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "me" => Some(Self::Me),
            "them" => Some(Self::Them),
            _ => {
                if let Some(rest) = raw.strip_prefix("them_") {
                    rest.parse::<u32>().ok().map(Self::Cluster)
                } else if let Some(rest) = raw.strip_prefix("name:") {
                    let trimmed = rest.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(Self::Named(trimmed.to_string()))
                    }
                } else {
                    None
                }
            }
        }
    }

    /// Render to the canonical raw string form. Round-trips with
    /// [`Self::parse`].
    pub fn to_raw(&self) -> String {
        match self {
            Self::Me => "me".into(),
            Self::Them => "them".into(),
            Self::Cluster(n) => format!("them_{n}"),
            Self::Named(s) => format!("name:{s}"),
        }
    }
}

/// Semantic color category for a speaker, decoupled from any specific
/// palette. The UI maps `ColorIndex` to bruhs design tokens
/// (dragonfruit / pandan / lychee / blueberry / durian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorIndex {
    Me,
    Them,
    Cluster(u32),
    Unknown,
}

/// Resolved view of a speaker — what to display and how to color it.
/// Produced by [`SpeakerIdentity::resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerIdentity {
    pub display: String,
    pub color_index: ColorIndex,
}

impl SpeakerIdentity {
    /// Resolve a `speaker_id` against the meeting's `Speaker` entries.
    ///
    /// Resolution order:
    /// 1. Look up `Meeting.speakers` for a matching entry — user renames
    ///    take precedence over derived labels.
    /// 2. Parse the raw id and produce a default human label
    ///    ("Me" / "Them" / "Speaker N" / the legacy name).
    /// 3. Fall back to "Unknown" for unparseable / missing ids.
    pub fn resolve(raw: Option<&str>, speakers: &[Speaker]) -> Self {
        let Some(raw) = raw else {
            return Self {
                display: "Unknown".into(),
                color_index: ColorIndex::Unknown,
            };
        };

        // Color is always derived from the raw id (so a user-renamed
        // "them_2" → "Alice" still uses the cluster-2 color slot).
        let color_index = match SpeakerId::parse(raw) {
            Some(SpeakerId::Me) => ColorIndex::Me,
            Some(SpeakerId::Them) | Some(SpeakerId::Named(_)) => ColorIndex::Them,
            Some(SpeakerId::Cluster(n)) => ColorIndex::Cluster(n),
            None => ColorIndex::Unknown,
        };

        // 1) User rename wins.
        if let Some(sp) = speakers.iter().find(|s| s.id == raw) {
            return Self {
                display: sp.label.clone(),
                color_index,
            };
        }

        // 2) Default labels from the raw grammar.
        let display = match SpeakerId::parse(raw) {
            Some(SpeakerId::Me) => "Me".into(),
            Some(SpeakerId::Them) => "Them".into(),
            Some(SpeakerId::Cluster(n)) => format!("Speaker {n}"),
            Some(SpeakerId::Named(s)) => s,
            None => "Unknown".into(),
        };
        Self {
            display,
            color_index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(id: &str, label: &str) -> Speaker {
        Speaker {
            id: id.into(),
            label: label.into(),
            person_id: None,
            color: None,
        }
    }

    #[test]
    fn parse_me_them() {
        assert_eq!(SpeakerId::parse("me"), Some(SpeakerId::Me));
        assert_eq!(SpeakerId::parse("them"), Some(SpeakerId::Them));
    }

    #[test]
    fn parse_cluster() {
        assert_eq!(SpeakerId::parse("them_1"), Some(SpeakerId::Cluster(1)));
        assert_eq!(SpeakerId::parse("them_99"), Some(SpeakerId::Cluster(99)));
    }

    #[test]
    fn parse_named() {
        assert_eq!(
            SpeakerId::parse("name:Alice"),
            Some(SpeakerId::Named("Alice".into()))
        );
        // trimmed
        assert_eq!(
            SpeakerId::parse("name:  Bob  "),
            Some(SpeakerId::Named("Bob".into()))
        );
    }

    #[test]
    fn parse_rejects_unknown() {
        assert_eq!(SpeakerId::parse(""), None);
        assert_eq!(SpeakerId::parse("them_abc"), None);
        assert_eq!(SpeakerId::parse("name:"), None);
        assert_eq!(SpeakerId::parse("name:   "), None);
        assert_eq!(SpeakerId::parse("random_thing"), None);
    }

    #[test]
    fn roundtrip() {
        for raw in ["me", "them", "them_0", "them_42", "name:Alice"] {
            let parsed = SpeakerId::parse(raw).expect("parses");
            assert_eq!(parsed.to_raw(), raw);
        }
    }

    #[test]
    fn resolve_user_rename_wins() {
        let speakers = vec![sp("them_1", "Alice")];
        let id = SpeakerIdentity::resolve(Some("them_1"), &speakers);
        assert_eq!(id.display, "Alice");
        assert_eq!(id.color_index, ColorIndex::Cluster(1));
    }

    #[test]
    fn resolve_falls_back_to_default_label() {
        let speakers: Vec<Speaker> = vec![];
        assert_eq!(
            SpeakerIdentity::resolve(Some("me"), &speakers).display,
            "Me"
        );
        assert_eq!(
            SpeakerIdentity::resolve(Some("them"), &speakers).display,
            "Them"
        );
        assert_eq!(
            SpeakerIdentity::resolve(Some("them_3"), &speakers).display,
            "Speaker 3"
        );
        assert_eq!(
            SpeakerIdentity::resolve(Some("name:Carol"), &speakers).display,
            "Carol"
        );
    }

    #[test]
    fn resolve_unknown() {
        assert_eq!(SpeakerIdentity::resolve(None, &[]).display, "Unknown");
        assert_eq!(
            SpeakerIdentity::resolve(Some("garbage"), &[]).display,
            "Unknown"
        );
        assert_eq!(
            SpeakerIdentity::resolve(Some("them_xyz"), &[]).color_index,
            ColorIndex::Unknown
        );
    }
}
