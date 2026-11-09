use super::*;

fn checkpoint() -> Checkpoint {
    Checkpoint {
        id: "owned-meeting".into(),
        minimum_version: 20,
        request: SummaryGenerationRequest::new("owned-meeting".into(), true, false),
    }
}

fn receipt(checkpoint: &Checkpoint, state: SummaryGenerationState) -> SummaryGenerationReceipt {
    SummaryGenerationReceipt {
        request: checkpoint.request.clone(),
        state,
    }
}

#[test]
fn checkpoint_roundtrip_preserves_exact_request_and_version() {
    let checkpoint = checkpoint();
    let saved = serde_json::to_vec(&checkpoint).unwrap();
    let restored: Checkpoint = serde_json::from_slice(&saved).unwrap();
    restored.validate("owned-meeting").unwrap();
    assert_eq!(restored, checkpoint);
}

#[test]
fn checkpoint_requires_matching_storage_and_request_identities() {
    let checkpoint = checkpoint();
    assert!(checkpoint.validate("foreign-key").is_err());
    let mut foreign = checkpoint.clone();
    foreign.request.meeting_id = "foreign-meeting".into();
    assert!(foreign.validate(&checkpoint.id).is_err());
    let mut invalid = checkpoint.clone();
    invalid.request.request_id = "not-a-request-token".into();
    assert!(invalid.validate(&checkpoint.id).is_err());
}

#[test]
fn checkpoint_rejects_invalid_versions_and_observation_options() {
    let mut checkpoint = checkpoint();
    checkpoint.minimum_version = -1;
    assert!(checkpoint.validate(&checkpoint.id).is_err());
    checkpoint.minimum_version = 20;
    checkpoint.request.replace_observations = true;
    checkpoint.request.extract_observations = false;
    assert!(checkpoint.validate(&checkpoint.id).is_err());
}

#[test]
fn pending_receipt_never_authorizes_clearing_saved_uncertainty() {
    let checkpoint = checkpoint();
    assert!(checkpoint
        .confirm_terminal(&receipt(&checkpoint, SummaryGenerationState::Pending))
        .is_err());
}

#[test]
fn all_matching_terminal_receipts_allow_latest_meeting_readback() {
    let checkpoint = checkpoint();
    let summary = bruhs_models::MeetingSummary {
        tldr: "Original result; a newer meeting may have a different summary".into(),
        action_items: vec![],
        decisions: vec![],
        topics: vec![],
        generated_by: "synthetic".into(),
        generated_at: 21,
    };
    for state in [
        SummaryGenerationState::Failed,
        SummaryGenerationState::Cancelled,
        SummaryGenerationState::Succeeded { summary },
    ] {
        checkpoint
            .confirm_terminal(&receipt(&checkpoint, state))
            .unwrap();
    }
}

#[test]
fn terminal_receipt_requires_exact_request_and_observation_options() {
    let checkpoint = checkpoint();
    for field in 0..4 {
        let mut receipt = receipt(&checkpoint, SummaryGenerationState::Failed);
        match field {
            0 => receipt.request.request_id = uuid::Uuid::new_v4().to_string(),
            1 => receipt.request.meeting_id = "foreign".into(),
            2 => receipt.request.extract_observations = false,
            _ => receipt.request.replace_observations = true,
        }
        assert!(
            checkpoint.confirm_terminal(&receipt).is_err(),
            "field {field}"
        );
    }
}

#[test]
fn malformed_or_future_receipts_do_not_deserialize_as_terminal_success() {
    let checkpoint = checkpoint();
    for state in [
        serde_json::json!({"status":"succeeded"}),
        serde_json::json!({"status":"future-state"}),
        serde_json::json!({"status":"failed","unrecognized":true}),
    ] {
        let value = serde_json::json!({"request": checkpoint.request, "state":state});
        assert!(serde_json::from_value::<SummaryGenerationReceipt>(value).is_err());
    }
}

#[test]
fn incomplete_or_future_checkpoint_fields_fail_closed() {
    let checkpoint = checkpoint();
    let mut value = serde_json::to_value(&checkpoint).unwrap();
    value["unexpected"] = true.into();
    assert!(serde_json::from_value::<Checkpoint>(value).is_err());
    let mut value = serde_json::to_value(&checkpoint).unwrap();
    value.as_object_mut().unwrap().remove("request");
    assert!(serde_json::from_value::<Checkpoint>(value).is_err());
}
