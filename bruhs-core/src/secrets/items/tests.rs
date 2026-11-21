use super::*;
use crate::secrets::storage_key;
use bruhs_models::{
    backup::PathRewrite,
    secrets::SavedCredentialState,
    vault::{AppTokenMeta, CredentialScope, LoginFields, LoginMeta, VaultEntry, VaultKind},
};
use std::{fs, path::PathBuf};

fn canary(label: &str) -> String {
    format!("{label}-{}", uuid::Uuid::new_v4().simple())
}

fn vault_id(digit: char) -> String {
    format!("v1-{}", digit.to_string().repeat(32))
}

/// A legacy id's shape: 64 lowercase hex.
fn legacy(digit: char) -> String {
    digit.to_string().repeat(64)
}

fn staged_key() -> String {
    format!("staged:{}", "ab".repeat(16))
}

fn project_key(path: &str) -> String {
    CredentialScope::Project { path: path.into() }.key()
}

fn item_ref(service: &str, account: &str) -> ItemRef {
    ItemRef {
        service: service.into(),
        account: account.into(),
    }
}

fn item(service: &str, account: &str, value: &str) -> KeychainItem {
    KeychainItem {
        service: service.into(),
        account: account.into(),
        value: Zeroizing::new(value.into()),
    }
}

const GLOBAL_VALUE: char = '1';
const BRUH_LOGIN: char = '2';
const APP_TOKEN: char = '3';
const PROJECT_LOGIN: char = '4';
const ONE_PASSWORD: char = '5';
const RAW_GLOBAL: char = '6';
const RAW_PROJECT: char = '7';
const OLD_PROJECT: &str = "/home/ada/src/a";
const OTHER_PROJECT: &str = "/opt/b";

/// A data root with the legacy catalog, `vault.json`, a connection and a
/// calendar folder, as a backup sees them.
struct Fixture {
    _root: tempfile::TempDir,
    credentials: PathBuf,
    connections: PathBuf,
    calendar: PathBuf,
    /// The text generators' database; not there unless a test makes it.
    generators: PathBuf,
}

fn entry(digit: char, name: &str, scope: CredentialScope, kind: VaultKind) -> serde_json::Value {
    serde_json::to_value(VaultEntry {
        id: vault_id(digit),
        name: name.into(),
        scope,
        kind,
        state: SavedCredentialState::Saved,
        attention: None,
    })
    .unwrap()
}

/// A row as a newer Bruhs might write it.
fn raw_row(digit: char, scope: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"id": vault_id(digit), "name": "Passkey", "scope": scope,
        "kind": {"kind": "passkey", "rp": "example.com"},
        "state": "saved", "future": {"field": true}})
}

fn saved_login() -> VaultKind {
    VaultKind::Login(LoginMeta {
        url: "https://example.com/login".into(),
        fields: LoginFields::Saved {
            username: "ada".into(),
            totp: false,
        },
    })
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let credentials = root.path().join("credentials");
    let connections = root.path().join("connections");
    fs::create_dir_all(&credentials).unwrap();
    fs::create_dir_all(&connections).unwrap();
    let legacy = |path: &str, name: &str| {
        let id = storage_key(Path::new(path), name);
        (
            id.clone(),
            serde_json::json!({"id": id, "name": name, "project_path": path, "state": "saved"}),
        )
    };
    let catalog: serde_json::Map<_, _> = [
        legacy(OLD_PROJECT, "API_KEY"),
        legacy(OTHER_PROJECT, "TOKEN"),
    ]
    .into_iter()
    .collect();
    fs::write(
        credentials.join("catalog.json"),
        serde_json::to_vec(&serde_json::json!({ "entries": catalog })).unwrap(),
    )
    .unwrap();
    let rows = [
        entry(
            GLOBAL_VALUE,
            "KEY",
            CredentialScope::Global,
            VaultKind::Value,
        ),
        entry(
            BRUH_LOGIN,
            "Login",
            CredentialScope::Bruh {
                agent_id: "agent-1".into(),
            },
            saved_login(),
        ),
        entry(
            APP_TOKEN,
            "TOKEN",
            CredentialScope::Global,
            VaultKind::AppToken(AppTokenMeta {
                app: "github".into(),
                account: "me".into(),
                params: BTreeMap::new(),
            }),
        ),
        entry(
            PROJECT_LOGIN,
            "Site",
            CredentialScope::Project {
                path: OLD_PROJECT.into(),
            },
            saved_login(),
        ),
        entry(
            ONE_PASSWORD,
            "Shared",
            CredentialScope::Global,
            VaultKind::OnePassword {
                reference: "op://vault/item/field".into(),
            },
        ),
        raw_row(RAW_GLOBAL, serde_json::json!({"type": "global"})),
        raw_row(
            RAW_PROJECT,
            serde_json::json!({"type": "project", "path": OLD_PROJECT}),
        ),
    ];
    let entries: serde_json::Map<_, _> = rows
        .into_iter()
        .map(|row| (row["id"].as_str().unwrap().to_owned(), row))
        .collect();
    fs::write(
        credentials.join("vault.json"),
        serde_json::to_vec(&serde_json::json!({"version": 1, "entries": entries})).unwrap(),
    )
    .unwrap();
    fs::write(connections.join("github.json"), b"{}").unwrap();
    // Not a provider: never named.
    fs::write(connections.join("notes.json"), b"{}").unwrap();
    Fixture {
        calendar: root.path().join("calendar").join("google"),
        generators: root.path().join("text-generators.sqlite3"),
        _root: root,
        credentials,
        connections,
    }
}

#[test]
fn the_inventory_names_every_bruhs_item_and_nothing_else() {
    let fixture = fixture();
    let secret = |account: String| item_ref(SECRETS_SERVICE, &account);
    let old = project_key(OLD_PROJECT);
    let mut expected = vec![
        secret(storage_key(Path::new(OLD_PROJECT), "API_KEY")),
        secret(storage_key(Path::new(OTHER_PROJECT), "TOKEN")),
        secret(format!("vault:global:{}", vault_id(GLOBAL_VALUE))),
        secret(format!("vault:global:{}", vault_id(APP_TOKEN))),
        secret(format!("vault:global:{}", vault_id(RAW_GLOBAL))),
        secret(format!("vault:bruh:agent-1:{}", vault_id(BRUH_LOGIN))),
        secret(format!("vault:{old}:{}", vault_id(PROJECT_LOGIN))),
        secret(format!("vault:{old}:{}", vault_id(RAW_PROJECT))),
        item_ref(CONNECTIONS_SERVICE, "github"),
    ];
    expected.sort();

    // No calendar state: no calendar item (ruling F11).
    let inventory = keychain_inventory(
        &fixture.credentials,
        &fixture.connections,
        &fixture.calendar,
        &fixture.generators,
    )
    .unwrap();
    assert_eq!(inventory, expected);
    fs::create_dir_all(&fixture.calendar).unwrap();
    let inventory = keychain_inventory(
        &fixture.credentials,
        &fixture.connections,
        &fixture.calendar,
        &fixture.generators,
    )
    .unwrap();
    assert_eq!(
        inventory, expected,
        "an empty calendar folder holds no state"
    );

    fs::write(fixture.calendar.join(".DS_Store"), b"").unwrap();
    let inventory = keychain_inventory(
        &fixture.credentials,
        &fixture.connections,
        &fixture.calendar,
        &fixture.generators,
    )
    .unwrap();
    assert_eq!(inventory, expected, "a hidden entry is not calendar state");
    let file = fixture.calendar.join("calendars.json");
    fs::write(&file, b"[]").unwrap();
    assert_eq!(
        keychain_inventory(
            &fixture.credentials,
            &fixture.connections,
            &file,
            &fixture.generators
        )
        .unwrap(),
        expected,
        "a file in the folder's place holds no state"
    );
    let inventory = keychain_inventory(
        &fixture.credentials,
        &fixture.connections,
        &fixture.calendar,
        &fixture.generators,
    )
    .unwrap();
    expected.push(item_ref(CALENDAR_SERVICE, CALENDAR_ACCOUNT));
    expected.sort();
    assert_eq!(inventory, expected);
    assert!(inventory.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(!inventory.iter().any(
        |item| item.service == BACKUP_SERVICE || item.account.contains(&vault_id(ONE_PASSWORD))
    ));

    // A missing folder is an empty one; a damaged catalog is an error.
    let empty = tempfile::tempdir().unwrap();
    let nowhere = empty.path().join("missing");
    assert_eq!(
        keychain_inventory(&nowhere, &nowhere, &nowhere, &nowhere).unwrap(),
        Vec::new()
    );
    // A connections path that can't be read fails, never drops a token.
    let not_a_folder = empty.path().join("connections");
    fs::write(&not_a_folder, b"").unwrap();
    assert!(matches!(
        keychain_inventory(
            &fixture.credentials,
            &not_a_folder,
            &fixture.calendar,
            &fixture.generators
        ),
        Err(SecretError::Catalog)
    ));
    fs::write(fixture.credentials.join("vault.json"), b"{").unwrap();
    assert!(matches!(
        keychain_inventory(
            &fixture.credentials,
            &fixture.connections,
            &fixture.calendar,
            &fixture.generators,
        ),
        Err(SecretError::Catalog)
    ));
}

/// Main's text generators' table, with one row per `(id, record)`.
fn generators_db(path: &Path, records: &[serde_json::Value]) {
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS generators (id TEXT PRIMARY KEY, revision INTEGER NOT NULL, \
         record TEXT NOT NULL); CREATE TABLE IF NOT EXISTS retired_keys (id TEXT PRIMARY KEY);",
    )
    .unwrap();
    for record in records {
        db.execute(
            "INSERT INTO generators VALUES (?1, 1, ?2)",
            rusqlite::params![record["id"].as_str().unwrap(), record.to_string()],
        )
        .unwrap();
    }
}

fn generator(id: &str, key: Option<&str>, reference: bool) -> serde_json::Value {
    let mut record = serde_json::json!({"id": id, "revision": 1, "preset": "openai",
        "label": "Local", "url": "http://127.0.0.1:1", "enabled": true, "models": [],
        "key": key});
    if reference {
        record["reference"] = serde_json::json!({"source": "s1", "reference": "op://v/i/f"});
    }
    record
}

const GENERATOR_KEY: &str = "0b9a7a43-3f0e-4c63-9a55-5a8f2a1c7d10";
const RETIRED_KEY: &str = "7c1d2e3f-4a5b-4c6d-8e7f-901a2b3c4d5e";

/// Post-merge ruling 2: each generator row's own key id is an item, read
/// from the database, never from the keychain. A row that reads its key
/// from a secret provider, or has none, names nothing; nor does a key
/// queued for deletion.
#[test]
fn generator_keys_come_from_the_generators_database() {
    let fixture = fixture();
    let catalogs = |generators: &Path| {
        keychain_inventory(
            &fixture.credentials,
            &fixture.connections,
            &fixture.calendar,
            generators,
        )
    };
    let without = catalogs(&fixture.generators).unwrap();
    assert!(!without
        .iter()
        .any(|item| item.service == GENERATORS_SERVICE));

    generators_db(
        &fixture.generators,
        &[
            generator(
                "a1f0c1de-0000-4000-8000-000000000001",
                Some(GENERATOR_KEY),
                false,
            ),
            generator("a1f0c1de-0000-4000-8000-000000000002", None, false),
            generator("a1f0c1de-0000-4000-8000-000000000003", None, true),
        ],
    );
    rusqlite::Connection::open(&fixture.generators)
        .unwrap()
        .execute("INSERT INTO retired_keys VALUES (?1)", [RETIRED_KEY])
        .unwrap();
    let mut expected = without.clone();
    expected.push(item_ref(GENERATORS_SERVICE, GENERATOR_KEY));
    expected.sort();
    assert_eq!(catalogs(&fixture.generators).unwrap(), expected);

    // A database without the table names none.
    let empty = tempfile::tempdir().unwrap();
    let bare = empty.path().join("bare.sqlite3");
    rusqlite::Connection::open(&bare)
        .unwrap()
        .execute_batch("CREATE TABLE other (x);")
        .unwrap();
    assert_eq!(catalogs(&bare).unwrap(), without);
    // A link in its place is never followed: the walk leaves it out too.
    let link = empty.path().join("link.sqlite3");
    std::os::unix::fs::symlink(&fixture.generators, &link).unwrap();
    assert_eq!(catalogs(&link).unwrap(), without);
    // A file that isn't a database, or a key that isn't a UUID, is a
    // damaged catalog, never a dropped key.
    let junk = empty.path().join("junk.sqlite3");
    fs::write(&junk, b"not a database, just some bytes in its place").unwrap();
    assert!(matches!(catalogs(&junk), Err(SecretError::Catalog)));
    let odd = empty.path().join("odd.sqlite3");
    generators_db(
        &odd,
        &[generator(
            "a1f0c1de-0000-4000-8000-000000000004",
            Some("not-a-uuid"),
            false,
        )],
    );
    assert!(matches!(catalogs(&odd), Err(SecretError::Catalog)));
}

/// The generators' service takes its key ids only, as Bruhs writes them.
#[test]
fn generator_items_are_uuids() {
    let store = SecretStore::in_memory();
    store
        .write_item(GENERATORS_SERVICE, GENERATOR_KEY, "v")
        .unwrap();
    for account in [
        GENERATOR_KEY.to_uppercase(),
        GENERATOR_KEY.replace('-', ""),
        format!("{{{GENERATOR_KEY}}}"),
        "github".to_owned(),
    ] {
        assert!(
            matches!(
                store.write_item(GENERATORS_SERVICE, &account, "v"),
                Err(SecretError::Invalid(_))
            ),
            "{account}"
        );
    }
    assert!(check_items(&[item(GENERATORS_SERVICE, GENERATOR_KEY, "v")]).is_ok());
}

#[tokio::test]
async fn the_inventory_reads_back_what_the_store_holds() {
    let fixture = fixture();
    let store = SecretStore::file_test(fixture.credentials.clone());
    store
        .write_item(SECRETS_SERVICE, &legacy('f'), "unlisted")
        .unwrap();
    store_seal_key(&store, &staged_key(), &[9; 32]).unwrap();
    let inventory = keychain_inventory(
        &fixture.credentials,
        &fixture.connections,
        &fixture.calendar,
        &fixture.generators,
    )
    .unwrap();
    let values: Vec<String> = inventory.iter().map(|_| canary("value")).collect();
    // Every item but the last has a value.
    for (item, value) in inventory.iter().zip(&values).take(inventory.len() - 1) {
        store
            .write_item(&item.service, &item.account, value)
            .unwrap();
    }

    let (items, missing) = read_items(&store, &inventory, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(missing, 1);
    assert_eq!(items.len(), inventory.len() - 1);
    for ((read, item), value) in items.iter().zip(&inventory).zip(&values) {
        assert_eq!(
            (&read.service, &read.account, &*read.value),
            (&item.service, &item.account, value)
        );
    }
}

#[tokio::test]
async fn items_are_limited_to_bruhs_services() {
    let store = SecretStore::in_memory();
    assert!(matches!(
        store.write_item("com.apple.x", "a", "v"),
        Err(SecretError::Invalid(_))
    ));
    assert!(matches!(
        store.read_item(CALENDAR_SERVICE, "other"),
        Err(SecretError::Invalid(_))
    ));
    assert!(matches!(
        store.remove_item(SECRETS_SERVICE, "a\0b"),
        Err(SecretError::Invalid(_))
    ));
    assert!(matches!(
        store.write_item(BACKUP_SERVICE, "anything", "v"),
        Err(SecretError::Invalid(_))
    ));
    // A seal key never travels: a valid one is refused all the same.
    for refused in [
        item_ref("login", "x"),
        item_ref(BACKUP_SERVICE, &staged_key()),
    ] {
        assert!(matches!(
            read_items(&store, &[refused], Duration::from_secs(1)).await,
            Err(ItemsError::Refused)
        ));
    }

    // Secrets accounts have the two shapes Bruhs writes.
    let id = vault_id(GLOBAL_VALUE);
    let project = project_key(OLD_PROJECT);
    for account in [
        legacy('a'),
        format!("vault:global:{id}"),
        format!("vault:{project}:{id}"),
        format!("vault:bruh:agent-1:{id}"),
    ] {
        store.write_item(SECRETS_SERVICE, &account, "v").unwrap();
    }
    for account in [
        "github".to_owned(),
        "a".repeat(63),
        "A".repeat(64),
        "vault:global:x".to_owned(),
        format!("vault::{id}"),
        format!("vault:project:zz:{id}"),
        format!("vault:bruh::{id}"),
        format!("vault:bruh:{}:{id}", "b".repeat(600)),
        format!("vault:global:{id}\n"),
    ] {
        assert!(
            matches!(
                store.write_item(SECRETS_SERVICE, &account, "v"),
                Err(SecretError::Invalid(_))
            ),
            "{account}"
        );
    }

    // Services never share a slot.
    let (connection, calendar, secret) =
        (canary("connection"), canary("calendar"), canary("secret"));
    store
        .write_item(CONNECTIONS_SERVICE, "github", &connection)
        .unwrap();
    store
        .write_item(CALENDAR_SERVICE, CALENDAR_ACCOUNT, &calendar)
        .unwrap();
    store
        .write_item(SECRETS_SERVICE, &legacy('c'), &secret)
        .unwrap();
    let read = |service, account: &str| {
        store
            .read_item(service, account)
            .unwrap()
            .map(|v| v.to_string())
    };
    assert_eq!(
        read(CONNECTIONS_SERVICE, "github"),
        Some(connection.clone())
    );
    assert_eq!(read(CALENDAR_SERVICE, CALENDAR_ACCOUNT), Some(calendar));
    assert_eq!(read(SECRETS_SERVICE, &legacy('c')), Some(secret));
    store
        .remove_item(CALENDAR_SERVICE, CALENDAR_ACCOUNT)
        .unwrap();
    assert_eq!(read(CALENDAR_SERVICE, CALENDAR_ACCOUNT), None);
    assert_eq!(read(CONNECTIONS_SERVICE, "github"), Some(connection));
    assert!(read(SECRETS_SERVICE, &legacy('c')).is_some());
    // Removing a missing item is fine.
    store
        .remove_item(CALENDAR_SERVICE, CALENDAR_ACCOUNT)
        .unwrap();
}

#[tokio::test]
async fn a_slow_read_is_the_owner_needed() {
    let store = SecretStore::in_memory();
    let held = item_ref(SECRETS_SERVICE, &legacy('1'));
    store
        .write_item(&held.service, &held.account, &canary("value"))
        .unwrap();
    store.hold_reads(true);
    let result = read_items(
        &store,
        std::slice::from_ref(&held),
        Duration::from_millis(50),
    )
    .await;
    store.hold_reads(false);
    assert!(matches!(result, Err(ItemsError::NeedsOwner)));

    let (items, missing) = read_items(
        &store,
        &[item_ref(SECRETS_SERVICE, &legacy('2')), held],
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!((items.len(), missing), (1, 1));
    assert_eq!(items[0].account, legacy('1'));

    let broken = SecretStore::unavailable();
    assert!(matches!(
        read_items(
            &broken,
            &[item_ref(SECRETS_SERVICE, &legacy('3'))],
            Duration::from_secs(5)
        )
        .await,
        Err(ItemsError::NeedsOwner)
    ));
}

#[tokio::test]
async fn rekeying_moves_project_items_to_the_new_home() {
    let fixture = fixture();
    let rules = [PathRewrite {
        from: "/home/ada".into(),
        to: "/Users/ada".into(),
    }];
    let new_project = "/Users/ada/src/a";
    let (old_id, new_id) = (
        storage_key(Path::new(OLD_PROJECT), "API_KEY"),
        storage_key(Path::new(new_project), "API_KEY"),
    );
    let kept_id = storage_key(Path::new(OTHER_PROJECT), "TOKEN");

    let map = rekey_catalogs(&fixture.credentials, &rules).unwrap();

    let (old, new) = (project_key(OLD_PROJECT), project_key(new_project));
    let vault = |key: &str, digit| format!("vault:{key}:{}", vault_id(digit));
    let expected: BTreeMap<String, String> = [
        (old_id.clone(), new_id.clone()),
        (vault(&old, PROJECT_LOGIN), vault(&new, PROJECT_LOGIN)),
        (vault(&old, RAW_PROJECT), vault(&new, RAW_PROJECT)),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        map.0, expected,
        "Global, Bruh and unmoved ids keep their accounts"
    );

    let catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.credentials.join("catalog.json")).unwrap())
            .unwrap();
    let entries = catalog["entries"].as_object().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[&new_id]["id"], new_id.as_str());
    assert_eq!(entries[&new_id]["project_path"], new_project);
    assert_eq!(entries[&kept_id]["id"], kept_id.as_str());
    assert_eq!(entries[&kept_id]["project_path"], OTHER_PROJECT);

    // An older binary's checks pass on the rewritten files.
    let store = SecretStore::file_test(fixture.credentials.clone());
    let listed = store.list().await.unwrap();
    assert_eq!(listed.len(), 2);
    let rows = store.vault_list().await.unwrap();
    let path_of = |digit| {
        rows.iter()
            .find(|row| row.id == vault_id(digit))
            .map(|row| row.scope.clone())
            .unwrap()
    };
    let moved = CredentialScope::Project {
        path: new_project.into(),
    };
    assert_eq!(path_of(PROJECT_LOGIN), moved);
    assert_eq!(path_of(RAW_PROJECT), moved);
    assert_eq!(path_of(GLOBAL_VALUE), CredentialScope::Global);
    let file: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.credentials.join("vault.json")).unwrap()).unwrap();
    let raw = &file["entries"][vault_id(RAW_PROJECT)];
    assert_eq!(raw["scope"]["path"], new_project);
    assert_eq!(
        raw["future"],
        serde_json::json!({"field": true}),
        "a newer row keeps its fields"
    );

    let mut items = [
        item(SECRETS_SERVICE, &old_id, "a"),
        item(CONNECTIONS_SERVICE, &old_id, "b"),
        item(SECRETS_SERVICE, &vault(&old, PROJECT_LOGIN), "c"),
        item(SECRETS_SERVICE, &kept_id, "d"),
    ];
    map.apply(&mut items);
    let accounts: Vec<_> = items
        .iter()
        .map(|item| {
            (
                item.service.as_str(),
                item.account.as_str(),
                item.value.as_str(),
            )
        })
        .collect();
    assert_eq!(
        accounts,
        [
            (SECRETS_SERVICE, new_id.as_str(), "a"),
            (CONNECTIONS_SERVICE, old_id.as_str(), "b"),
            (SECRETS_SERVICE, vault(&new, PROJECT_LOGIN).as_str(), "c"),
            (SECRETS_SERVICE, kept_id.as_str(), "d"),
        ]
    );

    // Nothing left to move: the files stay as they are.
    assert_eq!(
        rekey_catalogs(&fixture.credentials, &rules).unwrap(),
        AccountMap::default()
    );
}

#[test]
fn rekeying_refuses_two_entries_landing_on_one_id() {
    let fixture = fixture();
    let rules = [PathRewrite {
        from: OTHER_PROJECT.into(),
        to: OLD_PROJECT.into(),
    }];
    let catalog = fixture.credentials.join("catalog.json");
    let mut file: serde_json::Value = serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    let id = storage_key(Path::new(OTHER_PROJECT), "API_KEY");
    file["entries"][&id] = serde_json::json!({"id": id, "name": "API_KEY",
        "project_path": OTHER_PROJECT, "state": "saved"});
    fs::write(&catalog, serde_json::to_vec(&file).unwrap()).unwrap();
    let before = fs::read(&catalog).unwrap();

    assert!(matches!(
        rekey_catalogs(&fixture.credentials, &rules),
        Err(RekeyError::Clash)
    ));
    assert_eq!(fs::read(&catalog).unwrap(), before);
}

#[tokio::test]
async fn sealed_sets_round_trip_and_need_their_key() {
    // A Login blob is JSON, so its value holds quotes and backslashes.
    let (one, two) = (canary(r#"{"password":"p\"w\\d"}"#), canary("two"));
    let items = [
        item(
            SECRETS_SERVICE,
            &format!("vault:global:{}", vault_id(BRUH_LOGIN)),
            &one,
        ),
        item(CONNECTIONS_SERVICE, "github", &two),
    ];
    let key = [7; 32];
    let blob = seal_items(&items, [3; 23], &key).unwrap();
    assert!(!String::from_utf8_lossy(&blob).contains(&one));

    // Values serialize as base64, which never needs a JSON escape.
    let text = serde_json::to_string(&items[0]).unwrap();
    assert!(!text.contains("password") && !text.contains('\\'), "{text}");
    let back: KeychainItem = serde_json::from_str(&text).unwrap();
    assert_eq!(*back.value, one);
    assert!(serde_json::from_str::<KeychainItem>(
        r#"{"service":"bruhs-ai-secrets","account":"a","value":"not base64!"}"#
    )
    .is_err());

    let opened = unseal_items(&blob, &key).unwrap();
    let triples = |items: &[KeychainItem]| -> Vec<(String, String, String)> {
        items
            .iter()
            .map(|item| {
                (
                    item.service.clone(),
                    item.account.clone(),
                    item.value.to_string(),
                )
            })
            .collect()
    };
    assert_eq!(triples(&opened), triples(&items));
    assert!(matches!(
        unseal_items(&blob, &[8; 32]),
        Err(SecretError::Invalid(_))
    ));
    // A sealed set never carries a seal key or an item outside Bruhs'
    // services: sealing refuses one, and so does opening a crafted set.
    for stray in [
        item(BACKUP_SERVICE, &staged_key(), "k"),
        item("login", "x", "k"),
    ] {
        assert!(matches!(
            seal_items(&[stray], [3; 23], &key),
            Err(SecretError::Invalid(_))
        ));
    }
    let crafted = serde_json::json!({"items": [
        {"service": BACKUP_SERVICE, "account": staged_key(), "value": "aw=="},
    ]});
    let crafted = bruhs_backup::seal(
        &serde_json::to_vec(&crafted).unwrap(),
        &bruhs_backup::Key::from_bytes(key),
        [3; 23],
    );
    assert!(matches!(
        unseal_items(&crafted, &key),
        Err(SecretError::Invalid(_))
    ));

    let store = SecretStore::in_memory();
    let staged = format!("staged:{}", "ab".repeat(16));
    let kept = "pre-restore:2026-11-20-1430-ab12";
    for bad in ["staged:../x", "pre-restore:2026", "other", "staged:"] {
        assert!(matches!(
            store_seal_key(&store, bad, &key),
            Err(SecretError::Invalid(_))
        ));
    }
    store_seal_key(&store, &staged, &key).unwrap();
    store_seal_key(&store, kept, &[1; 32]).unwrap();
    assert_eq!(*read_seal_key(&store, &staged).unwrap(), key);
    assert_eq!(*read_seal_key(&store, kept).unwrap(), [1; 32]);
    // Only the backup service names a seal key.
    assert!(matches!(
        store.read_item(SECRETS_SERVICE, &staged),
        Err(SecretError::Invalid(_))
    ));
    forget_seal_key(&store, &staged).unwrap();
    assert!(matches!(
        read_seal_key(&store, &staged),
        Err(SecretError::NotSaved)
    ));
    assert_eq!(*read_seal_key(&store, kept).unwrap(), [1; 32]);
}
