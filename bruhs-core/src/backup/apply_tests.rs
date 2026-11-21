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

use super::apply::{apply_pending_with, Point};
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

/// Takes `file`'s lock for a test, trying again for a moment: a process
/// another test forks holds a copy of the lock until it execs.
fn hold(file: &File) {
    for _ in 0..200 {
        if file.try_lock().is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the lock stayed held");
}

fn never_stop(_: &Step, _: Point) -> bool {
    false
}

fn no_env(_: &str) -> Option<std::ffi::OsString> {
    None
}

/// Applies with a stop at the `at`-th crash point (counting from 0).
/// Answers whether the stop was reached.
fn apply_stopping_at(fixture: &Fixture, at: usize) -> bool {
    let seen = Cell::new(0_usize);
    let stop = |_: &Step, point: Point| {
        if point == Point::Before {
            return false;
        }
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
    let stop = |step: &Step, crash: Point| *step == Step::Sealed && crash == Point::Recorded;
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
    hold(&server);
    assert_eq!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Busy(Holder::Server)
    );
    untouched();
    // Its reactive database moved elsewhere: its lock is beside it there
    // (final review C-I1), and a relative override's can't be found
    // (ruling F10 c).
    drop(server);
    let outside = fixture.base.join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let moved_lock = outside.join("r.sqlite3.lock");
    std::fs::write(&moved_lock, b"").unwrap();
    let moved = File::open(&moved_lock).unwrap();
    hold(&moved);
    let db = outside.join("r.sqlite3");
    let elsewhere =
        |name: &str| (name == "BRUHS_REACTIVE_DB").then(|| std::ffi::OsString::from(&db));
    assert_eq!(
        apply_pending_with(roots, &fixture.store, None, &elsewhere, &never_stop).unwrap(),
        ApplyOutcome::Busy(Holder::Server)
    );
    untouched();
    drop(moved);
    let relative =
        |name: &str| (name == "BRUHS_REACTIVE_DB").then(|| std::ffi::OsString::from("r.sqlite3"));
    assert_eq!(
        apply_pending_with(roots, &fixture.store, None, &relative, &never_stop).unwrap(),
        ApplyOutcome::Busy(Holder::Server)
    );
    untouched();
    // Another apply.
    let restore = File::options()
        .read(true)
        .write(true)
        .open(roots.backups(RootKey::Core).join("restore.lock"))
        .unwrap();
    hold(&restore);
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

/// Final review C-I1: with `BRUHS_REACTIVE_DB` outside the roots and no
/// server holding its lock, the apply goes ahead and never touches that
/// database; a start then finds nothing waiting.
#[tokio::test]
async fn a_reactive_database_outside_the_roots_never_blocks_the_apply() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    let outside = fixture.base.join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let db = outside.join("reactive.sqlite3");
    std::fs::write(&db, b"the crew").unwrap();
    // A server that ran there and stopped left its lock file.
    std::fs::write(outside.join("reactive.sqlite3.lock"), b"").unwrap();
    let elsewhere =
        |name: &str| (name == "BRUHS_REACTIVE_DB").then(|| std::ffi::OsString::from(&db));
    let outcome = apply_pending_with(roots, &fixture.store, None, &elsewhere, &never_stop);
    assert!(
        matches!(outcome, Ok(ApplyOutcome::Applied(_))),
        "{outcome:?}"
    );
    assert_applied(&fixture, &before);
    assert_eq!(std::fs::read(&db).unwrap(), b"the crew");
    assert_eq!(
        apply_pending_with(roots, &fixture.store, None, &elsewhere, &never_stop).unwrap(),
        ApplyOutcome::NothingPending
    );
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
    let stop = |step: &Step, crash: Point| {
        if matches!(step, Step::Placed { .. }) && crash == Point::Recorded {
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
    let stop = |step: &Step, crash: Point| *step == Step::KeysWritten && crash == Point::Acted;
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

/// Final review B-I2: an item of today's that won't leave the keychain
/// after the swap (a denied prompt) never stops the apply: it stays,
/// harmless since the seal holds it, and the result counts it.
#[tokio::test]
async fn an_item_that_wont_go_is_counted_not_fatal() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, true).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    let store = fixture.store.clone();
    let refuse = |step: &Step, point: Point| {
        if *step == Step::KeysWritten && point == Point::Recorded {
            store.refuse_changes_after(Some(0));
        }
        false
    };
    let outcome = apply_pending_with(roots, &fixture.store, None, &no_env, &refuse);
    fixture.store.refuse_changes_after(None);
    let Ok(ApplyOutcome::Applied(applied)) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(applied.warnings, [items_stayed_line(1).unwrap()]);
    assert_eq!(result(roots).unwrap().warnings, applied.warnings);
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("new"));
    assert_eq!(item(&fixture.store, EXTRA).as_deref(), Some("extra"));
    assert!(!applying(roots));
    assert_eq!(pre_restores(roots).len(), 1);
}

/// Final review B-I2: the restored catalogs are read when the apply is
/// planned. Ones that don't read refuse the restore before anything
/// moves, so the apply never fails at the prune on every start.
#[tokio::test]
async fn restored_catalogs_that_dont_read_refuse_before_anything_moves() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, true).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    // Damaged since the open, as a crafted or newer catalog would be.
    let staged = staging_dir(roots, RootKey::Core, &before.staged);
    std::fs::write(staged.join("credentials/catalog.json"), b"{damaged").unwrap();
    let today = files_under(&roots.core, &["backups"]);
    let outcome = apply_pending(roots, &fixture.store, None);
    assert!(
        matches!(&outcome, Err(BackupError::Format(message)) if *message == DAMAGED),
        "{outcome:?}"
    );
    assert!(!roots.backups(RootKey::Core).join("apply.json").exists());
    assert!(pending(roots).is_none());
    assert!(!applying(roots));
    assert_eq!(files_under(&roots.core, &["backups"]), today);
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("old"));
    assert_eq!(item(&fixture.store, EXTRA).as_deref(), Some("extra"));
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
    // A second upload (a browser restore retried after its write) never
    // replaces the first: that one holds the data from before (B Task 14 I2).
    let (id, _) = web_part_upload(&host, UploadKind::PreviousWebPart, b"{\"restored\":1}");
    let upload = host.uploaded(UploadKind::PreviousWebPart, &id).unwrap();
    keep_previous_web_part(roots, &upload).unwrap();
    assert!(!upload.exists());
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

/// The stamp of the apply in progress, read from its journal.
fn journal_stamp(roots: &Roots) -> Option<String> {
    let text = std::fs::read(roots.backups(RootKey::Core).join("apply.json")).ok()?;
    let journal: serde_json::Value = serde_json::from_slice(&text).ok()?;
    journal["stamp"].as_str().map(str::to_owned)
}

fn kept_folders(roots: &Roots) -> Vec<String> {
    let mut names = Vec::new();
    for root in [RootKey::Core, RootKey::Desktop] {
        if let Ok(entries) = std::fs::read_dir(roots.backups(root)) {
            names.extend(
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with("pre-restore-")),
            );
        }
    }
    names
}

/// The end state of a rollback (B Task 8 I1): today's data byte for byte
/// and today's items, no kept set or its key, nothing waiting, the staged
/// restore left for the sweep, and the result saying so.
fn assert_rolled_back(fixture: &Fixture, before: &Before, stamp: Option<&str>) {
    let roots = &fixture.roots;
    assert_eq!(files_under(&roots.core, &["backups"]), before.core);
    if !roots.same() {
        assert_eq!(files_under(&roots.desktop, &["backups"]), before.desktop);
    }
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("old"));
    assert_eq!(item(&fixture.store, EXTRA).as_deref(), Some("extra"));
    assert!(pending(roots).is_none());
    assert!(!roots.backups(RootKey::Core).join("pending.json").exists());
    assert!(!roots.backups(RootKey::Core).join("apply.json").exists());
    assert!(pre_restores(roots).is_empty());
    assert_eq!(kept_folders(roots), Vec::<String>::new());
    if let Some(stamp) = stamp {
        assert!(no_key(&fixture.store, &format!("pre-restore:{stamp}")));
    }
    assert!(staging_dir(roots, RootKey::Core, &before.staged).exists());
    assert_eq!(
        result(roots).unwrap().warnings,
        vec![RESTORE_ROLLED_BACK.to_owned()]
    );
}

fn rolled_back(outcome: Result<ApplyOutcome, BackupError>) -> bool {
    matches!(outcome, Err(BackupError::Refused(message)) if message == RESTORE_ROLLED_BACK)
}

/// A rename that fails for good, at each rename in turn, puts today's
/// data back as it was (B Task 8 I1).
#[tokio::test]
async fn a_rename_that_fails_for_good_rolls_back() {
    for at in 0.. {
        let fixture = fixture(false, false);
        let before = stage_b(&fixture, BackupOptions::default(), true, true).await;
        schedule(&fixture.roots, &staged_source(&before), true).unwrap();
        let renames = Cell::new(0_usize);
        let stamp = std::cell::RefCell::new(None);
        let fail = |_: &Step, point: Point| {
            if point != Point::Before {
                return false;
            }
            *stamp.borrow_mut() = journal_stamp(&fixture.roots);
            let now = renames.get();
            renames.set(now + 1);
            now == at
        };
        let outcome = apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &fail);
        if matches!(outcome, Ok(ApplyOutcome::Applied(_))) {
            assert!(at > 6, "too few renames: {at}");
            assert_applied(&fixture, &before);
            break;
        }
        assert!(rolled_back(outcome), "rename {at}");
        assert_rolled_back(&fixture, &before, stamp.borrow().as_deref());
        // The service still starts again, and a new restore can be made.
        assert!(take_service_restart(&fixture.roots));
        schedule(&fixture.roots, &staged_source(&before), false).unwrap();
        assert!(matches!(
            apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
            ApplyOutcome::Applied(_)
        ));
    }
}

/// A stop anywhere in a rollback resumes into the rollback, even when the
/// resume stops too.
#[tokio::test]
async fn a_stop_during_a_rollback_resumes_the_rollback() {
    for at in 0.. {
        let fixture = fixture(false, false);
        let before = stage_b(&fixture, BackupOptions::default(), false, true).await;
        schedule(&fixture.roots, &staged_source(&before), false).unwrap();
        // Fail the sixth rename (past several moves and a place), then
        // stop at the `at`-th point of the rollback.
        let renames = Cell::new(0_usize);
        let points = Cell::new(0_usize);
        let stop = |step: &Step, point: Point| match point {
            Point::Before => {
                renames.set(renames.get() + 1);
                renames.get() == 6
            }
            _ if *step == Step::RollingBack || point == Point::Undone => {
                points.set(points.get() + 1);
                points.get() == at + 1
            }
            _ => false,
        };
        let stamp_before = std::cell::RefCell::new(None);
        let outcome = apply_pending_with(
            &fixture.roots,
            &fixture.store,
            None,
            &no_env,
            &|step, point| {
                if stamp_before.borrow().is_none() {
                    *stamp_before.borrow_mut() = journal_stamp(&fixture.roots);
                }
                stop(step, point)
            },
        );
        let stamp = stamp_before.borrow().clone();
        if rolled_back(outcome) {
            assert!(at > 4, "too few rollback points: {at}");
            assert_rolled_back(&fixture, &before, stamp.as_deref());
            break;
        }
        assert!(fixture
            .roots
            .backups(RootKey::Core)
            .join("apply.json")
            .exists());
        if at == 0 {
            // Stopped before `RollingBack` was recorded: nothing decided
            // the rollback yet, so the next start goes forward (where a
            // rename that fails for good would fail again).
            assert!(matches!(
                apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
                ApplyOutcome::Applied(_)
            ));
            assert_applied(&fixture, &before);
            continue;
        }
        // A second stop while resuming, then a clean resume.
        let again = Cell::new(0_usize);
        let second = |_: &Step, point: Point| {
            if point == Point::Before {
                return false;
            }
            again.set(again.get() + 1);
            again.get() == 2
        };
        let outcome = apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &second);
        if !rolled_back(outcome) {
            let last = apply_pending(&fixture.roots, &fixture.store, None);
            let shown = format!("{last:?}");
            assert!(rolled_back(last), "rollback point {at}: {shown}");
        }
        assert_rolled_back(&fixture, &before, stamp.as_deref());
    }
}

/// Something that takes a name mid-apply (the capture daemon's `audio/`,
/// say) is in the way for good: the restore rolls back, and the newcomer
/// is kept aside in a listed set rather than lost.
#[tokio::test]
async fn an_entry_in_the_way_rolls_back_and_is_kept() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, true).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    let intruder = fixture.roots.core.join("server/intruder.txt");
    let planted = Cell::new(false);
    let plant = |step: &Step, point: Point| {
        if matches!(step, Step::Placed { .. }) && point == Point::Recorded && !planted.get() {
            planted.set(true);
            write(&intruder, b"made mid-apply");
        }
        false
    };
    let outcome = apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &plant);
    assert!(rolled_back(outcome));
    let roots = &fixture.roots;
    assert_eq!(files_under(&roots.core, &["backups"]), before.core);
    assert_eq!(files_under(&roots.desktop, &["backups"]), before.desktop);
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("old"));
    assert!(pending(roots).is_none());
    let kept = pre_restores(roots);
    assert_eq!(kept.len(), 1);
    let set = kept_dir(roots, RootKey::Core, &kept[0].stamp);
    assert_eq!(
        std::fs::read(set.join(".in-the-way-1/server/intruder.txt")).unwrap(),
        b"made mid-apply"
    );
    assert!(!set.join("meta/keychain.sealed").exists());
    assert!(no_key(
        &fixture.store,
        &format!("pre-restore:{}", kept[0].stamp)
    ));
}

/// Forward too: a resume that stops is resumed again (B Task 8 M9).
#[tokio::test]
async fn a_second_stop_while_resuming_still_finishes() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), true, true).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    let first =
        |step: &Step, point: Point| matches!(step, Step::Placed { .. }) && point == Point::Acted;
    assert!(apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &first).is_err());
    let second = |step: &Step, point: Point| *step == Step::KeysPruned && point == Point::Acted;
    assert!(apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, &second).is_err());
    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert_applied(&fixture, &before);
}

/// Refusals that can never pass remove `pending.json`, so backups and
/// restores aren't blocked; nothing else changes (B Task 8 I2).
#[tokio::test]
async fn permanent_refusals_clear_the_pending_restore() {
    let backups = |fixture: &Fixture| fixture.roots.backups(RootKey::Core);
    // A damaged pending.json, and one a newer Bruhs wrote.
    for (text, expected) in [
        (&b"{not json"[..], DAMAGED),
        (
            &br#"{"source":{"type":"from_the_future"},"created_at_ms":0}"#[..],
            INVALID_REQUEST,
        ),
    ] {
        let fixture = fixture(false, false);
        std::fs::create_dir_all(backups(&fixture)).unwrap();
        std::fs::write(backups(&fixture).join("pending.json"), text).unwrap();
        let outcome = apply_pending(&fixture.roots, &fixture.store, None);
        assert!(
            matches!(&outcome, Err(error) if error.message() == expected),
            "{outcome:?}"
        );
        assert!(!backups(&fixture).join("pending.json").exists());
    }

    // The staged items' key is gone.
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    crate::secrets::forget_seal_key(&fixture.store, &format!("staged:{}", before.staged)).unwrap();
    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None),
        Err(BackupError::Keychain(message)) if message == KEYCHAIN_SET_UNREADABLE
    ));
    assert!(pending(&fixture.roots).is_none());
    assert_eq!(files_under(&fixture.roots.core, &["backups"]), before.core);

    // A folder this user can't move (I1 f): refused before anything moves.
    let fixture = self::fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    let server = fixture.roots.core.join("server");
    std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o500)).unwrap();
    let outcome = apply_pending(&fixture.roots, &fixture.store, None);
    std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        matches!(&outcome, Err(error) if error.message() == RESTORE_CANT_MOVE),
        "{outcome:?}"
    );
    assert!(pending(&fixture.roots).is_none());
    assert!(!backups(&fixture).join("apply.json").exists());
    assert_eq!(files_under(&fixture.roots.core, &["backups"]), before.core);
    assert_eq!(kept_folders(&fixture.roots), Vec::<String>::new());
}

/// A keychain that may answer later keeps the restore waiting.
#[tokio::test]
async fn a_transient_refusal_keeps_the_pending_restore() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    // A key the keychain answers with something unreadable is a storage
    // error, which may pass on a retry.
    fixture
        .store
        .write_item(
            crate::secrets::BACKUP_SERVICE,
            &format!("staged:{}", before.staged),
            "not a key",
        )
        .unwrap();
    assert!(matches!(
        apply_pending(&fixture.roots, &fixture.store, None),
        Err(BackupError::Keychain(message)) if message == KEYCHAIN_NEEDS_YOU
    ));
    assert!(pending(&fixture.roots).is_some());
    assert_eq!(files_under(&fixture.roots.core, &["backups"]), before.core);
}

/// Staged data the manifest doesn't name is never placed (B Task 8 M4):
/// a file is damage, an empty folder stays behind.
#[tokio::test]
async fn staged_data_the_manifest_doesnt_name_is_refused() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    let staging = staging_dir(roots, RootKey::Core, &before.staged);
    write(&staging.join("smuggled/extra.json"), b"{}");
    assert!(matches!(
        schedule(roots, &staged_source(&before), false),
        Err(BackupError::Format(DAMAGED))
    ));
    std::fs::remove_dir_all(staging.join("smuggled")).unwrap();
    schedule(roots, &staged_source(&before), false).unwrap();
    // Added after the schedule: refused at apply, and the wait is over.
    write(&staging.join("late.json"), b"{}");
    assert!(matches!(
        apply_pending(roots, &fixture.store, None),
        Err(BackupError::Format(DAMAGED))
    ));
    assert!(pending(roots).is_none());
    assert_eq!(files_under(&roots.core, &["backups"]), before.core);

    std::fs::remove_file(staging.join("late.json")).unwrap();
    std::fs::create_dir_all(staging.join("empty/inside")).unwrap();
    schedule(roots, &staged_source(&before), false).unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert!(!roots.core.join("empty").exists());
    assert_applied(&fixture, &before);
}

/// A journal is never acted on for other roots than this computer's, or
/// with a stamp that names a set already kept (B Task 8 M3).
#[tokio::test]
async fn a_journal_that_doesnt_fit_is_refused() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    let sealed = |step: &Step, point: Point| *step == Step::Sealed && point == Point::Acted;
    assert!(apply_pending_with(roots, &fixture.store, None, &no_env, &sealed).is_err());

    // The same core root seen as one root: a two-pass journal won't do.
    let one = Roots {
        desktop: roots.core.clone(),
        ..roots.clone()
    };
    assert!(matches!(
        apply_pending(&one, &fixture.store, None),
        Err(BackupError::Format(DAMAGED))
    ));

    // A stamp that names a kept set already there.
    let taken = "2026-01-01-0000-abcd";
    let set = kept_dir(roots, RootKey::Core, taken);
    write(&set.join("meta/keychain.sealed"), b"theirs");
    write(
        &set.join("meta/info.json"),
        &serde_json::to_vec(&serde_json::json!({
            "stamp": taken, "made_at_ms": 1, "bytes": 0, "keychain": true,
            "roots": [{ "root": "core", "names": [] }, { "root": "desktop", "names": [] }],
        }))
        .unwrap(),
    );
    std::fs::create_dir_all(kept_dir(roots, RootKey::Desktop, taken)).unwrap();
    let journal = roots.backups(RootKey::Core).join("apply.json");
    let mut text: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
    let own = text["stamp"].as_str().unwrap().to_owned();
    text["stamp"] = taken.into();
    std::fs::write(&journal, serde_json::to_vec(&text).unwrap()).unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None),
        Err(BackupError::Format(DAMAGED))
    ));
    assert_eq!(
        std::fs::read(set.join("meta/keychain.sealed")).unwrap(),
        b"theirs"
    );
    assert_eq!(files_under(&roots.core, &["backups"]), before.core);

    // Its own stamp back, it finishes.
    text["stamp"] = own.into();
    std::fs::write(&journal, serde_json::to_vec(&text).unwrap()).unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
}

#[tokio::test]
async fn a_linked_desktop_backups_folder_is_never_acted_on() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    let real = fixture.base.join("desktop-backups");
    std::fs::rename(roots.backups(RootKey::Desktop), &real).unwrap();
    std::os::unix::fs::symlink(&real, roots.backups(RootKey::Desktop)).unwrap();
    assert!(apply_pending(roots, &fixture.store, None).is_err());
    assert!(pending(roots).is_none());
    assert!(pre_restores(roots).is_empty());
    assert_eq!(files_under(&roots.core, &["backups"]), before.core);
    assert!(!roots.backups(RootKey::Core).join("apply.json").exists());
    // The real folder back, the restore still waits and applies.
    std::fs::remove_file(roots.backups(RootKey::Desktop)).unwrap();
    std::fs::rename(&real, roots.backups(RootKey::Desktop)).unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    assert_applied(&fixture, &before);
}

/// Linux's layout: one data root and a config root of its own.
#[tokio::test]
async fn calendar_goes_back_to_the_config_root_on_one_root() {
    let fixture = fixture(true, true);
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

/// While an apply holds `restore.lock`, nothing schedules a restore or
/// changes a kept set (B Task 8 M7), and the sweep leaves its partial
/// records alone; with the lock free, the sweep removes them (M11).
#[tokio::test]
async fn the_restore_lock_holds_off_schedule_the_web_part_and_the_sweep() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    apply_pending(roots, &fixture.store, None).unwrap();
    let host = host(&fixture);
    let (id, _) = web_part_upload(&host, UploadKind::PreviousWebPart, b"{}");
    let upload = host.uploaded(UploadKind::PreviousWebPart, &id).unwrap();

    let backups = roots.backups(RootKey::Core);
    let partials: Vec<PathBuf> = ["apply.json", "pending.json", "result.json", "last.json"]
        .iter()
        .map(|name| backups.join(format!(".{name}.partial-{}", "a".repeat(32))))
        .collect();
    for partial in &partials {
        std::fs::write(partial, b"half").unwrap();
    }
    let lock = File::options()
        .read(true)
        .write(true)
        .open(backups.join("restore.lock"))
        .unwrap();
    hold(&lock);
    assert!(matches!(
        schedule(roots, &staged_source(&before), false),
        Err(BackupError::Busy(BACKUP_BUSY))
    ));
    assert!(matches!(
        keep_previous_web_part(roots, &upload),
        Err(BackupError::Busy(BACKUP_BUSY))
    ));
    host.sweep();
    assert!(!partials[3].exists());
    assert!(partials[..3].iter().all(|partial| partial.exists()));
    drop(lock);
    keep_previous_web_part(roots, &upload).unwrap();
    host.sweep();
    assert!(partials.iter().all(|partial| !partial.exists()));
}

/// Applies with `hook` and expects a rollback.
fn apply_rolling_back(fixture: &Fixture, hook: &dyn Fn(&Step, Point) -> bool) {
    let outcome = apply_pending_with(&fixture.roots, &fixture.store, None, &no_env, hook);
    assert!(rolled_back(outcome));
}

fn placed(step: &Step, wanted: &str) -> bool {
    matches!(step, Step::Placed { name, .. } if name == wanted)
}

/// A rollback leaves an earlier restore's waiting browser part waiting
/// (B Task 8 N1).
#[tokio::test]
async fn a_rollback_keeps_a_waiting_web_part() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), true, false).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    apply_pending(&fixture.roots, &fixture.store, None).unwrap();
    let waiting = waiting_web_part(&fixture.roots).unwrap();
    let stamp = pre_restores(&fixture.roots)[0].stamp.clone();
    schedule(&fixture.roots, &RestoreSource::PreRestore { stamp }, false).unwrap();
    apply_rolling_back(&fixture, &|_, point| point == Point::Before);
    assert_eq!(waiting_web_part(&fixture.roots), Some(waiting));
}

/// A park never lands on an earlier one (N2), and a file written inside a
/// restored folder after it was placed is parked, not sent back to the
/// staging the sweep removes (N3).
#[tokio::test]
async fn a_rollback_parks_what_was_made_mid_apply_under_free_names() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, true).await;
    schedule(&fixture.roots, &staged_source(&before), false).unwrap();
    let roots = &fixture.roots;
    let hook = |step: &Step, point: Point| {
        if placed(step, "attachments") && point == Point::Recorded {
            let stamp = journal_stamp(roots).unwrap();
            // An earlier park of the same name.
            write(
                &kept_dir(roots, RootKey::Core, &stamp).join(".in-the-way-1/server/old.txt"),
                b"parked before",
            );
            write(&roots.core.join("server/intruder.txt"), b"made mid-apply");
            write(
                &roots.core.join("attachments/new-note.txt"),
                b"written after placing",
            );
        }
        false
    };
    apply_rolling_back(&fixture, &hook);
    assert_eq!(files_under(&roots.core, &["backups"]), before.core);
    assert_eq!(files_under(&roots.desktop, &["backups"]), before.desktop);
    let kept = pre_restores(roots);
    assert_eq!(kept.len(), 1);
    let set = kept_dir(roots, RootKey::Core, &kept[0].stamp);
    let read = |rel: &str| std::fs::read(set.join(rel)).unwrap();
    assert_eq!(read(".in-the-way-1/server/old.txt"), b"parked before");
    assert_eq!(read(".in-the-way-2/server/intruder.txt"), b"made mid-apply");
    assert_eq!(
        read(".in-the-way-1/attachments/new-note.txt"),
        b"written after placing"
    );
    let staged = files_under(
        &staging_dir(roots, RootKey::Core, &before.staged),
        &["meta"],
    );
    assert_eq!(staged, before.staged_core);

    // That set holds only what was parked: there is nothing to put back,
    // whether asked now or found waiting at a start, and nothing changes.
    let source = RestoreSource::PreRestore {
        stamp: kept[0].stamp.clone(),
    };
    let today = files_under(&roots.core, &["backups"]);
    let set_files = files_under(&set, &[]);
    assert!(matches!(
        schedule(roots, &source, false),
        Err(BackupError::Refused(message)) if message == NOTHING_TO_PUT_BACK
    ));
    std::fs::write(
        roots.backups(RootKey::Core).join("pending.json"),
        serde_json::to_vec(&serde_json::json!({ "source": source, "created_at_ms": 0 })).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None),
        Err(BackupError::Refused(message)) if message == NOTHING_TO_PUT_BACK
    ));
    assert!(pending(roots).is_none());
    assert_eq!(files_under(&roots.core, &["backups"]), today);
    assert_eq!(files_under(&set, &[]), set_files);
    assert_eq!(pre_restores(roots), kept);
}

/// A put back carries a set's parked folders into the new set and never
/// places them (N7); an owner's own entries that only look alike, a
/// `.in-the-way-notes/` folder and a file named `.in-the-way`, are placed
/// like any other (R2-M1, R3-M1).
#[tokio::test]
async fn a_put_back_carries_parked_folders_into_the_new_set() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    write(
        &roots.core.join(".in-the-way-notes/mine.txt"),
        b"owner's own",
    );
    write(&roots.core.join(".in-the-way"), b"an owner's file");
    write(&roots.core.join(".in-the-way-7"), b"a file, not a folder");
    let today = files_under(&roots.core, &["backups"]);
    schedule(roots, &staged_source(&before), false).unwrap();
    apply_pending(roots, &fixture.store, None).unwrap();
    let first = pre_restores(roots)[0].stamp.clone();
    write(
        &kept_dir(roots, RootKey::Core, &first).join(".in-the-way-1/audio/late.txt"),
        b"parked",
    );
    schedule(
        roots,
        &RestoreSource::PreRestore {
            stamp: first.clone(),
        },
        false,
    )
    .unwrap();
    let ApplyOutcome::Applied(result) = apply_pending(roots, &fixture.store, None).unwrap() else {
        panic!("not applied");
    };
    assert!(result.warnings.is_empty());
    assert_eq!(files_under(&roots.core, &NEVER), without(&today, &NEVER));
    assert!(!roots.core.join(".in-the-way-1").exists());
    let kept = pre_restores(roots);
    assert_eq!(kept.len(), 1);
    let set = kept_dir(roots, RootKey::Core, &kept[0].stamp);
    assert_eq!(
        std::fs::read(set.join(".in-the-way-1/audio/late.txt")).unwrap(),
        b"parked"
    );
    assert!(!kept_dir(roots, RootKey::Core, &first).exists());
}

/// A carry that fails after the keychain writes is skipped with a warning
/// rather than repeated (R2-M3): the parked entries stay in the set put
/// back, which stays as a set with nothing to put back.
#[tokio::test]
async fn a_carry_that_fails_is_skipped_and_its_set_kept() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    apply_pending(roots, &fixture.store, None).unwrap();
    let first = pre_restores(roots)[0].stamp.clone();
    let parked = kept_dir(roots, RootKey::Core, &first).join(".in-the-way-1");
    write(&parked.join("audio/late.txt"), b"parked");
    std::fs::set_permissions(&parked, std::fs::Permissions::from_mode(0o500)).unwrap();
    schedule(
        roots,
        &RestoreSource::PreRestore {
            stamp: first.clone(),
        },
        false,
    )
    .unwrap();
    let outcome = apply_pending(roots, &fixture.store, None);
    std::fs::set_permissions(&parked, std::fs::Permissions::from_mode(0o700)).unwrap();
    let Ok(ApplyOutcome::Applied(result)) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(result.warnings, vec![PARKED_STAYED.to_owned()]);
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("old"));
    assert!(!roots.backups(RootKey::Core).join("apply.json").exists());
    let kept = pre_restores(roots);
    assert_eq!(kept.len(), 2);
    let old = kept.iter().find(|set| set.stamp == first).unwrap();
    assert!(old.bytes > 0);
    assert_eq!(
        files_under(&kept_dir(roots, RootKey::Core, &first), &["meta"]),
        BTreeMap::from([(
            PathBuf::from(".in-the-way-1/audio/late.txt"),
            b"parked".to_vec()
        )])
    );
    assert!(!kept_dir(roots, RootKey::Core, &first)
        .join("meta/keychain.sealed")
        .exists());
    assert!(no_key(&fixture.store, &format!("pre-restore:{first}")));
    assert!(matches!(
        schedule(roots, &RestoreSource::PreRestore { stamp: first }, false),
        Err(BackupError::Refused(message)) if message == NOTHING_TO_PUT_BACK
    ));
}

/// The sweep never judges a set whose manifest is there but doesn't read
/// (R2-I1), and finishes a delete that stopped part way.
#[tokio::test]
async fn the_sweep_keeps_unreadable_sets_and_finishes_deletes() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    apply_pending(roots, &fixture.store, None).unwrap();
    let stamp = pre_restores(roots)[0].stamp.clone();
    let info = kept_dir(roots, RootKey::Core, &stamp).join("meta/info.json");
    let good = std::fs::read(&info).unwrap();
    std::fs::write(&info, b"{written by something else").unwrap();
    let doomed = roots
        .backups(RootKey::Desktop)
        .join(".deleting-2026-01-01-0000-abcd");
    write(&doomed.join("half/file.txt"), b"going");
    let host = host(&fixture);
    host.sweep();
    for root in [RootKey::Core, RootKey::Desktop] {
        assert!(kept_dir(roots, root, &stamp).exists());
    }
    assert!(!no_key(&fixture.store, &format!("pre-restore:{stamp}")));
    assert!(!doomed.exists());
    // A newer Bruhs' info with fields this one doesn't know still reads.
    let mut newer: serde_json::Value = serde_json::from_slice(&good).unwrap();
    newer["from_the_future"] = true.into();
    newer.as_object_mut().unwrap().remove("bytes");
    std::fs::write(&info, serde_json::to_vec(&newer).unwrap()).unwrap();
    assert_eq!(pre_restores(roots).len(), 1);
    // A delete leaves no `.deleting-` folder behind.
    delete_pre_restore(roots, &fixture.store, &stamp).unwrap();
    for root in [RootKey::Core, RootKey::Desktop] {
        let left: Vec<String> = std::fs::read_dir(roots.backups(root))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".deleting-") || name.starts_with("pre-restore-"))
            .collect();
        assert_eq!(left, Vec::<String>::new());
    }
}

/// A seal that fails for good, with nothing moved yet, drops the apply:
/// no journal, nothing waiting, no kept folders (N4).
#[tokio::test]
async fn a_seal_that_fails_for_good_drops_the_apply() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    std::fs::write(roots.core.join("credentials/catalog.json"), b"{damaged").unwrap();
    let today = files_under(&roots.core, &["backups"]);
    assert!(matches!(
        apply_pending(roots, &fixture.store, None),
        Err(BackupError::Keychain(message)) if message == KEYCHAIN_READ_FAILED
    ));
    assert!(!roots.backups(RootKey::Core).join("apply.json").exists());
    assert!(pending(roots).is_none());
    assert_eq!(kept_folders(roots), Vec::<String>::new());
    assert_eq!(files_under(&roots.core, &["backups"]), today);
    assert!(staging_dir(roots, RootKey::Core, &before.staged).exists());
}

/// Final review B-I1: a folder in today's data that can't be read (a
/// bind mount another user owns) never stops the apply past the keychain
/// writes: the set's size counts what can be read and the set is listed.
#[tokio::test]
async fn an_unreadable_folder_in_todays_data_never_stops_the_apply() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    let locked = roots.core.join("server/bruhs/a-bruh/work/pgdata");
    write(&locked.join("PG_VERSION"), b"16");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        // Running as root: nothing is unreadable here.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    schedule(roots, &staged_source(&before), false).unwrap();
    let outcome = apply_pending(roots, &fixture.store, None);
    let kept = pre_restores(roots);
    let stamp = kept.first().map(|set| set.stamp.clone());
    if let Some(stamp) = &stamp {
        let moved = kept_dir(roots, RootKey::Core, stamp).join("server/bruhs/a-bruh/work/pgdata");
        let _ = std::fs::set_permissions(&moved, std::fs::Permissions::from_mode(0o700));
    }
    let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700));
    assert!(
        matches!(outcome, Ok(ApplyOutcome::Applied(_))),
        "{outcome:?}"
    );
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(kept[0].bytes > 0);
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("new"));
    assert!(!applying(roots));
}

/// The sweep removes kept folders without a manifest, but never while an
/// apply could be making one (N6), and forgets the keys a removal failed
/// for once their set is gone.
#[tokio::test]
async fn the_sweep_removes_unlisted_kept_folders_and_their_keys() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    apply_pending(roots, &fixture.store, None).unwrap();
    let listed = pre_restores(roots)[0].stamp.clone();
    let stray = "2026-01-01-0000-abcd";
    let gone = "2026-01-02-0000-abcd";
    for root in [RootKey::Core, RootKey::Desktop] {
        write(&kept_dir(roots, root, stray).join("half/file.txt"), b"left");
    }
    for stamp in [stray, gone] {
        crate::secrets::store_seal_key(&fixture.store, &format!("pre-restore:{stamp}"), &[7; 32])
            .unwrap();
    }
    let backups = roots.backups(RootKey::Core);
    std::fs::write(
        backups.join("keys-to-forget.json"),
        serde_json::to_vec(&[
            format!("pre-restore:{gone}"),
            format!("pre-restore:{listed}"),
            "not an account".to_owned(),
        ])
        .unwrap(),
    )
    .unwrap();
    let host = host(&fixture);

    // A journal waiting: nothing of a set goes.
    std::fs::write(backups.join("apply.json"), b"{}").unwrap();
    host.sweep();
    assert!(kept_dir(roots, RootKey::Core, stray).exists());
    std::fs::remove_file(backups.join("apply.json")).unwrap();
    // An apply holding the lock: likewise.
    let lock = File::options()
        .read(true)
        .write(true)
        .open(backups.join("restore.lock"))
        .unwrap();
    hold(&lock);
    host.sweep();
    assert!(kept_dir(roots, RootKey::Desktop, stray).exists());
    drop(lock);

    host.sweep();
    for root in [RootKey::Core, RootKey::Desktop] {
        assert!(!kept_dir(roots, root, stray).exists());
        assert!(kept_dir(roots, root, &listed).exists());
    }
    assert!(no_key(&fixture.store, &format!("pre-restore:{stray}")));
    assert!(no_key(&fixture.store, &format!("pre-restore:{gone}")));
    assert!(!no_key(&fixture.store, &format!("pre-restore:{listed}")));
    // The listed set's key stays on the list until that set goes (R2-M2).
    let list: Vec<String> =
        serde_json::from_slice(&std::fs::read(backups.join("keys-to-forget.json")).unwrap())
            .unwrap();
    assert_eq!(list, vec![format!("pre-restore:{listed}")]);
    std::fs::remove_dir_all(kept_dir(roots, RootKey::Core, &listed)).unwrap();
    std::fs::remove_dir_all(kept_dir(roots, RootKey::Desktop, &listed)).unwrap();
    host.sweep();
    assert!(no_key(&fixture.store, &format!("pre-restore:{listed}")));
    assert!(!backups.join("keys-to-forget.json").exists());
}

/// Clearing the service restart waits for an apply's lock (N8).
#[tokio::test]
async fn the_service_restart_is_taken_under_the_restore_lock() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    schedule(&fixture.roots, &staged_source(&before), true).unwrap();
    apply_pending(&fixture.roots, &fixture.store, None).unwrap();
    let lock = File::options()
        .read(true)
        .write(true)
        .open(fixture.roots.backups(RootKey::Core).join("restore.lock"))
        .unwrap();
    hold(&lock);
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(lock);
    });
    let started = std::time::Instant::now();
    assert!(take_service_restart(&fixture.roots));
    assert!(started.elapsed() >= Duration::from_millis(250));
    release.join().unwrap();
    assert!(!take_service_restart(&fixture.roots));
}

/// A kept set's info without `keychain` reads it from the sealed file,
/// and one without `roots` is damage, not a set with nothing in it
/// (R3-M3).
#[tokio::test]
async fn an_info_missing_fields_reads_safely() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    schedule(roots, &staged_source(&before), false).unwrap();
    apply_pending(roots, &fixture.store, None).unwrap();
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("new"));
    let stamp = pre_restores(roots)[0].stamp.clone();
    let path = kept_dir(roots, RootKey::Core, &stamp).join("meta/info.json");
    let info: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let source = RestoreSource::PreRestore { stamp };

    let mut no_roots = info.clone();
    no_roots.as_object_mut().unwrap().remove("roots");
    std::fs::write(&path, serde_json::to_vec(&no_roots).unwrap()).unwrap();
    assert!(matches!(
        schedule(roots, &source, false),
        Err(BackupError::Format(DAMAGED))
    ));

    let mut no_keychain = info;
    no_keychain.as_object_mut().unwrap().remove("keychain");
    std::fs::write(&path, serde_json::to_vec(&no_keychain).unwrap()).unwrap();
    schedule(roots, &source, false).unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
    // The sealed items were written back: today's item is A's again.
    assert_eq!(item(&fixture.store, VALUE).as_deref(), Some("old"));
}

/// A lock a forked process still holds a copy of for a moment (until it
/// execs) isn't taken for a running apply: the lock is tried again first.
#[tokio::test]
async fn a_lock_a_forked_copy_still_holds_for_a_moment_is_free() {
    let fixture = fixture(false, false);
    let before = stage_b(&fixture, BackupOptions::default(), false, false).await;
    let roots = &fixture.roots;
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(roots.backups(RootKey::Core).join("restore.lock"))
        .unwrap();
    hold(&lock);
    // The same open file, as a forked child shares it until it execs.
    let copy = lock.try_clone().unwrap();
    drop(lock);
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        drop(copy);
    });
    schedule(roots, &staged_source(&before), false).unwrap();
    release.join().unwrap();
    assert!(matches!(
        apply_pending(roots, &fixture.store, None).unwrap(),
        ApplyOutcome::Applied(_)
    ));
}

/// One `restore.log` for the applier and both apps (B Task 10 I6):
/// appended to, made private when it was there before, never a link.
#[test]
fn the_restore_log_is_private_and_never_a_link() {
    let fixture = fixture(false, false);
    let core = &fixture.roots.core;
    let log = core.join("logs/restore.log");
    write(&log, b"old ");
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
    open_restore_log(core).unwrap().write_all(b"new").unwrap();
    assert_eq!(std::fs::read(&log).unwrap(), b"old new");
    let mode = std::fs::metadata(&log).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    std::fs::remove_file(&log).unwrap();
    let elsewhere = fixture.base.join("elsewhere.log");
    std::os::unix::fs::symlink(&elsewhere, &log).unwrap();
    assert!(open_restore_log(core).is_err());
    assert!(!elsewhere.exists());
    // A linked logs folder is refused too, and nothing lands where it
    // points (B Task 10 N3).
    std::fs::remove_dir_all(core.join("logs")).unwrap();
    let outside = fixture.base.join("outside-logs");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, core.join("logs")).unwrap();
    assert!(open_restore_log(core).is_err());
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}
