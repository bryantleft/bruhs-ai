use super::*;
use bruhs_models::{SummaryGenerationReceipt, SummaryGenerationRequest, SummaryGenerationState};

fn restore_generation(
    hub: &gpui::Entity<MeetingsHub>,
    cx: &mut TestAppContext,
) -> SummaryGenerationRequest {
    hub.update(cx, |hub, cx| {
        hub.summarize("action-meeting".into(), true, false, cx);
        let checkpoint = hub.meeting_actions["action-meeting"].checkpoint.clone();
        let saved = serde_json::to_vec(&hub.action_checkpoints()).unwrap();
        hub.restore_action_checkpoints(serde_json::from_slice(&saved).unwrap());
        checkpoint.generation.unwrap()
    })
}

#[gpui::test]
async fn unresolved_or_mismatched_receipts_cannot_clear_restored_generation(
    cx: &mut TestAppContext,
) {
    for scenario in 0..8 {
        let (hub, server) = fixture(cx);
        let mut request = restore_generation(&hub, cx);
        if scenario == 3 {
            request.request_id = uuid::Uuid::new_v4().to_string();
        }
        if scenario == 4 {
            request.meeting_id = "foreign".into();
        }
        if scenario == 5 {
            request.extract_observations = false;
        }
        {
            let mut state = server.state.lock().unwrap();
            state.meeting.summary = Some(summary());
            state.meeting.updated_at = 21;
            if scenario != 0 {
                state.generation_receipt = Some(SummaryGenerationReceipt {
                    request,
                    state: if scenario == 1 {
                        SummaryGenerationState::Pending
                    } else {
                        SummaryGenerationState::Succeeded { summary: summary() }
                    },
                });
            }
            if scenario == 2 {
                state.receipt_status = 503;
            }
            if scenario >= 6 {
                let mut payload = serde_json::to_value(&state.generation_receipt).unwrap();
                payload["state"] = if scenario == 6 {
                    serde_json::json!({"status": "succeeded"})
                } else {
                    serde_json::json!({"status": "unknown"})
                };
                state.receipt_payload = Some(payload);
            }
        }
        hub.update(cx, |hub, cx| {
            hub.retry_meeting_action("action-meeting".into(), cx)
        });
        cx.condition(&hub, |hub, _| {
            hub.meeting_actions["action-meeting"].phase == Phase::Recovery
        })
        .await;
        acknowledge(&hub, cx);
        hub.update(cx, |hub, cx| {
            assert!(
                hub.meeting_action_blocked("action-meeting"),
                "scenario {scenario}"
            );
            assert!(hub.meetings[0].summary.is_none());
            assert!(hub.meeting_actions["action-meeting"]
                .checkpoint
                .generation
                .is_some());
            hub.summarize("action-meeting".into(), true, false, cx);
        });
        let state = server.state.lock().unwrap();
        assert_eq!(
            (state.posts, state.patches, state.gets, state.receipt_gets),
            (0, 0, 0, 1)
        );
        drop(state);
    }
}

#[gpui::test]
async fn terminal_receipt_recovers_newer_meeting_without_replaying_or_overwriting_it(
    cx: &mut TestAppContext,
) {
    for terminal in [
        SummaryGenerationState::Failed,
        SummaryGenerationState::Cancelled,
        SummaryGenerationState::Succeeded { summary: summary() },
    ] {
        let (hub, server) = fixture(cx);
        let request = restore_generation(&hub, cx);
        let mut latest = summary();
        latest.tldr = "A newer, separately saved summary".into();
        latest.generated_at = 22;
        {
            let mut state = server.state.lock().unwrap();
            state.generation_receipt = Some(SummaryGenerationReceipt {
                request,
                state: terminal,
            });
            state.meeting.updated_at = 23;
            state.meeting.summary = Some(latest.clone());
        }
        hub.update(cx, |hub, cx| {
            hub.meeting_actions
                .get_mut("action-meeting")
                .unwrap()
                .checkpoint
                .expected = Some(Box::new(Expected {
                summary: Some(summary()),
                skip: None,
            }));
            hub.retry_meeting_action("action-meeting".into(), cx);
        });
        cx.condition(&hub, |hub, _| {
            hub.meeting_actions["action-meeting"].phase == Phase::AwaitingSave
        })
        .await;
        hub.update(cx, |hub, cx| {
            assert_eq!(hub.meetings[0].summary, Some(latest.clone()));
            hub.meeting_actions_save_failed(hub.action_save_receipts(), cx);
            assert!(hub.meeting_action_blocked("action-meeting"));
        });
        acknowledge(&hub, cx);
        hub.update(cx, |hub, _| {
            assert!(!hub.meeting_action_blocked("action-meeting"))
        });
        let state = server.state.lock().unwrap();
        assert_eq!(
            (state.posts, state.patches, state.gets, state.receipt_gets),
            (0, 0, 1, 1)
        );
        drop(state);
    }
}

#[gpui::test]
async fn known_unsent_generation_can_recover_without_a_receipt(cx: &mut TestAppContext) {
    let (hub, server) = fixture(cx);
    hub.update(cx, |hub, cx| {
        hub.summarize("action-meeting".into(), true, false, cx);
        hub.prefs.personality.extract_observations = false;
    });
    acknowledge(&hub, cx);
    hub.update(cx, |hub, cx| {
        assert!(hub.meeting_actions["action-meeting"]
            .checkpoint
            .generation
            .is_none());
        hub.retry_meeting_action("action-meeting".into(), cx);
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting_actions["action-meeting"].phase == Phase::AwaitingSave
    })
    .await;
    acknowledge(&hub, cx);
    let state = server.state.lock().unwrap();
    assert_eq!((state.posts, state.receipt_gets, state.gets), (0, 0, 1));
    drop(state);
}

#[test]
fn persisted_generation_requires_exact_checkpoint_identity_and_valid_options() {
    let token = uuid::Uuid::new_v4().to_string();
    let request = SummaryGenerationRequest {
        request_id: token.clone(),
        meeting_id: "action-meeting".into(),
        extract_observations: true,
        replace_observations: false,
    };
    let valid = serde_json::json!({"action-meeting": {
        "token": token, "minimum_version": 20, "expected": null, "generation": request,
    }});
    assert!(serde_json::from_value::<ActionCheckpoints>(valid.clone()).is_ok());
    for field in ["request_id", "meeting_id", "replace_observations"] {
        let mut invalid = valid.clone();
        if field == "replace_observations" {
            invalid["action-meeting"]["generation"][field] = true.into();
            invalid["action-meeting"]["generation"]["extract_observations"] = false.into();
        } else {
            invalid["action-meeting"]["generation"][field] = "foreign".into();
        }
        assert!(
            serde_json::from_value::<ActionCheckpoints>(invalid).is_err(),
            "{field}"
        );
    }
    let mut legacy = valid;
    legacy["action-meeting"]
        .as_object_mut()
        .unwrap()
        .remove("generation");
    assert!(serde_json::from_value::<ActionCheckpoints>(legacy).is_ok());
}

#[gpui::test]
async fn cancelling_a_restored_generation_fences_it_before_it_reaches_the_server(
    cx: &mut TestAppContext,
) {
    let (hub, server) = fixture(cx);
    let request = restore_generation(&hub, cx);
    hub.update(cx, |hub, cx| {
        let status = hub.meeting_action_status("action-meeting").unwrap();
        assert!(status.recovery && status.cancellable && !status.busy);
        hub.cancel_meeting_action("action-meeting".into(), cx);
        assert!(
            !hub.meeting_action_status("action-meeting")
                .unwrap()
                .cancellable
        );
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting_actions["action-meeting"].phase == Phase::AwaitingSave
    })
    .await;
    hub.update(cx, |hub, _| {
        assert!(hub.meetings[0].summary.is_none());
        assert!(hub.meeting_action_blocked("action-meeting"));
    });
    acknowledge(&hub, cx);
    hub.update(cx, |hub, _| {
        assert!(!hub.meeting_action_blocked("action-meeting"));
        assert!(serde_json::to_value(hub.action_checkpoints())
            .unwrap()
            .as_object()
            .unwrap()
            .is_empty());
    });
    let state = server.state.lock().unwrap();
    let receipt = state.generation_receipt.clone().unwrap();
    assert_eq!(receipt.request, request);
    assert_eq!(receipt.state, SummaryGenerationState::Cancelled);
    assert_eq!(
        (
            state.posts,
            state.patches,
            state.receipt_gets,
            state.receipt_deletes,
            state.gets
        ),
        (0, 0, 0, 1, 1)
    );
    drop(state);
}

#[gpui::test]
async fn unconfirmed_cancellations_retain_the_restored_generation(cx: &mut TestAppContext) {
    for scenario in 0..4 {
        let (hub, server) = fixture(cx);
        let mut request = restore_generation(&hub, cx);
        {
            let mut state = server.state.lock().unwrap();
            match scenario {
                0 => state.receipt_status = 503,
                1 => {
                    request.request_id = uuid::Uuid::new_v4().to_string();
                    state.receipt_payload = Some(
                        serde_json::to_value(SummaryGenerationReceipt {
                            request,
                            state: SummaryGenerationState::Cancelled,
                        })
                        .unwrap(),
                    );
                }
                2 => {
                    state.receipt_payload = Some(
                        serde_json::to_value(SummaryGenerationReceipt {
                            request,
                            state: SummaryGenerationState::Pending,
                        })
                        .unwrap(),
                    );
                }
                _ => {
                    // A cancelled receipt whose local clear cannot be acknowledged stays locked.
                    state.mode = 1;
                }
            }
        }
        hub.update(cx, |hub, cx| {
            hub.cancel_meeting_action("action-meeting".into(), cx)
        });
        cx.condition(&hub, |hub, _| {
            hub.meeting_actions["action-meeting"].phase == Phase::Recovery
        })
        .await;
        acknowledge(&hub, cx);
        hub.update(cx, |hub, cx| {
            let status = hub.meeting_action_status("action-meeting").unwrap();
            assert!(status.recovery && status.cancellable, "scenario {scenario}");
            assert!(
                hub.meeting_action_blocked("action-meeting"),
                "scenario {scenario}"
            );
            assert!(hub.meeting_actions["action-meeting"]
                .checkpoint
                .generation
                .is_some());
            hub.summarize("action-meeting".into(), true, false, cx);
        });
        let state = server.state.lock().unwrap();
        assert_eq!(
            (state.posts, state.receipt_deletes, state.gets),
            (0, 1, usize::from(scenario == 3)),
            "scenario {scenario}"
        );
        drop(state);
    }
}

#[gpui::test]
async fn discovered_foreign_request_can_be_fenced_without_generating(cx: &mut TestAppContext) {
    let (hub, server) = fixture(cx);
    let foreign = SummaryGenerationRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        meeting_id: "action-meeting".into(),
        extract_observations: false,
        replace_observations: false,
    };
    {
        let mut state = server.state.lock().unwrap();
        state.pending_payload = Some(
            serde_json::to_value(SummaryGenerationReceipt {
                request: foreign.clone(),
                state: SummaryGenerationState::Pending,
            })
            .unwrap(),
        );
    }
    hub.update(cx, |hub, cx| {
        hub.discover_pending_generation("action-meeting".into(), cx);
        let status = hub.meeting_action_status("action-meeting").unwrap();
        assert!(status.busy && !status.foreign_pending);
        assert!(hub.meeting_action_blocked("action-meeting"));
        // A read-only check is never persisted as saved uncertainty.
        assert!(serde_json::to_value(hub.action_checkpoints())
            .unwrap()
            .as_object()
            .unwrap()
            .is_empty());
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting_action_status("action-meeting")
            .is_some_and(|status| status.foreign_pending)
    })
    .await;
    hub.update(cx, |hub, cx| {
        assert!(!hub.meeting_action_blocked("action-meeting"));
        hub.cancel_foreign_generation("action-meeting".into(), cx);
        assert!(hub.meeting_action_status("action-meeting").unwrap().busy);
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting_action_status("action-meeting")
            .is_some_and(|status| !status.busy)
    })
    .await;
    hub.update(cx, |hub, cx| {
        let status = hub.meeting_action_status("action-meeting").unwrap();
        assert!(!status.foreign_pending && !status.recovery);
        assert!(status.message.contains("cancelled"));
        assert!(!hub.meeting_action_blocked("action-meeting"));
        hub.discover_pending_generation("action-meeting".into(), cx);
    });
    cx.condition(&hub, |hub, _| {
        hub.meeting_action_status("action-meeting")
            .is_some_and(|status| !status.busy)
    })
    .await;
    let state = server.state.lock().unwrap();
    let receipt = state.generation_receipt.clone().unwrap();
    assert_eq!(receipt.request, foreign);
    assert_eq!(receipt.state, SummaryGenerationState::Cancelled);
    assert_eq!(
        (
            state.posts,
            state.pending_gets,
            state.receipt_deletes,
            state.gets
        ),
        (0, 2, 1, 0)
    );
    drop(state);
}

#[gpui::test]
async fn unusable_discovery_and_cancellation_replies_change_nothing(cx: &mut TestAppContext) {
    for scenario in 0..4 {
        let (hub, server) = fixture(cx);
        let mut foreign = SummaryGenerationRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            meeting_id: "action-meeting".into(),
            extract_observations: true,
            replace_observations: false,
        };
        {
            let mut state = server.state.lock().unwrap();
            match scenario {
                0 => state.receipt_status = 503,
                1 => foreign.meeting_id = "other-meeting".into(),
                _ => {}
            }
            state.pending_payload = Some(
                serde_json::to_value(SummaryGenerationReceipt {
                    request: foreign.clone(),
                    state: if scenario == 2 {
                        SummaryGenerationState::Failed
                    } else {
                        SummaryGenerationState::Pending
                    },
                })
                .unwrap(),
            );
        }
        hub.update(cx, |hub, cx| {
            hub.discover_pending_generation("action-meeting".into(), cx)
        });
        cx.condition(&hub, |hub, _| {
            hub.meeting_action_status("action-meeting")
                .is_some_and(|status| !status.busy)
        })
        .await;
        if scenario < 3 {
            hub.update(cx, |hub, cx| {
                let status = hub.meeting_action_status("action-meeting").unwrap();
                assert!(!status.foreign_pending, "scenario {scenario}");
                assert!(!hub.meeting_action_blocked("action-meeting"));
                hub.cancel_foreign_generation("action-meeting".into(), cx);
                assert!(!hub.meeting_action_status("action-meeting").unwrap().busy);
            });
            let state = server.state.lock().unwrap();
            assert_eq!((state.posts, state.receipt_deletes), (0, 0));
            drop(state);
            continue;
        }
        // Discovered, but the server refuses to confirm the cancellation.
        {
            let mut state = server.state.lock().unwrap();
            state.receipt_payload = Some(
                serde_json::to_value(SummaryGenerationReceipt {
                    request: foreign.clone(),
                    state: SummaryGenerationState::Pending,
                })
                .unwrap(),
            );
        }
        hub.update(cx, |hub, cx| {
            assert!(
                hub.meeting_action_status("action-meeting")
                    .unwrap()
                    .foreign_pending
            );
            hub.cancel_foreign_generation("action-meeting".into(), cx);
        });
        cx.condition(&hub, |hub, _| {
            hub.meeting_action_status("action-meeting")
                .is_some_and(|status| !status.busy)
        })
        .await;
        hub.update(cx, |hub, cx| {
            let status = hub.meeting_action_status("action-meeting").unwrap();
            assert!(status.foreign_pending && !status.recovery);
            assert!(!hub.meeting_action_blocked("action-meeting"));
            hub.summarize("action-meeting".into(), true, false, cx);
        });
        let state = server.state.lock().unwrap();
        assert_eq!((state.posts, state.receipt_deletes), (0, 1));
        drop(state);
    }
}
