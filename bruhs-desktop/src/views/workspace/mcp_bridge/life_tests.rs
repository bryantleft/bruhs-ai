use super::super::{desktop_state_backup_path, load_desktop_state_from_path, SaveStatus};
use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::{AgentAccess, AgentDomain, FinanceTransaction, LifeContext, WorkflowOperation};
use gpui::{TestAppContext, VisualTestContext};

fn proposal() -> WorkflowProposal {
    WorkflowProposal {
        title: "Record a receipt and workout locally".into(),
        operations: vec![
            WorkflowOperation::AddFinanceTransaction {
                merchant: "Reviewed merchant".into(),
                category: "Manual ledger".into(),
                amount_minor: -9_007_199_254_740_993,
            },
            WorkflowOperation::LogWorkout {
                activity: "Reviewed walk".into(),
                minutes: 37,
            },
        ],
    }
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx.debug_bounds(selector).expect("workflow control");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
}

#[gpui::test]
async fn life_proposal_needs_independent_write_grants_without_enabling_reads(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    connect(&workspace, cx).await;
    let request = BridgeRequest::ProposeWorkflow {
        request_id: "life-grants".into(),
        proposal: proposal(),
    };
    for granted in [None, Some(AgentDomain::Finance), Some(AgentDomain::Health)] {
        workspace.update_in(cx, |w, _, _| {
            for domain in [AgentDomain::Finance, AgentDomain::Health] {
                w.automation_state.permissions.set(
                    domain,
                    AgentAccess::Write,
                    Some(domain) == granted,
                );
            }
        });
        assert!(matches!(
            call(&workspace, cx, request.clone()).await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_state.workflows.is_empty());
        for domain in [AgentDomain::Finance, AgentDomain::Health] {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Write, true);
        }
    });
    let BridgeResponse::Ok(response) = call(&workspace, cx, request).await else {
        panic!("Life proposal adapter must be connected")
    };
    assert_eq!(response["executed"], false);
    assert_eq!(response["requires_user_approval"], true);
    for domain in [AgentDomain::Finance, AgentDomain::Health] {
        assert!(matches!(
            call(
                &workspace,
                cx,
                BridgeRequest::Read {
                    domain,
                    query: String::new(),
                    offset: 0
                }
            )
            .await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_state.jobs.is_empty());
        assert!(!w.life_context.finance.privacy.allow_agent_context);
        assert!(!w.life_context.health.privacy.allow_agent_context);
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn reviewed_life_records_and_receipts_save_together_and_undo_preserves_other_context(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-life-workflow-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    let mut original = LifeContext::default();
    original
        .finance
        .transactions
        .push(FinanceTransaction::manual(
            "Keep this receipt",
            "Other",
            123,
            1,
        ));
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(path.clone());
        w.life_context = original.clone();
        for operation in proposal().operations {
            w.automation_state
                .permissions
                .set(operation.domain(), AgentAccess::Write, true);
        }
    });
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(response) = call(
        &workspace,
        cx,
        BridgeRequest::ProposeWorkflow {
            request_id: "durable-life".into(),
            proposal: proposal(),
        },
    )
    .await
    else {
        panic!("Life proposal adapter must be connected")
    };
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let id = response["workflow_id"].as_str().unwrap().to_string();
    click(cx, "desktop-start-workflow");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    for completed in 1..=2 {
        click(cx, "desktop-run-workflow-step");
        workspace.update_in(cx, |w, _, _| assert_eq!(w.save_status, SaveStatus::Saving));
        cx.condition(&workspace, |w, _| w.save_status.is_saved())
            .await;
        let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
        assert_eq!(saved.automation_state.jobs[0].next_step, completed);
        assert_eq!(
            saved.life_context.finance.transactions.len()
                + saved.life_context.health.workouts.len(),
            completed + 1
        );
        let transaction = saved
            .life_context
            .finance
            .transactions
            .iter()
            .find(|item| item.merchant == "Reviewed merchant")
            .unwrap();
        assert_eq!(transaction.amount_minor, -9_007_199_254_740_993);
        assert_eq!(transaction.currency, "USD");
        assert_eq!(
            saved.automation_state.workflows[0].steps[0]
                .target_snapshot
                .as_deref(),
            Some(serde_json::to_string(transaction).unwrap().as_str())
        );
    }
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.life_context.health.workouts[0].duration_minutes, 37);
        // A modified record must stop the entire undo, not just its own step.
        w.life_context.health.workouts[0].perceived_exertion = 8;
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        w.life_context.health.workouts[0].perceived_exertion = 5;
        w.automation_state
            .permissions
            .set(AgentDomain::Finance, AgentAccess::Write, false);
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        w.automation_state
            .permissions
            .set(AgentDomain::Finance, AgentAccess::Write, true);
    });
    let BridgeResponse::Ok(requested) = call(
        &workspace,
        cx,
        BridgeRequest::RequestRollback {
            workflow_id: id.clone(),
            dry_run: false,
        },
    )
    .await
    else {
        panic!("undo review")
    };
    assert_eq!(requested["completed_steps"], 2);
    let before_undo = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(before_undo.life_context.health.workouts.len(), 1);
    click(cx, "desktop-undo-workflow");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.life_context, original);
    let BridgeResponse::Ok(status) =
        call(&workspace, cx, BridgeRequest::Status { workflow_id: id }).await
    else {
        panic!("saved status")
    };
    assert_eq!(status["rolled_back"], true);
    assert_eq!(status["progress"]["durable_reverted_steps"], 2);
    let status_text = status.to_string();
    assert!(!status_text.contains("Reviewed merchant") && !status_text.contains("740993"));
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
async fn life_save_failure_blocks_next_step_and_reports_only_provisional_progress(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-life-save-failure-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(path.clone());
        w.life_context = LifeContext::default();
        for operation in proposal().operations {
            w.automation_state
                .permissions
                .set(operation.domain(), AgentAccess::Write, true);
        }
    });
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(response) = call(
        &workspace,
        cx,
        BridgeRequest::ProposeWorkflow {
            request_id: "failed-life".into(),
            proposal: proposal(),
        },
    )
    .await
    else {
        panic!("proposal")
    };
    let id = response["workflow_id"].as_str().unwrap().to_string();
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    click(cx, "desktop-start-workflow");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let durable_before = std::fs::read(&path).unwrap();
    let backup = desktop_state_backup_path(&path);
    let retained = directory.join("retained.json");
    std::fs::rename(&backup, &retained).unwrap();
    std::fs::create_dir(&backup).unwrap();
    click(cx, "desktop-run-workflow-step");
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    click(cx, "desktop-run-workflow-step");
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.life_context.finance.transactions.len(), 1);
        assert!(w.life_context.health.workouts.is_empty());
        assert_eq!(w.automation_state.jobs[0].next_step, 1);
    });
    assert_eq!(std::fs::read(&path).unwrap(), durable_before);
    let BridgeResponse::Ok(status) = call(
        &workspace,
        cx,
        BridgeRequest::Status {
            workflow_id: id.clone(),
        },
    )
    .await
    else {
        panic!("provisional status")
    };
    assert_eq!(status["durable"], false);
    assert_eq!(status["executed"], false);
    assert_eq!(status["progress"]["completed_steps"], 1);
    assert!(status["progress"]["durable_completed_steps"].is_null());
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::RequestRollback {
                workflow_id: id,
                dry_run: false
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    std::fs::remove_dir(&backup).unwrap();
    std::fs::rename(retained, backup).unwrap();
    click(cx, "desktop-retry-save");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.life_context.finance.transactions.len(), 1);
    assert_eq!(saved.automation_state.jobs[0].next_step, 1);
    assert!(saved.life_context.health.workouts.is_empty());
    // Revocation after a durable first step cannot authorize the second.
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Health, AgentAccess::Write, false)
    });
    click(cx, "desktop-run-workflow-step");
    workspace.update_in(cx, |w, _, _| {
        assert!(w.life_context.health.workouts.is_empty());
        assert_eq!(w.automation_state.jobs[0].next_step, 1);
        w.mcp_bridge = WorkspaceBridge::default();
    });
    std::fs::remove_dir_all(directory).unwrap();
}
