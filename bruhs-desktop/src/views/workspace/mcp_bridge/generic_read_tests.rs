//! Manual measurements of all five full-record read adapters.
use super::super::{
    DesktopCalendarEvent, DesktopPriority, DesktopState, DesktopWorkItem, DesktopWorkItemStatus,
    Workspace,
};
use super::tests::{call, connect, fixture};
use super::{ConnectionState, WorkspaceBridge};
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse};
use bruhs_models::{
    AgentAccess, AgentDomain, CaptureKind, CaptureRecord, DocumentVersion, EpochMs,
    InboxConversation, InboxMessage, InboxParticipant, InboxSource, KnowledgeDocument,
    WorkItemPlanning,
};
use chrono::{Duration, NaiveDate};
use gpui::TestAppContext;
use serde_json::Value;

const RECORDS: usize = 10_001;
const DOMAINS: [AgentDomain; 5] = [
    AgentDomain::Tasks,
    AgentDomain::Calendar,
    AgentDomain::Capture,
    AgentDomain::Knowledge,
    AgentDomain::Inbox,
];

fn document(index: usize, unicode: bool, records: usize) -> KnowledgeDocument {
    let now = i64::try_from(index).expect("fixture index fits timestamp");
    let body = if unicode {
        "ΟΣ Café İ K 👩🏽‍💻 ".repeat(100)
    } else {
        "Complete local document context. ".repeat(100)
    };
    let mut record = KnowledgeDocument::new(format!("Document {index}"), body, now);
    record.id = format!("record-{index}");
    record.versions = (0..2)
        .map(|version| DocumentVersion {
            id: format!("version-{index}-{version}"),
            title: format!("Earlier document {index}"),
            body_markdown: "Earlier complete context. ".repeat(25),
            tags: vec!["saved".into()],
            links: Vec::new(),
            citations: Vec::new(),
            recorded_at_ms: now,
        })
        .collect();
    if index == records - 1 {
        record.versions[0].body_markdown.push_str(" LastQuArtZ");
    }
    record
}

fn populate(workspace: &mut Workspace, records: usize) {
    let date = NaiveDate::from_ymd_opt(2026, 11, 10)
        .expect("fixture date")
        .and_hms_opt(9, 0, 0)
        .expect("fixture time");
    for index in 0..records {
        let now = i64::try_from(index).expect("fixture index fits timestamp");
        let id = format!("record-{index}");
        let mut body = "Complete local record context. ".repeat(50);
        if index == records - 1 {
            body.push_str(" LastQuArtZ");
        }
        workspace.work_items.push(DesktopWorkItem {
            id: id.clone(),
            project_id: None,
            title: format!("Task {index}"),
            description: Some(body.clone()),
            status: DesktopWorkItemStatus::Backlog,
            priority: DesktopPriority::None,
            due_date: None,
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::new(now).expect("fixture timestamp"),
        });
        workspace.calendar_events.push(DesktopCalendarEvent {
            id: id.clone(),
            title: format!("Event {index}"),
            start_at: date + Duration::minutes(now),
            end_at: date + Duration::minutes(now + 30),
            all_day: false,
            color: Some("pandan".into()),
            notes: Some(body.clone()),
        });
        let mut capture =
            CaptureRecord::new(CaptureKind::Note, &format!("Capture {index}\n{body}"), now)
                .expect("nonempty fixture capture");
        capture.id.clone_from(&id);
        workspace.captures.push(capture);
        workspace
            .knowledge_documents
            .push(document(index, false, records));
        let mut conversation = InboxConversation::new(
            InboxSource::Import,
            format!("Conversation {index}"),
            vec![InboxParticipant {
                id: format!("participant-{index}"),
                display_name: format!("Person {index}"),
                address: String::new(),
                person_id: None,
            }],
            now,
        );
        conversation.id = id;
        let mut message = InboxMessage::incoming("Person", body, now);
        message.id = format!("message-{index}");
        conversation.receive(message);
        workspace.inbox_conversations.push(conversation);
    }
    for domain in DOMAINS {
        workspace
            .automation_state
            .permissions
            .set(domain, AgentAccess::Read, true);
    }
}

fn request(domain: AgentDomain, query: &str, offset: usize) -> BridgeRequest {
    BridgeRequest::Read {
        domain,
        query: query.into(),
        offset,
    }
}

fn assert_page(page: &Value, offset: usize, count: usize, next: Option<usize>) {
    let records = page["records"].as_array().expect("record array");
    assert_eq!(records.len(), count);
    assert_eq!(page["next_offset"], serde_json::json!(next));
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record["id"], format!("record-{}", offset + index));
    }
}

#[gpui::test]
fn generic_reads_find_complete_bodies_and_document_history_in_each_adapter(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        populate(w, 3);
        for domain in DOMAINS {
            for (query, first, count) in [("", 0, 3), ("lastquartz", 2, 1), ("absent-quartz", 0, 0)]
            {
                let BridgeResponse::Ok(page) =
                    w.handle_bridge_request(request(domain, query, 0), cx)
                else {
                    panic!("Expected native records")
                };
                assert_page(&page, first, count, None);
            }
        }
        w.knowledge_documents = (0..3).map(|index| document(index, true, 3)).collect();
        for (query, first, count) in [("lastquartz", 2, 1), ("CAFÉ", 0, 3), ("ς", 0, 3)] {
            let BridgeResponse::Ok(page) =
                w.handle_bridge_request(request(AgentDomain::Knowledge, query, 0), cx)
            else {
                panic!("Expected Unicode records")
            };
            assert_page(&page, first, count, None);
        }
    });
}

#[gpui::test]
#[ignore = "manual optimized handler/full-stdio timing; set BRUHS_PERF_MCP_BIN and BRUHS_PERF_GENERIC_OUTPUT, then run generic_native_read_action_cost --ignored --nocapture"]
async fn generic_native_read_action_cost(cx: &mut TestAppContext) {
    let binary = std::env::var_os("BRUHS_PERF_MCP_BIN").expect("optimized MCP executable");
    let output = std::env::var_os("BRUHS_PERF_GENERIC_OUTPUT").expect("output prefix");
    let output = std::path::PathBuf::from(output);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let harness = root.join("scripts/performance-mcp-record-reads.py");
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| populate(w, RECORDS));
    connect(&workspace, cx).await;
    for shape in ["ascii", "unicode-knowledge"] {
        let domains: &[AgentDomain] = if shape == "ascii" {
            &DOMAINS
        } else {
            workspace.update_in(cx, |w, _, _| {
                w.knowledge_documents = (0..RECORDS)
                    .map(|index| document(index, true, RECORDS))
                    .collect();
            });
            &[AgentDomain::Knowledge]
        };
        workspace.update_in(cx, |w, _, cx| {
            for &domain in domains {
                for (label, query, offset, first, count, next) in [
                    ("first", "", 0, 0, 50, Some(50)),
                    ("deep", "", 9_950, 9_950, 50, Some(10_000)),
                    ("last", "lastquartz", 0, 10_000, 1, None),
                    ("none", "no-quartz-match", 0, 0, 0, None),
                ] {
                    let mut samples = Vec::new();
                    for iteration in 0..35 {
                        let request = request(domain, query, offset);
                        let started = std::time::Instant::now();
                        let response = w.handle_bridge_request(request, cx);
                        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                        let BridgeResponse::Ok(page) = response else {
                            panic!("Expected records, got {response:?}")
                        };
                        assert_page(&page, first, count, next);
                        if iteration >= 5 {
                            samples.push(elapsed);
                        }
                    }
                    eprintln!(
                        "GENERIC_HANDLER_COST {}",
                        serde_json::json!({"shape":shape,"domain":domain.key(),"case":label,"records":RECORDS,"samples_ms":samples})
                    );
                }
            }
        });
        let receiver = workspace.update_in(cx, |w, _, cx| {
            let ConnectionState::Connected { session, .. } = &w.mcp_bridge.state else {
                panic!("bridge connected")
            };
            let connection = session.connection_path().to_owned();
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let binary = binary.clone();
            let harness = harness.clone();
            let output = output.with_extension(format!("{shape}.json"));
            cx.global::<DesktopState>().runtime.spawn(async move {
                let result = tokio::process::Command::new("python3")
                    .arg(harness)
                    .arg("--binary")
                    .arg(binary)
                    .arg("--output")
                    .arg(output)
                    .arg("--connection")
                    .arg(connection)
                    .arg("--shape")
                    .arg(shape)
                    .kill_on_drop(true)
                    .output()
                    .await;
                let _ = sender.send(result);
            });
            receiver
        });
        let result = receiver.await.unwrap().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&result.stdout));
    }
    workspace.update_in(cx, |w, _, _| {
        for domain in DOMAINS {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Read, false);
        }
        assert_eq!(w.work_items.len(), RECORDS);
        assert_eq!(w.calendar_events.len(), RECORDS);
        assert_eq!(w.captures.len(), RECORDS);
        assert_eq!(w.knowledge_documents.len(), RECORDS);
        assert_eq!(w.inbox_conversations.len(), RECORDS);
        assert!(w.automation_state.workflows.is_empty());
    });
    for domain in DOMAINS {
        assert!(matches!(
            call(&workspace, cx, request(domain, "", 0)).await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
