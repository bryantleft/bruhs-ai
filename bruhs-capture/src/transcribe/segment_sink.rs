//! Final-pass segments are published only after a successful append. The first
//! write error is sticky: later callback success cannot hide a missing segment.
use super::segments_log::{append_segment, LogError};
use crate::state::CaptureState;
use bruhs_models::{MeetingEvent, TranscriptSegment};
use std::path::Path;

pub(super) struct SegmentSink<'a> {
    state: &'a CaptureState,
    path: &'a Path,
    error: Option<LogError>,
}

impl<'a> SegmentSink<'a> {
    pub fn new(state: &'a CaptureState, path: &'a Path) -> Self {
        Self {
            state,
            path,
            error: None,
        }
    }

    pub fn push(&mut self, segment: TranscriptSegment) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = append_segment(self.path, &segment) {
            self.error = Some(error);
            return;
        }
        self.state.emit(MeetingEvent::Segment {
            meeting_id: segment.meeting_id.clone(),
            segment,
        });
    }

    pub fn finish(self) -> Result<(), LogError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(id: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            meeting_id: "synthetic".into(),
            speaker_id: None,
            start_ms: 0,
            end_ms: 1000,
            text: "Synthetic speech".into(),
            confidence: 1.0,
            partial: false,
            speaker_confidence: None,
        }
    }

    #[test]
    fn successful_segment_is_persisted_before_its_event() {
        let (_root, state) = crate::state::isolated_state();
        let mut receiver = state.subscribe();
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut sink = SegmentSink::new(&state, file.path());
        sink.push(segment("saved"));
        let MeetingEvent::Segment {
            meeting_id,
            segment,
        } = receiver.try_recv().unwrap()
        else {
            panic!("expected segment")
        };
        assert_eq!(meeting_id, "synthetic");
        assert_eq!(
            super::super::segments_log::load_snapshot(file.path()).unwrap(),
            vec![segment]
        );
        sink.finish().unwrap();
    }

    #[test]
    fn write_failure_is_sticky_and_unsaved_segments_are_not_published() {
        let (_root, state) = crate::state::isolated_state();
        let mut receiver = state.subscribe();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/segments.jsonl");
        let mut sink = SegmentSink::new(&state, &path);
        sink.push(segment("failed"));
        // Storage returning mid-pass cannot silently omit the failed segment.
        std::fs::create_dir(path.parent().unwrap()).unwrap();
        sink.push(segment("later"));
        assert!(receiver.try_recv().is_err());
        assert!(!path.exists());
        assert!(
            matches!(sink.finish(), Err(LogError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn failed_later_segment_keeps_the_saved_prefix_and_reports_failure() {
        let (_root, state) = crate::state::isolated_state();
        let mut receiver = state.subscribe();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("segments.jsonl");
        let saved = dir.path().join("saved-prefix.jsonl");
        let mut sink = SegmentSink::new(&state, &path);
        sink.push(segment("saved"));
        receiver.try_recv().unwrap();
        std::fs::rename(&path, &saved).unwrap();
        std::fs::create_dir(&path).unwrap();
        sink.push(segment("failed"));
        assert!(sink.finish().is_err());
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            super::super::segments_log::load_snapshot(&saved).unwrap(),
            vec![segment("saved")]
        );
    }
}
