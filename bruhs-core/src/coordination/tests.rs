use super::*;

mod wakes;

fn fixture(parent_id: Option<ThreadId>, state: TaskState) -> CoordinatedTask {
    CoordinatedTask {
        id: ThreadId::new_v4(),
        parent_id,
        source_context: vec![],
        source_revision: None,
        relationship: Relationship::Root,
        title: "Test task".into(),
        project_path: "/project".into(),
        working_path: "/project/worktree".into(),
        model: "claude-sonnet-4-5".into(),
        permission_mode: PermissionMode::Plan,
        spec_id: None,
        spec_revision: None,
        state,
        messages: vec![TaskMessage {
            role: "user".into(),
            text: "Inspect the project".into(),
            at: EpochMs::now(),
        }],
        live_output: String::new(),
        error: None,
        session_id: None,
        approvals: vec![],
        created_at: EpochMs::now(),
        updated_at: EpochMs::now(),
        resume_at: None,
        auto_resume_on_limit: false,
        limit_reported: false,
    }
}

async fn runtime(root: &Path, tasks: Vec<CoordinatedTask>) -> CoordinationRuntime {
    let mut data = Data::default();
    for task in tasks {
        data.tasks.insert(task.id.clone(), task);
    }
    let path = root.join("tasks.json");
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    CoordinationRuntime::open(path, Arc::new(crate::AppState::new()))
        .await
        .unwrap()
}

#[tokio::test]
async fn completion_is_durable_and_a_restart_never_replays_active_work() {
    let root = tempfile::tempdir().unwrap();
    let first = fixture(None, TaskState::Completed);
    let interrupted = fixture(None, TaskState::Running);
    let task_id = first.id.clone();
    let interrupted_id = interrupted.id.clone();
    let runtime = runtime(root.path(), vec![first, interrupted]).await;
    assert_eq!(
        runtime.task(&interrupted_id).await.unwrap().state,
        TaskState::Interrupted
    );
    runtime
        .observe(&task_id, crate::QueryEvent::Text("Verified result".into()))
        .await
        .unwrap();
    runtime
        .finish(&task_id, TaskState::Completed, None)
        .await
        .unwrap();
    drop(runtime);
    let reopened = CoordinationRuntime::open(
        root.path().join("tasks.json"),
        Arc::new(crate::AppState::new()),
    )
    .await
    .unwrap();
    let result = reopened.task(&task_id).await.unwrap();
    assert_eq!(result.messages.last().unwrap().text, "Verified result");
    assert_eq!(result.state, TaskState::Completed);
    assert_eq!(
        reopened.task(&interrupted_id).await.unwrap().state,
        TaskState::Interrupted
    );
    drop(reopened);
}

#[tokio::test]
async fn cancelling_a_parent_cancels_only_its_descendants_and_clears_schedules() {
    let root = tempfile::tempdir().unwrap();
    let parent = fixture(None, TaskState::Completed);
    let child = fixture(Some(parent.id.clone()), TaskState::Completed);
    let unrelated = fixture(None, TaskState::Completed);
    let parent_id = parent.id.clone();
    let child_id = child.id.clone();
    let other_id = unrelated.id.clone();
    let runtime = runtime(root.path(), vec![parent, child, unrelated]).await;
    {
        let mut data = runtime.0.data.lock().await;
        for id in [&parent_id, &child_id, &other_id] {
            let task = data.tasks.get_mut(id).unwrap();
            task.state = TaskState::Running;
            task.resume_at = Some(EpochMs::now());
        }
        drop(data);
    }
    runtime.cancel(&parent_id).await.unwrap();
    assert_eq!(
        runtime.task(&parent_id).await.unwrap().state,
        TaskState::Cancelled
    );
    assert_eq!(
        runtime.task(&child_id).await.unwrap().state,
        TaskState::Cancelled
    );
    assert!(runtime.task(&child_id).await.unwrap().resume_at.is_none());
    assert_eq!(
        runtime.task(&other_id).await.unwrap().state,
        TaskState::Running
    );
    drop(runtime);
}

#[tokio::test]
async fn guests_cannot_act_or_read_other_tasks_and_messages_wait_for_owner_review() {
    let root = tempfile::tempdir().unwrap();
    let shared = fixture(None, TaskState::Completed);
    let other = fixture(None, TaskState::Completed);
    let shared_id = shared.id.clone();
    let other_id = other.id.clone();
    let runtime = runtime(root.path(), vec![shared, other]).await;
    let issued = runtime
        .issue_access(
            AccessScope::Guest {
                task_id: shared_id.clone(),
            },
            60,
        )
        .await
        .unwrap();
    let scope = runtime.authorize(&issued.token).await.unwrap();
    let visible = runtime.scoped_snapshot(&scope).await.unwrap();
    assert_eq!(visible.tasks.len(), 1);
    assert_eq!(visible.tasks[0].id, shared_id);
    assert!(visible.tasks[0].working_path.is_empty());
    assert!(runtime
        .scoped_command(&scope, CoordinationCommand::Cancel { id: other_id })
        .await
        .is_err());
    runtime
        .propose(&scope, "Please explain the result".into())
        .await
        .unwrap();
    assert_eq!(runtime.task(&shared_id).await.unwrap().messages.len(), 1);
    assert_eq!(runtime.snapshot().await.guest_proposals.len(), 1);
    let raw = std::fs::read_to_string(root.path().join("tasks.json")).unwrap();
    assert!(!raw.contains(&issued.token));
    runtime.revoke_access(&issued.grant.id).await.unwrap();
    assert!(runtime.authorize(&issued.token).await.is_err());
    drop(runtime);
}

#[tokio::test]
async fn a_second_process_cannot_take_over_an_open_task_store() {
    let root = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path(), vec![]).await;
    assert!(CoordinationRuntime::open(
        root.path().join("tasks.json"),
        Arc::new(crate::AppState::new())
    )
    .await
    .is_err());
    drop(runtime);
    assert!(CoordinationRuntime::open(
        root.path().join("tasks.json"),
        Arc::new(crate::AppState::new())
    )
    .await
    .is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn task_authority_unlocks_only_after_the_last_owner_even_with_an_inherited_descriptor() {
    let root = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path(), vec![]).await;
    let retained = runtime.clone();
    // dup retains the same file description as fork inheritance, without an
    // unsafe fork in this multithreaded test process.
    let inherited = runtime.0._lock.0.try_clone().unwrap();
    let path = root.path().join("tasks.json");
    drop(runtime);
    assert!(
        CoordinationRuntime::open(path.clone(), Arc::new(crate::AppState::new()))
            .await
            .is_err()
    );
    drop(retained);
    let reopened = CoordinationRuntime::open(path.clone(), Arc::new(crate::AppState::new()))
        .await
        .unwrap();
    drop(inherited);
    assert!(
        CoordinationRuntime::open(path.clone(), Arc::new(crate::AppState::new()))
            .await
            .is_err()
    );
    drop(reopened);
    assert!(
        CoordinationRuntime::open(path, Arc::new(crate::AppState::new()))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn stale_spec_revision_cannot_overwrite_a_newer_specification() {
    let root = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path(), vec![]).await;
    let project_path = root.path().to_string_lossy().into_owned();
    runtime
        .command(CoordinationCommand::SaveSpec {
            id: None,
            project_path: project_path.clone(),
            title: "Contract".into(),
            content: "Keep the original behavior".into(),
            expected_revision: 0,
        })
        .await
        .unwrap();
    let saved = runtime.snapshot().await.specs[0].clone();
    assert!(runtime
        .command(CoordinationCommand::SaveSpec {
            id: Some(saved.id),
            project_path,
            title: "Contract".into(),
            content: "Stale edits".into(),
            expected_revision: 0
        })
        .await
        .is_err());
    assert_eq!(
        runtime.snapshot().await.specs[0].content,
        "Keep the original behavior"
    );
    drop(runtime);
}

#[tokio::test]
async fn a_side_chat_reads_the_parent_worktree_and_uses_a_frozen_source_point() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    let mut parent = fixture(None, TaskState::Completed);
    parent.project_path = project
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    parent.working_path = worktree.path().to_string_lossy().into_owned();
    let parent_id = parent.id.clone();
    let runtime = runtime(root.path(), vec![parent.clone()]).await;
    let permits = runtime
        .0
        .slots
        .acquire_many(u32::try_from(runtime.0.capacity).unwrap())
        .await
        .unwrap();
    let id = runtime
        .create(
            NewTask {
                title: "Question".into(),
                project_path: parent.project_path,
                prompt: "Explain the current changes".into(),
                model: parent.model,
                permission_mode: PermissionMode::Plan,
                isolation: Isolation::Shared,
                spec_id: None,
            },
            Some(parent_id.clone()),
            Relationship::SideChat,
        )
        .await
        .unwrap();
    {
        let mut data = runtime.0.data.lock().await;
        data.tasks
            .get_mut(&parent_id)
            .unwrap()
            .messages
            .push(TaskMessage {
                role: "assistant".into(),
                text: "Newer answer".into(),
                at: EpochMs::now(),
            });
        drop(data);
    }
    let child = runtime.task(&id).await.unwrap();
    runtime.cancel(&id).await.unwrap();
    drop(permits);
    drop(runtime);
    assert_eq!(child.working_path, parent.working_path);
    assert_eq!(child.source_context.len(), 1);
    assert!(child.source_revision.is_some());
}

#[test]
fn an_approval_answer_cannot_resurrect_a_task_that_completed_concurrently() {
    for state in [
        TaskState::Completed,
        TaskState::Cancelled,
        TaskState::WaitingChild,
    ] {
        let mut task = fixture(None, state);
        task.approvals.push(PendingApproval {
            id: "approval".into(),
            tool: "fixture".into(),
            input: serde_json::json!({}),
        });
        answered_approval(&mut task, "approval");
        assert!(task.approvals.is_empty());
        assert_eq!(task.state, state);
    }
    let mut task = fixture(None, TaskState::WaitingApproval);
    answered_approval(&mut task, "approval");
    assert_eq!(task.state, TaskState::Running);
}

#[tokio::test]
async fn nested_delegation_yields_slots_and_restores_each_parent_on_a_one_slot_host() {
    let root = tempfile::tempdir().unwrap();
    let parent = fixture(None, TaskState::Completed);
    let child = fixture(Some(parent.id.clone()), TaskState::Completed);
    let grandchild = fixture(Some(child.id.clone()), TaskState::Completed);
    let ids = [parent.id.clone(), child.id.clone(), grandchild.id.clone()];
    let mut runtime = runtime(root.path(), vec![parent, child, grandchild]).await;
    Arc::get_mut(&mut runtime.0).unwrap().slots = Arc::new(Semaphore::new(1));
    {
        let mut data = runtime.0.data.lock().await;
        for id in &ids {
            data.tasks.get_mut(id).unwrap().state = TaskState::Running;
        }
        drop(data);
    }
    for id in &ids {
        runtime
            .0
            .cancels
            .lock()
            .await
            .insert(id.clone(), watch::channel(false).0);
    }
    let parent_slot = wait::RunSlot::hold(
        runtime.clone(),
        ids[0].clone(),
        runtime.0.slots.clone().acquire_owned().await.unwrap(),
    )
    .unwrap();
    let first = {
        let runtime = runtime.clone();
        let ids = ids.clone();
        tokio::spawn(async move { runtime.wait_for_child(&ids[0], &ids[1]).await })
    };
    let child_permit = tokio::time::timeout(
        crate::capture_client::REQUEST_TIMEOUT,
        runtime.0.slots.clone().acquire_owned(),
    )
    .await
    .unwrap()
    .unwrap();
    let child_slot = wait::RunSlot::hold(runtime.clone(), ids[1].clone(), child_permit).unwrap();
    let second = {
        let runtime = runtime.clone();
        let ids = ids.clone();
        tokio::spawn(async move { runtime.wait_for_child(&ids[1], &ids[2]).await })
    };
    let grandchild_permit = tokio::time::timeout(
        crate::capture_client::REQUEST_TIMEOUT,
        runtime.0.slots.clone().acquire_owned(),
    )
    .await
    .unwrap()
    .unwrap();
    let grandchild_slot =
        wait::RunSlot::hold(runtime.clone(), ids[2].clone(), grandchild_permit).unwrap();
    runtime
        .finish(&ids[2], TaskState::Completed, None)
        .await
        .unwrap();
    drop(grandchild_slot);
    assert_eq!(second.await.unwrap().unwrap().state, TaskState::Completed);
    assert_eq!(
        runtime.task(&ids[1]).await.unwrap().state,
        TaskState::Running
    );
    assert_eq!(runtime.0.slots.available_permits(), 0);
    runtime
        .finish(&ids[1], TaskState::Completed, None)
        .await
        .unwrap();
    drop(child_slot);
    assert_eq!(first.await.unwrap().unwrap().state, TaskState::Completed);
    assert_eq!(
        runtime.task(&ids[0]).await.unwrap().state,
        TaskState::Running
    );
    assert_eq!(runtime.0.slots.available_permits(), 0);
    drop(parent_slot);
    assert_eq!(runtime.0.slots.available_permits(), 1);
    drop(runtime);
}

#[tokio::test]
async fn disconnecting_a_parked_wait_stops_the_parent_instead_of_losing_its_slot() {
    let root = tempfile::tempdir().unwrap();
    let parent = fixture(None, TaskState::Completed);
    let child = fixture(Some(parent.id.clone()), TaskState::Completed);
    let parent_id = parent.id.clone();
    let child_id = child.id.clone();
    let runtime = runtime(root.path(), vec![parent, child]).await;
    {
        let mut data = runtime.0.data.lock().await;
        data.tasks.get_mut(&parent_id).unwrap().state = TaskState::Running;
        data.tasks.get_mut(&child_id).unwrap().state = TaskState::Running;
        drop(data);
    }
    let (cancel, mut cancelled) = watch::channel(false);
    runtime
        .0
        .cancels
        .lock()
        .await
        .insert(parent_id.clone(), cancel);
    let slot = wait::RunSlot::hold(
        runtime.clone(),
        parent_id.clone(),
        runtime.0.slots.clone().acquire_owned().await.unwrap(),
    )
    .unwrap();
    let mut changes = runtime.changes();
    let waiting = {
        let runtime = runtime.clone();
        let parent_id = parent_id.clone();
        tokio::spawn(async move { runtime.wait_for_child(&parent_id, &child_id).await })
    };
    loop {
        if runtime.task(&parent_id).await.unwrap().state == TaskState::WaitingChild {
            break;
        }
        changes.changed().await.unwrap();
    }
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    cancelled.changed().await.unwrap();
    assert!(*cancelled.borrow());
    loop {
        if runtime.task(&parent_id).await.unwrap().state == TaskState::Cancelled {
            break;
        }
        changes.changed().await.unwrap();
    }
    drop(slot);
    assert_eq!(runtime.0.slots.available_permits(), runtime.0.capacity);
    drop(runtime);
}
