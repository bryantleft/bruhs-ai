use super::*;
use bruhs_core::InMemoryMeetingStore;
use bruhs_core::MeetingRecords;
use bruhs_core::PeopleStore;
use bruhs_models::{
    Meeting, ObservationKind, PersonObservation, PersonProfile, Speaker, TranscriptSegment,
};

mod generation;

async fn fixture() -> (
    InMemoryMeetingStore,
    Meeting,
    Vec<TranscriptSegment>,
    PersonProfile,
) {
    let store = InMemoryMeetingStore::new();
    let mut meeting = Meeting::new("Generation fixture", 0.0);
    let mut person = PersonProfile::new("Alice");
    person.observations.push(PersonObservation {
        at_meeting_id: meeting.id.clone(),
        kind: ObservationKind::Topic,
        text: "Original evidence".into(),
        created_at: 1,
    });
    store.upsert_person(person.clone()).await.unwrap();
    meeting.speakers.push(Speaker {
        id: "speaker".into(),
        label: "Alice".into(),
        person_id: Some(person.id.clone()),
        color: None,
    });
    let meeting = store.create_meeting(meeting).await.unwrap();
    let segments = vec![TranscriptSegment {
        id: "generation-segment".into(),
        meeting_id: meeting.id.clone(),
        speaker_id: Some("speaker".into()),
        start_ms: 0,
        end_ms: 100,
        text: "Final speech".into(),
        confidence: 1.0,
        partial: false,
        speaker_confidence: None,
    }];
    store.append_segment(segments[0].clone()).await.unwrap();
    (store, meeting, segments, person)
}
fn generated() -> MeetingSummary {
    MeetingSummary {
        tldr: "Generated summary".into(),
        action_items: vec![],
        decisions: vec![],
        topics: vec![],
        generated_by: "fixture".into(),
        generated_at: 1,
    }
}
fn replace() -> SummarizeBody {
    SummarizeBody {
        request_id: None,
        extract_observations: true,
        replace_observations: true,
    }
}

#[tokio::test]
async fn model_failure_retains_original_observations() {
    let (store, meeting, segments, person) = fixture().await;
    assert!(generate_and_commit(
        &store,
        meeting.clone(),
        segments,
        replace(),
        |_, _, _| async { Err(summarize::SummarizeError::NoResponse) }
    )
    .await
    .is_err());
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(person));
    assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), Some(meeting));
}
#[tokio::test]
async fn malformed_observation_cannot_become_empty_replacement() {
    let (store, meeting, segments, person) = fixture().await;
    assert!(generate_and_commit(
        &store,
        meeting.clone(),
        segments,
        replace(),
        |_, _, _| async {
            Ok((
                generated(),
                vec![summarize::parser::PersonObservationRaw {
                    speaker_label: "Alice".into(),
                    kind: "invented_kind".into(),
                    text: "Unknown kind".into(),
                }],
            ))
        }
    )
    .await
    .is_err());
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(person));
    assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), Some(meeting));
}
#[tokio::test]
async fn opt_out_discards_unrequested_model_observations_without_deleting_prior_ones() {
    let (store, meeting, segments, person) = fixture().await;
    let meeting = store
        .patch_meeting_skip_observations(&meeting.id.parse().unwrap(), true)
        .await
        .unwrap();
    let saved = generate_and_commit(
        &store,
        meeting,
        segments,
        replace(),
        |_, _, extract| async move {
            assert!(!extract);
            Ok((
                generated(),
                vec![summarize::parser::PersonObservationRaw {
                    speaker_label: "Alice".into(),
                    kind: "topic".into(),
                    text: "Unrequested".into(),
                }],
            ))
        },
    )
    .await
    .unwrap();
    assert_eq!(saved, generated());
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(person));
}
#[tokio::test]
async fn changed_speaker_during_generation_rejects_result_without_side_effects() {
    let (store, meeting, segments, person) = fixture().await;
    let store_ref = &store;
    let id = meeting.id.clone();
    let result = generate_and_commit(&store, meeting, segments, replace(), |_, _, _| async move {
        store_ref
            .patch_speaker_label(
                &id.parse().unwrap(),
                &"speaker".parse().unwrap(),
                "Renamed".into(),
            )
            .await
            .unwrap();
        Ok((generated(), vec![]))
    })
    .await;
    assert!(matches!(result, Err(AppError::Conflict(_))));
    assert_eq!(store.get_person(&person.id).await.unwrap(), Some(person));
}
#[tokio::test]
async fn partial_transcript_rejects_before_calling_model() {
    let (store, meeting, mut segments, _) = fixture().await;
    segments[0].partial = true;
    let result = generate_and_commit(&store, meeting, segments, replace(), |_, _, _| async {
        panic!("Model must not be called for a partial transcript")
    })
    .await;
    assert!(matches!(result, Err(AppError::BadRequest(_))));
}
