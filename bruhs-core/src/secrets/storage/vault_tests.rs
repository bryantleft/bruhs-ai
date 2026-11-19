use super::vault::{StoredTotp, VaultSecret};
use super::*;
use bruhs_models::vault::{
    CredentialScope, LoginFields, LoginInput, SavedSource, TotpInput, VaultCreate, VaultKind,
    VaultSecretInput, VaultUpdate, DUPLICATE_NAME, KIND_FIXED,
};
use std::collections::HashSet;

const SETUP_KEY: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

fn canary(label: &str) -> String {
    format!("{label}-{}", uuid::Uuid::new_v4().simple())
}

fn key(name: &str, scope: CredentialScope, value: &str) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope,
        secret: VaultSecretInput::Value {
            value: Some(value.into()),
        },
    }
}

fn saved_login(name: &str, scope: CredentialScope, password: &str, totp: TotpInput) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope,
        secret: VaultSecretInput::Login {
            url: "https://example.com/in".into(),
            fields: LoginInput::Saved {
                username: "owner@example.com".into(),
                password: Some(password.into()),
                totp,
            },
        },
    }
}

fn one_password(name: &str, scope: CredentialScope) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope,
        secret: VaultSecretInput::OnePassword {
            reference: "op://Private/Item/credential".into(),
        },
    }
}

fn bruh(agent_id: &str) -> CredentialScope {
    CredentialScope::Bruh {
        agent_id: agent_id.into(),
    }
}

fn project(path: &str) -> CredentialScope {
    CredentialScope::Project { path: path.into() }
}

fn memory_keys(store: &SecretStore) -> MemoryKeys {
    match &store.0 {
        Backend::Memory(vault) => {
            match &vault.lock().unwrap_or_else(PoisonError::into_inner).keys {
                Keys::Memory(keys) => keys.clone(),
                _ => unreachable!("memory stores keep memory keys"),
            }
        }
        Backend::FileTest(_, keys) => keys.clone(),
        _ => unreachable!("tests use memory keys"),
    }
}

fn accounts(store: &SecretStore) -> Vec<String> {
    let keys = memory_keys(store);
    let mut accounts: Vec<_> = keys
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect();
    accounts.sort();
    accounts
}

fn item(store: &SecretStore, account: &str) -> Option<String> {
    memory_keys(store)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(account)
        .map(|value| value.as_str().to_owned())
}

async fn secret(store: &SecretStore, id: &str) -> VaultSecret {
    match store.vault_secret(id).await.unwrap() {
        Some(secret) => secret,
        None => panic!("the entry has a keychain item"),
    }
}

fn totp_of(secret: &VaultSecret) -> &StoredTotp {
    match &secret.totp {
        Some(totp) => totp,
        None => panic!("the login keeps a one-time code"),
    }
}

#[tokio::test]
async fn legacy_project_values_list_as_project_keys() {
    let store = SecretStore::in_memory();
    store
        .save("/p", "KEY", Zeroizing::new(canary("value")))
        .await
        .unwrap();
    let entries = store.vault_list().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, storage_key(Path::new("/p"), "KEY"));
    assert_eq!(entries[0].name, "KEY");
    assert_eq!(entries[0].scope, project("/p"));
    assert_eq!(entries[0].kind, VaultKind::Value);
    assert_eq!(entries[0].state, SavedCredentialState::Saved);
    assert_eq!(
        store.vault_entry(&entries[0].id).await.unwrap(),
        Some(entries[0].clone())
    );
}

#[tokio::test]
async fn entries_of_every_scope_and_kind_round_trip_without_values() {
    let store = SecretStore::in_memory();
    let (value, password) = (canary("value"), canary("password"));
    let global = store
        .vault_create(key("DEPLOY_KEY", CredentialScope::Global, &value))
        .await
        .unwrap();
    let login = store
        .vault_create(saved_login(
            "Example",
            bruh("a"),
            &password,
            TotpInput::Set {
                setup: SETUP_KEY.into(),
            },
        ))
        .await
        .unwrap();
    let reference = store
        .vault_create(one_password("TOKEN", project("/p")))
        .await
        .unwrap();
    for entry in [&global, &login, &reference] {
        assert!(entry.id.starts_with("v1-"), "{}", entry.id);
        assert_eq!(entry.state, SavedCredentialState::Saved);
        assert_eq!(entry.attention, None);
    }
    assert_eq!(
        login.kind,
        VaultKind::Login(bruhs_models::vault::LoginMeta {
            url: "https://example.com/in".into(),
            fields: LoginFields::Saved {
                username: "owner@example.com".into(),
                totp: true,
            },
        })
    );
    let entries = store.vault_list().await.unwrap();
    assert_eq!(entries.len(), 3);
    let listed = serde_json::to_string(&entries).unwrap();
    for hidden in [&value, &password, SETUP_KEY] {
        assert!(!listed.contains(hidden));
    }
    let stored = secret(&store, &global.id).await;
    assert_eq!(stored.value.as_deref().map(String::as_str), Some(&*value));
    let stored = secret(&store, &login.id).await;
    assert_eq!(
        stored.password.as_deref().map(String::as_str),
        Some(&*password)
    );
    assert_eq!(stored.value.as_deref().map(String::as_str), None);
    let totp = totp_of(&stored);
    assert_eq!(totp.secret.as_slice(), b"12345678901234567890");
    assert_eq!((totp.digits, totp.period), (6, 30));
    assert_eq!(totp.algorithm, bruhs_models::vault::TotpAlgorithm::Sha1);
    assert!(store.vault_secret(&reference.id).await.unwrap().is_none());
    assert_eq!(accounts(&store).len(), 2);
    assert!(matches!(
        store.vault_secret("v1-unknown").await,
        Err(SecretError::NotSaved)
    ));
    // A key or Saved login whose item went missing is NotSaved, never None.
    for (entry, account) in [
        (&global, format!("vault:global:{}", global.id)),
        (&login, format!("vault:bruh:a:{}", login.id)),
    ] {
        memory_keys(&store)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&account);
        assert!(matches!(
            store.vault_secret(&entry.id).await,
            Err(SecretError::NotSaved)
        ));
    }
}

#[tokio::test]
async fn keychain_accounts_include_the_scope() {
    let store = SecretStore::in_memory();
    let global = store
        .vault_create(key("DEPLOY_KEY", CredentialScope::Global, &canary("value")))
        .await
        .unwrap();
    let login = store
        .vault_create(saved_login(
            "Example",
            bruh("a"),
            &canary("password"),
            TotpInput::Keep,
        ))
        .await
        .unwrap();
    let mut expected = vec![
        format!("vault:global:{}", global.id),
        format!("vault:bruh:a:{}", login.id),
    ];
    expected.sort();
    assert_eq!(accounts(&store), expected);
}

#[tokio::test]
async fn names_are_unique_per_scope_across_both_files() {
    let store = SecretStore::in_memory();
    store
        .save("/p", "KEY", Zeroizing::new(canary("legacy")))
        .await
        .unwrap();
    for create in [
        key("KEY", project("/p"), &canary("value")),
        key("  KEY ", project("/p"), &canary("value")),
        one_password("KEY", project("/p")),
    ] {
        assert!(matches!(
            store.vault_create(create).await,
            Err(SecretError::Invalid(DUPLICATE_NAME))
        ));
    }
    let global = store
        .vault_create(key("KEY", CredentialScope::Global, &canary("value")))
        .await
        .unwrap();
    assert_eq!(global.name, "KEY");
    assert!(matches!(
        store
            .vault_create(one_password(" KEY", CredentialScope::Global))
            .await,
        Err(SecretError::Invalid(DUPLICATE_NAME))
    ));
    store
        .vault_create(one_password("KEY", bruh("a")))
        .await
        .unwrap();
    // The legacy value is untouched by the refused Project create.
    assert_eq!(store.vault_list().await.unwrap().len(), 3);
}

#[tokio::test]
async fn a_new_project_key_is_a_legacy_entry() {
    let store = SecretStore::in_memory();
    let value = canary("value");
    let entry = store
        .vault_create(key("KEY", project("/p"), &value))
        .await
        .unwrap();
    assert_eq!(entry.id, storage_key(Path::new("/p"), "KEY"));
    assert_eq!(store.list().await.unwrap()[0].id, entry.id);
    assert_eq!(accounts(&store), vec![entry.id.clone()]);
    assert_eq!(
        store
            .get(&entry.id)
            .await
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some(&*value)
    );
    let replaced = canary("replaced");
    store
        .vault_update(
            &entry.id,
            VaultUpdate {
                secret: VaultSecretInput::Value {
                    value: Some(replaced.clone()),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(item(&store, &entry.id).as_deref(), Some(&*replaced));
    assert!(matches!(
        store
            .vault_update(
                &entry.id,
                VaultUpdate {
                    secret: VaultSecretInput::OnePassword {
                        reference: "op://P/I/f".into()
                    }
                }
            )
            .await,
        Err(SecretError::Invalid(KIND_FIXED))
    ));
    store.vault_delete(&entry.id).await.unwrap();
    assert!(store.vault_list().await.unwrap().is_empty());
    assert!(accounts(&store).is_empty());
}

fn login_update(url: &str, password: Option<&str>, totp: TotpInput) -> VaultUpdate {
    VaultUpdate {
        secret: VaultSecretInput::Login {
            url: url.into(),
            fields: LoginInput::Saved {
                username: "renamed@example.com".into(),
                password: password.map(Into::into),
                totp,
            },
        },
    }
}

#[tokio::test]
async fn updates_keep_values_unless_given_and_kinds_fixed() {
    let store = SecretStore::in_memory();
    let password = canary("password");
    let login = store
        .vault_create(saved_login(
            "Example",
            CredentialScope::Global,
            &password,
            TotpInput::Set {
                setup: SETUP_KEY.into(),
            },
        ))
        .await
        .unwrap();
    let account = format!("vault:global:{}", login.id);

    // A new sign-in path and username keep the password and the setup key.
    let kept = store
        .vault_update(
            &login.id,
            login_update("https://example.com/login", None, TotpInput::Keep),
        )
        .await
        .unwrap();
    let VaultKind::Login(meta) = &kept.kind else {
        panic!("still a login");
    };
    assert_eq!(meta.url, "https://example.com/login");
    assert_eq!(
        meta.fields,
        LoginFields::Saved {
            username: "renamed@example.com".into(),
            totp: true
        }
    );
    let stored = secret(&store, &login.id).await;
    assert_eq!(
        stored.password.as_deref().map(String::as_str),
        Some(&*password)
    );
    assert_eq!(totp_of(&stored).secret.as_slice(), b"12345678901234567890");

    // A new password with Keep still keeps the setup key and totp: true.
    let newer = canary("newer");
    let kept = store
        .vault_update(
            &login.id,
            login_update("https://example.com/login", Some(&newer), TotpInput::Keep),
        )
        .await
        .unwrap();
    assert!(matches!(
        &kept.kind,
        VaultKind::Login(meta) if matches!(meta.fields, LoginFields::Saved { totp: true, .. })
    ));
    let stored = secret(&store, &login.id).await;
    assert_eq!(
        stored.password.as_deref().map(String::as_str),
        Some(&*newer)
    );
    assert!(stored.totp.is_some());

    // Remove drops the setup key from the blob and the flag from the row.
    let removed = store
        .vault_update(
            &login.id,
            login_update("https://example.com/login", None, TotpInput::Remove),
        )
        .await
        .unwrap();
    assert!(matches!(
        &removed.kind,
        VaultKind::Login(meta) if matches!(meta.fields, LoginFields::Saved { totp: false, .. })
    ));
    let blob = item(&store, &account).unwrap();
    assert!(!blob.contains("totp") && !blob.contains(SETUP_KEY));
    let stored = secret(&store, &login.id).await;
    assert_eq!(
        stored.password.as_deref().map(String::as_str),
        Some(&*newer)
    );
    assert!(stored.totp.is_none());

    // Kind and origin are fixed.
    assert!(matches!(
        store
            .vault_update(
                &login.id,
                VaultUpdate {
                    secret: VaultSecretInput::Value {
                        value: Some(canary("value"))
                    }
                }
            )
            .await,
        Err(SecretError::Invalid(KIND_FIXED))
    ));
    assert!(matches!(
        store
            .vault_update(
                &login.id,
                login_update("https://other.example.com/in", None, TotpInput::Keep)
            )
            .await,
        Err(SecretError::Invalid(bruhs_models::vault::ORIGIN_FIXED))
    ));
    assert!(matches!(
        store
            .vault_update(
                "v1-unknown",
                login_update("https://example.com/in", None, TotpInput::Keep)
            )
            .await,
        Err(SecretError::NotSaved)
    ));

    // Switching to 1Password deletes the keychain blob.
    let switched = store
        .vault_update(
            &login.id,
            VaultUpdate {
                secret: VaultSecretInput::Login {
                    url: "https://example.com/in".into(),
                    fields: LoginInput::OnePassword {
                        username: "op://P/I/username".into(),
                        password: "op://P/I/password".into(),
                        totp: None,
                    },
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(switched.state, SavedCredentialState::Saved);
    assert!(item(&store, &account).is_none());
    assert!(store.vault_secret(&login.id).await.unwrap().is_none());
    assert!(!serde_json::to_string(&store.vault_list().await.unwrap())
        .unwrap()
        .contains(&*newer));
}

#[tokio::test]
async fn older_readers_still_parse_the_catalog_after_vault_entries() {
    let directory = tempfile::tempdir().unwrap();
    let store = SecretStore(Backend::FileTest(directory.path().into(), Arc::default()));
    let canaries = [canary("legacy"), canary("value"), canary("password")];
    store
        .vault_create(key("KEY", project("/p"), &canaries[0]))
        .await
        .unwrap();
    store
        .vault_create(key("KEY", CredentialScope::Global, &canaries[1]))
        .await
        .unwrap();
    store
        .vault_create(one_password("TOKEN", bruh("a")))
        .await
        .unwrap();
    store
        .vault_create(saved_login(
            "Example",
            bruh("a"),
            &canaries[2],
            TotpInput::Set {
                setup: SETUP_KEY.into(),
            },
        ))
        .await
        .unwrap();
    let legacy = read_catalog(directory.path()).unwrap();
    assert_eq!(legacy.entries.len(), 1);
    let only = legacy.entries.values().next().unwrap();
    assert_eq!(
        (only.name.as_str(), only.project_path.as_str()),
        ("KEY", "/p")
    );
    let vault = fs::read_to_string(directory.path().join("vault.json")).unwrap();
    let catalog = fs::read_to_string(directory.path().join("catalog.json")).unwrap();
    for text in [&vault, &catalog] {
        for hidden in canaries.iter().map(String::as_str).chain([SETUP_KEY]) {
            assert!(!text.contains(hidden));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(directory.path().join("vault.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let reopened = SecretStore(Backend::FileTest(
        directory.path().into(),
        memory_keys(&store),
    ));
    assert_eq!(reopened.vault_list().await.unwrap().len(), 4);
}

#[tokio::test]
async fn a_damaged_vault_file_fails_closed_for_vault_operations_only() {
    let directory = tempfile::tempdir().unwrap();
    let store = SecretStore(Backend::FileTest(directory.path().into(), Arc::default()));
    let legacy = canary("legacy");
    store
        .save("/p", "KEY", Zeroizing::new(legacy.clone()))
        .await
        .unwrap();
    let id = storage_key(Path::new("/p"), "KEY");
    for damaged in [
        "broken",
        r#"{"version":1,"entries":{},"extra":true}"#,
        r#"{"version":2,"entries":{}}"#,
        r#"{"version":1,"entries":{"v1-a":{"id":"v1-b","name":"K","scope":{"type":"global"},"kind":{"kind":"value"},"state":"saved"}}}"#,
    ] {
        fs::write(directory.path().join("vault.json"), damaged).unwrap();
        assert!(matches!(
            store.vault_list().await,
            Err(SecretError::Catalog)
        ));
        assert!(matches!(
            store
                .vault_create(key("NEW", CredentialScope::Global, &canary("value")))
                .await,
            Err(SecretError::Catalog)
        ));
        // Thread saves and reads of project keys never need vault.json.
        let other = canary("other");
        store
            .save("/p", "OTHER", Zeroizing::new(other.clone()))
            .await
            .unwrap();
        store.replace(&id, legacy.clone()).await.unwrap();
        assert_eq!(
            store
                .find("/p", "OTHER")
                .await
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some(&*other)
        );
        assert_eq!(
            store.get(&id).await.unwrap().as_deref().map(String::as_str),
            Some(&*legacy)
        );
        assert_eq!(
            store.request_match("/p", "KEY").await.unwrap(),
            Some((id.clone(), SavedSource::Project))
        );
        assert_eq!(
            store
                .vault_entry(&id)
                .await
                .unwrap()
                .map(|entry| entry.name),
            Some("KEY".to_owned())
        );
        store
            .delete_saved(&storage_key(Path::new("/p"), "OTHER"))
            .await
            .unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert_eq!(
            fs::read_to_string(directory.path().join("vault.json")).unwrap(),
            damaged
        );
    }
}

#[tokio::test]
async fn forgetting_bruhs_removes_only_their_own_entries() {
    let store = SecretStore::in_memory();
    let a = store
        .vault_create(key("KEY", bruh("a"), &canary("a")))
        .await
        .unwrap();
    let b = store
        .vault_create(saved_login(
            "Example",
            bruh("b"),
            &canary("b"),
            TotpInput::Keep,
        ))
        .await
        .unwrap();
    let global = store
        .vault_create(key("KEY", CredentialScope::Global, &canary("global")))
        .await
        .unwrap();
    let gone = HashSet::from(["b".to_owned()]);
    assert_eq!(store.forget_bruh_entries(&gone).await.unwrap(), 1);
    let ids: Vec<_> = store
        .vault_list()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    assert!(ids.contains(&a.id) && ids.contains(&global.id) && !ids.contains(&b.id));
    assert!(item(&store, &format!("vault:bruh:b:{}", b.id)).is_none());
    assert!(item(&store, &format!("vault:bruh:a:{}", a.id)).is_some());
    assert_eq!(store.forget_bruh_entries(&gone).await.unwrap(), 0);
    assert_eq!(store.forget_bruh_entries(&HashSet::new()).await.unwrap(), 0);
}

#[tokio::test]
async fn deleting_an_entry_removes_its_row_and_item() {
    let store = SecretStore::in_memory();
    let login = store
        .vault_create(saved_login(
            "Example",
            CredentialScope::Global,
            &canary("password"),
            TotpInput::Keep,
        ))
        .await
        .unwrap();
    let reference = store
        .vault_create(one_password("TOKEN", CredentialScope::Global))
        .await
        .unwrap();
    store.vault_delete(&login.id).await.unwrap();
    store.vault_delete(&reference.id).await.unwrap();
    assert!(store.vault_list().await.unwrap().is_empty());
    assert!(accounts(&store).is_empty());
    assert!(matches!(
        store.vault_delete(&login.id).await,
        Err(SecretError::NotSaved)
    ));
}

#[tokio::test]
async fn a_failed_keychain_write_leaves_needs_attention() {
    assert!(matches!(
        SecretStore::unavailable()
            .vault_create(key("KEY", CredentialScope::Global, &canary("value")))
            .await,
        Err(SecretError::Storage)
    ));
    let directory = tempfile::tempdir().unwrap();
    let keys: MemoryKeys = Arc::default();
    keys.refuse_writes_after(Some(1));
    let store = SecretStore(Backend::FileTest(directory.path().into(), keys));
    let first = store
        .vault_create(key("KEY", CredentialScope::Global, &canary("value")))
        .await
        .unwrap();
    let password = canary("password");
    assert!(matches!(
        store
            .vault_create(saved_login(
                "Example",
                bruh("a"),
                &password,
                TotpInput::Keep
            ))
            .await,
        Err(SecretError::Storage)
    ));
    let entries = store.vault_list().await.unwrap();
    let state = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.state)
    };
    assert_eq!(state("KEY"), Some(SavedCredentialState::Saved));
    assert_eq!(state("Example"), Some(SavedCredentialState::NeedsAttention));
    assert_eq!(first.state, SavedCredentialState::Saved);
    assert!(!fs::read_to_string(directory.path().join("vault.json"))
        .unwrap()
        .contains(&*password));
}

#[tokio::test]
async fn a_blocked_keychain_read_never_holds_the_catalog_lock() {
    let directory = tempfile::tempdir().unwrap();
    let keys: MemoryKeys = Arc::default();
    let store = SecretStore(Backend::FileTest(directory.path().into(), keys.clone()));
    let global = store
        .vault_create(key("KEY", CredentialScope::Global, &canary("value")))
        .await
        .unwrap();
    store
        .save("/p", "KEY", Zeroizing::new(canary("legacy")))
        .await
        .unwrap();
    let release = Release::hold(&keys);
    let secret = {
        let store = store.clone();
        let id = global.id.clone();
        tokio::spawn(async move { store.vault_secret(&id).await.map(|found| found.is_some()) })
    };
    let legacy = {
        let store = store.clone();
        let id = storage_key(Path::new("/p"), "KEY");
        tokio::spawn(async move { store.get(&id).await.map(|found| found.is_some()) })
    };
    let found = {
        let store = store.clone();
        tokio::spawn(async move { store.find("/p", "KEY").await.map(|found| found.is_some()) })
    };
    held_reads(&keys, 3).await;
    let created = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store.vault_create(key("OTHER", CredentialScope::Global, &canary("value"))),
    )
    .await
    .expect("a blocked read doesn't hold the lock");
    created.unwrap();
    store
        .save("/p", "NEXT", Zeroizing::new(canary("next")))
        .await
        .unwrap();
    assert!(!secret.is_finished() && !legacy.is_finished() && !found.is_finished());
    drop(release);
    for reader in [secret, legacy, found] {
        assert!(reader.await.unwrap().unwrap());
    }
}

/// Holds keychain reads until dropped, so a failed assertion never leaves
/// the runtime's blocking pool waiting.
struct Release(MemoryKeys);

impl Release {
    fn hold(keys: &MemoryKeys) -> Self {
        keys.hold_reads(true);
        Self(keys.clone())
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        self.0.hold_reads(false);
    }
}

/// Wait until `count` reads are held.
async fn held_reads(keys: &MemoryKeys, count: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while keys.held_reads() < count {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the reads reach the keychain");
}

const OTHER_SETUP_KEY: &str = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";

#[tokio::test]
async fn a_crossed_save_never_writes_back_an_older_password() {
    let store = SecretStore::in_memory();
    let keys = memory_keys(&store);
    let login = store
        .vault_create(saved_login(
            "Example",
            CredentialScope::Global,
            &canary("old"),
            TotpInput::Keep,
        ))
        .await
        .unwrap();
    // Both saves below keep this row's metadata, so only the item's
    // revision tells them apart.
    store
        .vault_update(
            &login.id,
            login_update("https://example.com/in", None, TotpInput::Keep),
        )
        .await
        .unwrap();
    let release = Release::hold(&keys);
    // B adds a setup key and keeps the password, so it reads the old one.
    let crossed = {
        let store = store.clone();
        let id = login.id.clone();
        tokio::spawn(async move {
            let update = login_update(
                "https://example.com/in",
                None,
                TotpInput::Set {
                    setup: SETUP_KEY.into(),
                },
            );
            store.vault_update(&id, update).await.map(|_| ())
        })
    };
    held_reads(&keys, 1).await;
    // A sets a new password and needs no read.
    let newer = canary("newer");
    store
        .vault_update(
            &login.id,
            login_update("https://example.com/in", Some(&newer), TotpInput::Keep),
        )
        .await
        .unwrap();
    drop(release);
    let error = crossed.await.unwrap().unwrap_err();
    assert!(matches!(error, SecretError::Conflict));
    assert_eq!(error.to_string(), bruhs_models::vault::ENTRY_CHANGED);
    let stored = secret(&store, &login.id).await;
    assert_eq!(
        stored.password.as_deref().map(String::as_str),
        Some(&*newer)
    );
    assert!(stored.totp.is_none());
    // Tried again from a fresh read, B keeps A's password.
    store
        .vault_update(
            &login.id,
            login_update(
                "https://example.com/in",
                None,
                TotpInput::Set {
                    setup: SETUP_KEY.into(),
                },
            ),
        )
        .await
        .unwrap();
    let stored = secret(&store, &login.id).await;
    assert_eq!(
        stored.password.as_deref().map(String::as_str),
        Some(&*newer)
    );
    assert_eq!(totp_of(&stored).secret.as_slice(), b"12345678901234567890");
}

#[tokio::test]
async fn a_crossed_save_never_writes_back_an_older_setup_key() {
    let store = SecretStore::in_memory();
    let keys = memory_keys(&store);
    let login = store
        .vault_create(saved_login(
            "Example",
            CredentialScope::Global,
            &canary("old"),
            TotpInput::Set {
                setup: SETUP_KEY.into(),
            },
        ))
        .await
        .unwrap();
    // Both saves below keep this row's metadata, so only the item's
    // revision tells them apart.
    store
        .vault_update(
            &login.id,
            login_update("https://example.com/in", None, TotpInput::Keep),
        )
        .await
        .unwrap();
    let release = Release::hold(&keys);
    // B sets a password and keeps the setup key, so it reads the old one.
    let crossed = {
        let store = store.clone();
        let id = login.id.clone();
        let password = canary("crossed");
        tokio::spawn(async move {
            let update = login_update("https://example.com/in", Some(&password), TotpInput::Keep);
            store.vault_update(&id, update).await.map(|_| ())
        })
    };
    held_reads(&keys, 1).await;
    let newer = canary("newer");
    store
        .vault_update(
            &login.id,
            login_update(
                "https://example.com/in",
                Some(&newer),
                TotpInput::Set {
                    setup: OTHER_SETUP_KEY.into(),
                },
            ),
        )
        .await
        .unwrap();
    drop(release);
    assert!(matches!(crossed.await.unwrap(), Err(SecretError::Conflict)));
    let stored = secret(&store, &login.id).await;
    assert_eq!(
        stored.password.as_deref().map(String::as_str),
        Some(&*newer)
    );
    let other = bruhs_models::vault::base32_decode(OTHER_SETUP_KEY).unwrap();
    assert_eq!(totp_of(&stored).secret.as_slice(), other.as_slice());
}

#[tokio::test]
async fn a_failed_switch_to_one_password_removes_the_blob_on_the_next_save() {
    let store = SecretStore::in_memory();
    let keys = memory_keys(&store);
    let login = store
        .vault_create(saved_login(
            "Example",
            CredentialScope::Global,
            &canary("password"),
            TotpInput::Keep,
        ))
        .await
        .unwrap();
    let account = format!("vault:global:{}", login.id);
    let switch = || VaultUpdate {
        secret: VaultSecretInput::Login {
            url: "https://example.com/in".into(),
            fields: LoginInput::OnePassword {
                username: "op://P/I/username".into(),
                password: "op://P/I/password".into(),
                totp: None,
            },
        },
    };
    keys.refuse_writes_after(Some(0));
    assert!(matches!(
        store.vault_update(&login.id, switch()).await,
        Err(SecretError::Storage)
    ));
    let pending = store.vault_entry(&login.id).await.unwrap().unwrap();
    assert_eq!(pending.state, SavedCredentialState::NeedsAttention);
    assert!(matches!(
        &pending.kind,
        VaultKind::Login(meta) if matches!(meta.fields, LoginFields::OnePassword { .. })
    ));
    assert!(item(&store, &account).is_some());
    keys.refuse_writes_after(None);
    let saved = store.vault_update(&login.id, switch()).await.unwrap();
    assert_eq!(saved.state, SavedCredentialState::Saved);
    assert!(item(&store, &account).is_none());
    // Settled, a 1Password update touches the keychain no more.
    keys.refuse_writes_after(Some(0));
    assert_eq!(
        store.vault_update(&login.id, switch()).await.unwrap().state,
        SavedCredentialState::Saved
    );
}

#[tokio::test]
async fn refused_inputs_change_nothing() {
    let store = SecretStore::in_memory();
    let login = store
        .vault_create(saved_login(
            "Example",
            CredentialScope::Global,
            &canary("password"),
            TotpInput::Keep,
        ))
        .await
        .unwrap();
    // A missing item stops an update that keeps the password.
    memory_keys(&store)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    assert!(matches!(
        store
            .vault_update(
                &login.id,
                login_update(
                    "https://example.com/in",
                    None,
                    TotpInput::Set {
                        setup: SETUP_KEY.into()
                    }
                )
            )
            .await,
        Err(SecretError::NotSaved)
    ));
    assert!(matches!(
        store
            .vault_create(key(" ", CredentialScope::Global, &canary("value")))
            .await,
        Err(SecretError::Invalid(bruhs_models::vault::ENTER_NAME))
    ));
    assert_eq!(store.vault_list().await.unwrap(), vec![login]);
}

#[tokio::test]
async fn request_matches_project_then_global_and_never_logins() {
    let store = SecretStore::in_memory();
    store
        .save("/p", "KEY", Zeroizing::new(canary("legacy")))
        .await
        .unwrap();
    let global = store
        .vault_create(key("KEY", CredentialScope::Global, &canary("value")))
        .await
        .unwrap();
    let token = store
        .vault_create(one_password("TOKEN", CredentialScope::Global))
        .await
        .unwrap();
    let project_token = store
        .vault_create(one_password("TOKEN", project("/p")))
        .await
        .unwrap();
    store
        .vault_create(saved_login(
            "KEY2",
            CredentialScope::Global,
            &canary("password"),
            TotpInput::Keep,
        ))
        .await
        .unwrap();
    store
        .vault_create(key("MINE", bruh("a"), &canary("own")))
        .await
        .unwrap();
    assert_eq!(
        store.request_match("/p", "KEY").await.unwrap(),
        Some((storage_key(Path::new("/p"), "KEY"), SavedSource::Project))
    );
    assert_eq!(
        store.request_match("/q", "KEY").await.unwrap(),
        Some((global.id, SavedSource::Global))
    );
    assert_eq!(
        store.request_match("/q", "TOKEN").await.unwrap(),
        Some((token.id, SavedSource::OnePassword))
    );
    assert_eq!(
        store.request_match("/p", "TOKEN").await.unwrap(),
        Some((project_token.id, SavedSource::OnePassword))
    );
    assert_eq!(store.request_match("/q", "KEY2").await.unwrap(), None);
    assert_eq!(store.request_match("/q", "MINE").await.unwrap(), None);
}

#[tokio::test]
async fn request_match_registers_an_unindexed_legacy_value() {
    let store = SecretStore::in_memory();
    let id = storage_key(Path::new("/p"), "OLD");
    memory_keys(&store)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id.clone(), Zeroizing::new(canary("old")));
    assert!(store.vault_list().await.unwrap().is_empty());
    assert_eq!(
        store.request_match("/p", "OLD").await.unwrap(),
        Some((id.clone(), SavedSource::Project))
    );
    assert_eq!(store.vault_list().await.unwrap()[0].id, id);
}
