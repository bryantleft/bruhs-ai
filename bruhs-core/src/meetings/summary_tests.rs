use super::*;
use bruhs_models::{MeetingSummary, ObservationKind, PersonObservation, Speaker};

mod generation;

fn summary(text: &str) -> MeetingSummary {
    MeetingSummary {
        tldr: text.into(),
        action_items: vec![],
        decisions: vec![],
        topics: vec![],
        generated_by: "fixture".into(),
        generated_at: 42,
    }
}
async fn fixture(store: &dyn MeetingStore) -> (Meeting, Vec<TranscriptSegment>, PersonProfile) {
    let mut meeting = Meeting::new("Atomic summary", 0.0);
    meeting.id = "summary-meeting".into();
    meeting.summary = Some(summary("Original summary"));
    let mut person = PersonProfile::new("Alice");
    person.id = "summary-person".into();
    person.observations = vec![
        observation(&meeting.id, "Original observation"),
        observation("unrelated-meeting", "Unrelated observation"),
    ];
    let person = store.upsert_person(person).await.unwrap();
    meeting.speakers = vec![Speaker {
        id: "them".into(),
        label: "Alice".into(),
        person_id: Some(person.id.clone()),
        color: None,
    }];
    let meeting = store.create_meeting(meeting).await.unwrap();
    let transcript = vec![TranscriptSegment {
        id: "summary-segment".into(),
        meeting_id: meeting.id.clone(),
        speaker_id: Some("them".into()),
        start_ms: 0,
        end_ms: 1000,
        text: "Final speech".into(),
        confidence: 1.0,
        partial: false,
        speaker_confidence: None,
    }];
    store.append_segment(transcript[0].clone()).await.unwrap();
    (meeting, transcript, person)
}
fn observation(id: &str, text: &str) -> PersonObservation {
    PersonObservation {
        at_meeting_id: id.into(),
        kind: ObservationKind::Topic,
        text: text.into(),
        created_at: 1,
    }
}
fn update(meeting: &Meeting, transcript: &[TranscriptSegment]) -> SummaryUpdate {
    SummaryUpdate {
        expected: meeting.clone(),
        people_versions: std::collections::BTreeMap::from([("summary-person".into(), 0)]),
        transcript: transcript.to_vec(),
        summary: summary("New summary"),
        observations: HashMap::from([(
            "Alice".into(),
            vec![observation(&meeting.id, "New observation")],
        )]),
        replace_observations: true,
    }
}

async fn contract(store: &dyn MeetingStore) {
    let (meeting, transcript, person) = fixture(store).await;
    for invalid in 0..6 {
        let mut proposed = update(&meeting, &transcript);
        match invalid {
            0 => proposed.expected.title = "Stale meeting".into(),
            1 => proposed.transcript[0].text = "Stale speech".into(),
            2 => proposed.summary.tldr.clear(),
            3 => {
                proposed.observations.insert(
                    "Invented speaker".into(),
                    vec![observation(&meeting.id, "Invalid")],
                );
            }
            4 => {
                proposed.observations.get_mut("Alice").unwrap()[0].at_meeting_id =
                    "foreign-meeting".into()
            }
            _ => proposed.expected.skip_observations = true,
        }
        assert!(
            store.commit_summary(proposed).await.is_err(),
            "invalid scenario {invalid}"
        );
        assert_eq!(
            store.get_person(&person.id).await.unwrap(),
            Some(person.clone())
        );
        assert_eq!(
            store.get_meeting(&meeting.id).await.unwrap(),
            Some(meeting.clone())
        );
    }
    let mut proposed = update(&meeting, &transcript);
    proposed.people_versions = store.summary_people_versions(&meeting).await.unwrap();
    let saved = store.commit_summary(proposed).await.unwrap();
    assert_eq!(saved.summary, Some(summary("New summary")));
    let updated = store.get_person(&person.id).await.unwrap().unwrap();
    assert_eq!(
        updated.observations,
        vec![
            observation("unrelated-meeting", "Unrelated observation"),
            observation(&meeting.id, "New observation")
        ]
    );
    assert!(updated.updated_at > person.updated_at);
    // A concurrent generation based on the same inputs cannot append a duplicate.
    assert!(store
        .commit_summary(update(&meeting, &transcript))
        .await
        .is_err());
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(updated));
}

#[tokio::test]
async fn memory_summary_commit_is_atomic_and_rejects_stale_generation() {
    contract(&InMemoryMeetingStore::new()).await;
}
#[tokio::test]
async fn sqlite_summary_commit_is_atomic_and_rejects_stale_generation() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("summary.db"))
        .await
        .unwrap();
    contract(&store).await;
}
#[tokio::test]
async fn sqlite_late_summary_write_failure_rolls_back_observation_replacement_and_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("summary.db");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let (meeting, transcript, person) = fixture(&store).await;
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_summary BEFORE UPDATE ON meetings BEGIN SELECT RAISE(ABORT,'fixture late summary failure'); END;").unwrap();
    assert!(store
        .commit_summary(update(&meeting, &transcript))
        .await
        .is_err());
    drop(connection);
    drop(store);
    let reopened = SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(reopened.get_person(&person.id).await.unwrap(), Some(person));
    assert_eq!(
        reopened.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting)
    );
}

async fn tracking_contract(store: &dyn MeetingStore) {
    let (meeting, transcript, person) = fixture(store).await;
    let untracked = store
        .patch_person_tracked(&person.id.parse().unwrap(), false)
        .await
        .unwrap();
    let mut proposed = update(&meeting, &transcript);
    proposed.people_versions = store.summary_people_versions(&meeting).await.unwrap();
    let saved = store.commit_summary(proposed).await.unwrap();
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(untracked));
    let opted_out = store
        .patch_meeting_skip_observations(&saved.id.parse().unwrap(), true)
        .await
        .unwrap();
    assert!(store
        .commit_summary(update(&opted_out, &transcript))
        .await
        .is_err());
    assert_eq!(
        store.get_meeting(&meeting.id).await.unwrap(),
        Some(opted_out)
    );
}
#[tokio::test]
async fn memory_summary_respects_current_tracking_and_meeting_opt_out() {
    tracking_contract(&InMemoryMeetingStore::new()).await;
}
#[tokio::test]
async fn sqlite_summary_respects_current_tracking_and_meeting_opt_out() {
    let directory = tempfile::tempdir().unwrap();
    tracking_contract(
        &SqliteMeetingStore::open(directory.path().join("summary.db"))
            .await
            .unwrap(),
    )
    .await;
}

async fn participant_conflict(store: &dyn MeetingStore) {
    let (meeting, transcript, person) = fixture(store).await;
    let proposed = update(&meeting, &transcript);
    let renamed = store
        .rename_person(&person.id.parse().unwrap(), "New name".into())
        .await
        .unwrap();
    assert!(store.commit_summary(proposed).await.is_err());
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(renamed));
    assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), Some(meeting));
}
#[tokio::test]
async fn memory_participant_change_during_generation_preserves_newer_contact() {
    participant_conflict(&InMemoryMeetingStore::new()).await;
}
#[tokio::test]
async fn sqlite_participant_change_during_generation_preserves_newer_contact() {
    let directory = tempfile::tempdir().unwrap();
    participant_conflict(
        &SqliteMeetingStore::open(directory.path().join("summary.db"))
            .await
            .unwrap(),
    )
    .await;
}
