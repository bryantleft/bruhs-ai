use super::*;
use gpui::{AppContext, TestAppContext};
use test_support::{meeting, summary, ActionServer};
mod receipt_recovery;

fn fixture(cx: &mut TestAppContext) -> (gpui::Entity<MeetingsHub>, ActionServer) {
    cx.executor().allow_parking();
    let runtime = Arc::new(Runtime::new().expect("runtime"));
    let server = ActionServer::new(&runtime);
    let hub = cx.new(|cx| {
        MeetingsHub::new(
            ServerClient::new(&server.base, &server.base).unwrap(),
            runtime,
            MeetingPrefs::default(),
            cx,
        )
    });
    hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting(), vec![], cx);
        hub.prefs.personality.extract_observations = true;
    });
    (hub, server)
}

fn acknowledge(hub: &gpui::Entity<MeetingsHub>, cx: &mut TestAppContext) {
    hub.update(cx, |hub, cx| {
        hub.meeting_actions_saved(hub.action_save_receipts(), cx)
    });
}

#[gpui::test]
async fn meeting_mutations_wait_for_intent_save_and_serialize_until_result_save(
    cx: &mut TestAppContext,
) {
    let (hub, server) = fixture(cx);
    server.state.lock().unwrap().delay_ms = 100;
    hub.update(cx, |hub, cx| {
        hub.summarize("action-meeting".into(), true, false, cx);
        hub.set_skip_observations("action-meeting".into(), true, cx);
        let mut stale = hub.action_save_receipts();
        stale[0].token = uuid::Uuid::new_v4().to_string();
        hub.meeting_actions_saved(stale, cx);
        assert!(matches!(
            hub.meeting_actions["action-meeting"].phase,
            Phase::Queued(_)
        ));
    });
    assert_eq!(server.state.lock().unwrap().posts, 0);
    let saved_token = hub.update(cx, |hub, _| {
        hub.meeting_actions["action-meeting"]
            .checkpoint
            .token
            .clone()
    });
    acknowledge(&hub, cx);
    hub.update(cx, |hub, cx| {
        hub.summarize("action-meeting".into(), true, true, cx);
        hub.summarize("action-meeting".into(), false, false, cx);
        hub.set_skip_observations("action-meeting".into(), true, cx);
        assert!(hub.meetings[0].summary.is_none());
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting_actions["action-meeting"].phase == Phase::AwaitingSave
    })
    .await;
    hub.update(cx, |hub, cx| {
        assert!(hub.meeting_action_blocked("action-meeting"));
        assert_eq!(
            serde_json::to_value(hub.action_checkpoints()).unwrap(),
            serde_json::json!({})
        );
        hub.meeting_actions_save_failed(hub.action_save_receipts(), cx);
        assert!(hub
            .meeting_action_status("action-meeting")
            .unwrap()
            .message
            .contains("not saved"));
        hub.summarize("action-meeting".into(), true, false, cx);
    });
    acknowledge(&hub, cx);
    hub.update(cx, |hub, _| {
        assert!(!hub.meeting_action_blocked("action-meeting"));
        assert_eq!(hub.meetings[0].summary, Some(summary()));
    });
    let state = server.state.lock().unwrap();
    assert_eq!((state.posts, state.patches), (1, 0));
    assert_eq!(
        state.bodies[0]["request_id"].as_str(),
        Some(saved_token.as_str())
    );
    drop(state);
}

#[gpui::test]
async fn failed_or_mismatched_meeting_reply_retains_cache_and_retry_is_get_only(
    cx: &mut TestAppContext,
) {
    for mode in [1, 2, 3, 4, 5] {
        let (hub, server) = fixture(cx);
        server.state.lock().unwrap().mode = mode;
        hub.update(cx, |hub, cx| {
            hub.summarize("action-meeting".into(), true, false, cx)
        });
        acknowledge(&hub, cx);
        cx.condition(&hub, |hub, _| {
            hub.meeting_actions["action-meeting"].phase == Phase::Recovery
        })
        .await;
        hub.update(cx, |hub, cx| {
            assert!(hub.meetings[0].summary.is_none(), "mode {mode}");
            hub.summarize("action-meeting".into(), true, false, cx);
            hub.set_skip_observations("action-meeting".into(), true, cx);
        });
        server.state.lock().unwrap().mode = 0;
        hub.update(cx, |hub, cx| {
            hub.retry_meeting_action("action-meeting".into(), cx)
        });
        cx.condition(&hub, |hub, _| {
            hub.meeting_actions["action-meeting"].phase == Phase::AwaitingSave
        })
        .await;
        acknowledge(&hub, cx);
        assert_eq!(server.state.lock().unwrap().posts, 1);
        assert_eq!(server.state.lock().unwrap().patches, 0);
    }
}

#[gpui::test]
async fn restored_intent_never_replays_generation_and_validated_get_clears_it_only_after_save(
    cx: &mut TestAppContext,
) {
    let (hub, server) = fixture(cx);
    let encoded = hub.update(cx, |hub, cx| {
        hub.summarize("action-meeting".into(), true, false, cx);
        serde_json::to_vec(&hub.action_checkpoints()).unwrap()
    });
    hub.update(cx, |hub, _| {
        hub.restore_action_checkpoints(serde_json::from_slice(&encoded).unwrap())
    });
    acknowledge(&hub, cx);
    assert_eq!(server.state.lock().unwrap().posts, 0);
    {
        let mut state = server.state.lock().unwrap();
        state.meeting.summary = Some(summary());
        state.meeting.updated_at = 21;
        let saved: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        let request =
            serde_json::from_value(saved["action-meeting"]["generation"].clone()).unwrap();
        state.generation_receipt = Some(bruhs_models::SummaryGenerationReceipt {
            request,
            state: bruhs_models::SummaryGenerationState::Succeeded { summary: summary() },
        });
    }
    hub.update(cx, |hub, cx| {
        assert!(
            hub.meeting_action_status("action-meeting")
                .unwrap()
                .recovery
        );
        hub.retry_meeting_action("action-meeting".into(), cx);
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting_actions["action-meeting"].phase == Phase::AwaitingSave
    })
    .await;
    acknowledge(&hub, cx);
    assert_eq!(server.state.lock().unwrap().posts, 0);
    assert_eq!(server.state.lock().unwrap().gets, 1);
}

#[gpui::test]
async fn observation_setting_validates_receipt_and_recovery_does_not_repeat_patch(
    cx: &mut TestAppContext,
) {
    let (hub, server) = fixture(cx);
    server.state.lock().unwrap().mode = 6;
    hub.update(cx, |hub, cx| {
        hub.set_skip_observations("action-meeting".into(), true, cx)
    });
    acknowledge(&hub, cx);
    cx.condition(&hub, |hub, _| {
        hub.meeting_actions["action-meeting"].phase == Phase::Recovery
    })
    .await;
    hub.update(cx, |hub, _| assert!(!hub.meetings[0].skip_observations));
    server.state.lock().unwrap().mode = 0;
    hub.update(cx, |hub, cx| {
        hub.retry_meeting_action("action-meeting".into(), cx)
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting_actions["action-meeting"].phase == Phase::AwaitingSave
    })
    .await;
    acknowledge(&hub, cx);
    hub.update(cx, |hub, _| assert!(hub.meetings[0].skip_observations));
    assert_eq!(server.state.lock().unwrap().patches, 1);
}

#[gpui::test]
fn queued_extraction_rechecks_current_privacy_before_any_request(cx: &mut TestAppContext) {
    let (hub, server) = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.summarize("action-meeting".into(), true, false, cx);
        hub.prefs.personality.extract_observations = false;
    });
    acknowledge(&hub, cx);
    hub.update(cx, |hub, _| {
        assert_eq!(hub.meeting_actions["action-meeting"].phase, Phase::Recovery)
    });
    assert_eq!(server.state.lock().unwrap().posts, 0);
}

#[test]
fn malformed_checkpoint_identity_fails_closed_and_empty_legacy_state_is_valid() {
    assert!(serde_json::from_str::<ActionCheckpoints>("{}").is_ok());
    for (id, token) in [
        ("", uuid::Uuid::new_v4().to_string()),
        ("meeting", "not-a-token".into()),
    ] {
        let json = serde_json::json!({id: {"token":token,"minimum_version":20,"expected":null}});
        assert!(serde_json::from_value::<ActionCheckpoints>(json).is_err());
    }
}

#[gpui::test]
fn queued_mutations_reject_changed_deleted_or_ambiguous_targets(cx: &mut TestAppContext) {
    for mutation in 0..3 {
        let (hub, server) = fixture(cx);
        hub.update(cx, |hub, cx| {
            hub.summarize("action-meeting".into(), false, false, cx);
            match mutation {
                0 => hub.meetings[0].updated_at += 1,
                1 => hub.meetings.clear(),
                _ => hub.meetings.push(hub.meetings[0].clone()),
            }
        });
        acknowledge(&hub, cx);
        hub.update(cx, |hub, _| {
            assert_eq!(hub.meeting_actions["action-meeting"].phase, Phase::Recovery)
        });
        assert_eq!(server.state.lock().unwrap().posts, 0);
    }
}

#[test]
fn summary_timeout_override_accepts_a_reply_after_the_normal_twenty_second_deadline() {
    let runtime = Runtime::new().unwrap();
    let server = ActionServer::new(&runtime);
    server.state.lock().unwrap().delay_ms = 21_000;
    let started = std::time::Instant::now();
    let client = ServerClient::new(&server.base, &server.base).unwrap();
    let result = runtime.block_on(client.summarize_meeting(
        "action-meeting",
        "owned-timeout-request",
        false,
        false,
    ));
    assert_eq!(result.unwrap(), summary());
    assert!(started.elapsed() >= Duration::from_secs(21));
    assert_eq!(server.state.lock().unwrap().posts, 1);
}

#[gpui::test]
async fn late_generation_cannot_replace_newer_or_deleted_cached_meetings(cx: &mut TestAppContext) {
    for deleted in [false, true] {
        let (hub, server) = fixture(cx);
        server.state.lock().unwrap().delay_ms = 100;
        hub.update(cx, |hub, cx| {
            hub.summarize("action-meeting".into(), false, false, cx)
        });
        acknowledge(&hub, cx);
        hub.update(cx, |hub, cx| {
            if deleted {
                let baseline = hub.meeting_cache();
                hub.merge_meetings(vec![], &baseline, cx);
            } else {
                let mut newer = meeting();
                newer.updated_at = 22;
                newer.title = "Newer server title".into();
                hub.replace_meeting(newer, cx);
            }
        });
        cx.condition(&hub, |hub, _| {
            hub.meeting_actions["action-meeting"].phase == Phase::Recovery
        })
        .await;
        hub.update(cx, |hub, _| {
            if deleted {
                assert!(hub.meeting("action-meeting").is_none());
            } else {
                let meeting = hub.meeting("action-meeting").unwrap();
                assert_eq!(meeting.title, "Newer server title");
                assert!(meeting.summary.is_none());
            }
        });
        assert_eq!(server.state.lock().unwrap().posts, 1);
    }
}

#[gpui::test]
async fn unversioned_summary_event_uses_authoritative_read_and_completed_transcription_does_not_regenerate(
    cx: &mut TestAppContext,
) {
    let (hub, server) = fixture(cx);
    {
        let mut state = server.state.lock().unwrap();
        state.meeting.summary = Some(summary());
        state.meeting.updated_at = 21;
    }
    let mut unversioned = summary();
    unversioned.tldr = "Do not publish this event payload".into();
    hub.update(cx, |hub, cx| {
        hub.apply_meeting_event(
            MeetingEvent::Summary {
                meeting_id: "action-meeting".into(),
                summary: unversioned,
            },
            cx,
        );
        assert!(hub.meeting("action-meeting").unwrap().summary.is_none());
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting("action-meeting").unwrap().summary.is_some()
    })
    .await;
    hub.update(cx, |hub, cx| {
        assert_eq!(
            hub.meeting("action-meeting").unwrap().summary,
            Some(summary())
        );
        hub.apply_meeting_event(
            MeetingEvent::TranscriptionCompleted {
                meeting_id: "action-meeting".into(),
            },
            cx,
        );
        assert!(!hub.meeting_action_blocked("action-meeting"));
        assert!(hub.action_save_receipts().is_empty());
    });
    acknowledge(&hub, cx);
    assert_eq!(server.state.lock().unwrap().posts, 0);
}
