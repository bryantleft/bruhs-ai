//! Making a backup, over temporary roots, a file-test keychain and fake
//! bruh browsers only. Keys derive at `KdfParams::FLOOR`.
#![cfg(unix)]

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use bruhs_backup::{Decryptor, Encryptor, Header, Kdf, KdfParams, Key, Kind, Secret};
use bruhs_models::backup::*;
use bruhs_models::crew::{BruhAccess, BruhLook, CREW_PROTOCOL};
use bruhs_models::reactive::{ReactiveAgent, ReactiveCommand};
use bruhs_models::secrets::SavedCredentialState;
use bruhs_models::vault::{CredentialScope, VaultEntry, VaultKind};
use rusqlite::{Connection, OpenFlags};
use zeroize::Zeroizing;

use super::host::{Hook, Limits};
use super::*;
use crate::local_tools::{BruhComputer, LocalToolsHost};
use crate::reactive::ReactiveRuntime;
use crate::secrets::{SecretStore, CONNECTIONS_SERVICE, SECRETS_SERVICE};

const PASSWORD: &str = "correct horse 1";
const GLOBAL_VALUE: &str = "v1-11111111111111111111111111111111";

struct Fixture {
    _dir: tempfile::TempDir,
    roots: Roots,
    store: SecretStore,
    runtime: ReactiveRuntime,
    /// The keychain value of the Global Value; the connection's token is
    /// `<canary>-github`.
    canary: String,
    /// A folder outside both roots for chosen destinations.
    out: PathBuf,
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn agent(id: &str, name: &str) -> ReactiveAgent {
    ReactiveAgent {
        id: id.into(),
        name: name.into(),
        instructions: "Keep notes.".into(),
        memory: String::new(),
        model: "claude-sonnet-4-6".into(),
        project_path: String::new(),
        enabled: true,
    }
}

async fn hire(runtime: &ReactiveRuntime, id: &str, name: &str) {
    runtime
        .command(ReactiveCommand::CreateBruh {
            agent: agent(id, name),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
        })
        .await
        .unwrap();
}

/// Two roots under a resolved temp home: a reactive database with one
/// bruh, a rollback-mode mail database, a vault Global Value and a
/// connection with their keychain items, meeting audio and a transcript,
/// the bruh's browser profile and the desktop's state.
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let home = base.join("home");
    let core = home.join("core");
    let roots = Roots {
        home: home.clone(),
        core: core.clone(),
        desktop: home.join("desktop"),
        config: core.clone(),
    };
    std::fs::create_dir_all(core.join("server")).unwrap();
    let runtime = ReactiveRuntime::open(core.join("server/reactive.sqlite3"))
        .await
        .unwrap();
    hire(&runtime, "b1", "Scout").await;

    let mail = Connection::open(core.join("server/mail.sqlite3")).unwrap();
    mail.execute_batch("CREATE TABLE mail(subject TEXT); INSERT INTO mail VALUES ('hello');")
        .unwrap();
    drop(mail);

    let entry = serde_json::to_value(VaultEntry {
        id: GLOBAL_VALUE.into(),
        name: "API_KEY".into(),
        scope: CredentialScope::Global,
        kind: VaultKind::Value,
        state: SavedCredentialState::Saved,
        attention: None,
    })
    .unwrap();
    write(
        &core.join("credentials/vault.json"),
        &serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "entries": { GLOBAL_VALUE: entry },
        }))
        .unwrap(),
    );
    write(
        &core.join("connections/github.json"),
        b"{\"id\":\"github\"}",
    );
    let store = SecretStore::file_test(core.join("credentials"));
    let canary = format!("canary-{}", uuid::Uuid::new_v4().simple());
    store
        .write_item(
            SECRETS_SERVICE,
            &format!("vault:global:{GLOBAL_VALUE}"),
            &canary,
        )
        .unwrap();
    store
        .write_item(CONNECTIONS_SERVICE, "github", &format!("{canary}-github"))
        .unwrap();

    write(&core.join("audio/m1.wav"), &[7; 3000]);
    write(&core.join("audio/m2.wav"), &[8; 2000]);
    write(&core.join("audio/m1.jsonl"), b"{\"text\":\"hi\"}\n");
    write(
        &core.join("server/bruhs/b1/browser/Default/Cookies"),
        b"cookie jar",
    );
    write(
        &core.join("server/bruhs/b1/browser/Default/Code Cache/js/x"),
        &[1; 500],
    );
    write(&roots.desktop.join("desktop-state.json"), b"{\"open\":[]}");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    Fixture {
        _dir: dir,
        roots,
        store,
        runtime,
        canary,
        out,
    }
}

fn host(fixture: &Fixture) -> BackupHost {
    BackupHost::new(fixture.roots.clone(), fixture.store.clone())
        .with_reactive(fixture.runtime.clone())
        .for_tests(false)
}

fn request(output: Output, options: BackupOptions) -> CreateRequest {
    CreateRequest {
        password: Zeroizing::new(PASSWORD.into()),
        options,
        app: BackupApp::Desktop,
        web_part: None,
        web_db_version: None,
        output,
    }
}

/// The job's end state.
async fn finished(host: &BackupHost, id: &str) -> JobState {
    for _ in 0..3000 {
        let state = host.job(id).unwrap().state;
        if !matches!(state, JobState::Running { .. }) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the job never finished");
}

async fn created(host: &BackupHost, request: CreateRequest) -> BackupFile {
    let job = host.start_create(request).await.unwrap();
    match finished(host, &job.id).await {
        JobState::Created { file } => file,
        other => panic!("{other:?}"),
    }
}

/// Where a made backup is: its chosen path, or its download.
fn backup_path(host: &BackupHost, file: &BackupFile) -> PathBuf {
    match (&file.path, &file.download) {
        (Some(path), None) => PathBuf::from(path),
        (None, Some(url)) => host
            .outgoing(url.rsplit('/').next().unwrap())
            .expect("the download waits"),
        other => panic!("{other:?}"),
    }
}

struct Entry {
    path: String,
    folder: bool,
    mode: u32,
    bytes: Vec<u8>,
}

/// Every entry of a backup, decrypted with its password, after checking
/// that the stream ends where the tar does.
fn open_backup(path: &Path) -> Vec<Entry> {
    let password = normalize_password(PASSWORD);
    let mut decryptor = Decryptor::open(
        File::open(path).unwrap(),
        Secret::Password(&password),
        Kind::Full,
    )
    .unwrap();
    let mut entries = Vec::new();
    {
        let mut archive = tar::Archive::new(&mut decryptor);
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = String::from_utf8(entry.path_bytes().into_owned()).unwrap();
            let folder = entry.header().entry_type() == tar::EntryType::Directory;
            assert!(
                folder || entry.header().entry_type() == tar::EntryType::Regular,
                "{path}"
            );
            let mode = entry.header().mode().unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            entries.push(Entry {
                path,
                mode,
                folder,
                bytes,
            });
        }
    }
    let mut rest = Vec::new();
    decryptor.read_to_end(&mut rest).unwrap();
    assert!(rest.len() <= 10 * 1024 && rest.iter().all(|byte| *byte == 0));
    decryptor.finish().unwrap();
    entries
}

fn manifest_of(entries: &[Entry]) -> BackupManifest {
    let last = entries.last().unwrap();
    assert_eq!(last.path, "manifest.json", "the manifest is last");
    serde_json::from_slice(&last.bytes).unwrap()
}

fn entry<'a>(entries: &'a [Entry], path: &str) -> Option<&'a Entry> {
    entries.iter().find(|entry| entry.path == path)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The keychain values in `keychain.json`, by account.
fn keychain_values(entries: &[Entry]) -> Vec<(String, String)> {
    let json: serde_json::Value =
        serde_json::from_slice(&entry(entries, "keychain.json").unwrap().bytes).unwrap();
    json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let value = base64::engine::general_purpose::STANDARD
                .decode(item["value"].as_str().unwrap())
                .unwrap();
            (
                item["account"].as_str().unwrap().to_owned(),
                String::from_utf8(value).unwrap(),
            )
        })
        .collect()
}

/// A rows count read from a database entry.
fn rows(bytes: &[u8], table: &str) -> i64 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("copy.sqlite3");
    std::fs::write(&path, bytes).unwrap();
    let copy = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    copy.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn names_in(folder: &Path) -> Vec<String> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_backup_holds_every_file_and_opens_with_its_password() {
    let fixture = fixture().await;
    let host = host(&fixture).with_app_version("9.8.7".to_owned());
    let file = created(
        &host,
        request(
            Output::File {
                path: fixture.out.join("mine"),
                overwrite: false,
            },
            BackupOptions::default(),
        ),
    )
    .await;
    let path = fixture.out.join("mine.bruhsbak");
    assert_eq!(file.path.as_deref(), Some(path.to_str().unwrap()));
    assert_eq!(file.name, "mine.bruhsbak");
    assert_eq!(file.download, None);
    assert!(file.warnings.is_empty(), "{:?}", file.warnings);
    let metadata = std::fs::metadata(&path).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(file.bytes, metadata.len());
    assert_eq!(names_in(&fixture.out), ["mine.bruhsbak"], "no partial file");
    assert!(
        names_in(&host.tmp_dir()).is_empty(),
        "snapshots are deleted"
    );

    let entries = open_backup(&path);
    let manifest = manifest_of(&entries);
    let listed = inventory(&fixture.roots, &BackupOptions::default()).unwrap();
    let data: Vec<&str> = entries
        .iter()
        .filter(|entry| {
            !entry.folder && (entry.path.starts_with("core/") || entry.path.starts_with("desktop/"))
        })
        .map(|entry| entry.path.as_str())
        .collect();
    let expected: Vec<String> = listed
        .files
        .iter()
        .map(|file| match file.root {
            RootKey::Core => format!("core/{}", file.rel),
            RootKey::Desktop => format!("desktop/{}", file.rel),
        })
        .collect();
    assert_eq!(data, expected);
    assert!(data.contains(&"core/server/reactive.sqlite3"));
    assert!(data.contains(&"core/audio/m1.wav"));
    assert!(!data.iter().any(|path| path.contains("/browser/")));
    for file in listed.files.iter().filter(|file| !file.sqlite) {
        let prefix = match file.root {
            RootKey::Core => "core",
            RootKey::Desktop => "desktop",
        };
        let stored = entry(&entries, &format!("{prefix}/{}", file.rel)).unwrap();
        assert_eq!(
            stored.bytes,
            std::fs::read(&file.path).unwrap(),
            "{}",
            file.rel
        );
    }
    // Databases are SQLite snapshots: WAL and rollback mode alike.
    let reactive = entry(&entries, "core/server/reactive.sqlite3").unwrap();
    assert_eq!(rows(&reactive.bytes, "reactive_agents"), 1);
    let mail = entry(&entries, "core/server/mail.sqlite3").unwrap();
    assert_eq!(rows(&mail.bytes, "mail"), 1);

    // Folder entries carry no trailing slash and never name a root.
    let folders: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.folder)
        .map(|entry| entry.path.as_str())
        .collect();
    assert!(folders.contains(&"core/server/bruhs/b1"), "{folders:?}");
    assert!(folders
        .iter()
        .all(|folder| !folder.ends_with('/') && !matches!(*folder, "core" | "desktop")));

    let mut values = keychain_values(&entries);
    values.sort();
    assert_eq!(
        values,
        [
            ("github".to_owned(), format!("{}-github", fixture.canary)),
            (
                format!("vault:global:{GLOBAL_VALUE}"),
                fixture.canary.clone()
            ),
        ]
    );

    // Every entry but the folders and the manifest is listed with its size
    // and hash; nothing else is.
    let stored: Vec<(&str, u64, String)> = entries
        .iter()
        .filter(|entry| !entry.folder && entry.path != "manifest.json")
        .map(|entry| {
            (
                entry.path.as_str(),
                u64::try_from(entry.bytes.len()).unwrap(),
                sha256(&entry.bytes),
            )
        })
        .collect();
    let in_manifest: Vec<(&str, u64, String)> = manifest
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.bytes, file.sha256.clone()))
        .collect();
    assert_eq!(in_manifest, stored);
    assert!(stored.iter().any(|(path, ..)| *path == "keychain.json"));
    assert_eq!(manifest.format, MANIFEST_FORMAT);
    assert_eq!(manifest.app_version, "9.8.7");
    assert_eq!(manifest.reactive_protocol, CREW_PROTOCOL);
    assert_eq!(manifest.made_on, BackupApp::Desktop);
    assert_eq!(manifest.home, fixture.roots.home.to_str().unwrap());
    assert_eq!(manifest.core_root, fixture.roots.core.to_str().unwrap());
    assert_eq!(
        manifest.desktop_root,
        fixture.roots.desktop.to_str().unwrap()
    );
    assert!(!manifest.same_root);
    assert_eq!(manifest.web_db_version, None);
    assert_eq!(manifest.keychain_items, 2);
    assert_eq!(manifest.counts.keychain_items, 2);
    assert_eq!(manifest.counts.bruhs, 1);
    assert_eq!(manifest.counts.vault_entries, 1);
    assert_eq!(
        manifest.counts.files,
        u64::try_from(expected.len()).unwrap()
    );
    assert_eq!(manifest.options, BackupOptions::default());
    assert_eq!(manifest.changed_files, 0);
}

/// Every regular file under `folder` changed since `since`, read without
/// following links.
fn files_under(folder: &Path, since: SystemTime, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.is_dir() {
            files_under(&entry.path(), since, found);
        } else if metadata.is_file()
            && metadata.len() <= 64 << 20
            && metadata.modified().is_ok_and(|changed| changed >= since)
        {
            found.push(entry.path());
        }
    }
}

fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[tokio::test]
async fn keychain_values_appear_only_inside_the_ciphertext() {
    let since = SystemTime::now() - Duration::from_secs(2);
    let fixture = fixture().await;
    let host = host(&fixture);
    let file = created(&host, request(Output::Outgoing, BackupOptions::default())).await;
    let path = backup_path(&host, &file);
    assert!(path.starts_with(fixture.roots.backups(RootKey::Core).join("outgoing")));
    assert_eq!(path.extension().unwrap(), "bruhsbak");
    let manifest = manifest_of(&open_backup(&path));
    assert_eq!(file.name, backup_file_name(manifest.made_at_ms));
    assert!(file.path.is_none());

    let plain = fixture.canary.as_bytes().to_vec();
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(&fixture.canary)
        .into_bytes();
    let mut scanned = Vec::new();
    for folder in [
        fixture.roots.core.clone(),
        fixture.roots.desktop.clone(),
        fixture.out.clone(),
        std::env::temp_dir(),
    ] {
        files_under(&folder, since, &mut scanned);
    }
    assert!(scanned.contains(&path), "the scan covers the backup itself");
    for scanned in &scanned {
        let Ok(bytes) = std::fs::read(scanned) else {
            continue;
        };
        assert!(
            !holds(&bytes, &plain) && !holds(&bytes, &encoded),
            "a keychain value is readable in {}",
            scanned.display()
        );
    }
    // The values are in the backup all the same.
    assert!(keychain_values(&open_backup(&path))
        .iter()
        .any(|(_, value)| *value == fixture.canary));
}

#[tokio::test]
async fn options_leave_out_what_the_owner_unticks() {
    let fixture = fixture().await;
    let host = host(&fixture);
    let file = created(
        &host,
        request(
            Output::Outgoing,
            BackupOptions {
                keychain: false,
                audio: false,
                browser_profiles: false,
            },
        ),
    )
    .await;
    let entries = open_backup(&backup_path(&host, &file));
    let manifest = manifest_of(&entries);
    assert!(entry(&entries, "keychain.json").is_none());
    assert_eq!(manifest.keychain_items, 0);
    assert!(!entries
        .iter()
        .any(|entry| entry.path.starts_with("core/audio/") && entry.path.ends_with(".wav")));
    // Audio off leaves out the media only (ruling F9).
    assert!(entry(&entries, "core/audio/m1.jsonl").is_some());

    // Browser profiles: each bruh's browser is closed first; a busy one
    // stays out and is named.
    hire(&fixture.runtime, "b2", "Busy Bee").await;
    let profile = |id: &str| {
        fixture
            .roots
            .core
            .join(format!("server/bruhs/{id}/browser"))
    };
    write(&profile("b2").join("Default/Cookies"), b"busy jar");
    // An owner file whose name a restore would refuse is left out and
    // counted.
    write(
        &fixture
            .roots
            .core
            .join("server/bruhs/b1/work/odd\\name.txt"),
        b"odd",
    );
    let (tools, fake) = LocalToolsHost::with_fake_pages();
    let computer = |id: &str| BruhComputer {
        agent_id: id.into(),
        name: id.into(),
        paused: false,
        profile_dir: profile(id),
    };
    tools.sync_bruhs(vec![computer("b1"), computer("b2")]).await;
    tools.open_computer("b1").await.unwrap();
    tools.open_computer("b2").await.unwrap();
    let held = fake.hold_next_action("b2");
    let step = tokio::spawn({
        let tools = Arc::clone(&tools);
        async move {
            tools
                .bruh_command("b2", bruhs_models::local_tools::BrowserAction::Snapshot)
                .await
        }
    });
    for _ in 0..500 {
        if fake.actions("b2").len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(fake.actions("b2").len(), 1, "b2's step is running");

    let host = host.with_computers(Arc::clone(&tools));
    let file = created(
        &host,
        request(
            Output::Outgoing,
            BackupOptions {
                browser_profiles: true,
                ..BackupOptions::default()
            },
        ),
    )
    .await;
    held.notify_one();
    let _ = step.await;
    assert_eq!(fake.closes("b1"), 1, "b1's browser was closed first");
    let entries = open_backup(&backup_path(&host, &file));
    assert!(entry(&entries, "core/server/bruhs/b1/browser/Default/Cookies").is_some());
    assert!(!entries
        .iter()
        .any(|entry| entry.path.contains("Code Cache")));
    assert!(!entries
        .iter()
        .any(|entry| entry.path.starts_with("core/server/bruhs/b2/browser")));
    assert_eq!(
        file.warnings,
        [
            browser_busy_line("Busy Bee"),
            skipped_names_line(1).unwrap()
        ]
    );
    let manifest = manifest_of(&entries);
    assert!(manifest.options.browser_profiles);
    assert_eq!(manifest.skipped_names, 1);
    assert!(!entries.iter().any(|entry| entry.path.contains("odd")));
}

#[test]
fn destinations_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let core = base.join("core");
    let roots = Roots {
        home: base.clone(),
        core: core.clone(),
        desktop: base.join("desktop"),
        config: core.clone(),
    };
    std::fs::create_dir_all(&core).unwrap();
    let refused = |path: &Path, overwrite: bool| match check_destination(&roots, path, overwrite) {
        Err(BackupError::Refused(message)) => message,
        other => panic!("{other:?}"),
    };
    assert_eq!(refused(&core.join("x.bruhsbak"), false), DESTINATION_INSIDE);
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let existing = out.join("kept.bruhsbak");
    std::fs::write(&existing, b"mine").unwrap();
    assert_eq!(refused(&existing, false), DESTINATION_EXISTS);
    assert_eq!(std::fs::read(&existing).unwrap(), b"mine");
    assert_eq!(
        check_destination(&roots, &existing, true).unwrap(),
        existing
    );
    std::fs::create_dir_all(out.join("folder.bruhsbak")).unwrap();
    assert_eq!(
        refused(&out.join("folder.bruhsbak"), true),
        DESTINATION_EXISTS
    );
    assert_eq!(
        refused(&base.join("missing/x.bruhsbak"), false),
        DESTINATION_MISSING
    );
    assert_eq!(
        refused(Path::new("relative.bruhsbak"), false),
        DESTINATION_MISSING
    );
    assert_eq!(
        check_destination(&roots, &out.join("x"), false).unwrap(),
        out.join("x.bruhsbak")
    );
    assert_eq!(
        check_destination(&roots, &out.join("x.zip"), false).unwrap(),
        out.join("x.zip.bruhsbak")
    );
    // Its partial file `.<name>.partial-<32 hex>` must fit a 255-byte name.
    // `.` + 9 for `.partial-` + 32 hex leaves 213 bytes for the name.
    let longest = format!("{}.bruhsbak", "a".repeat(255 - 42 - 9));
    assert!(check_destination(&roots, &out.join(&longest), false).is_ok());
    assert_eq!(
        refused(&out.join(format!("a{longest}")), false),
        DESTINATION_NAME_LONG
    );
}

#[tokio::test]
async fn one_job_at_a_time_and_none_while_a_restore_waits() {
    let fixture = fixture().await;
    let reached = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let hook = {
        let (reached, released) = (Arc::clone(&reached), Arc::clone(&released));
        Arc::new(move |stage: Hook, _: &str| {
            if stage != Hook::BeforeFile {
                return;
            }
            reached.store(true, Ordering::SeqCst);
            while !released.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    let host = host(&fixture).with_before_file(hook);
    let job = host
        .start_create(request(Output::Outgoing, BackupOptions::default()))
        .await
        .unwrap();
    assert!(valid_id(&job.id));
    for _ in 0..1000 {
        if reached.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(reached.load(Ordering::SeqCst), "the job reached its files");
    assert!(host.running());
    let outgoing = fixture.roots.backups(RootKey::Core).join("outgoing");
    let partial = names_in(&outgoing);
    assert!(
        partial.len() == 1 && partial[0].starts_with('.') && partial[0].contains(".partial-"),
        "{partial:?}"
    );
    assert!(matches!(
        host.start_create(request(Output::Outgoing, BackupOptions::default()))
            .await,
        Err(BackupError::Busy(BACKUP_BUSY))
    ));

    assert!(
        !host.cancel("0".repeat(32).as_str()),
        "only the job's own id"
    );
    assert!(host.cancel(&job.id));
    released.store(true, Ordering::SeqCst);
    assert_eq!(
        finished(&host, &job.id).await,
        JobState::Failed {
            message: BACKUP_STOPPED.into()
        }
    );
    assert!(!host.running());
    assert!(names_in(&outgoing).is_empty(), "the partial file is gone");
    assert!(names_in(&host.tmp_dir()).is_empty());
    assert!(!host.cancel(&job.id), "a finished job can't be stopped");

    let pending = fixture.roots.backups(RootKey::Core).join("pending.json");
    std::fs::write(&pending, b"{}").unwrap();
    assert!(matches!(
        host.start_create(request(Output::Outgoing, BackupOptions::default()))
            .await,
        Err(BackupError::Refused(RESTORE_PENDING))
    ));
    std::fs::remove_file(&pending).unwrap();
    // A `backups` folder that isn't a real folder, in either root, refuses
    // the backup before any job starts; a `pending.json` behind a link
    // isn't trusted either way.
    let refused_up_front =
        |result: Result<BackupJob, BackupError>| matches!(result, Err(BackupError::Io(_)));
    let backups = fixture.roots.backups(RootKey::Core);
    let moved = fixture.roots.core.join("moved-backups");
    std::fs::rename(&backups, &moved).unwrap();
    std::fs::write(moved.join("pending.json"), b"{}").unwrap();
    std::os::unix::fs::symlink(&moved, &backups).unwrap();
    assert!(refused_up_front(
        host.start_create(request(Output::Outgoing, BackupOptions::default()))
            .await
    ));
    std::fs::remove_file(&backups).unwrap();
    std::fs::remove_file(moved.join("pending.json")).unwrap();
    std::fs::rename(&moved, &backups).unwrap();
    let desktop_backups = fixture.roots.backups(RootKey::Desktop);
    std::fs::write(&desktop_backups, b"not a folder").unwrap();
    assert!(refused_up_front(
        host.start_create(request(Output::Outgoing, BackupOptions::default()))
            .await
    ));
    std::fs::remove_file(&desktop_backups).unwrap();
    assert_eq!(
        host.job(&job.id).map(|job| job.id),
        Some(job.id.clone()),
        "no new job replaced the last one"
    );

    // While the capture daemon records, no backup starts (ruling F24).
    let recording = BackupHost::new(fixture.roots.clone(), fixture.store.clone()).for_tests(true);
    assert!(matches!(
        recording
            .start_create(request(Output::Outgoing, BackupOptions::default()))
            .await,
        Err(BackupError::Refused(RECORDING_FIRST))
    ));
    let mut short = request(Output::Outgoing, BackupOptions::default());
    short.password = Zeroizing::new("short".into());
    assert!(matches!(
        host.start_create(short).await,
        Err(BackupError::Refused(PASSWORD_SHORT))
    ));
}

#[tokio::test]
async fn a_locked_keychain_stops_the_backup() {
    let fixture = fixture().await;
    let host = host(&fixture);
    fixture.store.hold_reads(true);
    let job = host
        .start_create(request(
            Output::File {
                path: fixture.out.join("locked"),
                overwrite: false,
            },
            BackupOptions::default(),
        ))
        .await
        .unwrap();
    let state = finished(&host, &job.id).await;
    fixture.store.hold_reads(false);
    assert_eq!(
        state,
        JobState::Failed {
            message: KEYCHAIN_NEEDS_YOU.into()
        }
    );
    assert!(names_in(&fixture.out).is_empty(), "no file, partial or not");
    assert!(names_in(&host.tmp_dir()).is_empty());
}

/// A container of `kind` holding `plain`, as the browser makes its part.
fn container(kind: Kind, plain: &[u8]) -> Vec<u8> {
    let header = Header::new(kind, Kdf::Argon2id(KdfParams::FLOOR), [9; 23]);
    let key = Key::derive(&normalize_password(PASSWORD), &header).unwrap();
    let mut writer = Encryptor::new(Vec::new(), &header, &key).unwrap();
    std::io::Write::write_all(&mut writer, plain).unwrap();
    writer.finish().unwrap()
}

#[tokio::test]
async fn a_web_part_rides_along() {
    let fixture = fixture().await;
    let host = host(&fixture);
    let part = container(Kind::WebPart, b"{\"version\":1,\"stores\":{}}");
    let mut upload = host.begin_upload(UploadKind::WebPart).unwrap();
    upload.write(&part[..10]).unwrap();
    upload.write(&part[10..]).unwrap();
    let id = upload.finish().unwrap();
    assert!(valid_id(&id));
    let stored = host.uploaded(UploadKind::WebPart, &id).unwrap();
    assert_eq!(std::fs::read(&stored).unwrap(), part);

    let mut asked = request(Output::Outgoing, BackupOptions::default());
    asked.app = BackupApp::Web;
    asked.web_part = Some(stored);
    asked.web_db_version = Some(13);
    let file = created(&host, asked).await;
    let entries = open_backup(&backup_path(&host, &file));
    let manifest = manifest_of(&entries);
    let web = entry(&entries, "web/part.bin").unwrap();
    assert_eq!(web.bytes, part, "byte for byte");
    assert_eq!(manifest.web_db_version, Some(13));
    assert_eq!(manifest.made_on, BackupApp::Web);
    assert!(manifest
        .files
        .iter()
        .any(|file| file.path == "web/part.bin" && file.sha256 == sha256(&part)));
    let order: Vec<&str> = entries
        .iter()
        .rev()
        .take(3)
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(order, ["manifest.json", "web/part.bin", "keychain.json"]);

    // A full backup's header is no web part: refused as an upload, and as
    // a stored file.
    let full = container(Kind::Full, b"not a web part");
    let incoming = fixture.roots.backups(RootKey::Core).join("incoming");
    let before = names_in(&incoming);
    let mut upload = host.begin_upload(UploadKind::WebPart).unwrap();
    upload.write(&full).unwrap();
    assert!(matches!(
        upload.finish(),
        Err(BackupError::Format(NOT_A_BACKUP))
    ));
    let mut dropped = host.begin_upload(UploadKind::Restore).unwrap();
    dropped.write(&full).unwrap();
    drop(dropped);
    assert_eq!(names_in(&incoming), before, "unfinished uploads are gone");

    // A web part needs the IndexedDB version it was dumped from.
    let mut unversioned = request(Output::Outgoing, BackupOptions::default());
    unversioned.web_part = host.uploaded(UploadKind::WebPart, &id);
    assert!(unversioned.web_part.is_some());
    assert!(matches!(
        host.start_create(unversioned).await,
        Err(BackupError::Refused(INVALID_REQUEST))
    ));

    let wrong = fixture.out.join("part.bin");
    std::fs::write(&wrong, &full).unwrap();
    let mut asked = request(Output::Outgoing, BackupOptions::default());
    asked.web_part = Some(wrong);
    asked.web_db_version = Some(13);
    let job = host.start_create(asked).await.unwrap();
    assert_eq!(
        finished(&host, &job.id).await,
        JobState::Failed {
            message: NOT_A_BACKUP.into()
        }
    );
}

#[tokio::test]
async fn the_last_backup_is_recorded() {
    let fixture = fixture().await;
    let host = host(&fixture);
    assert_eq!(host.last(), None);
    // A catalog names the connection, but its item is gone: counted (D5).
    fixture
        .store
        .remove_item(CONNECTIONS_SERVICE, "github")
        .unwrap();
    // An existing `backups` folder is made private.
    let backups = fixture.roots.backups(RootKey::Core);
    std::fs::create_dir_all(&backups).unwrap();
    std::fs::set_permissions(&backups, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut asked = request(Output::Outgoing, BackupOptions::default());
    asked.app = BackupApp::Web;
    let file = created(&host, asked).await;
    let manifest = manifest_of(&open_backup(&backup_path(&host, &file)));
    assert_eq!(file.warnings, [missing_items_line(1).unwrap()]);
    assert_eq!(manifest.keychain_items, 1);
    assert_eq!(
        std::fs::metadata(&backups).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let last = host.last().unwrap();
    assert_eq!(
        last,
        LastBackup {
            made_at_ms: manifest.made_at_ms,
            app: BackupApp::Web,
            bytes: file.bytes,
        }
    );

    let overview = host.overview().await;
    assert!(overview.supported);
    assert_eq!(overview.last, Some(last));
    assert_eq!(overview.keychain_items, 2);
    assert_eq!(overview.sizes.audio_bytes, 5000);
    assert!(overview.sizes.browser_bytes > 0);
    assert!(matches!(
        overview.job.map(|job| job.state),
        Some(JobState::Created { .. })
    ));

    // A finished download goes once it was sent.
    let id = file
        .download
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    assert!(host.outgoing(&id).is_some());
    host.finish_download(&id);
    assert!(host.outgoing(&id).is_none());
    assert!(host.outgoing("../x").is_none());

    // Neither reads through a `backups` folder that became a link, nor a
    // `last.json` that isn't a regular file.
    let file = created(&host, request(Output::Outgoing, BackupOptions::default())).await;
    let id = file
        .download
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    let moved = fixture.roots.core.join("elsewhere");
    std::fs::rename(&backups, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &backups).unwrap();
    assert_eq!(host.last(), None);
    assert!(host.outgoing(&id).is_none());
    std::fs::remove_file(&backups).unwrap();
    std::fs::rename(&moved, &backups).unwrap();
    assert!(host.last().is_some() && host.outgoing(&id).is_some());
    std::fs::remove_file(backups.join("last.json")).unwrap();
    std::fs::create_dir(backups.join("last.json")).unwrap();
    assert_eq!(host.last(), None);
}

#[tokio::test]
async fn owner_execute_bits_survive_and_nothing_else_does() {
    let fixture = fixture().await;
    let work = fixture.roots.core.join("server/bruhs/b1/work");
    write(&work.join("run.sh"), b"#!/bin/sh\necho hi\n");
    std::fs::set_permissions(work.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    write(&work.join("notes.md"), b"notes");
    std::fs::set_permissions(
        work.join("notes.md"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let host = host(&fixture);
    let file = created(&host, request(Output::Outgoing, BackupOptions::default())).await;
    let entries = open_backup(&backup_path(&host, &file));
    let mode = |path: &str| entry(&entries, path).unwrap().mode;
    assert_eq!(mode("core/server/bruhs/b1/work/run.sh"), 0o700);
    assert_eq!(mode("core/server/bruhs/b1/work/notes.md"), 0o600);
    assert_eq!(mode("core/server/reactive.sqlite3"), 0o600);
    assert_eq!(mode("keychain.json"), 0o600);
    assert_eq!(mode("manifest.json"), 0o600);
}

/// A host whose writer runs `act` at `stage` of the entry `path`.
fn host_acting(
    fixture: &Fixture,
    stage: Hook,
    path: &'static str,
    act: impl Fn() + Send + Sync + 'static,
) -> BackupHost {
    host(fixture).with_before_file(Arc::new(move |at: Hook, entry: &str| {
        if at == stage && entry == path {
            act();
        }
    }))
}

#[tokio::test]
async fn a_file_that_grows_while_read_keeps_its_listed_part() {
    let fixture = fixture().await;
    let log = fixture.roots.core.join("server/bruhs/b1/work/build.log");
    write(&log, b"first line\n");
    let grown = log.clone();
    let host = host_acting(
        &fixture,
        Hook::Read,
        "core/server/bruhs/b1/work/build.log",
        move || {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&grown)
                .unwrap();
            file.write_all(b"second line\n").unwrap();
        },
    );
    // A file gone since the walk is left out and not counted.
    let gone = fixture.roots.core.join("audio/m2.wav");
    let host = {
        let before = host.config().before_file.clone().unwrap();
        host.with_before_file(Arc::new(move |at: Hook, entry: &str| {
            if at == Hook::BeforeFile && entry == "core/audio/m2.wav" {
                std::fs::remove_file(&gone).unwrap();
            }
            before(at, entry);
        }))
    };
    let file = created(&host, request(Output::Outgoing, BackupOptions::default())).await;
    assert_eq!(file.warnings, [changed_files_line(1).unwrap()]);
    let entries = open_backup(&backup_path(&host, &file));
    assert_eq!(
        entry(&entries, "core/server/bruhs/b1/work/build.log")
            .unwrap()
            .bytes,
        b"first line\n"
    );
    assert!(entry(&entries, "core/audio/m2.wav").is_none());
    let manifest = manifest_of(&entries);
    let data = entries
        .iter()
        .filter(|entry| {
            !entry.folder && (entry.path.starts_with("core/") || entry.path.starts_with("desktop/"))
        })
        .count();
    assert_eq!(manifest.counts.files, u64::try_from(data).unwrap());
    assert_eq!(manifest.changed_files, 1);
}

#[tokio::test]
async fn a_file_that_shrinks_while_read_fails_the_backup() {
    let fixture = fixture().await;
    let shrunk = fixture.roots.core.join("audio/m1.wav");
    let host = host_acting(&fixture, Hook::Opened, "core/audio/m1.wav", move || {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&shrunk)
            .unwrap();
        file.set_len(10).unwrap();
    });
    let job = host
        .start_create(request(
            Output::File {
                path: fixture.out.join("shrunk"),
                overwrite: false,
            },
            BackupOptions::default(),
        ))
        .await
        .unwrap();
    assert_eq!(
        finished(&host, &job.id).await,
        JobState::Failed {
            message: FILE_CHANGED.into()
        }
    );
    assert!(names_in(&fixture.out).is_empty());
}

#[tokio::test]
async fn a_root_that_is_not_utf8_is_refused() {
    use std::os::unix::ffi::OsStrExt as _;
    let fixture = fixture().await;
    let mut roots = fixture.roots.clone();
    roots.desktop = PathBuf::from(std::ffi::OsStr::from_bytes(b"/nowhere/\xff"));
    let host = BackupHost::new(roots, fixture.store.clone()).for_tests(false);
    let job = host
        .start_create(request(Output::Outgoing, BackupOptions::default()))
        .await
        .unwrap();
    assert_eq!(
        finished(&host, &job.id).await,
        JobState::Failed {
            message: ROOT_NAME_REFUSED.into()
        }
    );
    assert!(
        !fixture.roots.backups(RootKey::Core).exists(),
        "nothing was written"
    );
}

#[tokio::test]
async fn partial_files_left_by_a_stopped_process_are_swept() {
    use std::os::unix::fs::MetadataExt as _;
    let fixture = fixture().await;
    let host = host(&fixture);
    let tmp = host.tmp_dir();
    std::fs::create_dir_all(&tmp).unwrap();
    let marker = |id: char, file: &Path, of: &Path| {
        let metadata = std::fs::symlink_metadata(of).unwrap();
        std::fs::write(
            tmp.join(format!(
                "{}{}",
                id.to_string().repeat(32),
                super::host::PARTIAL_MARKER
            )),
            super::host::marker_text(metadata.dev(), metadata.ino(), file),
        )
        .unwrap();
    };
    let partial = |id: char| {
        fixture.out.join(format!(
            ".mine.bruhsbak.partial-{}",
            id.to_string().repeat(32)
        ))
    };
    let ours = partial('a');
    std::fs::write(&ours, b"half").unwrap();
    marker('1', &ours, &ours);
    // A marker naming another partial-shaped file, but not its identity.
    let decoy = partial('b');
    std::fs::write(&decoy, b"someone else's").unwrap();
    marker('2', &decoy, &ours);
    // A file without a partial's name, even with its own identity.
    let theirs = fixture.out.join("notes.txt");
    std::fs::write(&theirs, b"mine").unwrap();
    marker('3', &theirs, &theirs);
    // A FIFO in a marker's place is never read.
    let fifo = tmp.join(format!("{}{}", "4".repeat(32), super::host::PARTIAL_MARKER));
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    // A leftover `last.json` partial goes too.
    let backups = fixture.roots.backups(RootKey::Core);
    let last_partial = backups.join(format!(".last.json.partial-{}", "d".repeat(32)));
    std::fs::write(&last_partial, b"{").unwrap();

    let swept = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking({
            let host = host.clone();
            move || host.sweep()
        }),
    )
    .await;
    assert!(swept.is_ok(), "the sweep never blocks on a FIFO");
    assert!(!ours.exists(), "our partial file is gone");
    assert!(decoy.exists(), "a file that isn't the one marked stays");
    assert!(theirs.exists(), "a file without our partial name stays");
    assert!(!last_partial.exists());
    assert_eq!(
        names_in(&tmp),
        [fifo.file_name().unwrap().to_string_lossy()]
    );
}

#[tokio::test]
async fn a_file_that_shrank_before_its_second_open_is_read_at_that_size() {
    let fixture = fixture().await;
    let log = fixture.roots.core.join("server/bruhs/b1/work/run.log");
    write(&log, &[b'x'; 10_000]);
    let (cut, regrow) = (log.clone(), log.clone());
    let host = host(&fixture).with_before_file(Arc::new(move |at: Hook, entry: &str| {
        if entry != "core/server/bruhs/b1/work/run.log" {
            return;
        }
        match at {
            // Truncated by `>` before the first open, then written again
            // before the second.
            Hook::BeforeFile => std::fs::write(&cut, b"").unwrap(),
            Hook::Reopening => std::fs::write(&regrow, [b'y'; 2000]).unwrap(),
            _ => {}
        }
    }));
    let file = created(&host, request(Output::Outgoing, BackupOptions::default())).await;
    assert_eq!(file.warnings, [changed_files_line(1).unwrap()]);
    let entries = open_backup(&backup_path(&host, &file));
    assert_eq!(
        entry(&entries, "core/server/bruhs/b1/work/run.log")
            .unwrap()
            .bytes,
        vec![b'y'; 2000]
    );
    assert_eq!(manifest_of(&entries).changed_files, 1);
}

#[tokio::test]
async fn a_rewrite_that_only_moves_the_time_is_counted() {
    let fixture = fixture().await;
    let note = fixture.roots.core.join("server/bruhs/b1/work/note.md");
    write(&note, b"same size");
    let touched = note.clone();
    let host = host_acting(
        &fixture,
        Hook::Read,
        "core/server/bruhs/b1/work/note.md",
        move || {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&touched)
                .unwrap();
            file.set_modified(SystemTime::now() + Duration::from_secs(60))
                .unwrap();
        },
    );
    let file = created(&host, request(Output::Outgoing, BackupOptions::default())).await;
    assert_eq!(file.warnings, [changed_files_line(1).unwrap()]);
    let entries = open_backup(&backup_path(&host, &file));
    assert_eq!(
        entry(&entries, "core/server/bruhs/b1/work/note.md")
            .unwrap()
            .bytes,
        b"same size"
    );
    assert_eq!(manifest_of(&entries).changed_files, 1);
}

/// B Task 7 I1: a backup a restore wouldn't take isn't made. At the
/// limits a backup is made; one entry or one manifest byte over, the job
/// fails with `BACKUP_TOO_MANY` and leaves no file. The entry count comes
/// from the walk up front, so nothing is written; the manifest's exact
/// size is known only once it is serialized, just before it goes in.
#[tokio::test]
async fn backups_past_the_restore_limits_are_refused() {
    let fixture = fixture().await;
    let first = created(
        &host(&fixture),
        request(Output::Outgoing, BackupOptions::default()),
    )
    .await;
    let entries = open_backup(&backup_path(&host(&fixture), &first));
    let count = u64::try_from(entries.len()).unwrap();
    let manifest_bytes = u64::try_from(entries.last().unwrap().bytes.len()).unwrap();
    let outgoing = fixture.roots.backups(RootKey::Core).join("outgoing");
    let waiting = || std::fs::read_dir(&outgoing).unwrap().count();
    assert_eq!(waiting(), 1);

    for (entries, manifest, made, written) in [
        (count, manifest_bytes, true, true),
        (count - 1, manifest_bytes, false, false),
        (count, manifest_bytes - 1, false, true),
        (count, 100, false, false),
    ] {
        let started = Arc::new(AtomicBool::new(false));
        let seen = started.clone();
        let host = host(&fixture)
            .with_limits(Limits {
                entries,
                manifest_bytes: manifest,
            })
            .with_before_file(Arc::new(move |_, _| seen.store(true, Ordering::SeqCst)));
        let before = waiting();
        let job = host
            .start_create(request(Output::Outgoing, BackupOptions::default()))
            .await
            .unwrap();
        let state = finished(&host, &job.id).await;
        if made {
            assert!(matches!(state, JobState::Created { .. }), "{state:?}");
            assert_eq!(waiting(), before + 1);
        } else {
            assert_eq!(
                state,
                JobState::Failed {
                    message: BACKUP_TOO_MANY.into()
                },
                "{entries} {manifest}"
            );
            assert_eq!(waiting(), before, "no file is left");
        }
        assert_eq!(
            started.load(Ordering::SeqCst),
            written,
            "{entries} {manifest}"
        );
    }
}
