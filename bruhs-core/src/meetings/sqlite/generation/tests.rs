use super::*;
use crate::meetings::GenerationOutcome;
use crate::meetings::MeetingRecords;
use crate::meetings::PeopleStore;
use crate::meetings::SummaryGenerationStore;
use bruhs_models::{
    MeetingSummary, ObservationKind, PersonObservation, PersonProfile, Speaker, TranscriptSegment,
};

#[tokio::test]
async fn receipt_write_failure_rolls_back_summary_and_observations_together() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(dir.path().join("private.sqlite3"))
        .await
        .unwrap();
    let person = PersonProfile::new("Alice");
    store.upsert_person(person.clone()).await.unwrap();
    let mut meeting = Meeting::new("Atomic receipt", 0.0);
    meeting.speakers = vec![Speaker {
        id: "speaker".into(),
        label: "Alice".into(),
        person_id: Some(person.id.clone()),
        color: None,
    }];
    store.create_meeting(meeting.clone()).await.unwrap();
    let segment = TranscriptSegment {
        id: "speech".into(),
        meeting_id: meeting.id.clone(),
        speaker_id: Some("speaker".into()),
        start_ms: 0,
        end_ms: 100,
        text: "Final".into(),
        confidence: 1.0,
        partial: false,
        speaker_confidence: None,
    };
    store.append_segment(segment.clone()).await.unwrap();
    let request = SummaryGenerationRequest::new(meeting.id.clone(), true, true);
    store
        .begin_summary_generation(request.clone())
        .await
        .unwrap();
    store.call(|connection| {
        connection.execute_batch("CREATE TRIGGER fail_generation_receipt BEFORE UPDATE ON summary_generations BEGIN SELECT RAISE(ABORT,'owned receipt failure'); END;")?;
        Ok(())
    }).await.unwrap();
    let proposed = SummaryUpdate {
        expected: meeting.clone(),
        people_versions: store.summary_people_versions(&meeting).await.unwrap(),
        transcript: vec![segment],
        summary: MeetingSummary {
            tldr: "New summary".into(),
            action_items: vec![],
            decisions: vec![],
            topics: vec![],
            generated_by: "fixture".into(),
            generated_at: 1,
        },
        observations: std::collections::HashMap::from([(
            "Alice".into(),
            vec![PersonObservation {
                at_meeting_id: meeting.id.clone(),
                kind: ObservationKind::Topic,
                text: "New observation".into(),
                created_at: 1,
            }],
        )]),
        replace_observations: true,
    };
    assert!(store
        .commit_summary_generation(&request.request_id, proposed)
        .await
        .is_err());
    assert_eq!(
        store.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting.clone())
    );
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(person));
    assert_eq!(
        store
            .summary_generation(&meeting.id, &request.request_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        SummaryGenerationState::Pending
    );
}

#[tokio::test]
async fn schema_one_upgrade_preserves_existing_records_and_adds_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = Meeting::new("Preserved", 0.0);
    store.create_meeting(meeting.clone()).await.unwrap();
    // Reconstruct the prior schema in this owned fixture, not a user database.
    store
        .call(|connection| {
            connection.execute_batch("DROP TABLE segment_removals; DROP TABLE local_focus; DROP TABLE speaker_link_jobs; DROP TABLE summary_generations; PRAGMA user_version=1;")?;
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(
        store.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting.clone())
    );
    store
        .begin_summary_generation(SummaryGenerationRequest::new(
            meeting.id.clone(),
            false,
            false,
        ))
        .await
        .unwrap();
    assert!(store
        .pending_summary_generation(&meeting.id)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn corrupt_receipt_cannot_be_read_replayed_or_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(dir.path().join("private.sqlite3"))
        .await
        .unwrap();
    let meeting = Meeting::new("Preserved", 0.0);
    store.create_meeting(meeting.clone()).await.unwrap();
    let request = SummaryGenerationRequest::new(meeting.id.clone(), false, false);
    store
        .begin_summary_generation(request.clone())
        .await
        .unwrap();
    store
        .call(|connection| {
            connection.execute(
                "UPDATE summary_generations SET payload='corrupt original'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(store
        .summary_generation(&meeting.id, &request.request_id)
        .await
        .is_err());
    assert!(store
        .begin_summary_generation(request.clone())
        .await
        .is_err());
    assert!(store
        .finish_summary_generation(
            &meeting.id,
            &request.request_id,
            GenerationOutcome::Cancelled
        )
        .await
        .is_err());
    let original: String = store
        .call(|connection| {
            Ok(
                connection.query_row("SELECT payload FROM summary_generations", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert_eq!(original, "corrupt original");
}

#[tokio::test]
async fn fence_tombstones_unsent_requests_and_never_downgrades_terminal_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(dir.path().join("private.sqlite3"))
        .await
        .unwrap();
    let meeting = Meeting::new("Fenced generation", 0.0);
    store.create_meeting(meeting.clone()).await.unwrap();

    // An identity the server has never seen becomes a cancelled tombstone.
    let unsent = SummaryGenerationRequest::new(meeting.id.clone(), true, false);
    let receipt = store
        .fence_summary_generation(unsent.clone())
        .await
        .unwrap();
    assert_eq!(receipt.request, unsent);
    assert_eq!(receipt.state, SummaryGenerationState::Cancelled);
    assert!(store
        .pending_summary_generation(&meeting.id)
        .await
        .unwrap()
        .is_none());
    // A late arrival of that request replays the tombstone instead of reserving.
    match store
        .begin_summary_generation(unsent.clone())
        .await
        .unwrap()
    {
        GenerationReservation::Existing(late) => {
            assert_eq!(late.state, SummaryGenerationState::Cancelled)
        }
        GenerationReservation::Started => panic!("fenced request must not start"),
    }
    let mut changed = unsent.clone();
    changed.extract_observations = false;
    assert!(store
        .begin_summary_generation(changed.clone())
        .await
        .is_err());
    assert!(store.fence_summary_generation(changed).await.is_err());

    // A pending reservation is cancelled; repeating the fence is idempotent.
    let pending = SummaryGenerationRequest::new(meeting.id.clone(), false, false);
    store
        .begin_summary_generation(pending.clone())
        .await
        .unwrap();
    for _ in 0..2 {
        let receipt = store
            .fence_summary_generation(pending.clone())
            .await
            .unwrap();
        assert_eq!(receipt.state, SummaryGenerationState::Cancelled);
    }
    assert!(store
        .pending_summary_generation(&meeting.id)
        .await
        .unwrap()
        .is_none());

    // A terminal receipt keeps its state.
    let failed = SummaryGenerationRequest::new(meeting.id.clone(), false, false);
    store
        .begin_summary_generation(failed.clone())
        .await
        .unwrap();
    store
        .finish_summary_generation(&meeting.id, &failed.request_id, GenerationOutcome::Failed)
        .await
        .unwrap();
    let receipt = store.fence_summary_generation(failed).await.unwrap();
    assert_eq!(receipt.state, SummaryGenerationState::Failed);

    // Fencing requires the meeting to exist; nothing is written for unknown meetings.
    let foreign = SummaryGenerationRequest::new("missing-meeting".into(), true, false);
    assert!(store
        .fence_summary_generation(foreign.clone())
        .await
        .is_err());
    assert!(store
        .summary_generation("missing-meeting", &foreign.request_id)
        .await
        .unwrap()
        .is_none());
}
