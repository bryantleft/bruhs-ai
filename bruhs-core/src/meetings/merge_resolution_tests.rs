use super::*;

async fn resolves_current_target_without_mutating(store: &dyn MeetingStore) {
    let source = PersonProfile::new("Source");
    let middle = PersonProfile::new("Middle");
    let target = PersonProfile::new("Target");
    for person in [&source, &middle, &target] {
        store.upsert_person(person.clone()).await.unwrap();
    }
    store
        .merge_person_into(&source.id.parse().unwrap(), &middle.id.parse().unwrap())
        .await
        .unwrap();
    store
        .merge_person_into(&middle.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    let latest = store
        .rename_person(&target.id.parse().unwrap(), "Current name".into())
        .await
        .unwrap();
    let result = store
        .resolve_person_merges(&[
            source.id.clone(),
            middle.id.clone(),
            target.id.clone(),
            "unknown".into(),
        ])
        .await
        .unwrap();
    assert_eq!(result.len(), 2);
    assert!(result.iter().all(|item| item.target == latest));
    assert!(result.iter().any(|item| item.source_id == source.id));
    assert!(result.iter().any(|item| item.source_id == middle.id));
    assert_eq!(store.list_people().await.unwrap(), vec![latest]);
    assert!(store
        .resolve_person_merges(&[source.id.clone(), source.id])
        .await
        .is_err());
    assert!(store.resolve_person_merges(&[" ".into()]).await.is_err());
    let too_many: Vec<_> = (0..65).map(|index| format!("id-{index}")).collect();
    assert!(store.resolve_person_merges(&too_many).await.is_err());
    assert!(store.resolve_person_merges(&[]).await.unwrap().is_empty());
}

#[tokio::test]
async fn memory_resolves_chained_merges_and_rejects_invalid_batches() {
    resolves_current_target_without_mutating(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_resolves_chained_merges_and_rejects_invalid_batches() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
        .await
        .unwrap();
    resolves_current_target_without_mutating(&store).await;
}
