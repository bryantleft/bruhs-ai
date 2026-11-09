use super::*;
use gpui::{AppContext, TestAppContext};
mod http;

#[gpui::test]
fn late_transcript_reply_cannot_restore_older_speaker_annotations(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        let receipt = hub.transcript_read_receipt("cached-meeting");
        let mut newer = segment();
        newer.speaker_id = Some("corrected-speaker".into());
        hub.apply_transcript_snapshot("cached-meeting".into(), vec![newer.clone()], cx)
            .unwrap();
        hub.receive_transcript_since("cached-meeting".into(), receipt, Ok(vec![segment()]), cx);
        assert_eq!(hub.segments["cached-meeting"], [newer]);
        assert!(hub
            .notice
            .as_ref()
            .unwrap()
            .message
            .contains("newer transcript"));
    });
}

pub(super) fn fixture(cx: &mut TestAppContext) -> gpui::Entity<MeetingsHub> {
    cx.executor().allow_parking();
    let runtime = Arc::new(tokio::runtime::Runtime::new().expect("runtime"));
    cx.new(|cx| {
        MeetingsHub::new(
            ServerClient::new("http://127.0.0.1:0", "http://127.0.0.1:0").unwrap(),
            runtime,
            MeetingPrefs::default(),
            cx,
        )
    })
}

pub(super) fn meeting() -> Meeting {
    let mut meeting = Meeting::new("Saved meeting", 10.0);
    meeting.id = "cached-meeting".into();
    meeting.updated_at = 20;
    meeting
}

pub(super) fn segment() -> TranscriptSegment {
    TranscriptSegment {
        id: "saved-segment".into(),
        meeting_id: "cached-meeting".into(),
        text: "Keep these finalized words".into(),
        start_ms: 0,
        end_ms: 10,
        speaker_id: None,
        confidence: 0.9,
        partial: false,
        speaker_confidence: None,
    }
}

#[gpui::test]
fn restored_meeting_cache_preserves_context_but_never_resumes_capture(cx: &mut TestAppContext) {
    let source = fixture(cx);
    let cache = source.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        hub.meeting_cache()
    });
    let encoded = serde_json::to_vec(&cache).expect("serialize cache");
    let target = fixture(cx);
    target.update(cx, |hub, _| {
        hub.restore_meetings(serde_json::from_slice(&encoded).expect("validate restored cache"));
        assert_eq!(hub.meetings, vec![meeting()]);
        assert_eq!(hub.segments["cached-meeting"], vec![segment()]);
        assert!(hub.live_meeting_id.is_none());
        assert!(hub.capture_status.recording().is_none());
        assert!(!hub.meeting_action_blocked("cached-meeting"));
    });
}

#[gpui::test]
fn workspace_snapshots_share_text_and_exclude_unconfirmed_speaker_edits(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        let first = hub.meeting_cache();
        hub.set_speaker_label("cached-meeting", "them", "Unconfirmed rename".into());
        let second = hub.meeting_cache();
        assert!(Arc::ptr_eq(
            &first.records["cached-meeting"],
            &second.records["cached-meeting"]
        ));
        assert!(Arc::ptr_eq(
            &first.transcripts["cached-meeting"],
            &second.transcripts["cached-meeting"]
        ));
        assert_eq!(second.records["cached-meeting"].as_ref(), &meeting());
        assert_eq!(
            hub.meetings[0]
                .speakers
                .iter()
                .find(|s| s.id == "them")
                .unwrap()
                .label,
            "Unconfirmed rename"
        );
    });
}

#[gpui::test]
fn deleted_chunk_stays_gone_when_capture_replays_or_a_snapshot_still_includes_it(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    let cache = hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        let receipt = hub.transcript_read_receipt("cached-meeting");
        let mut later = segment();
        later.id = "later-segment".into();
        later.text = "Later words".into();
        later.start_ms = 20;
        later.end_ms = 30;
        hub.receive_authorized_transcript(
            "cached-meeting".into(),
            receipt,
            Ok(vec![segment(), later.clone()]),
            Ok(vec!["saved-segment".into()]),
            cx,
        );
        assert_eq!(hub.segments["cached-meeting"], vec![later.clone()]);
        assert!(hub.meeting_cache().transcripts["cached-meeting"]
            .iter()
            .all(|row| row.id != "saved-segment"));
        hub.apply_meeting_event(
            MeetingEvent::Segment {
                meeting_id: "cached-meeting".into(),
                segment: segment(),
            },
            cx,
        );
        assert_eq!(hub.segments["cached-meeting"], vec![later.clone()]);
        assert!(hub
            .apply_transcript_snapshot("cached-meeting".into(), vec![segment(), later.clone()], cx)
            .is_ok());
        assert_eq!(hub.segments["cached-meeting"], vec![later.clone()]);
        assert!(hub
            .apply_transcript_snapshot("cached-meeting".into(), vec![], cx)
            .is_err());
        assert_eq!(hub.segments["cached-meeting"], vec![later]);
        hub.meeting_cache()
    });
    let restored = fixture(cx);
    restored.update(cx, |hub, _| {
        hub.restore_meetings(cache);
        assert_eq!(hub.segments["cached-meeting"][0].id, "later-segment");
        assert!(hub.segments["cached-meeting"]
            .iter()
            .all(|row| row.text != "Keep these finalized words"));
    });
}

#[gpui::test]
fn stream_fragment_deletion_is_not_merged_back_on_restore(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    let cache = hub.update(cx, |hub, cx| {
        hub.replace_meeting(meeting(), cx);
        hub.apply_meeting_event(
            MeetingEvent::Segment {
                meeting_id: "cached-meeting".into(),
                segment: segment(),
            },
            cx,
        );
        hub.apply_meeting_event(
            MeetingEvent::SegmentRemoved {
                meeting_id: "cached-meeting".into(),
                segment_id: "saved-segment".into(),
            },
            cx,
        );
        assert!(hub.segments["cached-meeting"].is_empty());
        assert!(!hub
            .meeting_cache()
            .fragments
            .get("cached-meeting")
            .is_some_and(|fragments| fragments.iter().any(|row| row.id == "saved-segment")));
        hub.apply_meeting_event(
            MeetingEvent::Segment {
                meeting_id: "cached-meeting".into(),
                segment: segment(),
            },
            cx,
        );
        assert!(hub.segments["cached-meeting"].is_empty());
        hub.meeting_cache()
    });
    let restored = fixture(cx);
    restored.update(cx, |hub, _| {
        hub.restore_meetings(cache);
        assert!(hub
            .segments
            .get("cached-meeting")
            .is_none_or(|rows| rows.is_empty()));
    });
}

#[gpui::test]
fn transcript_refresh_cannot_remove_or_rewrite_finalized_speech(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        let before = hub.meeting_cache();
        for incoming in [
            vec![],
            vec![TranscriptSegment {
                text: "Rewritten".into(),
                ..segment()
            }],
            vec![segment(), segment()],
            vec![TranscriptSegment {
                meeting_id: "foreign".into(),
                ..segment()
            }],
        ] {
            assert!(hub
                .apply_transcript_snapshot("cached-meeting".into(), incoming, cx)
                .is_err());
            assert_eq!(hub.meeting_cache(), before);
            assert_eq!(hub.segments["cached-meeting"], vec![segment()]);
        }
        let annotated = TranscriptSegment {
            speaker_id: Some("speaker".into()),
            ..segment()
        };
        hub.apply_transcript_snapshot("cached-meeting".into(), vec![annotated.clone()], cx)
            .expect("verified annotation");
        assert_eq!(hub.segments["cached-meeting"], vec![annotated]);
    });
}

#[gpui::test]
fn stream_replays_upsert_partials_without_duplicating_or_rewriting_final_words(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.replace_meeting(meeting(), cx);
        for text in ["Keep", "Keep these"] {
            hub.apply_meeting_event(
                MeetingEvent::Segment {
                    meeting_id: "cached-meeting".into(),
                    segment: TranscriptSegment {
                        partial: true,
                        text: text.into(),
                        ..segment()
                    },
                },
                cx,
            );
        }
        hub.apply_meeting_event(
            MeetingEvent::Segment {
                meeting_id: "cached-meeting".into(),
                segment: segment(),
            },
            cx,
        );
        hub.apply_meeting_event(
            MeetingEvent::Segment {
                meeting_id: "cached-meeting".into(),
                segment: segment(),
            },
            cx,
        );
        hub.apply_meeting_event(
            MeetingEvent::Segment {
                meeting_id: "cached-meeting".into(),
                segment: TranscriptSegment {
                    text: "Attack".into(),
                    ..segment()
                },
            },
            cx,
        );
        hub.apply_meeting_event(
            MeetingEvent::Segment {
                meeting_id: "cached-meeting".into(),
                segment: TranscriptSegment {
                    meeting_id: "foreign".into(),
                    ..segment()
                },
            },
            cx,
        );
        assert_eq!(hub.segments["cached-meeting"], vec![segment()]);
        assert!(
            hub.meeting_cache().transcripts.is_empty(),
            "stream fragments are not complete snapshot receipts"
        );
        assert_eq!(
            hub.meeting_cache().fragments["cached-meeting"]
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![segment()]
        );
    });
}

#[gpui::test]
fn stale_or_malformed_meeting_refresh_never_erases_cached_records(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        let before = hub.meeting_cache();
        hub.merge_meetings(
            vec![Meeting {
                title: "Stale".into(),
                updated_at: 19,
                ..meeting()
            }],
            &before,
            cx,
        );
        hub.merge_meetings(vec![meeting(), meeting()], &before, cx);
        assert_eq!(hub.meeting_cache(), before);
        assert_eq!(hub.meetings, vec![meeting()]);
    });
}

#[test]
fn malformed_saved_meeting_cache_is_rejected_not_defaulted() {
    let valid = serde_json::json!({"records":{"cached-meeting":meeting()},"transcripts":{"cached-meeting":[segment()]}});
    assert!(serde_json::from_value::<MeetingCache>(valid.clone()).is_ok());
    for field in ["id", "started_at_ms"] {
        let mut invalid = valid.clone();
        invalid["records"]["cached-meeting"][field] = serde_json::Value::Null;
        assert!(serde_json::from_value::<MeetingCache>(invalid).is_err());
    }
    let mut foreign = valid.clone();
    foreign["transcripts"]["cached-meeting"][0]["meeting_id"] = "foreign".into();
    assert!(serde_json::from_value::<MeetingCache>(foreign).is_err());
    let mut duplicate = valid;
    duplicate["transcripts"]["cached-meeting"] = serde_json::json!([segment(), segment()]);
    assert!(serde_json::from_value::<MeetingCache>(duplicate).is_err());
}

#[gpui::test]
fn authoritative_removal_prunes_transcript_without_erasing_post_request_updates(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        let baseline = hub.meeting_cache();
        let newer = Meeting {
            title: "New confirmed title".into(),
            updated_at: 21,
            ..meeting()
        };
        hub.replace_meeting(newer.clone(), cx);
        hub.merge_meetings(vec![], &baseline, cx);
        assert_eq!(hub.meetings, vec![newer]);
        let current = hub.meeting_cache();
        hub.merge_meetings(vec![], &current, cx);
        assert!(hub.meetings.is_empty());
        assert!(hub.segments.is_empty());
        assert!(hub.meeting_cache().records.is_empty());
        assert!(hub.meeting_cache().transcripts.is_empty());
        hub.replace_meeting(meeting(), cx);
        assert!(
            hub.meetings.is_empty(),
            "late mutation reply cannot resurrect deletion"
        );
        assert!(hub
            .apply_transcript_snapshot("cached-meeting".into(), vec![segment()], cx)
            .is_err());
    });
}
