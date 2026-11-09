use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    /// Capture in flight; transcript is being appended live.
    Live,
    /// Capture stopped cleanly; final transcript present.
    #[default]
    Ended,
    /// Capture died (daemon crash, disk error). Partial transcript preserved.
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Speaker {
    pub id: String,
    /// "Me" | "Speaker 1" | user-edited.
    pub label: String,
    pub person_id: Option<String>,
    /// Bruhs color token used for this speaker's transcript bubbles.
    pub color: Option<String>,
}

impl Speaker {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            label: label.into(),
            person_id: None,
            color: None,
        }
    }
}

/// One transcribed snippet within a meeting. Times are relative to the
/// meeting's `started_at_ms` so segments don't need re-anchoring if the
/// containing Meeting's start is corrected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    pub id: String,
    pub meeting_id: String,
    pub speaker_id: Option<String>,
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
    pub confidence: f32,
    /// True while Whisper is still producing this segment; replaced once
    /// the final pass completes. UI renders `partial: true` segments with
    /// reduced opacity.
    pub partial: bool,
    /// Cosine similarity of this segment's embedding to its assigned cluster's
    /// centroid, written by `subdiarize::assign_subspeakers`. None when
    /// subdiarization was skipped or the segment belongs to "me".
    /// Serialized with `#[serde(default)]` for backward compatibility with
    /// existing JSONL that predates Voice ID v2.
    #[serde(default)]
    pub speaker_confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionItem {
    pub text: String,
    pub assignee_person_id: Option<String>,
    /// Free-form, e.g. `"by Friday"`.
    pub due_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Topic {
    pub title: String,
    pub bullets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeetingSummary {
    pub tldr: String,
    pub action_items: Vec<ActionItem>,
    pub decisions: Vec<String>,
    pub topics: Vec<Topic>,
    /// Model identifier that produced the summary, e.g. `"claude-opus-4-7"`.
    /// Populated by the caller after deserialization; absent from LLM output.
    #[serde(default)]
    pub generated_by: String,
    /// Unix-ms timestamp when the summary was produced; populated by caller.
    #[serde(default)]
    pub generated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub started_at_ms: f64,
    pub ended_at_ms: Option<f64>,
    pub status: MeetingStatus,
    pub calendar_event_id: Option<String>,
    /// Absolute local filesystem path to the captured audio. NEVER sent
    /// over the wire to remote services.
    pub audio_path: Option<String>,
    pub language: Option<String>,
    #[serde(default)]
    pub speakers: Vec<Speaker>,
    pub summary: Option<MeetingSummary>,
    pub thread_id: Option<String>,
    /// When true, the summarization step skips the per_person_observations
    /// prompt section for this meeting regardless of the global
    /// `extract_observations` toggle. Defaults to false.
    #[serde(default)]
    pub skip_observations: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Meeting {
    /// Build a fresh Meeting record in `Live` status. `started_at_ms` is
    /// the caller's responsibility — usually `Date.now()` at record-press
    /// time.
    pub fn new(title: impl Into<String>, started_at_ms: f64) -> Self {
        let now = crate::EpochMs::saturating_from_f64(started_at_ms).as_millis();
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            started_at_ms,
            ended_at_ms: None,
            status: MeetingStatus::Live,
            calendar_event_id: None,
            audio_path: None,
            language: None,
            speakers: vec![
                Speaker {
                    id: "me".into(),
                    label: "Me".into(),
                    person_id: None,
                    color: None,
                },
                Speaker {
                    id: "them".into(),
                    label: "Them".into(),
                    person_id: None,
                    color: None,
                },
            ],
            summary: None,
            thread_id: None,
            skip_observations: false,
            created_at: now,
            updated_at: now,
        }
    }
}

/// SSE payload from `GET /api/meetings/stream`. Idempotent in the same
/// sense as `CalendarSyncEvent` — replaying the stream produces the same
/// state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingEvent {
    /// A fresh stream can follow a disconnected interval with missed updates.
    StreamOpened,
    Started {
        meeting_id: String,
        started_at_ms: f64,
    },
    Segment {
        meeting_id: String,
        segment: TranscriptSegment,
    },
    /// One chunk was removed from the transcript. Later capture replays of
    /// `segment_id` are ignored, including partial updates of the same id.
    SegmentRemoved {
        meeting_id: String,
        segment_id: String,
    },
    SpeakerLabelChanged {
        meeting_id: String,
        speaker: Speaker,
    },
    Summary {
        meeting_id: String,
        summary: MeetingSummary,
    },
    Ended {
        meeting_id: String,
        ended_at_ms: f64,
        status: MeetingStatus,
    },
    CaptureError {
        meeting_id: String,
        message: String,
    },
    CaptureWarning {
        meeting_id: String,
        message: String,
    },
    TranscriptionCompleted {
        meeting_id: String,
    },
    SpeakersAssigned {
        meeting_id: String,
    },
    /// Emitted by the server after cross-meeting centroid matching completes.
    /// The web reducer refetches the meeting to pick up updated speaker labels.
    SpeakersLinked {
        meeting_id: String,
    },
    PersonObservationsAppended {
        meeting_id: String,
    },
    /// Bulk invalidation or recovery after missed events.
    PeopleUpdated,
    /// A narrow invalidation avoids downloading the address book for one edit.
    PersonChanged {
        person_id: String,
        updated_at: i64,
    },
    /// Resolve the source to its current live target before changing a local cache.
    PersonMerged {
        source_id: String,
    },
    /// Emitted by the activity monitor when sustained microphone activity
    /// is detected for at least `START_ACTIVITY_SECS` continuously. Fired
    /// BEFORE any meeting is created — carries no meeting_id.
    AudioActivityDetected {
        /// Unix epoch milliseconds at which the sustained activity window
        /// started (i.e. when consecutive_active_secs first reached the
        /// threshold). Used by the web to build a synthetic CalendarEvent id.
        started_at_ms: f64,
    },
    /// Emitted every ~250ms by the capture recorders while a recording is
    /// active. Provides real-time RMS levels so the UI can display a live
    /// audio-level meter without polling.
    AudioLevel {
        meeting_id: String,
        /// Root-mean-square of the microphone audio samples in [0, 1].
        mic_rms: f32,
        /// Root-mean-square of the system audio samples in [0, 1].
        /// `None` when system audio capture is inactive.
        sys_rms: Option<f32>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_skip_observations_defaults_false_on_legacy_json() {
        // Legacy JSON without the `skip_observations` field.
        let m = Meeting::new("Test", 1_000.0);
        let mut json: serde_json::Value = serde_json::to_value(&m).unwrap();
        json.as_object_mut().unwrap().remove("skip_observations");
        let back: Meeting = serde_json::from_value(json).unwrap();
        assert!(
            !back.skip_observations,
            "skip_observations should default false"
        );
    }

    #[test]
    fn meeting_skip_observations_roundtrips() {
        let mut m = Meeting::new("Test", 1_000.0);
        m.skip_observations = true;
        let json = serde_json::to_string(&m).unwrap();
        let back: Meeting = serde_json::from_str(&json).unwrap();
        assert!(back.skip_observations);
    }

    #[test]
    fn meeting_new_starts_in_live_status() {
        let m = Meeting::new("Sync", 1_700_000_000_000.0);
        assert_eq!(m.title, "Sync");
        assert_eq!(m.status, MeetingStatus::Live);
        assert_eq!(m.started_at_ms, 1_700_000_000_000.0);
        assert!(m.ended_at_ms.is_none());
        assert_eq!(m.speakers.len(), 2);
        assert_eq!(m.speakers[0].id, "me");
        assert_eq!(m.speakers[1].id, "them");
        assert!(m.audio_path.is_none());
    }

    #[test]
    fn meeting_status_default_is_ended() {
        assert_eq!(MeetingStatus::default(), MeetingStatus::Ended);
    }

    #[test]
    fn meeting_roundtrip_with_speakers_and_summary() {
        let mut m = Meeting::new("Design review", 1_700_000_000_000.0);
        m.speakers.push(Speaker::new("Me"));
        m.summary = Some(MeetingSummary {
            tldr: "Discussed roadmap".into(),
            action_items: vec![ActionItem {
                text: "File ticket".into(),
                assignee_person_id: None,
                due_hint: Some("by Friday".into()),
            }],
            decisions: vec!["Ship behind a flag".into()],
            topics: vec![Topic {
                title: "Roadmap".into(),
                bullets: vec!["Q3 focus".into()],
            }],
            generated_by: "claude-opus-4-7".into(),
            generated_at: 1,
        });
        let json = serde_json::to_string(&m).unwrap();
        let _back: Meeting = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn transcript_segment_roundtrip() {
        let s = TranscriptSegment {
            id: "seg-1".into(),
            meeting_id: "m-1".into(),
            speaker_id: Some("sp-1".into()),
            start_ms: 0,
            end_ms: 4_200,
            text: "Hello world".into(),
            confidence: 0.93,
            partial: false,
            speaker_confidence: Some(0.91),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: TranscriptSegment = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn transcript_segment_speaker_confidence_defaults_none_on_legacy_json() {
        // Legacy JSON without speaker_confidence — back-compat.
        let s = TranscriptSegment {
            id: "seg-2".into(),
            meeting_id: "m-1".into(),
            speaker_id: None,
            start_ms: 0,
            end_ms: 1_000,
            text: "hi".into(),
            confidence: 0.8,
            partial: false,
            speaker_confidence: None,
        };
        let mut json: serde_json::Value = serde_json::to_value(&s).unwrap();
        json.as_object_mut().unwrap().remove("speaker_confidence");
        let back: TranscriptSegment = serde_json::from_value(json).unwrap();
        assert!(
            back.speaker_confidence.is_none(),
            "speaker_confidence must default to None"
        );
    }

    #[test]
    fn meeting_event_variants_tagged_with_kind() {
        let e = MeetingEvent::Started {
            meeting_id: "m-1".into(),
            started_at_ms: 1.0,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"started\""));

        let e = MeetingEvent::Ended {
            meeting_id: "m-1".into(),
            ended_at_ms: 2.0,
            status: MeetingStatus::Ended,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"ended\""));
        assert!(json.contains("\"status\":\"ended\""));
    }

    #[test]
    fn segment_removed_serializes_with_kind() {
        let event = MeetingEvent::SegmentRemoved {
            meeting_id: "m-1".into(),
            segment_id: "chunk-1".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"kind\":\"segment_removed\""));
        assert!(json.contains("\"segment_id\":\"chunk-1\""));
        let back: MeetingEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            MeetingEvent::SegmentRemoved { segment_id, .. } if segment_id == "chunk-1"
        ));
    }

    #[test]
    fn transcription_completed_serializes_with_kind() {
        let e = MeetingEvent::TranscriptionCompleted {
            meeting_id: "m-1".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"transcription_completed\""));
        assert!(json.contains("\"meeting_id\":\"m-1\""));
        let back: MeetingEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, MeetingEvent::TranscriptionCompleted { .. }));
    }

    #[test]
    fn speakers_linked_serializes_with_kind() {
        let e = MeetingEvent::SpeakersLinked {
            meeting_id: "m-2".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"speakers_linked\""));
        assert!(json.contains("\"meeting_id\":\"m-2\""));
        let back: MeetingEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, MeetingEvent::SpeakersLinked { meeting_id } if meeting_id == "m-2"));
    }

    #[test]
    fn audio_level_serializes_with_kind() {
        let e = MeetingEvent::AudioLevel {
            meeting_id: "m-1".into(),
            mic_rms: 0.42,
            sys_rms: Some(0.21),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"audio_level\""));
        assert!(json.contains("\"meeting_id\":\"m-1\""));
        assert!(json.contains("\"mic_rms\""));
        let back: MeetingEvent = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(back, MeetingEvent::AudioLevel { meeting_id, mic_rms, sys_rms }
            if meeting_id == "m-1" && (mic_rms - 0.42).abs() < 1e-6 && sys_rms.is_some())
        );
    }

    #[test]
    fn audio_activity_detected_serializes_with_kind() {
        let e = MeetingEvent::AudioActivityDetected {
            started_at_ms: 1_700_000_060_000.0,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"audio_activity_detected\""));
        assert!(json.contains("\"started_at_ms\""));
        // No meeting_id — event fires before any meeting is created.
        assert!(!json.contains("\"meeting_id\""));
        let back: MeetingEvent = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(back, MeetingEvent::AudioActivityDetected { started_at_ms } if started_at_ms == 1_700_000_060_000.0)
        );
    }
}
