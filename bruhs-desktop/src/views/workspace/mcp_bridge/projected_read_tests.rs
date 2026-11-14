//! Real handler and stdio measurements for projected discovery/read sections.
use super::super::{DesktopState, Workspace};
use super::tests::{call, connect, fixture};
use super::{ConnectionState, WorkspaceBridge};
use bruhs_core::workspace_bridge::{
    BridgeRequest, BridgeResponse, FinanceReadSection, HealthReadSection,
};
use bruhs_models::*;
use gpui::TestAppContext;
use serde_json::{json, Value};

mod fixtures;
use fixtures::populate;

const RECORDS: usize = 100_001;
const CHAT_RECORDS: usize = 10_001;
const HIDDEN: &str = "hidden-perf-body";
const DOMAINS: [AgentDomain; 5] = [
    AgentDomain::Finance,
    AgentDomain::Health,
    AgentDomain::Meetings,
    AgentDomain::People,
    AgentDomain::Chat,
];

#[derive(Clone, Copy)]
enum Target {
    Finance(FinanceReadSection, bool),
    Health(HealthReadSection),
    Meetings,
    Notes,
    People,
    Chat,
}

impl Target {
    fn descriptor(self, records: usize) -> Value {
        let (identity, tool, args) = match self {
            Self::Finance(section, exact) => {
                let section = json!(section);
                (
                    format!(
                        "finance/{}/{}",
                        section.as_str().expect("serialized section string"),
                        if exact { "exact" } else { "redacted" }
                    ),
                    "bruhs_read_finance",
                    json!({"section":section}),
                )
            }
            Self::Health(section) => {
                let section = json!(section);
                (
                    format!(
                        "health/{}",
                        section.as_str().expect("serialized section string")
                    ),
                    "bruhs_read_health",
                    json!({"section":section}),
                )
            }
            Self::Meetings => ("meetings".into(), "bruhs_read_meetings", json!({})),
            Self::Notes => (
                "meeting-notes".into(),
                "bruhs_read_meeting_notes",
                json!({}),
            ),
            Self::People => ("people".into(), "bruhs_read_people", json!({})),
            Self::Chat => ("chat".into(), "bruhs_read_chat", json!({})),
        };
        json!({"identity":identity,"tool":tool,"args":args,"records":records})
    }

    fn request(self, query: &str, offset: usize) -> BridgeRequest {
        match self {
            Self::Finance(section, _) => BridgeRequest::ReadFinance {
                section,
                query: query.into(),
                offset,
            },
            Self::Health(section) => BridgeRequest::ReadHealth {
                section,
                query: query.into(),
                offset,
            },
            Self::Notes => BridgeRequest::ReadMeetingNotes {
                query: query.into(),
                offset,
            },
            _ => BridgeRequest::Read {
                domain: match self {
                    Self::Meetings => AgentDomain::Meetings,
                    Self::People => AgentDomain::People,
                    Self::Chat => AgentDomain::Chat,
                    _ => unreachable!(),
                },
                query: query.into(),
                offset,
            },
        }
    }

    fn prepare(self, workspace: &mut Workspace) {
        if let Self::Finance(_, exact) = self {
            workspace
                .life_context
                .finance
                .privacy
                .reveal_exact_amounts_to_agent = exact;
        }
    }
}

fn targets(group: &str) -> Vec<Target> {
    match group {
        "life" => [false, true]
            .into_iter()
            .flat_map(|exact| {
                [
                    FinanceReadSection::Accounts,
                    FinanceReadSection::Transactions,
                    FinanceReadSection::Budgets,
                    FinanceReadSection::IncomeStreams,
                    FinanceReadSection::Holdings,
                ]
                .map(|section| Target::Finance(section, exact))
            })
            .chain(
                [
                    HealthReadSection::Workouts,
                    HealthReadSection::Sleep,
                    HealthReadSection::Recovery,
                    HealthReadSection::Habits,
                ]
                .map(Target::Health),
            )
            .collect(),
        "summaries" => vec![Target::Meetings, Target::Notes, Target::People],
        "chat" => vec![Target::Chat],
        _ => unreachable!(),
    }
}

fn record_id(index: usize, records: usize) -> String {
    let mut id = format!("record-{index}");
    if index.is_multiple_of(2) {
        id.push_str("-evenslot");
    }
    if index == records - 1 {
        id.push_str("-lastquartz");
    }
    id
}

struct Case {
    label: &'static str,
    query: &'static str,
    offset: usize,
    first: usize,
    count: usize,
    step: usize,
    next: Option<usize>,
}

fn cases(records: usize) -> [Case; 7] {
    [
        Case {
            label: "first",
            query: "",
            offset: 0,
            first: 0,
            count: 50,
            step: 1,
            next: Some(50),
        },
        Case {
            label: "deep",
            query: "",
            offset: records - 51,
            first: records - 51,
            count: 50,
            step: 1,
            next: Some(records - 1),
        },
        Case {
            label: "last",
            query: "LASTQUARTZ",
            offset: 0,
            first: records - 1,
            count: 1,
            step: 1,
            next: None,
        },
        Case {
            label: "none",
            query: "no-quartz-match",
            offset: 0,
            first: 0,
            count: 0,
            step: 1,
            next: None,
        },
        Case {
            label: "filtered-deep",
            query: "EVENSLOT",
            offset: records / 2 - 50,
            first: records - 101,
            count: 50,
            step: 2,
            next: Some(records / 2),
        },
        Case {
            label: "filtered-final",
            query: "EVENSLOT",
            offset: records / 2,
            first: records - 1,
            count: 1,
            step: 1,
            next: None,
        },
        Case {
            label: "hidden-miss",
            query: HIDDEN,
            offset: 0,
            first: 0,
            count: 0,
            step: 1,
            next: None,
        },
    ]
}

fn assert_page(response: BridgeResponse, target: Target, case: &Case, records: usize) {
    let BridgeResponse::Ok(page) = response else {
        panic!("Expected projected page: {response:?}")
    };
    let values = page["records"].as_array().expect("record array");
    assert_eq!(values.len(), case.count, "{}", case.label);
    assert_eq!(page["next_offset"], json!(case.next), "{}", case.label);
    for (index, value) in values.iter().enumerate() {
        assert_eq!(
            value["id"],
            record_id(case.first + index * case.step, records)
        );
    }
    if let Target::Finance(section, exact) = target {
        assert_eq!(page["amounts_redacted"], !exact);
        assert_eq!(page["notes_excluded"], true);
        let amount = match section {
            FinanceReadSection::Accounts => "balance_minor",
            FinanceReadSection::Transactions => "amount_minor",
            FinanceReadSection::Budgets => "monthly_limit_minor",
            FinanceReadSection::IncomeStreams => "expected_monthly_minor",
            FinanceReadSection::Holdings => "quantity",
        };
        assert!(values
            .iter()
            .all(|record| record.get(amount).is_some() == exact));
        assert!(!page.to_string().contains(HIDDEN));
    }
    if matches!(
        target,
        Target::Meetings | Target::Notes | Target::People | Target::Chat
    ) {
        assert!(!page.to_string().contains(HIDDEN));
    }
}

#[gpui::test]
fn projected_read_fixture_keeps_pages_metadata_and_selective_offsets(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, window, cx| {
        for group in ["life", "summaries", "chat"] {
            populate(w, group, 123, window, cx);
            for target in targets(group) {
                target.prepare(w);
                for case in cases(123) {
                    assert_page(
                        w.handle_bridge_request(target.request(case.query, case.offset), cx),
                        target,
                        &case,
                        123,
                    );
                }
            }
        }
        assert!(w.automation_state.workflows.is_empty());
    });
}

#[gpui::test]
#[ignore = "manual optimized projected read timing; set BRUHS_PERF_MCP_BIN and BRUHS_PERF_PROJECTED_OUTPUT"]
async fn projected_native_read_action_cost(cx: &mut TestAppContext) {
    let binary = std::env::var_os("BRUHS_PERF_MCP_BIN").expect("optimized MCP executable");
    let output = std::path::PathBuf::from(
        std::env::var_os("BRUHS_PERF_PROJECTED_OUTPUT").expect("output prefix"),
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let harness = root.join("scripts/performance-mcp-projected-reads.py");
    let smoke = std::env::var_os("BRUHS_PERF_PROJECTED_SMOKE").is_some();
    let (warmups, sample_count) = if smoke { (1, 2) } else { (5, 30) };
    let (workspace, cx) = fixture(cx);
    connect(&workspace, cx).await;
    for group in ["life", "summaries", "chat"] {
        let records = if smoke {
            123
        } else if group == "chat" {
            CHAT_RECORDS
        } else {
            RECORDS
        };
        workspace.update_in(cx, |w, window, cx| populate(w, group, records, window, cx));
        for target in targets(group) {
            let descriptor = target.descriptor(records);
            workspace.update_in(cx, |w, _, cx| {
                target.prepare(w);
                for case in cases(records) {
                    let mut samples = Vec::new();
                    for iteration in 0..warmups + sample_count {
                        let request = target.request(case.query, case.offset);
                        let start = std::time::Instant::now();
                        let response = w.handle_bridge_request(request, cx);
                        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                        assert_page(response, target, &case, records);
                        if iteration >= warmups {
                            samples.push(elapsed);
                        }
                    }
                    eprintln!(
                        "PROJECTED_HANDLER_COST {}",
                        json!({"target":descriptor,"case":case.label,"samples_ms":samples})
                    );
                }
            });
            let receiver = workspace.update_in(cx, |w, _, cx| {
                let ConnectionState::Connected { session, .. } = &w.mcp_bridge.state else {
                    panic!("bridge connected")
                };
                let connection = session.connection_path().to_owned();
                let (sender, receiver) = tokio::sync::oneshot::channel();
                let (binary, harness) = (binary.clone(), harness.clone());
                let output = output.with_extension(format!(
                    "{}.json",
                    descriptor["identity"].as_str().unwrap().replace('/', "-")
                ));
                let descriptor = descriptor.to_string();
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
                        .arg("--warmups")
                        .arg(warmups.to_string())
                        .arg("--samples")
                        .arg(sample_count.to_string())
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
        for domain in DOMAINS {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Read, false);
        }
        assert_eq!(w.threads.len(), if smoke { 123 } else { CHAT_RECORDS });
        assert!(w.automation_state.workflows.is_empty());
    });
    for group in ["life", "summaries", "chat"] {
        for target in targets(group) {
            assert!(matches!(
                call(&workspace, cx, target.request("", 0)).await,
                BridgeResponse::Error(_)
            ));
        }
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
