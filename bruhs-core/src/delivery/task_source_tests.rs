use super::*;
use bruhs_models::{
    coordination::{CoordinatedTask, Relationship, TaskState},
    ids::ThreadId,
    workbench_handoff::WorkItemTaskOrigin,
    EpochMs, PermissionMode, WorkItemSource,
};

fn task(project: &Path, working: &Path) -> CoordinatedTask {
    CoordinatedTask {
        id: ThreadId::new_v4(),
        work_item_origin: Some(WorkItemTaskOrigin {
            handoff_id: ThreadId::new_v4(),
            item_id: "personal-task".into(),
            item_title: "The owner's action item".into(),
            item_updated_at: 42,
            source: Some(WorkItemSource::Meeting {
                meeting_id: "planning-meeting".into(),
                title: "Private planning <img src=x>".into(),
            }),
            time_block_event_ids: vec!["calendar-block".into()],
        }),
        parent_id: None,
        source_context: Vec::new(),
        source_revision: None,
        relationship: Relationship::Root,
        title: "Review the completed task".into(),
        project_path: project.to_string_lossy().into_owned(),
        working_path: working.to_string_lossy().into_owned(),
        model: "codex/default".into(),
        effort: None,
        execution_timeout_seconds: None,
        permission_mode: PermissionMode::AcceptEdits,
        spec_id: None,
        spec_revision: None,
        state: TaskState::Completed,
        messages: vec![bruhs_models::coordination::TaskMessage {
            role: "user".into(),
            text: "Private transcript is not part of a delivery source copy".into(),
            at: EpochMs::UNIX_EPOCH,
        }],
        live_output: String::new(),
        error: None,
        session_id: Some("private-provider-session".into()),
        approvals: Vec::new(),
        created_at: EpochMs::UNIX_EPOCH,
        updated_at: EpochMs::UNIX_EPOCH,
        resume_at: None,
        auto_resume_on_limit: false,
        limit_reported: false,
        archived_at: None,
        removed_worktree: None,
    }
}

#[tokio::test]
async fn task_delivery_uses_the_actual_worktree_and_persists_only_source_identity() {
    let project = tests::repository();
    let directory = tempfile::tempdir().unwrap();
    let worktree = directory.path().join("task-worktree");
    let added = std::process::Command::new("git")
        .args(["worktree", "add", "-b", "bruhs/delivery-test"])
        .arg(&worktree)
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(added.status.success(), "{:?}", added);
    let task = task(project.path(), &worktree);
    let original_task = serde_json::to_value(&task).unwrap();
    let journal = directory.path().join("delivery");
    let service = DeliveryService::open(&journal).await.unwrap();
    let context = service.task_context(&task).await.unwrap();
    assert_eq!(Path::new(&context.path), worktree.canonicalize().unwrap());
    assert_eq!(context.branch, "bruhs/delivery-test");
    assert_eq!(context.source, TaskDeliverySource::from(&task));
    let action = DeliveryAction::CreatePull {
        title: "Owner's reviewed title".into(),
        body: "Owner's reviewed public description".into(),
        base: "main".into(),
        head: context.branch.clone(),
        draft: true,
    };
    let ordinary = service
        .prepare(PrepareRequest {
            path: context.path.clone(),
            forge: Forge::GitHub,
            action: action.clone(),
            timeout_seconds: 5,
        })
        .await
        .unwrap();
    let prepared = service
        .prepare_task(
            &task,
            TaskPrepareRequest {
                forge: Forge::GitHub,
                action,
                timeout_seconds: 5,
            },
        )
        .await
        .unwrap();
    assert_eq!(prepared.path, context.path);
    assert_eq!(prepared.commands, ordinary.commands);
    assert_eq!(prepared.expected_head, context.head);
    assert_eq!(prepared.source_task, Some(TaskDeliverySource::from(&task)));
    assert!(service
        .snapshot()
        .await
        .actions
        .iter()
        .all(|receipt| receipt.status == ActionStatus::Prepared));
    let source_json = serde_json::to_string(&prepared.source_task).unwrap();
    assert!(!source_json.contains("Private transcript"));
    assert!(!source_json.contains("private-provider-session"));
    assert!(!format!("{:?}", prepared.commands).contains("planning-meeting"));
    assert_eq!(serde_json::to_value(&task).unwrap(), original_task);
    drop(service);
    let reopened = DeliveryService::open(&journal).await.unwrap();
    let snapshot = reopened.snapshot().await;
    let retained = snapshot
        .actions
        .iter()
        .find(|receipt| receipt.prepared.id == prepared.id)
        .unwrap();
    assert_eq!(retained.prepared, prepared);
    let forgotten = reopened.forget_task_source(&prepared.id).await.unwrap();
    assert!(forgotten.prepared.source_task.is_none());
    assert_eq!(forgotten.prepared.commands, prepared.commands);
    assert_eq!(forgotten.prepared.action, prepared.action);
    assert_eq!(forgotten.prepared.expected_head, prepared.expected_head);
    assert_eq!(forgotten.status, ActionStatus::Prepared);
    assert!(reopened
        .forget_task_source(&prepared.id)
        .await
        .unwrap()
        .prepared
        .source_task
        .is_none());
    drop(reopened);
    let reopened = DeliveryService::open(&journal).await.unwrap();
    assert!(reopened
        .snapshot()
        .await
        .actions
        .iter()
        .find(|receipt| receipt.prepared.id == prepared.id)
        .unwrap()
        .prepared
        .source_task
        .is_none());
}

#[tokio::test]
async fn source_removal_refuses_a_running_action_and_legacy_proposals_remain_readable() {
    let repository = tests::repository();
    let directory = tempfile::tempdir().unwrap();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    let task = task(repository.path(), repository.path());
    let prepared = service
        .prepare_task(
            &task,
            TaskPrepareRequest {
                forge: Forge::GitHub,
                action: DeliveryAction::Comment {
                    number: 42,
                    body: "Explicitly reviewed text".into(),
                },
                timeout_seconds: 5,
            },
        )
        .await
        .unwrap();
    let mut legacy = serde_json::to_value(&prepared).unwrap();
    legacy.as_object_mut().unwrap().remove("source_task");
    assert!(serde_json::from_value::<PreparedAction>(legacy)
        .unwrap()
        .source_task
        .is_none());
    service
        .mutate(|state| {
            state.actions[0].status = ActionStatus::Running;
            Ok(())
        })
        .await
        .unwrap();
    assert!(matches!(
        service.forget_task_source(&prepared.id).await,
        Err(DeliveryError::Conflict(_))
    ));
    assert!(service.snapshot().await.actions[0]
        .prepared
        .source_task
        .is_some());
}
