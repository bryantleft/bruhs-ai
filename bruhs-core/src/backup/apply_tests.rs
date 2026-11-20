//! Applying a restore offline, over temporary roots and a file-test
//! keychain only. Each test makes a backup of data B with Task 6's
//! `create`, replaces the live data with A, opens the backup with Task 7
//! (so B is staged), then applies. Keys derive at `KdfParams::FLOOR`.
#![cfg(unix)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bruhs_backup::{Encryptor, Header, Kdf, KdfParams, Key, Kind};
use bruhs_models::backup::*;
use bruhs_models::secrets::SavedCredentialState;
use bruhs_models::vault::{CredentialScope, VaultEntry, VaultKind};
use rusqlite::Connection;
use zeroize::Zeroizing;

use super::apply::{apply_pending_with, Crash};
use super::journal::Step;
use super::open::staging_dir;
use super::*;
use crate::secrets::{read_seal_key, unseal_items, SecretError, SecretStore, SECRETS_SERVICE};

const PASSWORD: &str = "correct horse 1";
/// The Value both A and B hold.
const VALUE: &str = "v1-11111111111111111111111111111111";
/// A Value only A holds.
const EXTRA: &str = "v1-22222222222222222222222222222222";
/// Top-level entries an apply never moves: a backup always leaves them
/// out, `Worktrees` included, since a volume may fold it into `worktrees`.
const NEVER: [&str; 5] = ["backups", "capture", "models", "logs", "Worktrees"];

struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
    roots: Roots,
    store: SecretStore,
    /// A folder outside the roots for backup files.
    out: PathBuf,
}

/// Empty roots under a resolved temp home: two folders unless `same`, and
/// a config root of its own (as on Linux) when `config`.
fn fixture(same: bool, config: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let home = base.join("home");
    let core = home.join("core");
    let desktop = if same {
        core.clone()
    } else {
        home.join("desktop")
    };
    let roots = Roots {
        home,
        core: core.clone(),
        desktop: desktop.clone(),
        config: if config {
            base.join("home/config")
        } else {
            core.clone()
        },
    };
    for folder in [&core, &desktop, &roots.config] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    Fixture {
        _dir: dir,
        base,
        store: SecretStore::file_test(core.join("credentials")),
        roots,
        out,
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn account(id: &str) -> String {
    format!("vault:global:{id}")
}

fn item(store: &SecretStore, id: &str) -> Option<String> {
    store
        .read_item(SECRETS_SERVICE, &account(id))
        .unwrap()
        .map(|value| value.to_string())
}

/// Removes every top-level entry of the roots but `backups`, and the
/// config root's calendar.
fn clear(roots: &Roots) {
    for root in [&roots.core, &roots.desktop] {
        for entry in std::fs::read_dir(root).unwrap().flatten() {
            if entry.file_name() == "backups" {
                continue;
            }
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                std::fs::remove_dir_all(&path).unwrap();
            } else {
                std::fs::remove_file(&path).unwrap();
            }
        }
    }
    let _ = std::fs::remove_dir_all(roots.config.join("calendar"));
}

/// `vault.json` with a Global Value for each id.
fn vault(roots: &Roots, ids: &[&str]) {
    let entries: serde_json::Map<String, serde_json::Value> = ids
        .iter()
        .map(|id| {
            let entry = VaultEntry {
                id: (*id).into(),
                name: format!("KEY_{}", &id[3..7]),
                scope: CredentialScope::Global,
                kind: VaultKind::Value,
                state: SavedCredentialState::Saved,
                attention: None,
            };
            ((*id).to_owned(), serde_json::to_value(entry).unwrap())
        })
        .collect();
    write(
        &roots.core.join("credentials/vault.json"),
        &serde_json::to_vec(&serde_json::json!({ "version": 1, "entries": entries })).unwrap(),
    );
}

fn reactive(roots: &Roots, bruh: &str) {
    std::fs::create_dir_all(roots.core.join("server")).unwrap();
    let db = Connection::open(roots.core.join("server/reactive.sqlite3")).unwrap();
    db.execute_batch(&format!(
        "CREATE TABLE bruhs(id TEXT); INSERT INTO bruhs VALUES ('{bruh}');"
    ))
    .unwrap();
}

/// Data B, the backup's: a bruh, the Value with item `new`, a runnable
/// script, a plain file and the desktop's state.
fn write_b(fixture: &Fixture) {
    let roots = &fixture.roots;
    clear(roots);
    reactive(roots, "b-bruh");
    write(
        &roots.core.join("server/bruhs/b-bruh/work/notes.txt"),
        b"B notes",
    );
    vault(roots, &[VALUE]);
    fixture
        .store
        .write_item(SECRETS_SERVICE, &account(VALUE), "new")
        .unwrap();
    fixture
        .store
        .remove_item(SECRETS_SERVICE, &account(EXTRA))
        .unwrap();
    let script = roots.core.join("attachments/run.sh");
    write(&script, b"#!/bin/sh\necho b\n");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    write(&roots.core.join("attachments/plain.txt"), b"B plain");
    write(&roots.desktop.join("desktop-state.json"), b"{\"b\":true}");
    if roots.config != roots.core {
        write(
            &roots.config.join("calendar/google/state.json"),
            b"B calendar",
        );
    }
}

/// Data A, today's: another bruh, the Value with item `old` (and, with
/// `extra`, a second Value only A has), an entry B lacks, and folders an
/// apply never moves.
fn write_a(fixture: &Fixture, extra: bool) {
    let roots = &fixture.roots;
    clear(roots);
    reactive(roots, "a-bruh");
    write(
        &roots.core.join("server/bruhs/a-bruh/work/notes.txt"),
        b"A notes",
    );
    let store = &fixture.store;
    store
        .write_item(SECRETS_SERVICE, &account(VALUE), "old")
        .unwrap();
    if extra {
        vault(roots, &[VALUE, EXTRA]);
        store
            .write_item(SECRETS_SERVICE, &account(EXTRA), "extra")
            .unwrap();
    } else {
        vault(roots, &[VALUE]);
    }
    write(&roots.core.join("a-only.json"), b"{\"a\":1}");
    write(&roots.core.join("capture/daemon-token"), b"token");
    write(&roots.core.join("models/model.bin"), b"model");
    write(&roots.core.join("logs/server.log"), b"log");
    write(&roots.core.join("Worktrees/checkout.txt"), b"worktree");
    write(&roots.desktop.join("desktop-state.json"), b"{\"a\":true}");
    write(&roots.desktop.join("a-desktop.json"), b"{\"a\":2}");
    if !roots.same() {
        write(&roots.desktop.join("logs/desktop.log"), b"desktop log");
    }
    if roots.config != roots.core {
        write(
            &roots.config.join("calendar/google/state.json"),
            b"A calendar",
        );
        // The core root's own calendar folder isn't the one Bruhs uses
        // here, so a restore leaves it alone.
        write(&roots.core.join("calendar/own.txt"), b"core own");
    }
}

fn host(fixture: &Fixture) -> BackupHost {
    BackupHost::new(fixture.roots.clone(), fixture.store.clone()).for_tests(false)
}

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

/// A kind 2 part, as the browser uploads it, and its upload id.
fn web_part_upload(host: &BackupHost, kind: UploadKind, body: &[u8]) -> (String, Vec<u8>) {
    let header = Header::new(Kind::WebPart, Kdf::Argon2id(KdfParams::FLOOR), [5; 23]);
    let key = Key::derive("web password!", &header).unwrap();
    let mut part = Encryptor::new(Vec::new(), &header, &key).unwrap();
    part.write_all(body).unwrap();
    let part = part.finish().unwrap();
    let mut upload = host.begin_upload(kind).unwrap();
    upload.write(&part).unwrap();
    (upload.finish().unwrap(), part)
}

/// What the test compares against after an apply.
struct Before {
    staged: String,
    /// A's files in each root, `backups` aside.
    core: BTreeMap<PathBuf, Vec<u8>>,
    desktop: BTreeMap<PathBuf, Vec<u8>>,
    /// B's files as staged.
    staged_core: BTreeMap<PathBuf, Vec<u8>>,
    staged_desktop: BTreeMap<PathBuf, Vec<u8>>,
    /// The web part's bytes, when the backup has one.
    web_part: Option<Vec<u8>>,
}

/// Backs up B with `options`, swaps the live data for A and stages B.
async fn stage_b(fixture: &Fixture, options: BackupOptions, web: bool, extra: bool) -> Before {
    write_b(fixture);
    let host = host(fixture);
    let (web_part, part) = if web {
        let (id, part) = web_part_upload(&host, UploadKind::WebPart, b"{\"version\":1}");
        (host.uploaded(UploadKind::WebPart, &id), Some(part))
    } else {
        (None, None)
    };
    let path = fixture.out.join("b.bruhsbak");
    let job = host
        .start_create(CreateRequest {
            password: Zeroizing::new(PASSWORD.into()),
            options,
            app: if web {
                BackupApp::Web
            } else {
                BackupApp::Desktop
            },
            web_db_version: web.then_some(13),
            web_part,
            output: Output::File {
                path: path.clone(),
                overwrite: false,
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        finished(&host, &job.id).await,
        JobState::Created { .. }
    ));
    write_a(fixture, extra);
    let job = host
        .start_open(path, Zeroizing::new(PASSWORD.into()), false)
        .unwrap();
    assert!(matches!(
        finished(&host, &job.id).await,
        JobState::Opened { .. }
    ));
    let roots = &fixture.roots;
    Before {
        core: files_under(&roots.core, &["backups"]),
        desktop: if roots.same() {
            BTreeMap::new()
        } else {
            files_under(&roots.desktop, &["backups"])
        },
        staged_core: files_under(&staging_dir(roots, RootKey::Core, &job.id), &["meta"]),
        staged_desktop: if roots.same() {
            BTreeMap::new()
        } else {
            files_under(&staging_dir(roots, RootKey::Desktop, &job.id), &[])
        },
        staged: job.id,
        web_part: part,
    }
}

fn staged_source(before: &Before) -> RestoreSource {
    RestoreSource::Staged {
        id: before.staged.clone(),
    }
}

/// Every file under `folder` by path, with its bytes, leaving out the
/// top-level names in `skip`.
fn files_under(folder: &Path, skip: &[&str]) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(at) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(folder).unwrap().to_path_buf();
            if at == folder && skip.iter().any(|name| rel == Path::new(name)) {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                folders.push(path);
            } else {
                files.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// `files` without those under the top-level names in `skip`.
fn without(files: &BTreeMap<PathBuf, Vec<u8>>, skip: &[&str]) -> BTreeMap<PathBuf, Vec<u8>> {
    files
        .iter()
        .filter(|(path, _)| !skip.iter().any(|name| path.starts_with(name)))
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect()
}

/// `files` under the top-level names in `keep` only.
fn only(files: &BTreeMap<PathBuf, Vec<u8>>, keep: &[&str]) -> BTreeMap<PathBuf, Vec<u8>> {
    files
        .iter()
        .filter(|(path, _)| keep.iter().any(|name| path.starts_with(name)))
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect()
}

fn kept_dir(roots: &Roots, root: RootKey, stamp: &str) -> PathBuf {
    roots.backups(root).join(format!("pre-restore-{stamp}"))
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

fn no_key(store: &SecretStore, account: &str) -> bool {
    matches!(read_seal_key(store, account), Err(SecretError::NotSaved))
}

fn never_stop(_: &Step, _: Crash) -> bool {
    false
}

fn no_env(_: &str) -> Option<std::ffi::OsString> {
    None
}

/// Applies with a stop at the `at`-th crash point (counting from 0).
/// Answers whether the stop was reached.
fn apply_stopping_at(fixture: &Fixture, at: usize) -> bool {
    let seen = Cell::new(0_usize);
    let stop = |_: &Step, _: Crash| {
        let now = seen.get();
        seen.set(now + 1);
        now == at
    };
    match apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &stop) {
        Ok(ApplyOutcome::Applied(_)) => false,
        Err(_) => true,
        Ok(other) => panic!("{other:?}"),
    }
}

/// The end state of a staged restore applied in full: the live data is
/// B's, A's is kept aside whole with its keychain item sealed, the
/// folders an apply never moves stay, and nothing of the restore is left.
fn assert_applied(fixture: &Fixture, before: &Before) -> String {
    let roots = &fixture.roots;
    assert_eq!(files_under(&roots.core, &NEVER), before.staged_core);
    assert_eq!(
        only(&files_under(&roots.core, &[]), &NEVER[1..]),
        only(&before.core, &NEVER[1..])
    );
    if !roots.same() {
        assert_eq!(
            files_under(&roots.desktop, &["backups", "logs"]),
            before.staged_desktop
        );
        assert_eq!(
            only(&files_under(&roots.desktop, &[]), &["logs"]),
            only(&before.desktop, &["logs"])
        );
    }
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("new"));
    // F32: the owner's execute bit carries over as 0700, else 0600.
    assert_eq!(mode(&roots.core.join("attachments/run.sh")), 0o700);
    assert_eq!(mode(&roots.core.join("attachments/plain.txt")), 0o600);

    let kept = pre_restores(roots);
    assert_eq!(kept.len(), 1, "{kept:?}");
    let stamp = kept[0].stamp.clone();
    assert!(valid_stamp(&stamp));
    let kept_core = files_under(&kept_dir(roots, RootKey::Core, &stamp), &["meta"]);
    assert_eq!(kept_core, without(&before.core, &NEVER));
    let mut bytes: u64 = kept_core
        .values()
        .map(|file| u64::try_from(file.len()).unwrap())
        .sum();
    if !roots.same() {
        let kept_desktop = files_under(&kept_dir(roots, RootKey::Desktop, &stamp), &[]);
        assert_eq!(kept_desktop, without(&before.desktop, &["backups", "logs"]));
        bytes += kept_desktop
            .values()
            .map(|file| u64::try_from(file.len()).unwrap())
            .sum::<u64>();
    }
    assert_eq!(kept[0].bytes, bytes);
    let sealed =
        std::fs::read(kept_dir(roots, RootKey::Core, &stamp).join("meta/keychain.sealed")).unwrap();
    let key = read_seal_key(&fixture.store, &format!("pre-restore:{stamp}")).unwrap();
    let items = unseal_items(&sealed, *key).unwrap();
    assert!(items
        .iter()
        .any(|item| item.account == account(VALUE) && *item.value == "old"));

    let backups = roots.backups(RootKey::Core);
    assert!(!backups.join("pending.json").exists());
    assert!(!backups.join("apply.json").exists());
    assert!(pending(roots).is_none());
    for root in [RootKey::Core, RootKey::Desktop] {
        assert!(!staging_dir(roots, root, &before.staged).exists());
    }
    assert!(no_key(&fixture.store, &format!("staged:{}", before.staged)));
    assert_eq!(result(roots).unwrap().stamp, stamp);
    stamp
}

#[tokio::test]
async fn applying_swaps_both_roots_and_keeps_today_aside() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let pending_restore = schedule(&fixture.roots, &staged_source(&before), true).unwrap();
    assert_eq!(pending(&fixture.roots), Some(pending_restore));

    let outcome = apply_pending(&fixture.roots, &fixture.store, None).unwrap();
    let ApplyOutcome::Applied(applied) = outcome else {
        panic!("{outcome:?}");
    };
    let stamp = assert_applied(&fixture, &before);
    assert_eq!(applied.stamp, stamp);
    assert!(take_service_restart(&fixture.roots));
    assert!(!take_service_restart(&fixture.roots));
    assert_eq!(
        apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
        ApplyOutcome::NothingPending
    );
}

#[tokio::test]
async fn applying_onto_one_root_moves_each_entry_once() {
    let fixture = fixture(true, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    // The journal plans a single pass (ruling F33).
    let stop = |step: &Step, crash: Crash| *step == Step::Sealed && crash == Crash::Recorded;
    assert!(apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &stop).is_err());
    let journal: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fixture.roots.backups(RootKey::Core).join("apply.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(journal["passes"].as_array().unwrap().len(), 1);

    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    let stamp = assert_applied(&fixture, &before);
    // Only today's entries are kept, each once: none of the restored ones.
    let kept = files_under(&kept_dir(&fixture.roots, RootKey::Core, &stamp), &["meta"]);
    assert_eq!(
        kept.get(Path::new("desktop-state.json")).map(Vec::as_slice),
        Some(&b"{\"a\":true}"[..])
    );
    assert!(!kept.contains_key(Path::new("attachments/run.sh")));
}

#[tokio::test]
async fn calendar_goes_back_to_the_config_root() {
    let fixture = fixture(false, true);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    let roots = &fixture.roots;
    assert_eq!(
        std::fs::read(roots.config.join("calendar/google/state.json")).unwrap(),
        b"B calendar"
    );
    let stamp = pre_restores(roots)[0].stamp.clone();
    assert_eq!(
        std::fs::read(kept_dir(roots, RootKey::Core, &stamp).join("calendar/google/state.json"))
            .unwrap(),
        b"A calendar"
    );
    assert_eq!(
        std::fs::read(roots.core.join("calendar/own.txt")).unwrap(),
        b"core own"
    );
}

#[tokio::test]
async fn apply_waits_for_every_writer() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    let roots = &fixture.roots;
    // A running server.
    let server_lock = roots.core.join("server/reactive.sqlite3.lock");
    std::fs::write(&server_lock, b"").unwrap();
    let today = (
        files_under(&roots.core, &["backups"]),
        files_under(&roots.desktop, &["backups", "desktop.instance.lock"]),
    );
    let untouched = || {
        assert_eq!(files_under(&roots.core, &["backups"]), today.0);
        assert_eq!(
            files_under(&roots.desktop, &["backups", "desktop.instance.lock"]),
            today.1
        );
        assert!(!roots.backups(RootKey::Core).join("apply.json").exists());
        assert!(pending(roots).is_some());
    };
    let server = File::open(&server_lock).unwrap();
    server.try_lock().unwrap();
    assert_eq!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Busy(Holder::Server)
    );
    untouched();
    // Its reactive database moved elsewhere (ruling F10 c).
    drop(server);
    let elsewhere = |name: &str| {
        (name == "BRUHS_REACTIVE_DB").then(|| std::ffi::OsString::from("/elsewhere/r.sqlite3"))
    };
    assert_eq!(
        apply_pending_with(roots, &fixture.store, None, &elsewhere, &never_stop).unwrap(),
        ApplyOutcome::Busy(Holder::Server)
    );
    // Another apply.
    let restore = File::options()
        .read(true)
        .write(true)
        .open(roots.backups(RootKey::Core).join("restore.lock"))
        .unwrap();
    restore.try_lock().unwrap();
    assert_eq!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Busy(Holder::Restore)
    );
    untouched();
    drop(restore);
    // A running desktop app.
    let desktop = take_instance_lock(&instance_lock_path(&roots.desktop))
        .unwrap()
        .unwrap();
    assert!(take_instance_lock(&instance_lock_path(&roots.desktop))
        .unwrap()
        .is_none());
    assert_eq!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Busy(Holder::Desktop)
    );
    untouched();

    // The desktop's own lock lets it apply.
    assert!(matches!(
        apply_pending(roots, &fixture.store, Some(&desktop)).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("new"));
    // The lock file stays where the desktop holds it.
    assert!(instance_lock_path(&roots.desktop).exists());
}

#[tokio::test]
async fn an_interrupted_apply_finishes_on_the_next_start() {
    let first = fixture(false, false);
    let second = fixture(false, false);
    let fixture = first;
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    // Killed after the second entry was placed and recorded.
    let placed = Cell::new(0);
    let stop = |step: &Step, crash: Crash| {
        if matches!(step, Step::Placed { .. }) && crash == Crash::Recorded {
            placed.set(placed.get() + 1);
        }
        placed.get() == 2
    };
    assert!(apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &stop).is_err());
    assert!(fixture
        .roots
        .backups(RootKey::Core)
        .join("apply.json")
        .exists());
    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert_applied(&fixture, &before);

    // Killed after the items were written, before the journal said so:
    // the next start writes each once more.
    let fixture = second;
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    let stop = |step: &Step, crash: Crash| *step == Step::KeysWritten && crash == Crash::Acted;
    assert!(apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &stop).is_err());
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("new"));
    fixture
        .store
        .write_item(SECRETS_SERVICE, &account(VALUE), "changed meanwhile")
        .unwrap();
    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert_applied(&fixture, &before);
}

/// A stop at every crash point, before and after each step is recorded,
/// ends where an uninterrupted apply does once the next start resumes.
#[tokio::test]
async fn every_crash_point_resumes_to_the_same_end() {
    for at in 0.. {
        let fixture = fixture(false, false);
        let before = stage_b(&fixture, BackupOptions::default(), true, true).await;
        schedule(&fixture.roots, &staged_source(&before), false).unwrap();
        let stopped = apply_stopping_at(&fixture, at);
        if stopped {
            assert!(matches!(
                apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
                ApplyOutcome::Applied(_)
            ));
        }
        assert_applied(&fixture, &before);
        assert_eq!(item(&fixture.store, EXTRA), None, "crash point {at}");
        let part = waiting_web_part(&fixture.roots).unwrap();
        assert_eq!(part.id, before.staged);
        if !stopped {
            assert!(at > 10, "too few crash points: {at}");
            break;
        }
    }
}

#[tokio::test]
async fn items_not_in_the_backup_go_aside() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, true).await;
    assert_eq!(item(&fixture.store, EXTRA).as_deref(), Some("extra"));
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    let stamp = assert_applied(&fixture, &before);
    assert_eq!(item(&fixture.store, EXTRA), None);
    let sealed =
        std::fs::read(kept_dir(&fixture.roots, RootKey::Core, &stamp).join("meta/keychain.sealed"))
            .unwrap();
    let key = read_seal_key(&fixture.store, &format!("pre-restore:{stamp}")).unwrap();
    let items = unseal_items(&sealed, *key).unwrap();
    assert!(items
        .iter()
        .any(|item| item.account == account(EXTRA) && *item.value == "extra"));
    // No kept file holds a keychain value in plain text.
    for (path, bytes) in files_under(&kept_dir(&fixture.roots, RootKey::Core, &stamp), &[]) {
        assert!(
            !bytes.windows(5).any(|window| window == b"extra"),
            "{path:?}"
        );
    }
}

#[tokio::test]
async fn a_backup_without_keychain_keeps_todays_items() {
    let fixture = fixture(false, false);
    let options = BackupOptions {
        keychain: false,
        ..BackupOptions::default()
    };
    let before = stage_b(&fixture, options, false, true).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert_eq!(files_under(&fixture.roots.core, &NEVER), before.staged_core);
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("old"));
    assert_eq!(item(&fixture.store, EXTRA).as_deref(), Some("extra"));
}

#[tokio::test]
async fn putting_back_is_an_apply_of_the_kept_set() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, true).await;
    schedule(&fixture.roots, &staged_source(&before), true).unwrap();
    apply_pending(&fixture.roots, &fixture.store, None).unwrap();
    let first = assert_applied(&fixture, &before);
    assert!(take_service_restart(&fixture.roots));

    let roots = &fixture.roots;
    let put_back = RestoreSource::PreRestore {
        stamp: first.clone(),
    };
    schedule(roots, &put_back, false).unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert_eq!(
        files_under(&roots.core, &NEVER),
        without(&before.core, &NEVER)
    );
    assert_eq!(
        files_under(&roots.desktop, &["backups", "logs"]),
        without(&before.desktop, &["backups", "logs"])
    );
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("old"));
    assert_eq!(item(&fixture.store, EXTRA).as_deref(), Some("extra"));
    assert!(!take_service_restart(roots));

    // B is the kept set now; the first one is gone with its key.
    let kept = pre_restores(roots);
    assert_eq!(kept.len(), 1);
    assert_ne!(kept[0].stamp, first);
    assert_eq!(
        files_under(&kept_dir(roots, RootKey::Core, &kept[0].stamp), &["meta"]),
        before.staged_core
    );
    assert_eq!(
        files_under(&kept_dir(roots, RootKey::Desktop, &kept[0].stamp), &[]),
        before.staged_desktop
    );
    for root in [RootKey::Core, RootKey::Desktop] {
        assert!(!kept_dir(roots, root, &first).exists());
    }
    assert!(no_key(&fixture.store, &format!("pre-restore:{first}")));
}

#[tokio::test]
async fn deleting_a_kept_set_removes_its_key() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    apply_pending(&fixture.roots, &fixture.store, None).unwrap();
    let stamp = assert_applied(&fixture, &before);
    let roots = &fixture.roots;

    for bad in ["nope", "2026-11-20-1200-ABCD", "../../x"] {
        assert!(matches!(
            delete_pre_restore(roots, &fixture.store, bad),
            Err(BackupError::Refused(GONE))
        ));
    }
    delete_pre_restore(roots, &fixture.store, &stamp).unwrap();
    for root in [RootKey::Core, RootKey::Desktop] {
        assert!(!kept_dir(roots, root, &stamp).exists());
    }
    assert!(no_key(&fixture.store, &format!("pre-restore:{stamp}")));
    assert!(pre_restores(roots).is_empty());
    assert!(matches!(
        delete_pre_restore(roots, &fixture.store, &stamp),
        Err(BackupError::Refused(GONE))
    ));
}

#[tokio::test]
async fn a_kept_set_a_restore_waits_on_stays() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    apply_pending(&fixture.roots, &fixture.store, None).unwrap();
    let stamp = pre_restores(&fixture.roots)[0].stamp.clone();
    schedule(
        &fixture.roots,
        &RestoreSource::PreRestore {
            stamp: stamp.clone(),
        },
        false,
    )
    .unwrap();
    assert!(matches!(
        delete_pre_restore(&fixture.roots, &fixture.store, &stamp),
        Err(BackupError::Refused(RESTORE_PENDING))
    ));
    assert!(kept_dir(&fixture.roots, RootKey::Core, &stamp).exists());
}

#[tokio::test]
async fn the_web_part_waits_for_the_browser() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), true, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    let outcome = apply_pending(&fixture.roots, &fixture.store, None).unwrap();
    assert!(matches!(outcome, ApplyOutcome::Applied(_)), "{outcome:?}");
    let roots = &fixture.roots;
    let stamp = assert_applied(&fixture, &before);

    let path = roots
        .backups(RootKey::Core)
        .join("web-part")
        .join(format!("{}.bin", before.staged));
    let part = before.web_part.clone().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), part);
    let waiting = waiting_web_part(roots).unwrap();
    assert_eq!(waiting.id, before.staged);
    assert_eq!(waiting.bytes, u64::try_from(part.len()).unwrap());
    assert_eq!(result(roots).unwrap().stamp, stamp);

    // The browser's data from before its own restore joins the kept set.
    let host = host(&fixture);
    assert!(!pre_restores(roots)[0].has_web_part);
    let (id, previous) = web_part_upload(&host, UploadKind::PreviousWebPart, b"{\"old\":1}");
    let upload = host.uploaded(UploadKind::PreviousWebPart, &id).unwrap();
    keep_previous_web_part(roots, &upload).unwrap();
    assert!(!upload.exists());
    assert!(pre_restores(roots)[0].has_web_part);
    assert_eq!(
        std::fs::read(kept_dir(roots, RootKey::Core, &stamp).join("meta/web-part.bin")).unwrap(),
        previous
    );
    assert!(host.overview().await.web_part_waiting);
}

#[tokio::test]
async fn the_overview_names_the_waiting_restore_and_kept_sets() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let host = host(&fixture);
    let scheduled = schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    let overview = host.overview().await;
    assert_eq!(overview.pending, Some(scheduled));
    assert!(overview.pre_restores.is_empty());
    apply_pending(&fixture.roots, &fixture.store, None).unwrap();
    let overview = host.overview().await;
    assert_eq!(overview.pending, None);
    assert_eq!(overview.pre_restores, pre_restores(&fixture.roots));
    assert_eq!(overview.pre_restores.len(), 1);
    assert!(!overview.web_part_waiting);
}

#[tokio::test]
async fn only_whole_sources_are_scheduled_or_applied() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    for source in [
        RestoreSource::Staged { id: "0".repeat(32) },
        RestoreSource::Staged { id: "nope".into() },
        RestoreSource::PreRestore {
            stamp: "2026-11-20-1200-abcd".into(),
        },
        RestoreSource::Other,
    ] {
        assert!(matches!(
            schedule(roots, &source, false),
            Err(BackupError::Refused(GONE))
        ));
    }
    schedule(roots, &staged_source(&before), false).unwrap();
    assert!(matches!(
        schedule(roots, &staged_source(&before), false),
        Err(BackupError::Refused(RESTORE_PENDING))
    ));

    // A staging that no longer matches its manifest is never applied.
    std::fs::remove_dir_all(staging_dir(roots, RootKey::Core, &before.staged).join("server"))
        .unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None),
        Err(BackupError::Refused(GONE))
    ));
    assert_eq!(files_under(&roots.core, &["backups"]), before.core);
    assert!(pending(roots).is_none());
    assert!(!roots.backups(RootKey::Core).join("apply.json").exists());
}

#[tokio::test]
async fn a_linked_backups_folder_is_never_acted_on() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    // Other roots whose core `backups` is a link to the real one.
    let home = fixture.base.join("linked");
    let linked = Roots {
        home: home.clone(),
        core: home.join("core"),
        desktop: home.join("desktop"),
        config: home.join("core"),
    };
    std::fs::create_dir_all(&linked.core).unwrap();
    std::fs::create_dir_all(&linked.desktop).unwrap();
    write(&linked.core.join("mine.json"), b"{}");
    std::os::unix::fs::symlink(
        fixture.roots.backups(RootKey::Core),
        linked.backups(RootKey::Core),
    )
    .unwrap();
    assert!(apply_pending(&linked, &fixture.store, None).is_err());
    assert!(pending(&linked).is_none());
    assert!(pre_restores(&linked).is_empty());
    assert!(schedule(&linked, &staged_source(&before), false).is_err());
    assert!(linked.core.join("mine.json").exists());
    assert_eq!(files_under(&fixture.roots.core, &["backups"]), before.core);
    assert!(!fixture
        .roots
        .backups(RootKey::Core)
        .join("apply.json")
        .exists());
}
