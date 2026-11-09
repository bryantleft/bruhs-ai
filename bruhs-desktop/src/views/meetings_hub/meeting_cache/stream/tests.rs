use super::*;
use crate::views::meetings_hub::meeting_cache::tests::{fixture, meeting, segment};
use gpui::TestAppContext;
#[cfg(not(debug_assertions))]
mod performance;

#[test]
fn stream_snapshot_copies_only_one_page_of_pointers_and_retains_old_text() {
    let mut fragments = StreamFragments::default();
    for index in 0..512 {
        fragments
            .update(
                None,
                TranscriptSegment {
                    id: format!("s{index}"),
                    text: "x".repeat(1024),
                    ..segment()
                },
            )
            .unwrap();
    }
    let saved = fragments.clone();
    fragments
        .update(
            Some(270),
            TranscriptSegment {
                id: "s270".into(),
                text: "Changed fragment".into(),
                ..segment()
            },
        )
        .unwrap();
    for index in [0, 1, 3] {
        assert!(Arc::ptr_eq(&saved.0[index], &fragments.0[index]));
    }
    assert!(!Arc::ptr_eq(&saved.0[2], &fragments.0[2]));
    assert!(Arc::ptr_eq(&saved.0[2][0], &fragments.0[2][0]));
    assert_eq!(saved.0[2][14].text.len(), 1024);
    assert_eq!(fragments.0[2][14].text, "Changed fragment");
}

#[gpui::test]
fn stream_restore_preserves_partial_and_final_records_without_claiming_complete_snapshot(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    let encoded = hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![segment()], cx);
        hub.receive_segment(
            "cached-meeting".into(),
            TranscriptSegment {
                id: "live".into(),
                text: "Still talking 🗒".into(),
                partial: true,
                ..segment()
            },
            cx,
        );
        serde_json::to_vec(&hub.meeting_cache()).unwrap()
    });
    let restored = fixture(cx);
    restored.update(cx, |hub, cx| {
        hub.restore_meetings(serde_json::from_slice(&encoded).unwrap());
        assert_eq!(hub.segments["cached-meeting"].len(), 2);
        assert!(hub.segments["cached-meeting"]
            .iter()
            .any(|record| record.id == "live"
                && record.partial
                && record.text == "Still talking 🗒"));
        assert!(!hub.search_cached.contains("cached-meeting"));
        assert!(hub.live_meeting_id.is_none());
        assert!(hub.capture_status.recording().is_none());
        hub.receive_segment(
            "cached-meeting".into(),
            TranscriptSegment {
                id: "live".into(),
                text: "Finished talking 🗒".into(),
                ..segment()
            },
            cx,
        );
        assert_eq!(hub.segments["cached-meeting"].len(), 2);
        let complete = hub.segments["cached-meeting"].clone();
        hub.apply_transcript_snapshot("cached-meeting".into(), complete, cx)
            .unwrap();
        assert!(hub.meeting_cache.fragments.is_empty());
        assert!(hub.search_cached.contains("cached-meeting"));
    });
}

#[gpui::test]
fn duplicate_stream_events_do_not_dirty_cache_and_new_fragments_protect_against_old_list_deletion(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.replace_meeting(meeting(), cx);
        hub.receive_segment("cached-meeting".into(), segment(), cx);
        let before = hub.meeting_cache();
        let revision = hub.search_revision();
        hub.receive_segment("cached-meeting".into(), segment(), cx);
        assert_eq!(hub.search_revision(), revision);
        assert!(hub.meeting_cache.fragments["cached-meeting"]
            .same_snapshot(&before.fragments["cached-meeting"]));
        hub.receive_segment(
            "cached-meeting".into(),
            TranscriptSegment {
                id: "next".into(),
                ..segment()
            },
            cx,
        );
        hub.merge_meetings(vec![], &before, cx);
        assert_eq!(hub.meetings.len(), 1);
        let latest = hub.meeting_cache();
        hub.merge_meetings(vec![], &latest, cx);
        assert!(hub.meeting_cache.fragments.is_empty());
        assert!(hub.transcript_indexes.is_empty());
    });
}

#[gpui::test]
fn malformed_or_conflicting_saved_stream_fragments_fail_closed(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    let valid = hub.update(cx, |hub, cx| {
        hub.replace_meeting(meeting(), cx);
        hub.receive_segment("cached-meeting".into(), segment(), cx);
        serde_json::to_value(hub.meeting_cache()).unwrap()
    });
    for case in 0..6 {
        let mut invalid = valid.clone();
        match case {
            0 => invalid["fragments"]["cached-meeting"][0][0]["meeting_id"] = "foreign".into(),
            1 => {
                invalid["fragments"]["cached-meeting"][0] =
                    serde_json::json!([segment(), segment()])
            }
            2 => invalid["fragments"]["cached-meeting"] = serde_json::json!([[], [segment()]]),
            3 => invalid["fragments"]["cached-meeting"] = serde_json::Value::Null,
            4 => invalid["records"] = serde_json::json!({}),
            _ => {
                invalid["transcripts"]["cached-meeting"] = serde_json::json!([segment()]);
                invalid["fragments"]["cached-meeting"][0][0]["text"] = "Rewrite".into();
            }
        }
        assert!(
            serde_json::from_value::<MeetingCache>(invalid).is_err(),
            "case {case}"
        );
    }
}
