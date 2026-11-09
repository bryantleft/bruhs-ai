use super::*;
use bruhs_models::{SummaryGenerationRequest, SummaryGenerationState};

fn request(meeting: &Meeting) -> SummaryGenerationRequest {
    SummaryGenerationRequest::new(meeting.id.clone(), true, true)
}

async fn proposed(
    store: &dyn MeetingStore,
    meeting: &Meeting,
    transcript: &[TranscriptSegment],
) -> SummaryUpdate {
    let mut result = update(meeting, transcript);
    result.people_versions = store.summary_people_versions(meeting).await.unwrap();
    result
}

async fn reservation_contract(store: &dyn MeetingStore) {
    let (meeting, transcript, person) = fixture(store).await;
    let first = request(&meeting);
    assert!(matches!(
        store.begin_summary_generation(first.clone()).await.unwrap(),
        GenerationReservation::Started
    ));
    assert!(matches!(
        store.begin_summary_generation(first.clone()).await.unwrap(),
        GenerationReservation::Existing(_)
    ));
    assert!(matches!(
        store.begin_summary_generation(request(&meeting)).await,
        Err(MeetingStoreError::Conflict(_))
    ));
    let mut mismatched = first.clone();
    mismatched.extract_observations = false;
    assert!(matches!(
        store.begin_summary_generation(mismatched).await,
        Err(MeetingStoreError::Conflict(_))
    ));
    assert!(store
        .summary_generation("another-meeting", &first.request_id)
        .await
        .unwrap()
        .is_none());
    let stale = proposed(store, &meeting, &transcript).await;
    let cancelled = store
        .finish_summary_generation(&meeting.id, &first.request_id, GenerationOutcome::Cancelled)
        .await
        .unwrap();
    assert_eq!(cancelled.state, SummaryGenerationState::Cancelled);
    let second = request(&meeting);
    assert!(matches!(
        store
            .begin_summary_generation(second.clone())
            .await
            .unwrap(),
        GenerationReservation::Started
    ));
    assert!(matches!(
        store
            .commit_summary_generation(&first.request_id, stale)
            .await,
        Err(MeetingStoreError::Conflict(_))
    ));
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(person));
    assert_eq!(
        store.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting.clone())
    );
    let next = proposed(store, &meeting, &transcript).await;
    let expected = next.summary.clone();
    store
        .commit_summary_generation(&second.request_id, next)
        .await
        .unwrap();
    let receipt = store
        .summary_generation(&meeting.id, &second.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.state,
        SummaryGenerationState::Succeeded { summary: expected }
    );
    // A late cancellation/failure is not evidence that a committed result disappeared.
    assert_eq!(
        store
            .finish_summary_generation(
                &meeting.id,
                &second.request_id,
                GenerationOutcome::Cancelled
            )
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(
        store
            .finish_summary_generation(&meeting.id, &second.request_id, GenerationOutcome::Failed)
            .await
            .unwrap(),
        receipt
    );
    assert!(
        matches!(store.begin_summary_generation(second.clone()).await.unwrap(), GenerationReservation::Existing(found) if found == receipt)
    );
    store
        .delete_meeting(&meeting.id.parse().unwrap())
        .await
        .unwrap();
    assert!(store
        .summary_generation(&meeting.id, &second.request_id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn in_memory_generation_reservations_fence_cancelled_results_and_replay_commits() {
    reservation_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_generation_reservations_fence_cancelled_results_and_replay_commits() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(dir.path().join("private.sqlite3"))
        .await
        .unwrap();
    reservation_contract(&store).await;
}

#[tokio::test]
async fn pending_and_succeeded_receipts_survive_reopen_without_automatic_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let (meeting, transcript, _) = fixture(&store).await;
    let first = request(&meeting);
    store.begin_summary_generation(first.clone()).await.unwrap();
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert!(
        matches!(store.begin_summary_generation(first.clone()).await.unwrap(), GenerationReservation::Existing(receipt) if receipt.state == SummaryGenerationState::Pending)
    );
    let next = proposed(&store, &meeting, &transcript).await;
    store
        .commit_summary_generation(&first.request_id, next)
        .await
        .unwrap();
    let receipt = store
        .summary_generation(&meeting.id, &first.request_id)
        .await
        .unwrap();
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(
        store
            .summary_generation(&meeting.id, &first.request_id)
            .await
            .unwrap(),
        receipt
    );
}

#[tokio::test]
async fn independent_sqlite_connections_reserve_only_one_generation_per_meeting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite3");
    let a = SqliteMeetingStore::open(&path).await.unwrap();
    let b = SqliteMeetingStore::open(&path).await.unwrap();
    let (meeting, _, _) = fixture(&a).await;
    let (a, b) = tokio::join!(
        a.begin_summary_generation(request(&meeting)),
        b.begin_summary_generation(request(&meeting))
    );
    assert!(matches!(
        (&a, &b),
        (
            Ok(GenerationReservation::Started),
            Err(MeetingStoreError::Conflict(_))
        ) | (
            Err(MeetingStoreError::Conflict(_)),
            Ok(GenerationReservation::Started)
        )
    ));
}

#[tokio::test]
async fn reservation_does_not_authorize_unrequested_observations() {
    let store = InMemoryMeetingStore::new();
    let (meeting, transcript, person) = fixture(&store).await;
    let request = SummaryGenerationRequest::new(meeting.id.clone(), false, false);
    store
        .begin_summary_generation(request.clone())
        .await
        .unwrap();
    let next = proposed(&store, &meeting, &transcript).await;
    assert!(matches!(
        store
            .commit_summary_generation(&request.request_id, next)
            .await,
        Err(MeetingStoreError::Conflict(_))
    ));
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(person));
    assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), Some(meeting));
}
