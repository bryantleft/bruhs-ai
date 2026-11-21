use super::*;

fn request() -> WorkItemHandoffRequest {
    let mut item = WorkItem::new_at("Fix the meeting's export bug", 42);
    item.description = Some("Keep the CSV header and Unicode names intact.".into());
    item.source = Some(WorkItemSource::Meeting {
        meeting_id: "recording-42".into(),
        title: "Customer review".into(),
    });
    item.planning.time_block_event_ids = vec!["reviewed-calendar-block".into()];
    WorkItemHandoffRequest {
        item,
        project_path: String::new(),
        model: "codex/default".into(),
        effort: Some("high".into()),
        permission_mode: PermissionMode::Plan,
    }
}

#[test]
fn work_item_review_retains_source_and_settings_without_requiring_a_folder_yet() {
    let request = request();
    let original = request.clone();
    let task = work_item_draft(&request).unwrap();
    let context = task
        .prompt
        .split_once("Personal task snapshot (JSON):\n")
        .unwrap()
        .1;
    assert_eq!(
        serde_json::from_str::<WorkItem>(context).unwrap(),
        request.item
    );
    assert_eq!(task.model, request.model);
    assert_eq!(task.effort, request.effort);
    assert_eq!(task.permission_mode, request.permission_mode);
    assert!(crate::coordination::validate_new_task(&task).is_err());
    assert_eq!(request, original);
    let mut selected = task;
    selected.project_path = "/selected/project".into();
    crate::coordination::validate_new_task(&selected).unwrap();
}

#[test]
fn work_item_review_rejects_unusable_identity_model_and_excess_context() {
    let mut request = request();
    request.item.id.clear();
    assert!(work_item_draft(&request)
        .unwrap_err()
        .contains("Name and save"));
    request.item.id = "saved-task".into();
    request.model = " ".into();
    assert!(work_item_draft(&request).is_err());
    request.model = "codex/default".into();
    request.item.description = Some("x".repeat(crate::logic::composer_text::MESSAGE_CHAR_LIMIT));
    assert_eq!(
        work_item_draft(&request).unwrap_err(),
        crate::logic::composer_text::MESSAGE_LIMIT_HINT
    );
}
