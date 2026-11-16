//! Session-owned workflow tool measurements; never approves or executes a proposal.
use super::super::{AppView, AutomationState, DesktopState, SaveStatus, Workspace};
use super::tests::{connect, fixture};
use super::{ConnectionState, RollbackReview, WorkspaceBridge};
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::{
    AgentAccess, AgentDomain, CaptureKind, CaptureRecord, Workflow, WorkflowOperation,
    WorkflowStatus,
};
use gpui::{Entity, TestAppContext, VisualTestContext};
use serde_json::{json, Value};
use std::{path::PathBuf, process::Stdio};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const TITLE: &str = "Synthetic Café ΟΣ workflow";
const CONTENT: &str = "private-capture-sentinel: synthetic rollback record";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Preview,
    PreviewBlocked,
    Submit,
    Retry,
    Conflict,
    Status,
    StatusRevoked,
    StatusUnknown,
    StatusUnsaved,
    RollbackPreview,
    RollbackRequest,
    RollbackPending,
    RollbackDeclined,
    RollbackRevoked,
    RollbackUnsaved,
    RollbackChanged,
}

impl Case {
    fn all() -> [Self; 16] {
        [
            Self::Preview,
            Self::PreviewBlocked,
            Self::Submit,
            Self::Retry,
            Self::Conflict,
            Self::Status,
            Self::StatusRevoked,
            Self::StatusUnknown,
            Self::StatusUnsaved,
            Self::RollbackPreview,
            Self::RollbackRequest,
            Self::RollbackPending,
            Self::RollbackDeclined,
            Self::RollbackRevoked,
            Self::RollbackUnsaved,
            Self::RollbackChanged,
        ]
    }
    fn name(self) -> String {
        format!("{self:?}")
    }
    fn proposal(self) -> WorkflowProposal {
        if matches!(self, Self::Retry | Self::Conflict) {
            WorkflowProposal {
                title: if self == Self::Conflict {
                    "Changed title"
                } else {
                    TITLE
                }
                .into(),
                operations: vec![operation()],
            }
        } else {
            WorkflowProposal {
                title: TITLE.into(),
                operations: vec![
                    operation(),
                    WorkflowOperation::CreateTask {
                        title: "Synthetic task; never executed".into(),
                    },
                ],
            }
        }
    }
    fn request(self, id: &str) -> BridgeRequest {
        match self {
            Self::Preview | Self::PreviewBlocked => BridgeRequest::PreviewWorkflow {
                proposal: self.proposal(),
            },
            Self::Submit | Self::Retry | Self::Conflict => BridgeRequest::ProposeWorkflow {
                request_id: if self == Self::Submit {
                    "fresh-request"
                } else {
                    "target-request"
                }
                .into(),
                proposal: self.proposal(),
            },
            Self::Status | Self::StatusRevoked | Self::StatusUnsaved | Self::StatusUnknown => {
                BridgeRequest::Status {
                    workflow_id: if self == Self::StatusUnknown {
                        "not-session-owned"
                    } else {
                        id
                    }
                    .into(),
                }
            }
            _ => BridgeRequest::RequestRollback {
                workflow_id: id.into(),
                dry_run: !matches!(self, Self::RollbackRequest | Self::RollbackPending),
            },
        }
    }
    fn tool(self) -> &'static str {
        match self {
            Self::Preview | Self::PreviewBlocked | Self::Submit | Self::Retry | Self::Conflict => {
                "bruhs_propose_workflow"
            }
            Self::Status | Self::StatusRevoked | Self::StatusUnknown | Self::StatusUnsaved => {
                "bruhs_workflow_status"
            }
            _ => "bruhs_request_workflow_rollback",
        }
    }
    fn arguments(self, id: &str) -> Value {
        match self.request(id) {
            BridgeRequest::PreviewWorkflow { proposal } => {
                json!({"title":proposal.title,"operations":proposal.operations,"dry_run":true})
            }
            BridgeRequest::ProposeWorkflow {
                request_id,
                proposal,
            } => {
                json!({"title":proposal.title,"operations":proposal.operations,"dry_run":false,"request_id":request_id})
            }
            BridgeRequest::Status { workflow_id } => json!({"workflow_id":workflow_id}),
            BridgeRequest::RequestRollback {
                workflow_id,
                dry_run,
            } => json!({"workflow_id":workflow_id,"dry_run":dry_run}),
            _ => unreachable!(),
        }
    }
    fn error(self) -> Option<&'static str> {
        match self {
            Self::Conflict => {
                Some("This request_id was already used for a different title or ordered payload.")
            }
            Self::StatusRevoked | Self::RollbackRevoked => {
                Some("Write access was revoked. Review this workflow in the app.")
            }
            Self::StatusUnknown => Some("Unknown proposal for this connection session."),
            Self::RollbackUnsaved => Some("Wait for a durable local save before requesting undo."),
            Self::RollbackChanged => Some(
                "A created record was edited, removed, or is unavailable. No records were changed.",
            ),
            _ => None,
        }
    }
    fn expected(self, id: &str) -> Value {
        if matches!(self, Self::Preview | Self::PreviewBlocked) {
            return json!({"dry_run":true,"proposal":self.proposal(),"allowed":self!=Self::PreviewBlocked,
                "blocked_domains":if self==Self::PreviewBlocked {vec![AgentDomain::Capture]} else {Vec::new()},
                "requires_user_approval":true,"executed":false});
        }
        if matches!(
            self,
            Self::Retry | Self::Status | Self::StatusUnsaved | Self::Submit
        ) {
            let durable = !matches!(self, Self::StatusUnsaved | Self::Submit);
            let submitted = self == Self::Submit;
            return json!({"workflow_id":id,"workflow_status":if submitted {"previewed"} else {"completed"},
                "durable":durable,"requires_user_approval":submitted,"executed":durable&&!submitted,"rolled_back":false,
                "rollback_review":null,"progress":{"total_steps":if submitted {2} else {1},"completed_steps":if submitted {0} else {1},
                "reverted_steps":0,"durable_completed_steps":durable.then_some(1),"durable_reverted_steps":durable.then_some(0)},
                "job":if submitted {Value::Null} else {json!({"status":"completed","next_step":1,"background_enabled":false,"can_advance":false})}});
        }
        json!({"workflow_id":id,"dry_run":!matches!(self,Self::RollbackRequest|Self::RollbackPending),
            "eligible":self!=Self::RollbackDeclined,"completed_steps":usize::from(self!=Self::RollbackDeclined),
            "rollback_review":match self {Self::RollbackRequest|Self::RollbackPending=>Some("pending"),Self::RollbackDeclined=>Some("declined"),_=>None},
            "requires_user_approval":self!=Self::RollbackDeclined,"rolled_back":false,"checkpoint_durable":true})
    }
    fn check(self, response: BridgeResponse, id: &str) -> Value {
        if let Some(error) = self.error() {
            assert!(
                matches!(response,BridgeResponse::Error(ref text) if text==error),
                "{self:?}: {response:?}"
            );
            return json!({"error":error});
        }
        let BridgeResponse::Ok(mut value) = response else {
            panic!("{self:?}: {response:?}")
        };
        if self == Self::Submit {
            let actual = value["workflow_id"]
                .as_str()
                .expect("submitted workflow ID");
            assert_eq!(
                uuid::Uuid::parse_str(actual)
                    .expect("UUID workflow ID")
                    .get_version_num(),
                4
            );
            value["workflow_id"] = "generated-workflow-id".into();
        }
        assert_eq!(
            value,
            self.expected(if self == Self::Submit {
                "generated-workflow-id"
            } else {
                id
            }),
            "{self:?}"
        );
        if !matches!(self, Self::Preview | Self::PreviewBlocked) {
            assert!(!value.to_string().contains(CONTENT));
        }
        value
    }
}

fn operation() -> WorkflowOperation {
    WorkflowOperation::AddCapture {
        content: CONTENT.into(),
    }
}

struct Seed {
    state: AutomationState,
    capture: CaptureRecord,
    id: String,
}
impl Seed {
    fn new(records: usize) -> Self {
        assert!(records > 0);
        let mut state = AutomationState::default();
        for domain in [AgentDomain::Capture, AgentDomain::Tasks] {
            state.permissions.set(domain, AgentAccess::Write, true)
        }
        for index in 0..records {
            let mut workflow = Workflow::draft(TITLE, vec![operation()], 0);
            workflow.id = format!("workflow-{index}");
            workflow.status = WorkflowStatus::Declined;
            state.workflows.push(workflow);
        }
        let id = state.workflows.last().expect("nonempty seed").id.clone();
        state.workflows.last_mut().expect("nonempty seed").status = WorkflowStatus::Draft;
        assert!(state.preview(&id, 0).is_some());
        let job = state.start_job(&id, 0).expect("reviewed fixture workflow");
        let mut capture =
            CaptureRecord::new(CaptureKind::Note, CONTENT, 0).expect("fixture capture");
        capture.id = "capture-target".into();
        assert!(state.complete_step_with_snapshot(
            &job,
            &capture.id,
            serde_json::to_string(&capture).expect("fixture capture snapshot"),
            0
        ));
        Self { state, capture, id }
    }
    fn install(&self, w: &mut Workspace, cx: &mut gpui::Context<'_, Workspace>) {
        w.automation_state = self.state.clone();
        w.captures = vec![self.capture.clone()];
        w.work_items.clear();
        w.reviewed_workflow_id = None;
        w.mcp_bridge.reviews.proposals.clear();
        w.mcp_bridge.reviews.rollback_reviews.clear();
        // 99 reserves one of the existing 100 session slots for fresh submission.
        for workflow in self.state.workflows.iter().rev().take(99) {
            let key = if workflow.id == self.id {
                "target-request".into()
            } else {
                format!("request-{}", workflow.id)
            };
            w.mcp_bridge.reviews.proposals.insert(
                key,
                (
                    workflow.id.clone(),
                    WorkflowProposal {
                        title: TITLE.into(),
                        operations: vec![operation()],
                    },
                ),
            );
        }
        w.active_view = AppView::Chat;
        w.save_status = SaveStatus::Saved;
        w.persist_and_notify(cx);
    }
    fn adjust(&self, w: &mut Workspace, case: Case) {
        match case {
            Case::PreviewBlocked | Case::StatusRevoked | Case::RollbackRevoked => w
                .automation_state
                .permissions
                .set(AgentDomain::Capture, AgentAccess::Write, false),
            Case::StatusUnsaved | Case::RollbackUnsaved => w.save_status = SaveStatus::Saving,
            Case::RollbackPending => {
                w.mcp_bridge
                    .reviews
                    .rollback_reviews
                    .insert(self.id.clone(), RollbackReview::Pending);
            }
            Case::RollbackDeclined => {
                w.mcp_bridge
                    .reviews
                    .rollback_reviews
                    .insert(self.id.clone(), RollbackReview::Declined);
            }
            Case::RollbackChanged => w.captures[0].body.push_str(" changed after checkpoint"),
            _ => {}
        }
    }
    fn unchanged(&self, w: &Workspace, case: Case) {
        assert!(w.work_items.is_empty());
        assert_eq!(w.captures.len(), 1);
        if case != Case::RollbackChanged {
            assert_eq!(w.captures[0], self.capture)
        }
        assert_eq!(
            w.automation_state.workflows.len(),
            self.state.workflows.len() + usize::from(case == Case::Submit)
        );
        assert_eq!(
            w.automation_state.workflows[self.state.workflows.len() - 1],
            self.state.workflows[self.state.workflows.len() - 1]
        );
    }
}

async fn prepare(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    seed: &Seed,
    case: Case,
) {
    if matches!(case, Case::StatusUnsaved | Case::RollbackUnsaved) {
        workspace.update_in(cx, |w, _, _| w.save_status = SaveStatus::Saved)
    }
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, cx| seed.install(w, cx));
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, _| seed.adjust(w, case));
}

#[test]
fn pending_proposal_selection_preserves_owner_priority_through_history_mutations() {
    for records in [0_usize, 1, 2, 7, 31] {
        for proposals in [0_usize, 1, 2, 9, 99] {
            for previews in [false, true] {
                let mut bridge = WorkspaceBridge::default();
                let mut state = AutomationState::default();
                for index in 0..records {
                    let mut workflow = Workflow::draft(TITLE, vec![operation()], 0);
                    workflow.id = format!("workflow-{index}");
                    workflow.status = if previews {
                        WorkflowStatus::Previewed
                    } else {
                        WorkflowStatus::Declined
                    };
                    state.workflows.push(workflow);
                }
                for index in 0..proposals {
                    bridge.reviews.proposals.insert(
                        format!("request-{index}"),
                        (
                            format!("workflow-{}", index % records.max(1)),
                            WorkflowProposal {
                                title: TITLE.into(),
                                operations: vec![operation()],
                            },
                        ),
                    );
                }
                for mutation in 0..8 {
                    let expected = bridge
                        .reviews
                        .proposals
                        .values()
                        .find(|(id, _)| {
                            state.workflows.iter().any(|workflow| {
                                workflow.id == *id && workflow.status == WorkflowStatus::Previewed
                            })
                        })
                        .map(|(id, _)| id.as_str());
                    let actual = bridge.reviews.pending_proposal_id(&state);
                    assert_eq!(
                        actual, expected,
                        "{records}/{proposals}/{previews}/{mutation}"
                    );
                    if let (Some(actual), Some(expected)) = (actual, expected) {
                        assert!(
                            std::ptr::eq(actual, expected),
                            "The same owner entry must win"
                        );
                    }
                    if !state.workflows.is_empty() {
                        let len = state.workflows.len();
                        state.workflows.rotate_left(mutation % len);
                        state.workflows[0].status = WorkflowStatus::Previewed;
                        if mutation == 2 {
                            state.workflows.push(state.workflows[0].clone());
                        }
                        if mutation == 3 {
                            state.workflows.remove(0);
                        }
                    }
                    if mutation == 4 {
                        bridge.reviews.proposals.insert(
                            "inserted-owner".into(),
                            (
                                "workflow-0".into(),
                                WorkflowProposal {
                                    title: TITLE.into(),
                                    operations: vec![operation()],
                                },
                            ),
                        );
                    }
                    if mutation == 5 {
                        bridge.reviews.proposals.remove("request-0");
                    }
                }
            }
        }
    }
}

#[gpui::test]
async fn workflow_performance_fixture_checks_all_tool_variants(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let directory = tempfile::Builder::new()
        .prefix("bruhs-workflow-fixture-")
        .tempdir()
        .unwrap();
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(directory.path().join("state.json"))
    });
    let seed = Seed::new(3);
    for case in Case::all() {
        prepare(&workspace, cx, &seed, case).await;
        workspace.update_in(cx, |w, _, cx| {
            case.check(
                w.handle_bridge_request(case.request(&seed.id), cx),
                &seed.id,
            );
            seed.unchanged(w, case);
        });
        if matches!(case, Case::StatusUnsaved | Case::RollbackUnsaved) {
            workspace.update_in(cx, |w, _, _| w.save_status = SaveStatus::Saved)
        }
        cx.condition(&workspace, |w, _| w.save_status.is_saved())
            .await;
    }
    directory.close().unwrap();
}

#[test]
fn pending_proposals_ignore_unowned_previews_and_reflect_duplicate_history_changes() {
    let mut bridge = WorkspaceBridge::default();
    for id in ["owned-a", "owned-b"] {
        bridge.reviews.proposals.insert(
            id.into(),
            (id.into(), WorkflowProposal::single(operation())),
        );
    }
    let mut state = AutomationState::default();
    for id in ["owned-a", "owned-b", "other"] {
        let mut workflow = Workflow::draft(TITLE, vec![operation()], 0);
        workflow.id = id.into();
        workflow.status = if id == "other" {
            WorkflowStatus::Previewed
        } else {
            WorkflowStatus::Declined
        };
        state.workflows.push(workflow);
    }
    assert!(bridge.reviews.pending_proposal_id(&state).is_none());
    let mut duplicate = state.workflows[1].clone();
    duplicate.status = WorkflowStatus::Previewed;
    state.workflows.push(duplicate);
    assert_eq!(bridge.reviews.pending_proposal_id(&state), Some("owned-b"));
    state.workflows.reverse();
    assert_eq!(bridge.reviews.pending_proposal_id(&state), Some("owned-b"));
    state.workflows[0].id = "other-b".into();
    assert!(bridge.reviews.pending_proposal_id(&state).is_none());
}

#[gpui::test]
#[ignore = "manual optimized workflow handler/full-MCP baseline;set BRUHS_PERF_MCP_BIN and BRUHS_PERF_WORKFLOW_OUTPUT"]
async fn workflow_native_action_cost(cx: &mut TestAppContext) {
    let binary = PathBuf::from(std::env::var_os("BRUHS_PERF_MCP_BIN").unwrap());
    let output = PathBuf::from(std::env::var_os("BRUHS_PERF_WORKFLOW_OUTPUT").unwrap());
    let smoke = std::env::var_os("BRUHS_PERF_WORKFLOW_SMOKE").is_some();
    let (warmups, samples) = if smoke { (1, 2) } else { (5, 30) };
    let histories = if smoke { vec![3] } else { vec![1, 10_001] };
    let harness = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/performance-mcp-native-workflows.py");
    let directory = tempfile::Builder::new()
        .prefix("bruhs-workflow-cost-")
        .tempdir()
        .unwrap();
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(directory.path().join("state.json"))
    });
    connect(&workspace, cx).await;
    for records in histories {
        let seed = Seed::new(records);
        for case in Case::all() {
            let mut intervals = Vec::new();
            for iteration in 0..warmups + samples {
                if iteration == 0 || case == Case::Submit {
                    prepare(&workspace, cx, &seed, case).await;
                } else if case == Case::RollbackRequest {
                    workspace.update_in(cx, |w, _, _| {
                        w.mcp_bridge.reviews.rollback_reviews.clear();
                        w.reviewed_workflow_id = None;
                    });
                }
                let request = case.request(&seed.id);
                let elapsed = workspace.update_in(cx, |w, _, cx| {
                    let start = std::time::Instant::now();
                    let response = w.handle_bridge_request(request, cx);
                    let elapsed = start.elapsed().as_secs_f64() * 1000.;
                    case.check(response, &seed.id);
                    seed.unchanged(w, case);
                    elapsed
                });
                if iteration >= warmups {
                    intervals.push(elapsed)
                }
            }
            eprintln!(
                "WORKFLOW_HANDLER_COST {}",
                json!({"records":records,"case":case.name(),"samples_ms":intervals,"warmups":warmups})
            );
            let descriptor = json!({"records":records,"case":case.name(),"tool":case.tool(),"arguments":case.arguments(&seed.id),
                "expected":if let Some(error)=case.error(){json!({"error":error})}else{case.expected(if case==Case::Submit {"generated-workflow-id"}else{&seed.id})},"fresh":case==Case::Submit});
            let (prepare_tx, mut prepare_rx) =
                tokio::sync::mpsc::channel::<tokio::sync::oneshot::Sender<()>>(1);
            let receiver = workspace.update_in(cx, |w, _, cx| {
                let ConnectionState::Connected { session, .. } = &w.mcp_bridge.state else {
                    panic!("connected")
                };
                let connection = session.connection_path().to_owned();
                let (done_tx, done_rx) = tokio::sync::oneshot::channel();
                let (binary, harness) = (binary.clone(), harness.clone());
                let descriptor = descriptor.to_string();
                let output = output.with_extension(format!("{records}-{}.json", case.name()));
                cx.global::<DesktopState>().runtime.spawn(async move {
                    let result = async {
                        let mut child = tokio::process::Command::new("python3")
                            .arg(harness)
                            .arg("--binary")
                            .arg(binary)
                            .arg("--output")
                            .arg(output)
                            .arg("--connection")
                            .arg(connection)
                            .arg("--target")
                            .arg(descriptor)
                            .arg("--samples")
                            .arg(samples.to_string())
                            .arg("--warmups")
                            .arg(warmups.to_string())
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .kill_on_drop(true)
                            .spawn()?;
                        let mut stdin = child.stdin.take().unwrap();
                        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
                        while let Some(line) = lines.next_line().await? {
                            assert_eq!(line, "PREPARE");
                            let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
                            prepare_tx.send(ack_tx).await.unwrap();
                            ack_rx.await.unwrap();
                            stdin.write_all(b"READY\n").await?;
                        }
                        drop(prepare_tx);
                        let status = child.wait().await?;
                        let mut errors = String::new();
                        child
                            .stderr
                            .take()
                            .unwrap()
                            .read_to_string(&mut errors)
                            .await?;
                        Ok::<_, std::io::Error>((status, errors))
                    }
                    .await;
                    let _ = done_tx.send(result);
                });
                done_rx
            });
            let mut preparation = 0;
            while let Some(ack) = prepare_rx.recv().await {
                if preparation == 0 || case == Case::Submit {
                    prepare(&workspace, cx, &seed, case).await;
                } else if case == Case::RollbackRequest {
                    workspace.update_in(cx, |w, _, _| {
                        w.mcp_bridge.reviews.rollback_reviews.clear();
                        w.reviewed_workflow_id = None;
                    });
                }
                preparation += 1;
                ack.send(()).unwrap();
            }
            assert_eq!(preparation, warmups + samples);
            let (status, errors) = receiver.await.unwrap().unwrap();
            assert!(status.success(), "{errors}");
            workspace.update_in(cx, |w, _, _| seed.unchanged(w, case));
            if matches!(case, Case::StatusUnsaved | Case::RollbackUnsaved) {
                workspace.update_in(cx, |w, _, _| w.save_status = SaveStatus::Saved)
            }
            cx.condition(&workspace, |w, _| w.save_status.is_saved())
                .await;
        }
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
    assert!(std::fs::read(directory.path().join("state.json")).is_ok());
    directory.close().unwrap();
}
