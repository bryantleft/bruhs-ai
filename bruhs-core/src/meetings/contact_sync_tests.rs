use super::*;

async fn bounded_directory(store: &dyn MeetingStore) {
    for n in 0..300 {
        let mut person = PersonProfile::new(format!("Person {n}"));
        person.id = format!("person-{n:03}");
        person.updated_at = n;
        person.voice_embedding = Some(vec![0.125; 256]);
        store.upsert_person(person).await.unwrap();
    }
    let first = store.person_versions(None).await.unwrap();
    first.validate(None).unwrap();
    assert_eq!(first.entries.len(), 256);
    assert_eq!(first.next_after.as_deref(), Some("person-255"));
    assert_eq!(first.entries[255].updated_at, 255);
    let wire = serde_json::to_value(&first).unwrap();
    assert_eq!(wire["entries"][0].as_object().unwrap().len(), 2);
    let last = store
        .person_versions(first.next_after.as_deref())
        .await
        .unwrap();
    last.validate(first.next_after.as_deref()).unwrap();
    assert_eq!(last.entries.len(), 44);
    assert_eq!(last.entries[0].id, "person-256");
    assert_eq!(last.next_after, None);
    let ids = vec!["person-299".into(), "missing".into(), "person-000".into()];
    let profiles = store.get_people(&ids).await.unwrap();
    assert_eq!(
        profiles.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["person-299", "person-000"]
    );
    assert!(profiles
        .iter()
        .all(|p| p.voice_embedding.as_ref().unwrap().len() == 256));
    assert!(store
        .get_people(&vec!["person-000".into(); 65])
        .await
        .is_err());
    assert!(store
        .get_people(&["person-000".into(), "person-000".into()])
        .await
        .is_err());
    assert!(store.get_people(&[" ".into()]).await.is_err());
    assert!(store.get_people(&[]).await.unwrap().is_empty());
    let renamed = store
        .rename_person(&"person-000".parse().unwrap(), "Latest name".into())
        .await
        .unwrap();
    assert_eq!(
        store.person_versions(None).await.unwrap().entries[0].updated_at,
        renamed.updated_at
    );
    store
        .merge_person_into(
            &"person-000".parse().unwrap(),
            &"person-299".parse().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        store.get_people(&["person-000".into()]).await.unwrap(),
        vec![]
    );
    assert_eq!(
        store
            .resolve_person_merges(&["person-000".into()])
            .await
            .unwrap()[0]
            .target
            .id,
        "person-299"
    );
}

#[tokio::test]
async fn memory_pages_versions_and_fetches_only_requested_profiles() {
    bounded_directory(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_pages_versions_and_fetches_only_requested_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
        .await
        .unwrap();
    bounded_directory(&store).await;
}
