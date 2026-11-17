use super::*;
use crate::{AgentAccess, AgentDomain, WorkflowOperation, WorkflowStatus};

fn task(title: &str) -> WorkflowOperation {
    WorkflowOperation::CreateTask {
        title: title.into(),
    }
}

/// A started workflow with its one step done.
fn completed(state: &mut AutomationState, title: &str, at: i64) -> String {
    let id = state.create_workflow(title, vec![task(title)], at);
    state.preview(&id, at).unwrap();
    let job = state.start_job(&id, at).unwrap();
    assert!(state.complete_step(&job, format!("task-{at}"), at));
    id
}

/// Five workflows, oldest first: declined, completed, rolled back, running
/// and one still waiting in preview.
fn five() -> (AutomationState, Vec<String>) {
    let mut state = AutomationState::default();
    state
        .permissions
        .set(AgentDomain::Tasks, AgentAccess::Write, true);
    let declined = state.create_workflow("Declined plan", vec![task("a")], 1);
    state.preview(&declined, 1).unwrap();
    assert!(state.decline_workflow(&declined, 1));
    let done = completed(&mut state, "Completed plan", 2);
    let undone = completed(&mut state, "Undone plan", 3);
    assert!(state.mark_rolled_back(&undone, 3));
    let running = state.create_workflow("Running plan", vec![task("d"), task("e")], 4);
    state.preview(&running, 4).unwrap();
    state.start_job(&running, 4).unwrap();
    let previewed = state.create_workflow("Previewed plan", vec![task("f")], 5);
    state.preview(&previewed, 5).unwrap();
    (state, vec![declined, done, undone, running, previewed])
}

#[test]
fn an_empty_history_has_no_page() {
    let state = AutomationState::default();
    assert_eq!(page(&state, None), None);
    assert_eq!(reviewed(&state, Some("gone")), None);
}

#[test]
fn the_history_opens_on_the_newest_workflow() {
    let (state, ids) = five();
    let newest = page(&state, None).unwrap();
    assert_eq!(newest.workflow.id, ids[4]);
    assert_eq!((newest.number, newest.total), (5, 5));
    assert_eq!(newest.previous, Some(ids[3].as_str()));
    assert_eq!(newest.next, None);
    assert_eq!(newest.label(), "Previewed plan, Previewed, workflow 5 of 5");
}

#[test]
fn previous_walks_back_through_every_workflow_and_next_returns() {
    let (state, ids) = five();
    let mut seen = Vec::new();
    let mut at = page(&state, None);
    while let Some(stop) = at {
        seen.push((stop.workflow.id.clone(), stop.label()));
        at = stop.previous.and_then(|id| page(&state, Some(id)));
    }
    assert_eq!(
        seen.iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>(),
        [
            "Previewed plan, Previewed, workflow 5 of 5",
            "Running plan, Running, workflow 4 of 5",
            "Undone plan, Rolled back, workflow 3 of 5",
            "Completed plan, Completed, workflow 2 of 5",
            "Declined plan, Declined, workflow 1 of 5",
        ]
    );
    assert_eq!(
        seen.iter().map(|(id, _)| id).rev().collect::<Vec<_>>(),
        ids.iter().collect::<Vec<_>>()
    );

    let oldest = page(&state, Some(&ids[0])).unwrap();
    assert_eq!(oldest.previous, None);
    let mut forward = vec![oldest.workflow.id.clone()];
    let mut at = oldest.next.and_then(|id| page(&state, Some(id)));
    while let Some(stop) = at {
        forward.push(stop.workflow.id.clone());
        at = stop.next.and_then(|id| page(&state, Some(id)));
    }
    assert_eq!(forward, ids);
}

#[test]
fn a_selection_that_no_longer_exists_falls_back_to_the_newest() {
    let (state, ids) = five();
    assert_eq!(reviewed(&state, Some("removed")).unwrap().id, ids[4]);
    assert_eq!(reviewed(&state, Some(&ids[1])).unwrap().id, ids[1]);
    assert_eq!(
        page(&state, Some(&ids[1])).unwrap().status(),
        "Completed, workflow 2 of 5"
    );
}

#[test]
fn a_new_workflow_lengthens_the_history_without_moving_a_selection() {
    let (mut state, ids) = five();
    let before = page(&state, Some(&ids[2])).unwrap().label();
    let newer = state.create_workflow("Newer plan", vec![task("g")], 6);
    let after = page(&state, Some(&ids[2])).unwrap();
    assert_eq!(before, "Undone plan, Rolled back, workflow 3 of 5");
    assert_eq!(after.label(), "Undone plan, Rolled back, workflow 3 of 6");
    assert_eq!(page(&state, None).unwrap().workflow.id, newer);
    assert_eq!(
        page(&state, None).unwrap().label(),
        "Newer plan, Draft, workflow 6 of 6"
    );
}

#[test]
fn every_status_has_one_label() {
    let labels = [
        WorkflowStatus::Draft,
        WorkflowStatus::Previewed,
        WorkflowStatus::Declined,
        WorkflowStatus::Running,
        WorkflowStatus::Paused,
        WorkflowStatus::Completed,
        WorkflowStatus::Failed,
        WorkflowStatus::RolledBack,
    ]
    .map(WorkflowStatus::label);
    assert_eq!(
        labels,
        [
            "Draft",
            "Previewed",
            "Declined",
            "Running",
            "Paused",
            "Completed",
            "Failed",
            "Rolled back",
        ]
    );
}
