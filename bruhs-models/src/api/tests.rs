//! Round-trips against JSON copied from the current producers: the server's
//! route types and `json!` stream encoder, the web's `api.rs` bodies and
//! IndexedDB rows, the desktop's `server_client.rs` bodies, and bruhs-core's
//! `Thread` / `BackgroundTask`.

use serde_json::{json, Value};

use super::*;

fn ms(v: i64) -> EpochMs {
    EpochMs::new(v).unwrap()
}

/// `value` parses as `T` and writes back as exactly `value`.
fn round_trips<T>(value: Value) -> T
where
    T: Serialize + for<'de> Deserialize<'de> + std::fmt::Debug + PartialEq,
{
    let parsed: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), value, "{parsed:?}");
    parsed
}

#[test]
fn create_meeting_body_reads_web_floats_and_writes_integers() {
    // Web/desktop `CreateMeetingBody` serialize `started_at_ms` as an f64.
    let body: CreateMeetingBody = serde_json::from_value(
        json!({"title":"Sync","started_at_ms":1700000000000.0,"calendar_event_id":"g-1"}),
    )
    .unwrap();
    assert_eq!(body.started_at_ms, ms(1_700_000_000_000));
    assert_eq!(body.calendar_event_id.as_deref(), Some("g-1"));
    round_trips::<CreateMeetingBody>(
        json!({"title":"Sync","started_at_ms":1700000000000_i64,"calendar_event_id":null}),
    );
    // The server's `CreateMeetingRequest` defaults the event id.
    let bare: CreateMeetingBody =
        serde_json::from_value(json!({"title":"Sync","started_at_ms":1})).unwrap();
    assert_eq!(bare.calendar_event_id, None);

    // What we write still parses as the server's current `f64` field.
    #[derive(Deserialize)]
    struct ServerCreateMeetingRequest {
        started_at_ms: f64,
    }
    let server: ServerCreateMeetingRequest =
        serde_json::from_value(serde_json::to_value(&body).unwrap()).unwrap();
    assert_eq!(server.started_at_ms, 1_700_000_000_000.0);
}

#[test]
fn stop_and_patch_meeting_bodies() {
    let stop: StopMeetingBody =
        serde_json::from_value(json!({"ended_at_ms":1700000000500.0})).unwrap();
    assert_eq!(stop.ended_at_ms, ms(1_700_000_000_500));
    round_trips::<StopMeetingBody>(json!({"ended_at_ms":1700000000500_i64}));

    // Web `PatchMeetingBody` writes every field, `null` when unchanged.
    let patch = round_trips::<PatchMeetingBody>(
        json!({"title":null,"status":"ended","ended_at_ms":1700000000500_i64}),
    );
    assert_eq!(patch.status, Some(MeetingStatus::Ended));
    // The server's `PatchMeetingRequest` defaults missing fields.
    assert_eq!(
        serde_json::from_value::<PatchMeetingBody>(json!({})).unwrap(),
        PatchMeetingBody::default()
    );
}

#[test]
fn summarize_request_matches_server_defaults() {
    // Web/desktop write all three fields.
    let body = round_trips::<SummarizeRequest>(
        json!({"request_id":"r-1","extract_observations":false,"replace_observations":true}),
    );
    assert_eq!(body.request_id.as_deref(), Some("r-1"));
    // The server's `SummarizeBody` defaults: observations on, no replace.
    assert_eq!(
        serde_json::from_value::<SummarizeRequest>(json!({})).unwrap(),
        SummarizeRequest::default()
    );
    assert!(SummarizeRequest::default().extract_observations);
    assert_eq!(
        serde_json::to_value(SummarizeRequest::default()).unwrap(),
        json!({"extract_observations":true,"replace_observations":false})
    );
    // ... and rejects unknown fields.
    assert!(serde_json::from_value::<SummarizeRequest>(json!({"replace":true})).is_err());
}

#[test]
fn cancel_summary_generation_body() {
    round_trips::<CancelSummaryGenerationBody>(
        json!({"extract_observations":true,"replace_observations":false}),
    );
    let defaults: CancelSummaryGenerationBody = serde_json::from_value(json!({})).unwrap();
    assert!(defaults.extract_observations && !defaults.replace_observations);
    // Strict like the server's body: a misspelled option is an error.
    assert!(
        serde_json::from_value::<CancelSummaryGenerationBody>(json!({"replace":true})).is_err()
    );
}

#[test]
fn start_oauth_response() {
    // Core `google_calendar::StartOAuthResponse`.
    round_trips::<StartOAuthResponse>(
        json!({"auth_url":"https://accounts.google.com/o/oauth2/v2/auth?state=abc","state":"abc"}),
    );
    // The desktop only ever read `auth_url`.
    let legacy: StartOAuthResponse =
        serde_json::from_value(json!({"auth_url":"https://example.test"})).unwrap();
    assert_eq!(legacy.state, "");
}

#[test]
fn capture_status_reads_and_writes_the_flat_server_shape() {
    // Server `WebCaptureStatus` when the daemon is down.
    let down = round_trips::<CaptureStatus>(json!({
        "running": false, "recording": false, "meeting_id": null, "started_at_ms": null,
        "system_audio_permission": "", "system_audio_capturing": false
    }));
    assert_eq!(down, CaptureStatus::NotRunning);
    assert!(!down.is_running());

    // Daemon up, idle.
    let idle = round_trips::<CaptureStatus>(json!({
        "running": true, "recording": false, "meeting_id": null, "started_at_ms": null,
        "system_audio_permission": "granted", "system_audio_capturing": true
    }));
    assert_eq!(
        idle,
        CaptureStatus::Running(RunningCapture {
            recording: None,
            system_audio_permission: SystemAudioPermission::Granted,
            system_audio_capturing: true,
        })
    );

    // Daemon recording (the daemon writes `started_at_ms` as an f64).
    let recording = round_trips::<CaptureStatus>(json!({
        "running": true, "recording": true, "meeting_id": "m-1",
        "started_at_ms": 1700000000000.0,
        "system_audio_permission": "not_determined", "system_audio_capturing": false
    }));
    let active = recording.recording().unwrap();
    assert_eq!(active.meeting_id.as_str(), "m-1");
    assert_eq!(active.started_at_ms, ms(1_700_000_000_000));

    // `running: false` wins over stale fields; missing fields default.
    assert_eq!(
        serde_json::from_value::<CaptureStatus>(json!({"running":false,"recording":true})).unwrap(),
        CaptureStatus::NotRunning
    );
    let sparse: CaptureStatus = serde_json::from_value(json!({"running":true})).unwrap();
    assert_eq!(
        sparse,
        CaptureStatus::Running(RunningCapture {
            recording: None,
            system_audio_permission: SystemAudioPermission::Unknown,
            system_audio_capturing: false,
        })
    );
    // An unknown permission is not an error.
    let future: CaptureStatus =
        serde_json::from_value(json!({"running":true,"system_audio_permission":"restricted"}))
            .unwrap();
    assert!(matches!(
        future,
        CaptureStatus::Running(RunningCapture {
            system_audio_permission: SystemAudioPermission::Unknown,
            ..
        })
    ));
}

#[test]
fn contradictory_capture_status_is_rejected() {
    for bad in [
        json!({"running":true,"recording":true,"started_at_ms":1.0}),
        json!({"running":true,"recording":true,"meeting_id":"m-1"}),
        json!({"running":true,"recording":true,"meeting_id":"../x","started_at_ms":1.0}),
        json!({"running":true,"recording":true,"meeting_id":"m","started_at_ms":1e300}),
    ] {
        assert!(
            serde_json::from_value::<CaptureStatus>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn capture_start_body() {
    // Web/desktop `CaptureStartBody` with every option set.
    let full = round_trips::<CaptureStartBody>(json!({
        "meeting_id": "m-1", "started_at_ms": 1700000000000_i64,
        "model": "base_en", "live_model": "tiny_en",
        "subspeaker_enabled": true, "cross_meeting_linking_auto": false,
        "cross_meeting_linking_threshold": 0.5, "max_speakers": 6,
        "video_ocr_enabled": false
    }));
    assert_eq!(full.max_speakers, Some(6));
    // Unset options are omitted, which the daemon's `StartRequest` defaults.
    round_trips::<CaptureStartBody>(json!({"meeting_id":"m-1","started_at_ms":5}));
    assert!(serde_json::from_value::<CaptureStartBody>(
        json!({"meeting_id":"../capture/stop","started_at_ms":5})
    )
    .is_err());
}

#[test]
fn thread_reads_server_and_web_shapes() {
    // bruhs-core `Thread::for_meeting`.
    let server = round_trips::<Thread>(json!({
        "id": "0b6f2c1e-8f3a-4c5e-9d2b-7a1f0e3c4d5b", "title": "Meeting: Standup",
        "project_path": null, "project_name": null,
        "created_at": 1700000000000_i64, "updated_at": 1700000000001_i64,
        "system_prompt": "Meeting context: ...", "meeting_id": "m-123"
    }));
    assert_eq!(server.updated_at, ms(1_700_000_000_001));
    assert_eq!(
        server.meeting_id.as_ref().map(MeetingId::as_str),
        Some("m-123")
    );

    // bruhs-core threads persisted before the meeting fields existed.
    let old: Thread =
        serde_json::from_value(json!({"id":"t-1","title":"x","created_at":0,"updated_at":0}))
            .unwrap();
    assert!(old.system_prompt.is_none() && old.meeting_id.is_none());

    // Web IndexedDB rows: string timestamps, legacy "now", "" = general.
    let legacy: Thread = serde_json::from_value(
        json!({"id":"x","title":"New thread","project_name":"","updated_at":"now"}),
    )
    .unwrap();
    assert_eq!(legacy.updated_at, EpochMs::UNIX_EPOCH);
    assert_eq!(legacy.created_at, EpochMs::UNIX_EPOCH);
    assert_eq!(legacy.project_name, None);
    let web: Thread = serde_json::from_value(json!({
        "id": "x", "title": "Fix parser", "project_name": "bruhs",
        "updated_at": "1700000000000", "project_path": "/p", "session_id": "s-1",
        "branch": "main", "model": "gpt-5.5", "pinned": true, "settled": false
    }))
    .unwrap();
    assert_eq!(web.updated_at, ms(1_700_000_000_000));
    assert_eq!(web.project_name.as_deref(), Some("bruhs"));
    assert!(web.pinned && !web.settled);
    assert_eq!(web.session_id.as_deref(), Some("s-1"));
    // Written back in the server's shape, unset extras omitted.
    assert_eq!(
        serde_json::to_value(&web).unwrap(),
        json!({
            "id": "x", "title": "Fix parser", "project_path": "/p", "project_name": "bruhs",
            "created_at": 0, "updated_at": 1700000000000_i64, "session_id": "s-1",
            "branch": "main", "model": "gpt-5.5", "pinned": true
        })
    );
}

fn task_json() -> Value {
    // bruhs-core `BackgroundTask` as `BackgroundTaskRegistry::register` builds it.
    json!({
        "id": "task-1", "thread_id": "t-1", "parent_task_id": null, "kind": "shell",
        "label": "npm run dev", "status": "running", "started_at": 1700000000000_i64,
        "ended_at": null, "exit_code": null,
        "meta": {"command": "npm run dev", "run_in_background": true}, "log_tail": ""
    })
}

#[test]
fn background_task_round_trips_and_tolerates_unknown_states() {
    let task = round_trips::<BackgroundTask>(task_json());
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.kind, TaskKind::Shell);
    round_trips::<BackgroundTask>(json!({
        "id": "task-2", "thread_id": "t-1", "parent_task_id": "task-1", "kind": "shell",
        "label": "x", "status": "exited", "started_at": 1, "ended_at": 2, "exit_code": 0,
        "meta": null, "log_tail": "done\n"
    }));
    // The web's `state::BackgroundTask` subset, and statuses from a newer server.
    let web: BackgroundTask = serde_json::from_value(json!({
        "id": "task-3", "thread_id": "t-1", "label": "x", "status": "paused",
        "started_at": 1, "ended_at": null, "exit_code": null
    }))
    .unwrap();
    assert_eq!(web.status, TaskStatus::Unknown);
    assert_eq!(
        (web.kind, &web.meta, web.log_tail.as_str()),
        (TaskKind::Shell, &Value::Null, "")
    );
    // `/api/tasks/stream` snapshots are a JSON array of tasks.
    let snapshot: Vec<BackgroundTask> = serde_json::from_value(json!([task_json()])).unwrap();
    assert_eq!(snapshot.len(), 1);
}

#[test]
fn agent_request_and_approve_bodies() {
    // Web `send_agent_message_streaming` body.
    let req = round_trips::<AgentRequest>(json!({
        "thread_id": "t-1", "prompt": "hi", "project_path": "/p",
        "model": "claude-sonnet-4-6", "permission_mode": "acceptEdits", "session_id": null
    }));
    assert_eq!(req.permission_mode, Some(PermissionMode::AcceptEdits));
    assert!(!req.fresh_session);
    assert!(req.text_attachments.is_empty());
    let with_paste: AgentRequest = serde_json::from_value(json!({
        "thread_id": "t-1",
        "prompt": "hi",
        "text_attachments": [{"name": "paste.txt", "text": "body"}]
    }))
    .unwrap();
    assert_eq!(with_paste.text_attachments[0].text, "body");
    assert!(serde_json::to_value(&req)
        .unwrap()
        .get("text_attachments")
        .is_none());
    assert!(
        serde_json::from_value::<AgentRequest>(json!({
            "thread_id": "t-1", "prompt": "hi", "fresh_session": true
        }))
        .unwrap()
        .fresh_session
    );
    // The server's `AgentRequest` defaults the optional fields.
    let bare: AgentRequest =
        serde_json::from_value(json!({"thread_id":"t-1","prompt":"hi","project_path":"/p"}))
            .unwrap();
    assert_eq!(
        (bare.model, bare.permission_mode, bare.session_id),
        (None, None, None)
    );
    assert!(serde_json::from_value::<AgentRequest>(json!({
        "thread_id":"t-1","prompt":"hi","project_path":"/p","permission_mode":"yolo"
    }))
    .is_err());
    // A chat without a project omits the path (the server runs it in $HOME).
    let general = round_trips::<AgentRequest>(json!({
        "thread_id": "t-1", "prompt": "hi", "model": null, "permission_mode": null,
        "session_id": null
    }));
    assert_eq!(general.project_path, None);
    assert_eq!(
        serde_json::from_value::<AgentRequest>(
            json!({"thread_id":"t-1","prompt":"hi","project_path":null})
        )
        .unwrap()
        .project_path,
        None
    );

    // Web `approve_permission` body; the server also accepts camelCase.
    round_trips::<ApproveRequest>(json!({"request_id":"r-1","approved":true,"reason":null}));
    let camel: ApproveRequest =
        serde_json::from_value(json!({"requestId":"r-2","approved":false})).unwrap();
    assert_eq!((camel.request_id.as_str(), camel.approved), ("r-2", false));
    round_trips::<ApproveResponse>(json!({"success":true}));
}

/// Every event `bruhs-server/src/routes/agent.rs` emits, as it emits it.
fn server_stream_events() -> Vec<Value> {
    let usage = json!({
        "input_tokens": 1, "output_tokens": 2,
        "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4
    });
    vec![
        json!({"envelope":"query","type":"init","session_id":"s-1","model":null,
               "tools":["Bash","Read"],"mcp_servers":[{"name":"bruhs","status":"connected"}]}),
        json!({"envelope":"query","type":"init","session_id":"s-2","model":"gpt-5.5",
               "tools":["Bash"],"mcp_servers":[]}),
        json!({"envelope":"query","type":"text","text":"Hel"}),
        json!({"envelope":"query","type":"thinking","thinking":"hm"}),
        json!({"envelope":"query","type":"tool_call","id":"t1","tool":"Bash",
               "input":{"command":"ls","run_in_background":false}}),
        json!({"envelope":"query","type":"tool_result","tool_use_id":"t1","content":"a\nb",
               "is_error":false}),
        {
            let mut v = usage.clone();
            v["envelope"] = json!("query");
            v["type"] = json!("usage");
            v
        },
        json!({"envelope":"query","type":"result","result":"ok","session_id":"s-1",
               "duration_ms":10,"usage":usage,"is_error":false}),
        json!({"envelope":"query","type":"permission_request","id":"p-1","tool":"Bash",
               "input":{"command":"rm -rf x"},"tool_use_id":"t2"}),
        json!({"envelope":"query","type":"permission_request","id":"p-2","tool":"Edit",
               "input":{},"tool_use_id":null}),
        json!({"envelope":"query","type":"permission_cancelled","id":"p-1"}),
        json!({"envelope":"query","type":"error","message":"boom"}),
        json!({"envelope":"query","type":"raw",
               "data":{"type":"error","message":"Reconnecting... 1/5"}}),
        json!({"envelope":"background_task_registered","task":task_json()}),
    ]
}

#[test]
fn every_server_stream_event_round_trips() {
    for value in server_stream_events() {
        let message = round_trips::<AgentStreamMessage>(value.clone());
        assert!(
            !matches!(message, AgentStreamMessage::Unknown(_)),
            "{value}"
        );
    }
}

#[test]
fn stream_events_decode_to_typed_variants() {
    let events: Vec<AgentStreamMessage> = server_stream_events()
        .into_iter()
        .map(|v| serde_json::from_value(v).unwrap())
        .collect();
    assert_eq!(
        events[0],
        AgentStreamMessage::Event(AgentStreamEvent::Init {
            session_id: "s-1".into(),
            model: None,
            tools: vec!["Bash".into(), "Read".into()],
            mcp_servers: vec![McpServerInfo {
                name: "bruhs".into(),
                status: "connected".into()
            }],
        })
    );
    assert_eq!(
        events[6],
        AgentStreamMessage::Event(AgentStreamEvent::Usage(UsageInfo {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_input_tokens: 3,
            cache_creation_input_tokens: 4,
            context_tokens: None,
        }))
    );
    assert!(matches!(
        &events[8],
        AgentStreamMessage::Event(AgentStreamEvent::PermissionRequest { tool_use_id: Some(t), .. }) if t == "t2"
    ));
    match &events[13] {
        AgentStreamMessage::BackgroundTaskRegistered(task) => {
            assert_eq!(task.label, "npm run dev");
        }
        other => panic!("expected a task, got {other:?}"),
    }
}

#[test]
fn stream_reader_is_tolerant() {
    // The web reducer's test events carry no envelope and omit fields.
    let init: AgentStreamMessage =
        serde_json::from_value(json!({"type":"init","session_id":"s1"})).unwrap();
    assert!(matches!(
        init,
        AgentStreamMessage::Event(AgentStreamEvent::Init { ref session_id, model: None, .. }) if session_id == "s1"
    ));
    let result: AgentStreamMessage = serde_json::from_value(
        json!({"type":"result","result":"Rate limited","session_id":"s","duration_ms":1,"is_error":true,"usage":{}}),
    )
    .unwrap();
    assert!(matches!(
        result,
        AgentStreamMessage::Event(AgentStreamEvent::Result { is_error: true, .. })
    ));
    // Older usage names.
    let usage: AgentStreamEvent = serde_json::from_value(
        json!({"type":"usage","cache_read_tokens":3,"cache_creation_tokens":4}),
    )
    .unwrap();
    assert_eq!(
        usage,
        AgentStreamEvent::Usage(UsageInfo {
            cache_read_input_tokens: 3,
            cache_creation_input_tokens: 4,
            ..UsageInfo::default()
        })
    );

    // Unknown types, envelopes and malformed events are kept verbatim.
    for value in [
        json!({"envelope":"query","type":"rate_limit","retry_in":3}),
        json!({"envelope":"future_envelope","x":1}),
        json!({"envelope":"background_task_registered","task":{"id":"no-fields"}}),
        json!({"envelope":"query","type":"text","text":5}),
        json!({"no_type":true}),
        json!("[DONE]"),
    ] {
        let message = round_trips::<AgentStreamMessage>(value.clone());
        assert_eq!(message, AgentStreamMessage::Unknown(value));
    }
    // The bare event enum maps an unknown type to its unit `Unknown`.
    assert_eq!(
        serde_json::from_value::<AgentStreamEvent>(json!({"type":"rate_limit"})).unwrap(),
        AgentStreamEvent::Unknown
    );
}

#[test]
fn usage_carries_the_window_size_when_the_server_knows_it() {
    let event: AgentStreamEvent = serde_json::from_value(serde_json::json!({
        "type": "usage", "input_tokens": 2, "output_tokens": 1,
        "cache_read_input_tokens": 47_261, "cache_creation_input_tokens": 1_928,
        "context_tokens": 49_191,
    }))
    .unwrap();
    let AgentStreamEvent::Usage(usage) = event else {
        panic!("usage event");
    };
    assert_eq!(usage.context_tokens, Some(49_191));
    let null: AgentStreamEvent = serde_json::from_value(serde_json::json!({
        "type": "usage", "input_tokens": 1, "context_tokens": null,
    }))
    .unwrap();
    assert!(matches!(
        null,
        AgentStreamEvent::Usage(UsageInfo {
            context_tokens: None,
            ..
        })
    ));
}

#[test]
fn a_second_opinion_reports_what_the_review_spent() {
    let reply: SecondOpinionResponse = serde_json::from_value(serde_json::json!({
        "reviewer": "codex", "review": "- bug at a.py:2",
        "usage": {"input_tokens": 6_772, "cache_read_input_tokens": 13_184, "output_tokens": 10},
    }))
    .unwrap();
    assert_eq!(reply.usage.map(|u| u.cache_read_input_tokens), Some(13_184));
    let old: SecondOpinionResponse =
        serde_json::from_value(serde_json::json!({"reviewer": "claude", "error": "x"})).unwrap();
    assert_eq!(old.usage, None);
}
