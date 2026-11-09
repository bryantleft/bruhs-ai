use super::*;
use bruhs_models::{ObservationKind, PersonObservation};

#[tokio::test]
async fn merge_retry_returns_the_same_target_without_repeating_observations() {
    let store = InMemoryMeetingStore::new();
    let mut source = PersonProfile::new("Source");
    source.aliases.push("Older alias".into());
    source.tracked = false;
    source.observations.push(PersonObservation {
        at_meeting_id: "meeting".into(),
        kind: ObservationKind::Topic,
        text: "Original observation".into(),
        created_at: 1,
    });
    let target = PersonProfile::new("Target");
    store.upsert_person(source.clone()).await.unwrap();
    store.upsert_person(target.clone()).await.unwrap();
    let first = store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    let retry = store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(first, retry);
    assert_eq!(retry.observations, source.observations);
    assert!(retry.aliases.contains(&"Older alias".to_string()));
    assert!(!retry.tracked);
}

#[tokio::test]
async fn a_merged_source_cannot_be_recreated_or_retargeted_by_a_delayed_request() {
    let store = InMemoryMeetingStore::new();
    let source = PersonProfile::new("Source");
    let target = PersonProfile::new("Target");
    let other = PersonProfile::new("Other");
    for person in [&source, &target, &other] {
        store.upsert_person(person.clone()).await.unwrap();
    }
    store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    assert!(store.upsert_person(source.clone()).await.is_err());
    assert!(store
        .merge_person_into(&source.id.parse().unwrap(), &other.id.parse().unwrap())
        .await
        .is_err());
    assert_eq!(store.list_people().await.unwrap().len(), 2);
}

#[tokio::test]
async fn concurrent_merge_retries_commit_only_one_source_transfer() {
    let store = InMemoryMeetingStore::new();
    let source = PersonProfile::new("Source");
    let target = PersonProfile::new("Target");
    store.upsert_person(source.clone()).await.unwrap();
    store.upsert_person(target.clone()).await.unwrap();
    let (source_id, target_id) = (source.id.parse().unwrap(), target.id.parse().unwrap());
    let (first, second) = tokio::join!(
        store.merge_person_into(&source_id, &target_id),
        store.merge_person_into(&source_id, &target_id)
    );
    assert_eq!(first.unwrap(), second.unwrap());
}

#[tokio::test]
async fn a_delayed_full_profile_update_cannot_erase_a_completed_merge() {
    let store = InMemoryMeetingStore::new();
    let source = PersonProfile::new("Source");
    let target = PersonProfile::new("Target");
    store.upsert_person(source.clone()).await.unwrap();
    store.upsert_person(target.clone()).await.unwrap();
    let merged = store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    let mut stale = target;
    stale.display_name = "Delayed rename".into();
    stale.updated_at = merged.updated_at + 1;
    assert!(
        store.upsert_person(stale).await.is_err(),
        "A newer client clock is not proof that its full snapshot includes the merge"
    );
    assert_eq!(store.list_people().await.unwrap(), vec![merged]);
}

#[tokio::test]
async fn field_updates_advance_versions_even_when_client_clock_is_ahead() {
    let store = InMemoryMeetingStore::new();
    let mut person = PersonProfile::new("Future clock");
    person.updated_at = i64::MAX - 100;
    store.upsert_person(person.clone()).await.unwrap();
    let tracked = store
        .patch_person_tracked(&person.id.parse().unwrap(), false)
        .await
        .unwrap();
    assert!(tracked.updated_at > person.updated_at);
}

async fn observation_batch_failure_preserves_every_profile(store: &dyn MeetingStore) {
    let mut exhausted = PersonProfile::new("Exhausted");
    exhausted.updated_at = i64::MAX;
    let observation = PersonObservation {
        at_meeting_id: "remove-me".into(),
        kind: ObservationKind::Topic,
        text: "Keep until the entire mutation can commit".into(),
        created_at: 1,
    };
    exhausted.observations.push(observation.clone());
    let mut ordinary = PersonProfile::new("Ordinary");
    ordinary.observations.push(observation.clone());
    store.upsert_person(exhausted.clone()).await.unwrap();
    store.upsert_person(ordinary.clone()).await.unwrap();
    let mut batch = HashMap::new();
    batch.insert("Exhausted".into(), vec![observation.clone()]);
    batch.insert("Ordinary".into(), vec![observation.clone()]);
    batch.insert("Would be new".into(), vec![observation]);
    assert!(store.append_person_observations(batch).await.is_err());
    assert!(store
        .delete_observations_for_meeting(&"remove-me".parse().unwrap())
        .await
        .is_err());
    assert_eq!(store.list_people().await.unwrap().len(), 2);
    assert_eq!(
        store.get_person(&exhausted.id).await.unwrap(),
        Some(exhausted)
    );
    assert_eq!(
        store.get_person(&ordinary.id).await.unwrap(),
        Some(ordinary)
    );
}

#[tokio::test]
async fn memory_observation_batches_fail_without_partial_changes() {
    observation_batch_failure_preserves_every_profile(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_observation_batches_fail_without_partial_changes() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
        .await
        .unwrap();
    observation_batch_failure_preserves_every_profile(&store).await;
}
