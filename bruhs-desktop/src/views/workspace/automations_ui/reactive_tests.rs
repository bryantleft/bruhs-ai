//! A model draft may propose a workflow, but cannot approve or execute it.
use super::super::{theme, AutomationState, DesktopState, Workspace};
use crate::views::reactive_automations::ReactivePreview;
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation, WorkflowStatus};
use gpui::AppContext as _;
use std::{cell::RefCell, rc::Rc};

#[gpui::test]
fn reactive_draft_is_previewed_without_granting_write_or_starting_a_job(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let holder = Rc::new(RefCell::new(None));
    let capture = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = holder.borrow().clone().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let panel = workspace.update_in(cx, |workspace, _, _| {
        workspace.persistence_test_path = Some(directory.path().join("state.json"));
        workspace.automation_state = AutomationState::default();
        workspace.work_items.clear();
        workspace.reactive_automations.clone()
    });
    panel.update_in(cx, |_, _, cx| {
        cx.emit(ReactivePreview {
            title: "Follow up after the meeting".into(),
            operations: vec![WorkflowOperation::CreateTask {
                title: "Review the quote".into(),
            }],
            run_id: "run-42".into(),
        });
    });
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _, _| {
        let workflow = workspace.automation_state.workflows.last().unwrap();
        assert_eq!(workflow.title, "Follow up after the meeting");
        assert_eq!(workflow.status, WorkflowStatus::Previewed);
        assert_eq!(
            workflow
                .origin
                .as_ref()
                .map(|origin| origin.reactive_run_id.as_str()),
            Some("run-42"),
            "the draft's run receives the owner's decision"
        );
        assert_eq!(workspace.reviewed_workflow_id.as_ref(), Some(&workflow.id));
        assert!(!workspace
            .automation_state
            .permissions
            .allows(AgentDomain::Tasks, AgentAccess::Write));
        assert!(workspace.automation_state.jobs.is_empty());
        assert!(workspace.work_items.is_empty());
    });
}
