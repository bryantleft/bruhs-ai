use super::*;
use gpui::{AppContext, TestAppContext};
mod support;
use support::*;

#[gpui::test]
async fn reconnect_recovers_missed_speech_and_end_without_replaying_generation(
    cx: &mut TestAppContext,
) {
    let (hub, server) = fixture(cx);
    let checkpoint = serde_json::from_value::<ActionCheckpoints>(serde_json::json!({
        "reconnect": {
            "token": "00000000-0000-4000-8000-000000000001", "minimum_version": 20, "expected": null,
            "generation": {
                "request_id": "00000000-0000-4000-8000-000000000001", "meeting_id": "reconnect",
                "extract_observations": true, "replace_observations": false
            }
        }
    })).unwrap();
    hub.update(cx, |hub, cx| {
        hub.restore_action_checkpoints(checkpoint.clone());
        hub.live_meeting_id = Some("reconnect".into());
        hub.apply_meeting_event(MeetingEvent::StreamOpened, cx);
    });
    cx.condition(&hub, |hub, _| !hub.stream_recovery.in_flight)
        .await;
    hub.update(cx, |hub, _| {
        assert_eq!(hub.segments["reconnect"], recovered_segments("reconnect"));
        assert_eq!(
            hub.meeting("reconnect").unwrap().status,
            MeetingStatus::Ended
        );
        assert!(hub.live_meeting_id.is_none());
        assert_eq!(hub.action_checkpoints(), checkpoint);
        assert!(hub.meeting_action_blocked("reconnect"));
        let cache = hub.meeting_cache();
        hub.restore_meetings(serde_json::from_slice(&serde_json::to_vec(&cache).unwrap()).unwrap());
        assert_eq!(hub.segments["reconnect"], recovered_segments("reconnect"));
    });
    assert!(server
        .requests()
        .iter()
        .all(|request| request.starts_with("GET ")));
}

#[gpui::test]
async fn reconnect_bursts_coalesce_and_only_read_the_two_active_contexts(cx: &mut TestAppContext) {
    let (hub, server) = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting("second", false), vec![segment("second")], cx);
        hub.install_meeting_test_snapshot(
            meeting("historical", false),
            vec![segment("historical")],
            cx,
        );
        hub.live_meeting_id = Some("reconnect".into());
        hub.stream_recovery.last_requested = Some("second".into());
        for _ in 0..100 {
            hub.recover_meeting_stream(cx);
        }
    });
    cx.condition(&hub, |hub, _| !hub.stream_recovery.in_flight)
        .await;
    let requests = server.requests();
    // First pass: two contexts x three GETs. Follow-up reads only the remaining context.
    assert_eq!(requests.len(), 9, "{requests:?}");
    assert!(requests
        .iter()
        .all(|request| !request.contains("historical")));
    hub.update(cx, |hub, _| {
        assert_eq!(hub.segments["second"], recovered_segments("second"));
        assert_eq!(hub.segments["historical"], [segment("historical")]);
    });
}

#[gpui::test]
async fn unusable_reconnect_replies_preserve_cached_context_and_allow_retry(
    cx: &mut TestAppContext,
) {
    let (hub, server) = fixture(cx);
    let original = hub.update(cx, |hub, _| hub.meeting_cache());
    // Transport failure, foreign transcript, rewritten final speech, missing meeting.
    for mode in [1, 2, 3, 4] {
        server.mode.store(mode, std::sync::atomic::Ordering::SeqCst);
        hub.update(cx, |hub, cx| {
            hub.notice = None;
            hub.recover_meeting_stream(cx);
        });
        cx.condition(&hub, |hub, _| !hub.stream_recovery.in_flight)
            .await;
        hub.update(cx, |hub, _| {
            assert_eq!(hub.meeting_cache(), original);
            assert!(hub.notice.as_ref().unwrap().warning);
        });
    }
    server.mode.store(0, std::sync::atomic::Ordering::SeqCst);
    hub.update(cx, |hub, cx| hub.recover_meeting_stream(cx));
    cx.condition(&hub, |hub, _| !hub.stream_recovery.in_flight)
        .await;
    hub.update(cx, |hub, _| {
        assert_eq!(hub.segments["reconnect"], recovered_segments("reconnect"))
    });
    assert_eq!(server.requests().len(), 15);
}

#[gpui::test]
async fn reconnect_readback_cannot_overwrite_a_new_local_meeting_edit(cx: &mut TestAppContext) {
    let (hub, _server) = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.recover_meeting_stream(cx);
        // The reply is applied through a later UI job, even if HTTP already completed.
        hub.meetings[0].title = "New local edit".into();
    });
    cx.condition(&hub, |hub, _| !hub.stream_recovery.in_flight)
        .await;
    hub.update(cx, |hub, _| {
        assert_eq!(hub.meetings[0].title, "New local edit");
        assert_eq!(hub.segments["reconnect"], [segment("reconnect")]);
        assert!(hub
            .notice
            .as_ref()
            .unwrap()
            .message
            .contains("changed during"));
    });
}

#[gpui::test]
async fn reconnect_readback_retains_speech_received_while_http_was_in_flight(
    cx: &mut TestAppContext,
) {
    let (hub, _server) = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.recover_meeting_stream(cx);
        let mut live = segment("reconnect");
        live.id = "new-live".into();
        live.text = "Still speaking".into();
        live.partial = true;
        hub.receive_segment("reconnect".into(), live, cx);
    });
    cx.condition(&hub, |hub, _| !hub.stream_recovery.in_flight)
        .await;
    hub.update(cx, |hub, _| {
        assert!(hub.segments["reconnect"]
            .iter()
            .any(|row| row.text == "Still speaking"));
        assert!(hub
            .notice
            .as_ref()
            .unwrap()
            .message
            .contains("changed during"));
    });
}
