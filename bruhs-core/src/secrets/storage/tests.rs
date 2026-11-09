use super::*;

#[tokio::test]
async fn metadata_listing_does_not_wait_for_a_keychain_writer_prompt() {
    let directory = tempfile::tempdir().unwrap();
    let store = SecretStore(Backend::FileTest(directory.path().into(), Arc::default()));
    store
        .save("/project", "Key", Zeroizing::new("private".into()))
        .await
        .unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("catalog.lock"))
        .unwrap();
    lock.lock().unwrap();
    let entries = tokio::time::timeout(std::time::Duration::from_secs(5), store.list())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entries.len(), 1);
    drop(lock);
}

#[tokio::test]
async fn an_older_hashed_entry_is_registered_without_resaving_or_exposing_its_value() {
    let store = SecretStore::in_memory();
    let id = storage_key(Path::new("/project"), "Legacy key");
    let key = id.clone();
    store
        .access(move |vault| {
            if let Keys::Memory(keys) = &vault.keys {
                keys.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(key, Zeroizing::new("private-legacy".into()));
            }
            Ok(())
        })
        .await
        .unwrap();
    assert!(store.list().await.unwrap().is_empty());
    assert!(store
        .find("/project", "Legacy key")
        .await
        .unwrap()
        .is_some());
    let entries = store.list().await.unwrap();
    assert_eq!(entries[0].id, id);
    assert_eq!(entries[0].name, "Legacy key");
    assert!(!serde_json::to_string(&entries)
        .unwrap()
        .contains("private-legacy"));
}

#[tokio::test]
async fn failed_keychain_write_leaves_a_manageable_recovery_entry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_owned();
    let failed = tokio::task::spawn_blocking(move || {
        with_catalog(&path, Keys::Unavailable, |vault| {
            let entry = SavedCredential {
                id: storage_key(Path::new("/project"), "Key"),
                name: "Key".into(),
                project_path: "/project".into(),
                state: SavedCredentialState::Saved,
            };
            vault.save(entry, &Zeroizing::new("private-failed-write".into()))
        })
    })
    .await
    .unwrap();
    assert!(matches!(failed, Err(SecretError::Storage)));
    let store = SecretStore(Backend::FileTest(directory.path().into(), Arc::default()));
    let entries = store.list().await.unwrap();
    assert_eq!(entries[0].state, SavedCredentialState::NeedsAttention);
    store
        .replace(&entries[0].id, "private-recovered".into())
        .await
        .unwrap();
    assert_eq!(
        store.list().await.unwrap()[0].state,
        SavedCredentialState::Saved
    );
    assert!(!fs::read_to_string(directory.path().join("catalog.json"))
        .unwrap()
        .contains("private-"));
}

#[tokio::test]
async fn saved_metadata_never_contains_values_and_replacement_keeps_identity() {
    let store = SecretStore::in_memory();
    store
        .save(
            "/synthetic/project",
            "Service key",
            Zeroizing::new("private-original".into()),
        )
        .await
        .unwrap();
    let entries = store.list().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!serde_json::to_string(&entries)
        .unwrap()
        .contains("private-original"));
    store
        .replace(&entries[0].id, "private-replacement".into())
        .await
        .unwrap();
    assert_eq!(store.list().await.unwrap(), entries);
    assert_eq!(
        store.get(&entries[0].id).await.unwrap().unwrap().as_str(),
        "private-replacement"
    );
    store.delete_saved(&entries[0].id).await.unwrap();
    assert!(store.list().await.unwrap().is_empty());
    assert!(store.get(&entries[0].id).await.unwrap().is_none());
}

#[tokio::test]
async fn unknown_ids_and_invalid_replacements_do_not_modify_saved_values() {
    let store = SecretStore::in_memory();
    assert!(matches!(
        store
            .save("/project", "Key", Zeroizing::new(String::new()))
            .await,
        Err(SecretError::Invalid(_))
    ));
    assert!(matches!(
        store.replace("unknown", "private-value".into()).await,
        Err(SecretError::NotSaved)
    ));
    assert!(matches!(
        store.delete_saved("unknown").await,
        Err(SecretError::NotSaved)
    ));
    assert!(matches!(
        store.replace("unknown", String::new()).await,
        Err(SecretError::Invalid(_))
    ));
    assert!(store.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn disk_catalog_survives_reopen_serializes_writers_and_never_contains_values() {
    let directory = tempfile::tempdir().unwrap();
    let keys: MemoryKeys = Arc::default();
    let store = SecretStore(Backend::FileTest(directory.path().into(), keys.clone()));
    let other = SecretStore(Backend::FileTest(directory.path().into(), keys.clone()));
    let (a, b) = tokio::join!(
        store.save(
            "/project/a",
            "Token",
            Zeroizing::new("private-alpha".into())
        ),
        other.save("/project/b", "Token", Zeroizing::new("private-beta".into())),
    );
    a.unwrap();
    b.unwrap();
    let reopened = SecretStore(Backend::FileTest(directory.path().into(), keys));
    let entries = reopened.list().await.unwrap();
    assert_eq!(entries.len(), 2);
    let text = fs::read_to_string(directory.path().join("catalog.json")).unwrap();
    assert!(!text.contains("private-alpha"));
    assert!(!text.contains("private-beta"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(directory.path().join("catalog.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    reopened.delete_saved(&entries[0].id).await.unwrap();
    assert_eq!(store.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn damaged_catalog_fails_closed_instead_of_reporting_an_empty_vault() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("catalog.json"), "broken").unwrap();
    let store = SecretStore(Backend::FileTest(directory.path().into(), Arc::default()));
    assert!(matches!(store.list().await, Err(SecretError::Catalog)));
    assert!(matches!(
        store
            .save("/project", "Key", Zeroizing::new("private".into()))
            .await,
        Err(SecretError::Catalog)
    ));
    assert_eq!(
        fs::read_to_string(directory.path().join("catalog.json")).unwrap(),
        "broken"
    );
}
