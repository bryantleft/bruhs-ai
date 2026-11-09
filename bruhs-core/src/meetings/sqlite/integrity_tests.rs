use super::*;
use crate::meetings::MeetingRecords;
use crate::meetings::PeopleStore;
use bruhs_models::{Meeting, PersonProfile};

#[tokio::test]
async fn cyclic_merge_receipt_fails_resolution_without_hanging_or_changing_originals() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
        .await
        .unwrap();
    store
        .call(|connection| {
            connection.execute(
                "INSERT INTO merged_people VALUES('a','b','original-a','original-b')",
                [],
            )?;
            connection.execute(
                "INSERT INTO merged_people VALUES('b','a','original-b','original-a')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        store.resolve_person_merges(&["a".into()]),
    )
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(MeetingStoreError::PersistenceFailed(_))
    ));
    let count: i64 = store
        .call(|connection| {
            Ok(connection.query_row("SELECT count(*) FROM merged_people", [], |row| row.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[cfg(unix)]
#[tokio::test]
async fn parent_symlink_works_but_database_symlink_and_public_file_are_refused() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = directory.path().join("alias");
    symlink(&real, &alias).unwrap();
    let store = SqliteMeetingStore::open(alias.join("private.sqlite3"))
        .await
        .unwrap();
    let person = PersonProfile::new("Preserved");
    store.upsert_person(person.clone()).await.unwrap();
    drop(store);
    let path = real.join("private.sqlite3");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let before = std::fs::read(&path).unwrap();
    let link = directory.path().join("linked.sqlite3");
    symlink(&path, &link).unwrap();
    assert!(SqliteMeetingStore::open(&link).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(SqliteMeetingStore::open(&path).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn mismatched_contact_identity_cannot_be_listed_or_rewritten_by_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
        .await
        .unwrap();
    let person = PersonProfile::new("Original");
    store.upsert_person(person.clone()).await.unwrap();
    let mut wrong = person.clone();
    wrong.id = "wrong-key".into();
    let payload = encode(&wrong).unwrap();
    let expected = payload.clone();
    let id = person.id.clone();
    store
        .call(move |connection| {
            connection.execute("UPDATE people SET payload=?1 WHERE id=?2", [payload, id])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(store.list_people().await.is_err());
    assert!(store.get_person(&person.id).await.is_err());
    assert!(store
        .delete_observations_for_meeting(&"anything".parse().unwrap())
        .await
        .is_err());
    let actual: String = store
        .call(|connection| {
            Ok(connection.query_row("SELECT payload FROM people", [], |row| row.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn exhausted_version_rejects_field_change_without_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
        .await
        .unwrap();
    let mut original = PersonProfile::new("Original");
    original.updated_at = i64::MAX;
    store.upsert_person(original.clone()).await.unwrap();
    assert!(matches!(
        store
            .rename_person(&original.id.parse().unwrap(), "Changed".into())
            .await,
        Err(MeetingStoreError::Conflict(_))
    ));
    assert_eq!(
        store.get_person(&original.id).await.unwrap(),
        Some(original)
    );
}

#[tokio::test]
async fn failed_merge_commit_preserves_linked_meeting_and_original_contacts() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
        .await
        .unwrap();
    let source = PersonProfile::new("Source");
    let target = PersonProfile::new("Target");
    store.upsert_person(source.clone()).await.unwrap();
    store.upsert_person(target.clone()).await.unwrap();
    let meeting = store
        .create_meeting(Meeting::new("Linked", 1.0))
        .await
        .unwrap();
    let before = store
        .link_speaker_to_person(
            &meeting.id.parse().unwrap(),
            &"speaker".parse().unwrap(),
            &source.id.parse().unwrap(),
        )
        .await
        .unwrap();
    store.call(|connection| {
        connection.execute_batch("CREATE TABLE parent(id INTEGER PRIMARY KEY);
            CREATE TABLE deferred_failure(id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_commit AFTER INSERT ON merged_people BEGIN INSERT INTO deferred_failure VALUES(1); END;")?;
        Ok(())
    }).await.unwrap();
    assert!(store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .is_err());
    assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), Some(before));
    assert_eq!(store.get_person(&source.id).await.unwrap(), Some(source));
    assert_eq!(store.get_person(&target.id).await.unwrap(), Some(target));
}
