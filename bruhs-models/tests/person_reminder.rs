use bruhs_models::{
    PersonReminderReceipt, PersonReminderRollback, RelationshipItem, RelationshipItemKind,
    RelationshipProfile, WorkflowOperation, WorkflowStep, WorkflowStepStatus,
};

#[test]
fn legacy_name_only_reminder_loads_but_cannot_be_executed() {
    let operation: WorkflowOperation = serde_json::from_str(
        r#"{"type":"create_person_reminder","person":"Alex","reminder":"Send brief"}"#,
    )
    .unwrap();
    assert!(!operation.is_valid());
}

fn operation() -> WorkflowOperation {
    WorkflowOperation::CreatePersonReminder {
        person_id: "exact-person".into(),
        person: "Alex".into(),
        reminder: "  Send brief 🗒  ".into(),
    }
}

fn original() -> RelationshipProfile {
    let mut profile = RelationshipProfile::new("exact-person", 3);
    profile.notes = "Private existing context".into();
    profile.company = "Existing company".into();
    profile.add_item(
        RelationshipItem::new(RelationshipItemKind::Commitment, "Keep commitment", 4),
        4,
    );
    profile
}

fn step(receipt: &PersonReminderReceipt) -> WorkflowStep {
    WorkflowStep {
        operation: operation(),
        status: WorkflowStepStatus::Completed,
        target_id: Some(receipt.target_id().into()),
        target_snapshot: Some(serde_json::to_string(receipt).expect("valid reminder receipt")),
        error: None,
    }
}

#[test]
fn reminder_review_includes_exact_identity_and_full_text() {
    assert_eq!(
        operation().review_fields(|_| unreachable!()),
        vec![
            ("Person ID", "exact-person".into()),
            ("Person", "Alex".into()),
            ("Reminder", "  Send brief 🗒  ".into())
        ]
    );
}

#[test]
fn consecutive_reminder_undo_restores_existing_profile_exactly() {
    let original = original();
    let first = PersonReminderReceipt::prepare(&operation(), Some(&original), 10).unwrap();
    let second = PersonReminderReceipt::prepare(&operation(), Some(first.after()), 20).unwrap();
    let steps = [step(&first), step(&second)];
    let mut profiles = vec![
        second.into_after(),
        RelationshipProfile::new("unrelated", 30),
    ];
    let undo = PersonReminderRollback::prepare(&steps, &profiles).unwrap();
    assert_eq!(undo.snapshot(first.target_id()), steps[0].target_snapshot);
    undo.apply(&mut profiles);
    assert_eq!(
        profiles,
        vec![original, RelationshipProfile::new("unrelated", 30)]
    );
}

#[test]
fn undo_removes_only_the_profile_created_by_the_workflow() {
    let receipt = PersonReminderReceipt::prepare(&operation(), None, 10).unwrap();
    let steps = [step(&receipt)];
    let mut profiles = vec![receipt.into_after(), original()];
    // A duplicate key is never silently resolved to the first profile.
    assert!(PersonReminderRollback::prepare(&steps, &profiles).is_err());
    profiles.pop();
    PersonReminderRollback::prepare(&steps, &profiles)
        .unwrap()
        .apply(&mut profiles);
    assert!(profiles.is_empty());
}

#[test]
fn relationship_edits_and_completed_reminders_block_undo_without_mutation() {
    let receipt = PersonReminderReceipt::prepare(&operation(), Some(&original()), 10).unwrap();
    for field in ["notes", "completion", "timestamp"] {
        let mut profile = receipt.after().clone();
        match field {
            "notes" => profile.notes.push_str(" changed"),
            "completion" => profile.items[0].completed_at_ms = Some(11),
            _ => profile.updated_at_ms += 1,
        }
        let profiles = vec![profile];
        assert!(
            PersonReminderRollback::prepare(&[step(&receipt)], &profiles).is_err(),
            "{field}"
        );
    }
}

#[test]
fn receipt_cannot_smuggle_a_profile_edit_or_retarget_another_person() {
    let receipt = PersonReminderReceipt::prepare(&operation(), Some(&original()), 10).unwrap();
    let mut encoded = serde_json::to_value(&receipt).unwrap();
    encoded["after"]["notes"] = "Unreviewed overwrite".into();
    let mut forged = step(&receipt);
    forged.target_snapshot = Some(encoded.to_string());
    let altered: PersonReminderReceipt = serde_json::from_value(encoded).unwrap();
    assert!(PersonReminderRollback::prepare(&[forged], &[altered.into_after()]).is_err());
    let mut wrong_person = original();
    wrong_person.person_id = "someone-else".into();
    assert!(PersonReminderReceipt::prepare(&operation(), Some(&wrong_person), 10).is_err());
}

#[test]
fn changed_or_duplicate_target_receipts_are_rejected() {
    let receipt = PersonReminderReceipt::prepare(&operation(), None, 10).unwrap();
    let mut wrong = step(&receipt);
    wrong.target_id = Some("wrong-id".into());
    assert!(PersonReminderRollback::prepare(&[wrong], &[receipt.after().clone()]).is_err());
    assert!(PersonReminderRollback::prepare(
        &[step(&receipt), step(&receipt)],
        &[receipt.into_after()]
    )
    .is_err());
}
