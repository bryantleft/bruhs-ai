use super::*;
use bruhs_models::workbench_handoff::{work_item_draft, WorkItemHandoffRequest};
use bruhs_models::{WorkItem, WorkItemSource};

fn request(root: &Path) -> WorkItemHandoffRequest {
    let mut item = WorkItem::new_at("Ship the reviewed change", 42);
    item.source = Some(WorkItemSource::Meeting {
        meeting_id: "source-recording".into(),
        title: "Planning".into(),
    });
    item.description = Some("Use the accepted action and keep its source.".into());
    item.planning.time_block_event_ids = vec!["accepted-plan-block".into()];
    WorkItemHandoffRequest {
        item,
        project_path: root.to_string_lossy().into_owned(),
        model: "codex/default".into(),
        effort: Some("high".into()),
        permission_mode: PermissionMode::Plan,
    }
}

#[tokio::test]
async fn work_item_prepare_reopen_deduplicate_and_discard_never_create_a_run() {
    let root = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path(), vec![]).await;
    let request = request(root.path());
    let handoff = runtime.prepare_work_item(request.clone()).await.unwrap();
    let copies = runtime.work_item_handoffs().await;
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].id, handoff.id);
    assert!(serde_json::to_value(&copies[0])
        .unwrap()
        .get("request")
        .is_none());
    assert_eq!(
        runtime.prepare_work_item(request.clone()).await.unwrap().id,
        handoff.id
    );
    assert!(runtime.snapshot().await.tasks.is_empty());
    assert!(runtime.0.cancels.lock().await.is_empty());
    drop(runtime);
    let reopened = CoordinationRuntime::open(
        root.path().join("tasks.json"),
        Arc::new(crate::AppState::new()),
    )
    .await
    .unwrap();
    assert_eq!(
        reopened
            .work_item_handoff(&handoff.id)
            .await
            .unwrap()
            .request,
        request
    );
    assert!(reopened.snapshot().await.tasks.is_empty());
    reopened
        .forget_work_item_handoff(&handoff.id)
        .await
        .unwrap();
    assert!(reopened
        .start_work_item(&handoff.id, work_item_draft(&request).unwrap())
        .await
        .is_err());
    assert!(reopened.snapshot().await.tasks.is_empty());
    drop(reopened);
}

#[tokio::test]
async fn work_item_start_is_one_durable_run_across_concurrency_retry_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime(root.path(), vec![]).await;
    // Hold workers at admission: this checks real creation/persistence without
    // allowing a model or CLI to run inside this unit test.
    Arc::get_mut(&mut runtime.0).unwrap().slots = Arc::new(Semaphore::new(0));
    let handoff = runtime
        .prepare_work_item(request(root.path()))
        .await
        .unwrap();
    let mut task = work_item_draft(&handoff.request).unwrap();
    task.prompt = "Owner's reviewed implementation request".into();
    let (a, b) = tokio::join!(
        runtime.start_work_item(&handoff.id, task.clone()),
        runtime.start_work_item(&handoff.id, task.clone())
    );
    let id = a.unwrap();
    assert_eq!(id, b.unwrap());
    let saved = runtime.task(&id).await.unwrap();
    assert_eq!(saved.messages[0].text, task.prompt);
    assert_eq!(saved.effort, task.effort);
    assert_eq!(saved.work_item_origin, Some(handoff.origin()));
    assert_eq!(runtime.snapshot().await.tasks.len(), 1);
    let mut changed_effort = task.clone();
    changed_effort.effort = Some("low".into());
    assert!(matches!(
        runtime.start_work_item(&handoff.id, changed_effort).await,
        Err(CoordinationError::Conflict(_))
    ));
    let mut changed = task.clone();
    changed.permission_mode = PermissionMode::AcceptEdits;
    assert!(matches!(
        runtime.start_work_item(&handoff.id, changed).await,
        Err(CoordinationError::Conflict(_))
    ));
    runtime.cancel(&id).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while !runtime.0.cancels.lock().await.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(runtime);
    let reopened = CoordinationRuntime::open(
        root.path().join("tasks.json"),
        Arc::new(crate::AppState::new()),
    )
    .await
    .unwrap();
    assert_eq!(
        reopened.start_work_item(&handoff.id, task).await.unwrap(),
        id
    );
    assert!(reopened.0.cancels.lock().await.is_empty());
    assert_eq!(
        reopened.task(&id).await.unwrap().effort,
        Some("high".into())
    );
    assert_eq!(
        reopened.task(&id).await.unwrap().work_item_origin,
        Some(handoff.origin())
    );
    let guest = reopened
        .scoped_snapshot(&AccessScope::Guest {
            task_id: id.clone(),
        })
        .await
        .unwrap();
    assert!(guest.tasks[0].work_item_origin.is_none());
    assert_eq!(reopened.snapshot().await.tasks.len(), 1);
    reopened
        .forget_work_item_handoff(&handoff.id)
        .await
        .unwrap();
    assert!(reopened.work_item_handoffs().await.is_empty());
    assert!(reopened
        .start_work_item(&handoff.id, work_item_draft(&handoff.request).unwrap())
        .await
        .is_err());
    assert_eq!(
        reopened.task(&id).await.unwrap().work_item_origin,
        Some(handoff.origin())
    );
    drop(reopened);
}

#[tokio::test]
async fn work_item_failed_start_cannot_consume_or_change_the_reviewed_source() {
    let root = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path(), vec![]).await;
    let handoff = runtime
        .prepare_work_item(request(root.path()))
        .await
        .unwrap();
    let mut task = work_item_draft(&handoff.request).unwrap();
    task.project_path = root
        .path()
        .join("missing-project")
        .to_string_lossy()
        .into_owned();
    assert!(runtime.start_work_item(&handoff.id, task).await.is_err());
    let retained = runtime.work_item_handoff(&handoff.id).await.unwrap();
    assert!(retained.started.is_none());
    assert_eq!(retained.request, handoff.request);
    assert!(runtime.snapshot().await.tasks.is_empty());
    drop(runtime);
}
