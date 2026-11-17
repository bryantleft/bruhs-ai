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

/// The job of `workflow_id`, which `start_job` made.
fn job_of<'a>(state: &'a AutomationState, workflow_id: &str) -> &'a crate::BackgroundJob {
    state
        .jobs
        .iter()
        .find(|job| job.workflow_id == workflow_id)
        .unwrap()
}

#[test]
fn each_workflow_in_the_history_carries_only_what_it_can_do() {
    let (state, ids) = five();
    let at = |index: usize| controls(&state, reviewed(&state, Some(&ids[index])).unwrap());
    // Declined, rolled back and previewed: nothing to run or undo.
    for index in [0, 2, 4] {
        assert_eq!(at(index), Controls::default(), "workflow {}", index + 1);
        assert!(!at(index).any());
        assert_eq!(at(index).job_status(), None);
    }
    // Completed: Undo only, though it is not the newest.
    let done = at(1);
    assert!(done.undo && done.any());
    assert_eq!((done.running, done.paused), (None, None));
    // Running with no step done yet: its own job, nothing to undo.
    let running = at(3);
    assert_eq!(running.running, Some(job_of(&state, &ids[3])));
    assert_eq!((running.paused, running.undo), (None, false));
    assert_eq!(
        running.job_status().as_deref(),
        Some("Running, next checkpoint 1")
    );
}

#[test]
fn an_older_workflow_keeps_its_own_job_while_a_newer_one_runs() {
    let mut state = AutomationState::default();
    state
        .permissions
        .set(AgentDomain::Tasks, AgentAccess::Write, true);
    let mut start = |title: &str, at: i64| {
        let id = state.create_workflow(title, vec![task("a"), task("b")], at);
        state.preview(&id, at).unwrap();
        let job = state.start_job(&id, at).unwrap();
        (id, job)
    };
    let (older, older_job) = start("Older plan", 1);
    let (newer, newer_job) = start("Newer plan", 2);
    let older_controls = |state: &AutomationState| {
        let workflow = reviewed(state, Some(&older)).unwrap();
        let shown = controls(state, workflow);
        (
            shown.running.map(|job| job.id.clone()),
            shown.paused.map(|job| job.id.clone()),
            shown.undo,
            shown.job_status(),
        )
    };
    assert_eq!(
        older_controls(&state),
        (
            Some(older_job.clone()),
            None,
            false,
            Some("Running, next checkpoint 1".into())
        )
    );

    // One step done: the older workflow can now be undone as well.
    assert!(state.complete_step(&older_job, "task-1", 3));
    assert_eq!(
        older_controls(&state),
        (
            Some(older_job.clone()),
            None,
            true,
            Some("Running, next checkpoint 2".into())
        )
    );

    // Pausing it leaves the newer job running and offers Resume.
    assert!(state.pause_job(&older_job, 4));
    assert_eq!(
        older_controls(&state),
        (
            None,
            Some(older_job.clone()),
            true,
            Some("Paused, resumes at checkpoint 2".into())
        )
    );
    let newest = controls(&state, reviewed(&state, None).unwrap());
    assert_eq!(
        newest.running.map(|job| job.id.as_str()),
        Some(newer_job.as_str())
    );
    assert_eq!(newest.paused, None);
    assert_eq!(reviewed(&state, None).unwrap().id, newer);

    // Resumed and finished, it keeps only Undo.
    assert!(state.resume_job(&older_job, 5));
    assert!(state.complete_step(&older_job, "task-2", 6));
    assert_eq!(older_controls(&state), (None, None, true, None));
}

#[test]
fn run_in_background_reads_as_on_once_enabled() {
    let (mut state, ids) = five();
    let job = job_of(&state, &ids[3]).id.clone();
    let shown = |state: &AutomationState| {
        controls(state, reviewed(state, Some(&ids[3])).unwrap()).in_background()
    };
    assert!(!shown(&state));
    assert!(state.enable_background(&job));
    assert!(shown(&state));
}

#[test]
fn a_failed_workflow_with_a_completed_step_offers_undo() {
    let mut state = AutomationState::default();
    state
        .permissions
        .set(AgentDomain::Tasks, AgentAccess::Write, true);
    let id = state.create_workflow("Failed plan", vec![task("a"), task("b")], 1);
    state.preview(&id, 1).unwrap();
    let job = state.start_job(&id, 1).unwrap();
    assert!(state.complete_step(&job, "task-1", 2));
    // As a failing executor leaves it: the second step failed, so did the job.
    let workflow = state.workflows.iter_mut().find(|w| w.id == id).unwrap();
    workflow.status = WorkflowStatus::Failed;
    workflow.steps[1].status = crate::WorkflowStepStatus::Failed;
    state.jobs[0].status = crate::BackgroundJobStatus::Failed;
    let shown = controls(&state, reviewed(&state, Some(&id)).unwrap());
    assert_eq!(
        (shown.running, shown.paused, shown.undo),
        (None, None, true)
    );
    assert_eq!(shown.job_status(), None);
}
