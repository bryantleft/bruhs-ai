//! Agent workflows' review cards, as on the web (PARITY-05): a client's
//! proposal and a workflow left in preview each get a card, newest first,
//! with their own Start and Decline; the page's own dry run is reviewed in
//! Preview changes; a started workflow goes on show in the history.

use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse};
use bruhs_models::logic::workflow_review::{self as review, origin};
use bruhs_models::{AgentAccess, AgentDomain, EpochMs, WorkflowOperation, WorkflowStatus};
use gpui::{TestAppContext, VisualTestContext};

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

/// Tab stops in one full turn of the Tab order.
fn tab_cycle(cx: &mut VisualTestContext) -> usize {
    cx.simulate_keystrokes("tab");
    let first = cx
        .update(|window, cx| window.focused(cx))
        .expect("a Tab stop");
    for stops in 1..2000 {
        cx.simulate_keystrokes("tab");
        if cx.update(|window, cx| window.focused(cx)).as_ref() == Some(&first) {
            return stops;
        }
    }
    panic!("Tab never came back around");
}

#[gpui::test]
async fn proposals_and_leftover_previews_each_get_a_card_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
    });
    connect(&workspace, cx).await;
    // A client may only propose while nothing else waits, on both apps.
    let BridgeResponse::Ok(proposal) = call(
        &workspace,
        cx,
        BridgeRequest::Propose {
            request_id: "card-request".into(),
            operation: WorkflowOperation::CreateTask {
                title: "Client plan".into(),
            },
        },
    )
    .await
    else {
        panic!("proposal accepted")
    };
    let proposed = proposal["workflow_id"].as_str().unwrap().to_owned();
    // A preview no client owns, as one left from an earlier session.
    let leftover = workspace.update_in(cx, |w, _, cx| {
        let id = w.automation_state.create_workflow(
            "Left from before",
            vec![WorkflowOperation::CreateTask {
                title: "Old plan".into(),
            }],
            EpochMs::now().as_millis(),
        );
        w.automation_state
            .preview(&id, EpochMs::now().as_millis())
            .expect("previewed");
        cx.notify();
        id
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(
            review::awaiting_review(&w.automation_state, w.shown_dry_run()),
            vec![leftover.clone(), proposed.clone()],
            "newest first"
        );
        assert_eq!(
            origin(w.mcp_bridge.owns(&proposed)),
            review::PROPOSED_BY_CLIENT
        );
        assert_eq!(
            origin(w.mcp_bridge.owns(&leftover)),
            review::WAITING_IN_PREVIEW
        );
        // The history does not move for a proposal.
        assert_eq!(w.reviewed_workflow_id, None);
    });
    assert!(cx
        .debug_bounds("desktop-workflow-awaiting-review")
        .is_some());
    assert!(cx
        .debug_bounds("desktop-workflow-awaiting-review-1")
        .is_some());

    // The leftover's own Decline declines only it.
    click(cx, "desktop-decline-workflow");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, _| {
        let status = |id: &str| {
            w.automation_state
                .workflows
                .iter()
                .find(|workflow| workflow.id == id)
                .map(|workflow| workflow.status)
        };
        assert_eq!(status(&leftover), Some(WorkflowStatus::Declined));
        assert_eq!(status(&proposed), Some(WorkflowStatus::Previewed));
    });
    assert!(cx
        .debug_bounds("desktop-workflow-awaiting-review-1")
        .is_none());

    // Start puts the workflow on show in the history, with its Run.
    click(cx, "desktop-start-workflow");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.reviewed_workflow_id.as_deref(), Some(proposed.as_str()));
        assert_eq!(w.automation_state.jobs.len(), 1);
        assert!(w.work_items.is_empty(), "nothing runs before Run next step");
    });
    assert!(cx
        .debug_bounds("desktop-workflow-awaiting-review")
        .is_none());
    assert!(cx.debug_bounds("desktop-run-workflow-step").is_some());
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
fn the_pages_own_dry_run_is_reviewed_in_preview_changes_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    click(cx, "desktop-automation-new-dry-run");
    assert!(cx.debug_bounds("desktop-preview-changes").is_some());
    assert!(
        cx.debug_bounds("desktop-workflow-awaiting-review")
            .is_none(),
        "the dry run is not also a waiting card"
    );
    let dry_run = workspace.update_in(cx, |w, _, _| {
        w.shown_dry_run().map(str::to_owned).expect("on show")
    });
    // A second dry run replaces it; the first now waits in a card.
    click(cx, "desktop-automation-new-dry-run");
    workspace.update_in(cx, |w, _, _| {
        assert_ne!(w.shown_dry_run(), Some(dry_run.as_str()));
        assert_eq!(
            review::awaiting_review(&w.automation_state, w.shown_dry_run()),
            vec![dry_run.clone()]
        );
    });
    assert!(cx.debug_bounds("desktop-preview-changes").is_some());
    assert!(cx
        .debug_bounds("desktop-workflow-awaiting-review")
        .is_some());
}

/// Web: a proposal's card adds Recheck grants, Start transaction and Decline
/// proposal to the Tab order, and the history its Review workflow.
#[gpui::test]
async fn a_proposal_adds_the_webs_tab_stops(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
    });
    connect(&workspace, cx).await;
    cx.run_until_parked();
    let before = tab_cycle(cx);
    let BridgeResponse::Ok(_) = call(
        &workspace,
        cx,
        BridgeRequest::Propose {
            request_id: "tab-request".into(),
            operation: WorkflowOperation::CreateTask {
                title: "Keyboard plan".into(),
            },
        },
    )
    .await
    else {
        panic!("proposal accepted")
    };
    cx.run_until_parked();
    assert_eq!(tab_cycle(cx), before + 4);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

/// A dry run you decide against is declined from Preview changes, and no
/// longer holds up a client's proposal.
#[gpui::test]
async fn a_dry_run_can_be_declined_and_then_a_client_may_propose(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
    });
    click(cx, "desktop-automation-new-dry-run");
    let dry_run = workspace.update_in(cx, |w, _, _| {
        w.shown_dry_run().map(str::to_owned).expect("on show")
    });
    click(cx, "desktop-decline-dry-run");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    workspace.update_in(cx, |w, _, _| {
        let declined = w
            .automation_state
            .workflows
            .iter()
            .find(|workflow| workflow.id == dry_run)
            .map(|workflow| workflow.status);
        assert_eq!(declined, Some(WorkflowStatus::Declined));
        assert_eq!(w.shown_dry_run(), None);
    });
    assert!(cx.debug_bounds("desktop-preview-changes").is_none());
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(proposal) = call(
        &workspace,
        cx,
        BridgeRequest::Propose {
            request_id: "after-dry-run".into(),
            operation: WorkflowOperation::CreateTask {
                title: "Client plan".into(),
            },
        },
    )
    .await
    else {
        panic!("the proposal is accepted once the dry run is declined")
    };
    assert_eq!(proposal["workflow_status"], "previewed");
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

/// Leaving Agent workflows hands a waiting dry run to the waiting cards, as
/// the web's page-owned dry run does.
#[gpui::test]
fn leaving_the_page_turns_the_dry_run_into_a_waiting_card(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    click(cx, "desktop-automation-new-dry-run");
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.shown_dry_run().is_some());
        w.active_view = super::super::AppView::Today;
        cx.notify();
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        w.active_view = super::super::AppView::Automations;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-preview-changes").is_none());
    assert!(cx
        .debug_bounds("desktop-workflow-awaiting-review")
        .is_some());
}
