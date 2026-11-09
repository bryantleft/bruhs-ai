use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, MeetingReadSection};
use bruhs_models::{AgentAccess, AgentDomain, Meeting, MeetingSummary, TranscriptSegment};
use gpui::TestAppContext;
use serde_json::{json, Value};

fn request(section: MeetingReadSection, offset: usize) -> BridgeRequest {
    BridgeRequest::ReadMeeting {
        meeting_id: "meeting-fixture".into(),
        section,
        offset,
    }
}
fn listing(query: &str, offset: usize) -> BridgeRequest {
    BridgeRequest::Read {
        domain: AgentDomain::Meetings,
        query: query.into(),
        offset,
    }
}
fn meeting() -> Meeting {
    let mut meeting = Meeting::new("Planning 🗒", 1.0);
    meeting.id = "meeting-fixture".into();
    meeting.audio_path = Some("/private/user/audio/never-expose.wav".into());
    meeting.summary = Some(MeetingSummary {
        tldr: "private-summary".into(),
        action_items: vec![],
        decisions: vec!["Review the proposal".into()],
        topics: vec![],
        generated_by: "fixture".into(),
        generated_at: 1,
    });
    meeting
}
fn segment(index: usize) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("s{index}"),
        meeting_id: "meeting-fixture".into(),
        speaker_id: None,
        start_ms: u32::try_from(index).expect("fixture index fits in u32"),
        end_ms: u32::try_from(index).expect("fixture index fits in u32") + 1,
        text: format!("Saved words {index}"),
        confidence: 0.9,
        partial: index == 122,
        speaker_confidence: None,
    }
}
fn ok(response: BridgeResponse) -> Value {
    match response {
        BridgeResponse::Ok(value) => value,
        BridgeResponse::Error(message) => panic!("{message}"),
    }
}

struct MeetingFiles(std::path::PathBuf);
impl MeetingFiles {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "bruhs-native-meeting-cache-{}",
            uuid::Uuid::new_v4()
        )))
    }
    fn path(&self) -> std::path::PathBuf {
        self.0.join("state.json")
    }
}
impl Drop for MeetingFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[gpui::test]
async fn native_meeting_cache_events_save_real_file_and_restore_mcp_context_offline(
    cx: &mut TestAppContext,
) {
    let files = MeetingFiles::new();
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.persistence_test_path = Some(files.path());
        w.meetings_hub.update(cx, |hub, cx| {
            hub.install_meeting_test_snapshot(meeting(), vec![segment(0)], cx)
        });
    });
    cx.run_until_parked();
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = super::super::load_desktop_state_from_path(&files.path())
        .expect("read real file")
        .expect("saved state");
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.meetings.clear();
            hub.segments.clear();
            hub.restore_meetings(saved.meeting_cache);
        });
    });
    connect(&workspace, cx).await;
    let response = ok(call(&workspace, cx, request(MeetingReadSection::Transcript, 0)).await);
    assert_eq!(
        serde_json::from_value::<Vec<TranscriptSegment>>(response["records"].clone())
            .expect("typed transcript"),
        vec![segment(0)]
    );
    assert_eq!(response["source"], "workspace_cache");
    assert_eq!(
        ok(call(&workspace, cx, request(MeetingReadSection::Summary, 0)).await)["records"][0]
            ["summary"]["tldr"],
        "private-summary"
    );
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn native_meeting_cache_failed_save_remains_provisional_then_retry_keeps_same_context(
    cx: &mut TestAppContext,
) {
    let files = MeetingFiles::new();
    std::fs::write(&files.0, b"owned parent blocker").expect("create owned failure fixture");
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.persistence_test_path = Some(files.path());
        w.meetings_hub.update(cx, |hub, cx| {
            hub.install_meeting_test_snapshot(meeting(), vec![segment(0)], cx)
        });
    });
    cx.run_until_parked();
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, super::super::SaveStatus::Failed(_))
    })
    .await;
    assert!(!files.path().exists());
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(
            w.meetings_hub.read(cx).segments["meeting-fixture"],
            vec![segment(0)]
        );
    });
    std::fs::remove_file(&files.0).expect("remove only owned failure fixture");
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("desktop-retry-save")
        .expect("global save retry");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = super::super::load_desktop_state_from_path(&files.path())
        .expect("read retried save")
        .expect("saved state");
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.restore_meetings(saved.meeting_cache);
            assert_eq!(hub.meetings, vec![meeting()]);
            assert_eq!(hub.segments["meeting-fixture"], vec![segment(0)]);
        });
    });
}

#[gpui::test]
async fn meeting_reads_recheck_read_before_identity_and_never_infer_it_from_write(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub
            .update(cx, |hub, _| hub.meetings = vec![meeting()]);
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, false);
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Write, true);
    });
    connect(&workspace, cx).await;
    for req in [
        listing("", 0),
        request(MeetingReadSection::Summary, 0),
        request(MeetingReadSection::Transcript, 0),
    ] {
        assert!(matches!(
            call(&workspace, cx, req).await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, true)
    });
    assert_eq!(
        ok(call(&workspace, cx, request(MeetingReadSection::Summary, 0)).await)["records"][0]
            ["summary"]["tldr"],
        "private-summary"
    );
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, false)
    });
    for id in ["meeting-fixture", "missing"] {
        let BridgeResponse::Error(message) = call(
            &workspace,
            cx,
            BridgeRequest::ReadMeeting {
                meeting_id: id.into(),
                section: MeetingReadSection::Summary,
                offset: 0,
            },
        )
        .await
        else {
            panic!("read denied");
        };
        assert_eq!(message, "Read access is not granted for this domain.");
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn meeting_discovery_filters_identity_without_serializing_summaries_or_audio_paths(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, true);
        w.meetings_hub.update(cx, |hub, _| {
            let mut record = meeting();
            record.summary.as_mut().unwrap().tldr = "private-summary".repeat(100_000);
            hub.meetings = vec![record];
        });
    });
    connect(&workspace, cx).await;
    let page = ok(call(&workspace, cx, listing("PLANNING", 0)).await);
    assert_eq!(page["records"][0]["id"], "meeting-fixture");
    assert_eq!(page["source"], "workspace_cache");
    assert!(!page.to_string().contains("private-summary"));
    assert!(!page.to_string().contains("never-expose"));
    for query in ["private-summary", "never-expose"] {
        assert_eq!(
            ok(call(&workspace, cx, listing(query, 0)).await)["records"],
            json!([])
        );
    }
    assert!(matches!(
        call(&workspace, cx, request(MeetingReadSection::Summary, 0)).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub
            .update(cx, |hub, _| hub.meetings[0] = meeting())
    });
    let summary = ok(call(&workspace, cx, request(MeetingReadSection::Summary, 0)).await);
    assert!(!summary.to_string().contains("audio_path"));
    assert!(!summary.to_string().contains("never-expose"));
    assert_eq!(
        summary["records"][0]["summary"]["decisions"][0],
        "Review the proposal"
    );
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn transcript_pages_distinguish_unloaded_cache_and_preserve_partial_flags_without_mutation(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let original: Vec<_> = (0..123).map(segment).collect();
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, true);
        w.meetings_hub
            .update(cx, |hub, _| hub.meetings = vec![meeting()]);
    });
    connect(&workspace, cx).await;
    assert!(matches!(
        call(&workspace, cx, request(MeetingReadSection::Transcript, 0)).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.segments
                .insert("meeting-fixture".into(), original.clone());
        })
    });
    let mut received = vec![];
    for (offset, next) in [(0, json!(50)), (50, json!(100)), (100, Value::Null)] {
        let page = ok(call(
            &workspace,
            cx,
            request(MeetingReadSection::Transcript, offset),
        )
        .await);
        assert_eq!(page["next_offset"], next);
        received.extend(
            serde_json::from_value::<Vec<TranscriptSegment>>(page["records"].clone()).unwrap(),
        );
    }
    assert_eq!(received, original);
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(
            w.meetings_hub.read(cx).segments["meeting-fixture"],
            original
        );
        assert!(w.automation_state.workflows.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn meeting_context_rejects_foreign_duplicate_or_invalid_cached_transcripts(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, true);
        w.meetings_hub
            .update(cx, |hub, _| hub.meetings = vec![meeting()]);
    });
    connect(&workspace, cx).await;
    let mut foreign = segment(0);
    foreign.meeting_id = "another-meeting".into();
    let mut invalid = segment(0);
    invalid.confidence = f32::NAN;
    for records in [vec![foreign], vec![segment(0), segment(0)], vec![invalid]] {
        workspace.update_in(cx, |w, _, cx| {
            w.meetings_hub.update(cx, |hub, _| {
                hub.segments.insert("meeting-fixture".into(), records);
            })
        });
        assert!(matches!(
            call(&workspace, cx, request(MeetingReadSection::Transcript, 0)).await,
            BridgeResponse::Error(_)
        ));
    }
    for req in [
        request(MeetingReadSection::Summary, 1),
        request(MeetingReadSection::Transcript, 100_001),
        listing(&"q".repeat(513), 0),
    ] {
        assert!(matches!(
            call(&workspace, cx, req).await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
