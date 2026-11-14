//! Isolated exact-ID context and completed-history action measurements.
use super::super::{DesktopState, Workspace};
use super::tests::{call, connect, fixture};
use super::{ConnectionState, WorkspaceBridge};
use bruhs_core::workspace_bridge::{
    BridgeRequest, BridgeResponse, MeetingReadSection, PersonReadSection,
};
use bruhs_models::{AgentAccess, AgentDomain};
use gpui::TestAppContext;
use serde_json::{json, Value};

mod fixtures;
use fixtures::populate;

const HISTORY: usize = 100_000;
const IDENTITIES: usize = 100_001;
const CHAT_IDENTITIES: usize = 10_001;
const CHAT_ID: &str = "context-chat";
const MEETING_ID: &str = "context-meeting";
const NOTE_ID: &str = "context-note";
const PERSON_ID: &str = "context-person";

#[derive(Clone, Copy)]
enum Target {
    Chat,
    Summary,
    Transcript,
    Note,
    Profile,
    Observations,
    Items,
}

impl Target {
    fn paged(self) -> bool {
        matches!(
            self,
            Self::Chat | Self::Transcript | Self::Observations | Self::Items
        )
    }
    fn descriptor(self, records: usize, identities: usize) -> Value {
        let (identity, tool, args) = match self {
            Self::Chat => (
                "chat/messages",
                "bruhs_read_chat_messages",
                json!({"thread_id":CHAT_ID}),
            ),
            Self::Summary => (
                "meeting/summary",
                "bruhs_read_meeting",
                json!({"meeting_id":MEETING_ID,"section":"summary"}),
            ),
            Self::Transcript => (
                "meeting/transcript",
                "bruhs_read_meeting",
                json!({"meeting_id":MEETING_ID,"section":"transcript"}),
            ),
            Self::Note => (
                "meeting-note/context",
                "bruhs_read_meeting_note",
                json!({"note_id":NOTE_ID}),
            ),
            Self::Profile => (
                "person/profile",
                "bruhs_read_person",
                json!({"person_id":PERSON_ID,"section":"profile"}),
            ),
            Self::Observations => (
                "person/observations",
                "bruhs_read_person",
                json!({"person_id":PERSON_ID,"section":"observations"}),
            ),
            Self::Items => (
                "person/items",
                "bruhs_read_person",
                json!({"person_id":PERSON_ID,"section":"relationship_items"}),
            ),
        };
        json!({"identity":identity,"tool":tool,"args":args,"records":records,"identities":identities,"paged":self.paged()})
    }
    fn request(self, missing: bool, offset: usize) -> BridgeRequest {
        let id = |known: &str| {
            if missing {
                "context-missing".into()
            } else {
                known.to_owned()
            }
        };
        match self {
            Self::Chat => BridgeRequest::ReadChatMessages {
                thread_id: id(CHAT_ID),
                offset,
            },
            Self::Summary | Self::Transcript => BridgeRequest::ReadMeeting {
                meeting_id: id(MEETING_ID),
                section: if matches!(self, Self::Summary) {
                    MeetingReadSection::Summary
                } else {
                    MeetingReadSection::Transcript
                },
                offset,
            },
            Self::Note => BridgeRequest::ReadMeetingNote {
                note_id: id(NOTE_ID),
            },
            Self::Profile | Self::Observations | Self::Items => BridgeRequest::ReadPerson {
                person_id: id(PERSON_ID),
                section: match self {
                    Self::Profile => PersonReadSection::Profile,
                    Self::Observations => PersonReadSection::Observations,
                    _ => PersonReadSection::RelationshipItems,
                },
                offset,
            },
        }
    }
}

fn targets(group: &str) -> Vec<Target> {
    match group {
        "chat" => vec![Target::Chat],
        "meetings" => vec![Target::Summary, Target::Transcript, Target::Note],
        "people" => vec![Target::Profile, Target::Observations, Target::Items],
        _ => unreachable!(),
    }
}

fn cases(target: Target, records: usize) -> Vec<Value> {
    let mut cases = if target.paged() {
        vec![
            json!({"name":"first","offset":0,"first":0,"count":50,"next":50}),
            json!({"name":"deep","offset":records-51,"first":records-51,"count":50,"next":records-1}),
            json!({"name":"last","offset":records-1,"first":records-1,"count":1,"next":null}),
            json!({"name":"exhausted","offset":records,"first":0,"count":0,"next":null}),
            json!({"name":"invalid-offset","offset":100001,"error":true}),
        ]
    } else {
        vec![json!({"name":"context","offset":0,"count":1,"next":null})]
    };
    cases.push(json!({"name":"missing","offset":0,"missing":true,"error":true}));
    cases
}

fn number(value: &Value, key: &str) -> usize {
    usize::try_from(value[key].as_u64().expect("fixture index")).expect("index fits usize")
}

fn check(response: BridgeResponse, target: Target, case: &Value, records: usize) {
    if case["error"] == true {
        assert!(matches!(response, BridgeResponse::Error(_)), "{response:?}");
        return;
    }
    let BridgeResponse::Ok(page) = response else {
        panic!("Expected context: {response:?}")
    };
    let rows = page["records"].as_array().expect("records");
    assert_eq!(rows.len(), number(case, "count"));
    assert_eq!(page["next_offset"], case["next"]);
    for (n, row) in rows.iter().enumerate() {
        let index = if target.paged() {
            number(case, "first") + n
        } else {
            0
        };
        match target {
            Target::Chat => {
                assert!(row["content"]
                    .as_str()
                    .expect("completed content")
                    .starts_with(&format!("Completed {index}:")));
                assert!(row.get("files").is_none() && row.get("checkpoint").is_none());
                if index == records - 1 {
                    assert_eq!(row["thinking"], "Stored synthetic reasoning");
                    assert_eq!(row["error"], "Stored synthetic interruption");
                    assert_eq!(row["tool_calls"][0]["id"], "stored-tool");
                }
            }
            Target::Transcript => {
                assert_eq!(row["id"], format!("segment-{index}"));
                assert_eq!(row["meeting_id"], MEETING_ID);
                assert_eq!(row["partial"], index == records - 1);
            }
            Target::Observations => assert!(row["text"]
                .as_str()
                .expect("observation text")
                .starts_with(&format!("Observation {index}:"))),
            Target::Items => {
                assert_eq!(row["id"], format!("item-{index}"));
                assert!(row["text"]
                    .as_str()
                    .expect("relationship text")
                    .starts_with(&format!("Relationship {index}:")));
            }
            Target::Summary => {
                assert_eq!(row["meeting"]["id"], MEETING_ID);
                assert_eq!(row["summary"]["tldr"], fixtures::body("Summary", 0));
                assert!(row["meeting"].get("audio_path").is_none());
            }
            Target::Note => {
                assert_eq!(row["id"], NOTE_ID);
                assert_eq!(row["body"], fixtures::body("Note", 0));
            }
            Target::Profile => {
                assert_eq!(row["id"], PERSON_ID);
                assert_eq!(row["observation_count"], records);
                assert_eq!(row["relationship"]["item_count"], records);
                assert!(row.get("voice_embedding").is_none());
            }
        }
    }
    assert!(!page.to_string().contains("unshared-secret"));
}

#[gpui::test]
fn context_fixture_preserves_rich_pages_offsets_and_private_exclusions(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, window, cx| {
        for group in ["meetings", "people", "chat"] {
            populate(w, group, 123, 123, window, cx);
            for target in targets(group) {
                for case in cases(target, 123) {
                    check(
                        w.handle_bridge_request(
                            target.request(case["missing"] == true, number(&case, "offset")),
                            cx,
                        ),
                        target,
                        &case,
                        123,
                    );
                }
            }
        }
        assert!(w.automation_state.workflows.is_empty() && w.automation_state.jobs.is_empty());
    });
}

#[gpui::test]
#[ignore = "manual optimized context handler/full-stdio timing; set BRUHS_PERF_MCP_BIN and BRUHS_PERF_CONTEXT_OUTPUT"]
async fn context_native_read_action_cost(cx: &mut TestAppContext) {
    let binary = std::env::var_os("BRUHS_PERF_MCP_BIN").expect("optimized MCP executable");
    let output = std::path::PathBuf::from(
        std::env::var_os("BRUHS_PERF_CONTEXT_OUTPUT").expect("output prefix"),
    );
    let harness = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/performance-mcp-context-reads.py");
    let smoke = std::env::var_os("BRUHS_PERF_CONTEXT_SMOKE").is_some();
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let records = if smoke { 123 } else { HISTORY };
    let (workspace, cx) = fixture(cx);
    connect(&workspace, cx).await;
    for group in ["meetings", "people", "chat"] {
        let identities = if smoke {
            123
        } else if group == "chat" {
            CHAT_IDENTITIES
        } else {
            IDENTITIES
        };
        workspace.update_in(cx, |w, window, cx| {
            populate(w, group, records, identities, window, cx)
        });
        for target in targets(group) {
            let descriptor = target.descriptor(records, identities);
            let cases = cases(target, records);
            workspace.update_in(cx, |w, _, cx| {
                for case in &cases {
                    let mut times = Vec::new();
                    for iteration in 0..warmups + samples {
                        let request =
                            target.request(case["missing"] == true, number(case, "offset"));
                        let start = std::time::Instant::now();
                        let response = w.handle_bridge_request(request, cx);
                        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                        check(response, target, case, records);
                        if iteration >= warmups {
                            times.push(elapsed);
                        }
                    }
                    eprintln!(
                        "CONTEXT_HANDLER_COST {}",
                        json!({"target":descriptor,"case":case["name"],"samples_ms":times})
                    );
                }
            });
            let receiver = workspace.update_in(cx, |w, _, cx| {
                let ConnectionState::Connected { session, .. } = &w.mcp_bridge.state else {
                    panic!("connected")
                };
                let connection = session.connection_path().to_owned();
                let (sender, receiver) = tokio::sync::oneshot::channel();
                let (binary, harness) = (binary.clone(), harness.clone());
                let output = output.with_extension(format!(
                    "{}.json",
                    descriptor["identity"].as_str().unwrap().replace('/', "-")
                ));
                let descriptor = descriptor.to_string();
                let cases = serde_json::to_string(&cases).unwrap();
                cx.global::<DesktopState>().runtime.spawn(async move {
                    let result = tokio::process::Command::new("python3")
                        .arg(harness)
                        .arg("--binary")
                        .arg(binary)
                        .arg("--output")
                        .arg(output)
                        .arg("--connection")
                        .arg(connection)
                        .arg("--target")
                        .arg(descriptor)
                        .arg("--cases")
                        .arg(cases)
                        .arg("--warmups")
                        .arg(warmups.to_string())
                        .arg("--samples")
                        .arg(samples.to_string())
                        .kill_on_drop(true)
                        .output()
                        .await;
                    let _ = sender.send(result);
                });
                receiver
            });
            let result = receiver
                .await
                .expect("harness channel")
                .expect("harness process");
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            eprintln!("{}", String::from_utf8_lossy(&result.stdout));
        }
    }
    workspace.update_in(cx, |w, _, _| {
        for domain in [
            AgentDomain::Meetings,
            AgentDomain::People,
            AgentDomain::Chat,
        ] {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Read, false);
        }
        assert!(w.automation_state.workflows.is_empty() && w.automation_state.jobs.is_empty());
    });
    for group in ["meetings", "people", "chat"] {
        for target in targets(group) {
            assert!(matches!(
                call(&workspace, cx, target.request(false, 0)).await,
                BridgeResponse::Error(_)
            ));
        }
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
fn warm_transcript_reads_revalidate_mutations_and_current_grants(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, window, cx| {
        populate(w, "meetings", 123, 123, window, cx);
        let first = cases(Target::Transcript, 123).remove(0);
        for _ in 0..2 {
            check(
                w.handle_bridge_request(Target::Transcript.request(false, 0), cx),
                Target::Transcript,
                &first,
                123,
            );
        }
        w.meetings_hub.update(cx, |hub, _| {
            hub.segments.get_mut(MEETING_ID).expect("loaded transcript")[122].id =
                "segment-0".into();
        });
        assert!(matches!(
            w.handle_bridge_request(Target::Transcript.request(false, 0), cx),
            BridgeResponse::Error(_)
        ));
        w.meetings_hub.update(cx, |hub, _| {
            hub.segments.entry(MEETING_ID.into()).or_default()[122].id = "segment-122".into();
        });
        check(
            w.handle_bridge_request(Target::Transcript.request(false, 0), cx),
            Target::Transcript,
            &first,
            123,
        );
        w.automation_state
            .permissions
            .set(AgentDomain::Meetings, AgentAccess::Read, false);
        for missing in [false, true] {
            let BridgeResponse::Error(message) =
                w.handle_bridge_request(Target::Transcript.request(missing, 0), cx)
            else {
                panic!("revocation must deny cached reads")
            };
            assert_eq!(message, "Read access is not granted for this domain.");
        }
    });
}
