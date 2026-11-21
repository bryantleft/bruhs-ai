//! Opening a backup to restore, over temporary roots and a file-test
//! keychain only. Good backups come from Task 6's `create`; hostile ones
//! are tars written by hand and encrypted. Keys derive at
//! `KdfParams::FLOOR`.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bruhs_backup::{Decryptor, Encryptor, Header, Kdf, KdfParams, Key, Kind, Secret, CHUNK_BYTES};
use bruhs_models::backup::*;
use bruhs_models::crew::CREW_PROTOCOL;
use bruhs_models::secrets::SavedCredentialState;
use bruhs_models::vault::{CredentialScope, VaultEntry, VaultKind};
use bruhs_models::EpochMs;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::host::Limits;
use super::open::{extract, staging_dir};
use super::*;
use crate::secrets::{read_seal_key, unseal_items, SecretError, SecretStore, SECRETS_SERVICE};

const PASSWORD: &str = "correct horse 1";
const GLOBAL_VALUE: &str = "v1-11111111111111111111111111111111";
/// A path over the 100 bytes a tar header holds, so it travels as a GNU
/// long name.
const LONG_FOLDER: &str = "workbench/a-folder-name-long-enough-that-the-whole-path-is-past-one-hundred-bytes-of-tar-header";

/// A path whose archive name `core/<it>` is exactly 100 bytes: it fills
/// the tar header's name field with no NUL and no long name.
fn hundred_bytes() -> String {
    format!("workbench/{}", "h".repeat(100 - "core/workbench/".len()))
}

struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
    roots: Roots,
    store: SecretStore,
    canary: String,
    /// A folder outside both roots for backup files.
    out: PathBuf,
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Roots under a resolved temp home, two folders unless `same`: a mail
/// database, a vault Global Value with its keychain item, a transcript, a
/// runnable script, a long path, a 3 MiB attachment (so the backup has
/// several chunks) and the desktop's state.
fn fixture(same: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let home = base.join("home");
    let core = home.join("core");
    let roots = Roots {
        home: home.clone(),
        core: core.clone(),
        desktop: if same {
            core.clone()
        } else {
            home.join("desktop")
        },
        config: core.clone(),
    };
    std::fs::create_dir_all(core.join("server")).unwrap();
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
    let store = SecretStore::file_test(core.join("credentials"));
    let canary = format!("canary-{}", uuid::Uuid::new_v4().simple());
    store
        .write_item(
            SECRETS_SERVICE,
            &format!("vault:global:{GLOBAL_VALUE}"),
            &canary,
        )
        .unwrap();

    write(&core.join("server/notes.json"), b"{\"notes\":[]}");
    write(&core.join("audio/m1.jsonl"), b"{\"text\":\"hi\"}\n");
    write(&core.join("audio/m1.wav"), &[7; 3000]);
    let script = core.join("attachments/run.sh");
    write(&script, b"#!/bin/sh\necho hi\n");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    write(&core.join(LONG_FOLDER).join("note.txt"), b"far down");
    write(&core.join(hundred_bytes()), b"one hundred");
    write(&core.join("server/empty.json"), b"");
    let big: Vec<u8> = (0..3 * 1024 * 1024_u32)
        .map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).unwrap())
        .collect();
    write(&core.join("attachments/big.bin"), &big);
    write(&roots.desktop.join("desktop-state.json"), b"{\"open\":[]}");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    Fixture {
        _dir: dir,
        base,
        roots,
        store,
        canary,
        out,
    }
}

fn host(fixture: &Fixture) -> BackupHost {
    host_on(fixture.roots.clone(), fixture)
}

fn host_on(roots: Roots, fixture: &Fixture) -> BackupHost {
    BackupHost::new(roots, fixture.store.clone()).for_tests(false)
}

/// Fresh roots under the fixture's temp folder, for opening a backup
/// somewhere else.
fn other_roots(fixture: &Fixture, name: &str, same: bool) -> Roots {
    let home = fixture.base.join(name);
    let core = home.join("core");
    std::fs::create_dir_all(&core).unwrap();
    let desktop = if same {
        core.clone()
    } else {
        home.join("desktop")
    };
    std::fs::create_dir_all(&desktop).unwrap();
    Roots {
        home,
        core: core.clone(),
        desktop,
        config: core,
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

/// Makes a backup of `fixture`'s roots at `out/<name>.bruhsbak`.
async fn make_backup(fixture: &Fixture, name: &str) -> PathBuf {
    let host = host(fixture);
    let path = fixture.out.join(format!("{name}.bruhsbak"));
    let job = host
        .start_create(CreateRequest {
            password: Zeroizing::new(PASSWORD.into()),
            options: BackupOptions::default(),
            app: BackupApp::Desktop,
            web_part: None,
            web_db_version: None,
            output: Output::File {
                path: path.clone(),
                overwrite: false,
            },
        })
        .await
        .unwrap();
    match finished(&host, &job.id).await {
        JobState::Created { .. } => path,
        other => panic!("{other:?}"),
    }
}

/// Opens `path` with `password` and answers the job's id and end state.
async fn open_with(host: &BackupHost, path: &Path, password: &str) -> (String, JobState) {
    let job = host
        .start_open(path.to_path_buf(), Zeroizing::new(password.into()), false)
        .unwrap();
    let state = finished(host, &job.id).await;
    (job.id, state)
}

fn preview(state: JobState) -> RestorePreview {
    match state {
        JobState::Opened { preview } => *preview,
        other => panic!("{other:?}"),
    }
}

fn failure(state: JobState) -> String {
    match state {
        JobState::Failed { message } => message,
        other => panic!("{other:?}"),
    }
}

/// A backup's manifest, read with its password.
fn manifest_in(path: &Path) -> BackupManifest {
    let password = normalize_password(PASSWORD);
    let decryptor = Decryptor::open(
        File::open(path).unwrap(),
        Secret::Password(&password),
        Kind::Full,
    )
    .unwrap();
    let mut archive = tar::Archive::new(decryptor);
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if &*entry.path_bytes() == b"manifest.json" {
            let mut text = Vec::new();
            entry.read_to_end(&mut text).unwrap();
            return serde_json::from_slice(&text).unwrap();
        }
    }
    panic!("no manifest");
}

/// Every file under `folder`, by path relative to it, with its bytes.
fn files_under(folder: &Path, skip_backups: bool) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(at: &Path, base: &Path, skip: bool, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(at) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap().to_path_buf();
            if skip && rel.starts_with(BACKUPS) {
                continue;
            }
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(&path, base, skip, into);
            } else if kind.is_file() {
                into.insert(rel, std::fs::read(&path).unwrap());
            } else {
                into.insert(rel, b"(not a file)".to_vec());
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(folder, folder, skip_backups, &mut files);
    files
}

/// What both roots hold, `backups` aside.
fn live(roots: &Roots) -> Vec<BTreeMap<PathBuf, Vec<u8>>> {
    vec![
        files_under(&roots.core, true),
        files_under(&roots.desktop, true),
    ]
}

/// The ids staged in either root.
fn staged_ids(roots: &Roots) -> Vec<String> {
    let mut ids = Vec::new();
    for root in [RootKey::Core, RootKey::Desktop] {
        if let Ok(entries) = std::fs::read_dir(roots.backups(root).join("staging")) {
            ids.extend(
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned()),
            );
        }
    }
    ids
}

fn no_seal_key(store: &SecretStore, id: &str) {
    assert!(matches!(
        read_seal_key(store, &format!("staged:{id}")),
        Err(SecretError::NotSaved)
    ));
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test]
async fn a_good_backup_stages_and_previews() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let manifest = manifest_in(&backup);
    let host = host(&fixture);
    let before = live(&fixture.roots);

    let (id, state) = open_with(&host, &backup, PASSWORD).await;
    let preview = preview(state);
    assert_eq!(preview.id, id);
    assert_eq!(preview.counts, manifest.counts);
    assert_eq!(preview.made_at_ms, manifest.made_at_ms);
    assert_eq!(preview.made_on, BackupApp::Desktop);
    assert_eq!(preview.source_home, manifest.home);
    assert_eq!(
        preview.bytes,
        manifest.files.iter().map(|file| file.bytes).sum::<u64>()
    );
    assert!(preview.has_keychain && preview.has_audio);
    assert!(!preview.has_browser_profiles && !preview.has_web_part);
    assert_eq!(preview.skipped_links, manifest.skipped_links);
    assert_eq!(preview.skipped_names, manifest.skipped_names);
    assert_eq!(preview.changed_files, manifest.changed_files);

    // Every data file is staged with its listed bytes, in its own root.
    let core = staging_dir(&fixture.roots, RootKey::Core, &id);
    let desktop = staging_dir(&fixture.roots, RootKey::Desktop, &id);
    let mut data = 0;
    for file in &manifest.files {
        let (folder, live_root, rel) = if let Some(rel) = file.path.strip_prefix("core/") {
            (&core, &fixture.roots.core, rel)
        } else if let Some(rel) = file.path.strip_prefix("desktop/") {
            (&desktop, &fixture.roots.desktop, rel)
        } else {
            continue;
        };
        data += 1;
        let staged = std::fs::read(folder.join(rel)).unwrap();
        assert_eq!(staged.len(), usize::try_from(file.bytes).unwrap(), "{rel}");
        assert_eq!(sha(&staged), file.sha256, "{rel}");
        // Databases are snapshots; every other file is the original.
        if !rel.ends_with(".sqlite3") {
            assert_eq!(staged, std::fs::read(live_root.join(rel)).unwrap(), "{rel}");
        }
    }
    assert!(data >= 10, "{data}");
    assert!(core.join(LONG_FOLDER).join("note.txt").is_file());
    assert_eq!(format!("core/{}", hundred_bytes()).len(), 100);
    assert_eq!(
        std::fs::read(core.join(hundred_bytes())).unwrap(),
        b"one hundred"
    );
    assert_eq!(std::fs::read(core.join("server/empty.json")).unwrap(), b"");
    assert!(desktop.join("desktop-state.json").is_file());
    assert!(!core.join("desktop-state.json").exists());

    // Files 0600, a runnable file 0700, folders 0700.
    assert_eq!(mode(&core.join("attachments/run.sh")), 0o700);
    for folder in [&core, &desktop] {
        assert_eq!(mode(folder), 0o700);
        for (rel, _) in files_under(folder, false) {
            let path = folder.join(&rel);
            let expected = if rel.ends_with("run.sh") {
                0o700
            } else {
                0o600
            };
            assert_eq!(mode(&path), expected, "{rel:?}");
            for above in rel
                .ancestors()
                .skip(1)
                .filter(|above| !above.as_os_str().is_empty())
            {
                assert_eq!(mode(&folder.join(above)), 0o700, "{above:?}");
            }
        }
    }

    // The meta files, and nothing changed in the live roots.
    let staged_manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(core.join("meta/manifest.json")).unwrap()).unwrap();
    assert_eq!(staged_manifest, manifest);
    let staged_preview: RestorePreview =
        serde_json::from_slice(&std::fs::read(core.join("meta/preview.json")).unwrap()).unwrap();
    assert_eq!(staged_preview, preview);
    assert_eq!(live(&fixture.roots), before);
    assert!(backup.is_file(), "a chosen file stays");
    assert_eq!(host.staged(), Some(preview.clone()));
    assert_eq!(
        host.overview().await.staged.map(|staged| staged.preview),
        Some(preview)
    );
}

#[tokio::test]
async fn wrong_passwords_and_damage_change_nothing() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let bytes = std::fs::read(&backup).unwrap();
    let chunk = usize::try_from(CHUNK_BYTES).unwrap() + 16;
    assert!(
        bytes.len() > HEADER_SIZE + 2 * chunk,
        "the backup has a middle chunk"
    );
    let host = host(&fixture);
    let before = live(&fixture.roots);

    let mut flipped = bytes.clone();
    flipped[HEADER_SIZE + chunk + 100] ^= 1;
    let flipped_path = fixture.out.join("flipped.bruhsbak");
    std::fs::write(&flipped_path, &flipped).unwrap();
    let truncated_path = fixture.out.join("truncated.bruhsbak");
    std::fs::write(&truncated_path, &bytes[..bytes.len() - 100]).unwrap();
    let cut_path = fixture.out.join("cut.bruhsbak");
    std::fs::write(&cut_path, &bytes[..HEADER_SIZE + chunk]).unwrap();
    let other_path = fixture.out.join("notes.txt");
    std::fs::write(&other_path, b"just some notes, not a backup at all").unwrap();

    for (path, password, expected) in [
        (&backup, "wrong horse 12", WRONG_PASSWORD),
        (&flipped_path, PASSWORD, DAMAGED),
        (&truncated_path, PASSWORD, DAMAGED),
        (&cut_path, PASSWORD, DAMAGED),
        (&other_path, PASSWORD, NOT_A_BACKUP),
    ] {
        let (id, state) = open_with(&host, path, password).await;
        assert_eq!(failure(state), expected, "{path:?}");
        assert_eq!(staged_ids(&fixture.roots), Vec::<String>::new(), "{path:?}");
        no_seal_key(&fixture.store, &id);
        assert_eq!(live(&fixture.roots), before);
    }
    assert_eq!(host.staged(), None);

    // An upload goes once its open ends, opened or not.
    let upload = fixture.out.join("upload.bruhsbak");
    std::fs::write(&upload, &flipped).unwrap();
    let job = host
        .start_open(upload.clone(), Zeroizing::new(PASSWORD.into()), true)
        .unwrap();
    assert_eq!(failure(finished(&host, &job.id).await), DAMAGED);
    assert!(!upload.exists());
    std::fs::copy(&backup, &upload).unwrap();
    let job = host
        .start_open(upload.clone(), Zeroizing::new(PASSWORD.into()), true)
        .unwrap();
    preview(finished(&host, &job.id).await);
    assert!(!upload.exists());
}

const HEADER_SIZE: usize = bruhs_backup::HEADER_LEN;

/// How a crafted tar entry is typed.
#[derive(Clone, Copy)]
enum EntryKind {
    File,
    Folder,
    Symlink(&'static str),
    HardLink(&'static str),
    Fifo,
    /// Any other type byte, with no data.
    Typed(tar::EntryType),
}

/// A crafted entry: its name's bytes, its type and its data.
type Crafted<'a> = (&'a [u8], EntryKind, &'a [u8]);

/// One tar entry, written by hand so no path check of the tar crate's
/// stands in the way.
fn tar_entry(out: &mut Vec<u8>, name: &[u8], kind: EntryKind, data: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.as_old_mut().name[..name.len()].copy_from_slice(name);
    let (entry_type, mode) = match kind {
        EntryKind::File => (tar::EntryType::Regular, 0o600),
        EntryKind::Folder => (tar::EntryType::Directory, 0o700),
        EntryKind::Symlink(target) | EntryKind::HardLink(target) => {
            header.as_old_mut().linkname[..target.len()].copy_from_slice(target.as_bytes());
            let link = if matches!(kind, EntryKind::Symlink(_)) {
                tar::EntryType::Symlink
            } else {
                tar::EntryType::Link
            };
            (link, 0o777)
        }
        EntryKind::Fifo => (tar::EntryType::Fifo, 0o600),
        EntryKind::Typed(kind) => (kind, 0o600),
    };
    header.set_entry_type(entry_type);
    header.set_mode(mode);
    header.set_size(u64::try_from(data.len()).unwrap());
    header.set_mtime(0);
    header.set_cksum();
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(512) * 512, 0);
}

/// A tar of `entries` followed by the two end blocks.
fn tar_of(entries: &[Crafted<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, kind, data) in entries {
        tar_entry(&mut out, name, *kind, data);
    }
    out.extend_from_slice(&[0; 1024]);
    out
}

/// Encrypts `plain` as a full backup with [`PASSWORD`].
fn seal_tar(path: &Path, plain: &[u8]) {
    let mut random = [0; 23];
    random[0] = 9;
    random[22] = u8::try_from(plain.len() % 251).unwrap();
    let header = Header::new(Kind::Full, Kdf::Argon2id(KdfParams::FLOOR), random);
    let key = Key::derive(&normalize_password(PASSWORD), &header).unwrap();
    let mut out = Encryptor::new(File::create(path).unwrap(), &header, &key).unwrap();
    out.write_all(plain).unwrap();
    out.finish().unwrap();
}

/// A hand-made backup of `entries`.
fn craft(path: &Path, entries: &[Crafted<'_>]) {
    seal_tar(path, &tar_of(entries));
}

/// A manifest listing `files` with their sizes and hashes, changed by
/// `tweak`.
fn manifest_of(files: &[(&str, &[u8])], tweak: impl FnOnce(&mut BackupManifest)) -> Vec<u8> {
    let mut manifest = BackupManifest {
        format: MANIFEST_FORMAT,
        app_version: "0.1.0".into(),
        made_at_ms: EpochMs::default(),
        made_on: BackupApp::Desktop,
        reactive_protocol: CREW_PROTOCOL,
        web_db_version: None,
        os: "macos".into(),
        home: "/Users/a".into(),
        core_root: "/Users/a/core".into(),
        desktop_root: "/Users/a/desktop".into(),
        same_root: false,
        options: BackupOptions {
            keychain: false,
            audio: true,
            browser_profiles: false,
        },
        files: files
            .iter()
            .map(|(path, bytes)| ManifestFile {
                path: (*path).into(),
                bytes: u64::try_from(bytes.len()).unwrap(),
                sha256: sha(bytes),
            })
            .collect(),
        keychain_items: 0,
        skipped_links: 0,
        skipped_names: 0,
        changed_files: 0,
        counts: BackupCounts::default(),
    };
    tweak(&mut manifest);
    serde_json::to_vec(&manifest).unwrap()
}

/// Opens a hostile file: it fails with `expected`, nothing is staged, no
/// path outside staging appeared and the live roots are as they were.
async fn refused(
    fixture: &Fixture,
    host: &BackupHost,
    path: &Path,
    expected: &str,
    outside: &[PathBuf],
) {
    let before = live(&fixture.roots);
    let (id, state) = open_with(host, path, PASSWORD).await;
    assert_eq!(failure(state), expected, "{path:?}");
    assert_eq!(staged_ids(&fixture.roots), Vec::<String>::new(), "{path:?}");
    no_seal_key(&fixture.store, &id);
    for target in outside {
        assert!(
            std::fs::symlink_metadata(target).is_err(),
            "{target:?} from {path:?}"
        );
    }
    assert_eq!(live(&fixture.roots), before, "{path:?}");
}

#[tokio::test]
async fn hostile_archives_write_nothing_outside_staging() {
    use tar::EntryType as T;
    use EntryKind::{Fifo, File as F, Folder, HardLink, Symlink, Typed};
    let fixture = fixture(false);
    let host = host(&fixture);
    let core = &fixture.roots.core;
    let random = uuid::Uuid::new_v4().simple().to_string();
    let tmp_target = format!("/tmp/{random}/x");
    let pwned = format!("core/a/tmp/bruhs-pwned-{random}");
    let outside = vec![
        fixture.base.join("x"),
        fixture.roots.home.join("x"),
        core.join("x"),
        core.join("backups/x"),
        core.join("backups/staging/x"),
        PathBuf::from(&tmp_target),
        PathBuf::from(format!("/tmp/{random}")),
        PathBuf::from(format!("/tmp/bruhs-pwned-{random}")),
    ];
    let a = b"alpha".as_slice();
    let listing_a = manifest_of(&[("core/a", a)], |_| {});
    let path = fixture.out.join("crafted.bruhsbak");

    // The hand-made tar itself is sound: a clean one opens.
    craft(
        &path,
        &[
            (b"core/notes", Folder, b""),
            (b"core/a", F, a),
            (b"manifest.json", F, &listing_a),
        ],
    );
    let (id, state) = open_with(&host, &path, PASSWORD).await;
    preview(state);
    assert!(host.discard_staged(&id).is_ok());

    let ab = manifest_of(&[("core/a", a), ("core/b", a)], |_| {});
    let longer = manifest_of(&[("core/a", a)], |manifest| manifest.files[0].bytes += 1);
    let other_hash = manifest_of(&[("core/a", b"other")], |manifest| {
        manifest.files[0].bytes = 5
    });
    let big_keychain = vec![b' '; 5 << 20];
    let says_keychain = manifest_of(&[("core/a", a)], |manifest| {
        manifest.options.keychain = true
    });
    let says_web = manifest_of(&[("core/a", a)], |manifest| {
        manifest.web_db_version = Some(13)
    });
    let tmp_bytes = tmp_target.as_bytes();
    let cases: Vec<(Vec<Crafted<'_>>, &str)> = vec![
        (vec![(b"../x", F, a)], UNSAFE_ENTRY),
        (vec![(tmp_bytes, F, a)], UNSAFE_ENTRY),
        (vec![(b"core/../../x", F, a)], UNSAFE_ENTRY),
        (
            vec![(b"core/a", Symlink("/"), b""), (pwned.as_bytes(), F, a)],
            UNSAFE_ENTRY,
        ),
        (
            vec![(b"core/a", F, a), (b"core/b", HardLink("core/a"), b"")],
            UNSAFE_ENTRY,
        ),
        (vec![(b"core/f", Fifo, b"")], UNSAFE_ENTRY),
        (vec![(b"core/c", Typed(T::Char), b"")], UNSAFE_ENTRY),
        (vec![(b"core/d", Typed(T::Block), b"")], UNSAFE_ENTRY),
        (
            vec![(b"core/a", F, a), (b"core/b", Typed(T::Link), b"")],
            UNSAFE_ENTRY,
        ),
        (vec![(b"core/x", Typed(T::XHeader), b"")], UNSAFE_ENTRY),
        (
            vec![(b"core/x", Typed(T::XGlobalHeader), b"")],
            UNSAFE_ENTRY,
        ),
        (vec![(b"core/x", Typed(T::GNUSparse), b"")], UNSAFE_ENTRY),
        (vec![(b"core/x", Typed(T::GNULongLink), b"")], UNSAFE_ENTRY),
        (vec![(b"core/a", F, a), (b"core/a", F, a)], UNSAFE_ENTRY),
        (
            vec![(b"core/n", Folder, b""), (b"core/n", Folder, b"")],
            UNSAFE_ENTRY,
        ),
        (vec![(b"core/a", F, a), (b"core/a/b", F, a)], UNSAFE_ENTRY),
        (
            vec![
                (b"core/a", Folder, b""),
                (b"core/a/b", F, a),
                (b"core/a", F, a),
            ],
            UNSAFE_ENTRY,
        ),
        (
            vec![(b"core/a", Folder, b""), (b"core/a", F, a)],
            UNSAFE_ENTRY,
        ),
        // A listed file missing, an unlisted one, a size and a hash off.
        (vec![(b"core/a", F, a), (b"manifest.json", F, &ab)], DAMAGED),
        (
            vec![
                (b"core/a", F, a),
                (b"core/b", F, a),
                (b"manifest.json", F, &listing_a),
            ],
            DAMAGED,
        ),
        (
            vec![(b"core/a", F, a), (b"manifest.json", F, &longer)],
            DAMAGED,
        ),
        (
            vec![(b"core/a", F, a), (b"manifest.json", F, &other_hash)],
            DAMAGED,
        ),
        (vec![(b"keychain.json", F, &big_keychain)], DAMAGED),
        // The keychain and web parts are there exactly when it says so.
        (
            vec![(b"core/a", F, a), (b"manifest.json", F, &says_keychain)],
            DAMAGED,
        ),
        (
            vec![(b"core/a", F, a), (b"manifest.json", F, &says_web)],
            DAMAGED,
        ),
        // Names one volume would merge, and names a restore refuses.
        (
            vec![(b"core/Notes.txt", F, a), (b"core/notes.txt", F, a)],
            NAMES_CLASH,
        ),
        (
            vec![
                ("core/caf\u{e9}".as_bytes(), F, a),
                ("core/cafe\u{301}".as_bytes(), F, a),
            ],
            NAMES_CLASH,
        ),
        (
            vec![
                (b"core/x", Folder, b""),
                (b"core/x/A", Folder, b""),
                (b"core/x/A/1", F, a),
                (b"core/x/a", Folder, b""),
                (b"core/x/a/2", F, a),
            ],
            NAMES_CLASH,
        ),
        (vec![(b"core/\xff", F, a)], UNSAFE_ENTRY),
        (
            vec![(b"core/credentials/brokers/b.json", F, a)],
            UNSAFE_ENTRY,
        ),
        (
            vec![(b"core/server/bruhs/b1/browser-cache/x", F, a)],
            UNSAFE_ENTRY,
        ),
        (vec![(b"core/server/x.lock", F, a)], UNSAFE_ENTRY),
        (vec![(b"core/backups/x", F, a)], UNSAFE_ENTRY),
        (vec![(b"core/meta/x", F, a)], UNSAFE_ENTRY),
        (vec![(b"core/Meta", Folder, b"")], UNSAFE_ENTRY),
        (vec![(b"manifest.json", Folder, b"")], UNSAFE_ENTRY),
        // The manifest is last and there is one.
        (
            vec![(b"manifest.json", F, &listing_a), (b"core/a", F, a)],
            DAMAGED,
        ),
        (vec![(b"core/a", F, a)], DAMAGED),
    ];
    for (entries, expected) in &cases {
        craft(&path, entries);
        refused(&fixture, &host, &path, expected, &outside).await;
    }

    // A long name over 4 KiB, two long names for one entry, and a long
    // name after the manifest with no entry for it (which would otherwise
    // open: the manifest is sound).
    let mut over = Vec::new();
    tar_entry(&mut over, b"core/a", F, a);
    tar_long_name(&mut over, &vec![b'a'; 5000]);
    tar_entry(&mut over, b"core/z", F, a);
    tar_entry(&mut over, b"manifest.json", F, &listing_a);
    let mut doubled = Vec::new();
    tar_entry(&mut doubled, b"core/a", F, a);
    tar_long_name(&mut doubled, b"core/b");
    tar_long_name(&mut doubled, b"core/c");
    tar_entry(&mut doubled, b"core/z", F, a);
    tar_entry(&mut doubled, b"manifest.json", F, &listing_a);
    let mut dangling = Vec::new();
    tar_entry(&mut dangling, b"core/a", F, a);
    tar_entry(&mut dangling, b"manifest.json", F, &listing_a);
    tar_long_name(&mut dangling, b"core/b");
    for (mut tar, expected) in [
        (over, UNSAFE_ENTRY),
        (doubled, DAMAGED),
        (dangling, DAMAGED),
    ] {
        tar.extend_from_slice(&[0; 1024]);
        seal_tar(&path, &tar);
        refused(&fixture, &host, &path, expected, &outside).await;
    }

    // The tar's end: missing, not zero, or longer than 10 KiB.
    let body = {
        let mut body = tar_of(&[(b"core/a", F, a), (b"manifest.json", F, &listing_a)]);
        body.truncate(body.len() - 1024);
        body
    };
    for tail in [vec![], vec![0; 512 + 3], vec![0; 12 * 1024], {
        let mut noisy = vec![0; 1024];
        noisy[700] = 1;
        noisy
    }] {
        let mut plain = body.clone();
        plain.extend_from_slice(&tail);
        seal_tar(&path, &plain);
        refused(&fixture, &host, &path, DAMAGED, &outside).await;
    }

    // A header that asks for 2 GiB of memory, refused before any work.
    let header = Header {
        kind: Kind::Full,
        kdf: Kdf::Argon2id(KdfParams {
            memory_kib: 2 << 20,
            passes: 3,
            lanes: 1,
        }),
        salt: [1; 16],
        nonce_prefix: [2; 7],
        chunk_bytes: CHUNK_BYTES,
    };
    let mut file = header.to_bytes().to_vec();
    file.extend_from_slice(&[0; 4096]);
    std::fs::write(&path, &file).unwrap();
    refused(&fixture, &host, &path, DAMAGED, &outside).await;
}

/// A GNU long name entry for the next entry.
fn tar_long_name(out: &mut Vec<u8>, name: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.as_old_mut().name[..13].copy_from_slice(b"././@LongLink");
    header.set_entry_type(tar::EntryType::GNULongName);
    header.set_mode(0o600);
    header.set_size(u64::try_from(name.len() + 1).unwrap());
    header.set_mtime(0);
    header.set_cksum();
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(name);
    out.push(0);
    out.resize(out.len().div_ceil(512) * 512, 0);
}

#[tokio::test]
async fn staging_holds_no_keychain_value() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let host = host(&fixture);
    let (id, state) = open_with(&host, &backup, PASSWORD).await;
    assert_eq!(preview(state).counts.keychain_items, 1);

    let canary = fixture.canary.as_bytes();
    for root in [&fixture.roots.core, &fixture.roots.desktop] {
        for (rel, bytes) in files_under(root, false) {
            assert!(
                !bytes.windows(canary.len()).any(|window| window == canary),
                "{rel:?} holds the canary"
            );
        }
    }
    let sealed =
        std::fs::read(staging_dir(&fixture.roots, RootKey::Core, &id).join("meta/keychain.sealed"))
            .unwrap();
    let key = read_seal_key(&fixture.store, &format!("staged:{id}")).unwrap();
    let items = unseal_items(&sealed, &key).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].service, SECRETS_SERVICE);
    assert_eq!(items[0].account, format!("vault:global:{GLOBAL_VALUE}"));
    assert_eq!(items[0].value.as_str(), fixture.canary);
}

#[tokio::test]
async fn newer_backups_and_overlapping_roots_are_refused() {
    let fixture = fixture(false);
    let host = host(&fixture);
    let a = b"alpha".as_slice();
    let path = fixture.out.join("newer.bruhsbak");
    let outside: [PathBuf; 0] = [];
    for tweak in [
        (|manifest: &mut BackupManifest| manifest.reactive_protocol = CREW_PROTOCOL + 1)
            as fn(&mut BackupManifest),
        |manifest| manifest.format = MANIFEST_FORMAT + 1,
    ] {
        let manifest = manifest_of(&[("core/a", a)], tweak);
        craft(
            &path,
            &[
                (b"core/a", EntryKind::File, a),
                (b"manifest.json", EntryKind::File, &manifest),
            ],
        );
        refused(&fixture, &host, &path, NEWER_BACKUP, &outside).await;
    }

    // Manifest roots must be plain absolute paths, and agree on one root.
    for tweak in [
        (|manifest: &mut BackupManifest| manifest.core_root = "Users/a/core".into())
            as fn(&mut BackupManifest),
        |manifest| manifest.home = "/Users/a/../b".into(),
        |manifest| manifest.same_root = true,
        // Final review A minor: the text a preview shows is bounded.
        |manifest| manifest.app_version = "1".repeat(65),
        |manifest| manifest.app_version = "0.1\u{202E}1".into(),
        |manifest| manifest.home = format!("/{}", "h".repeat(4096)),
    ] {
        let manifest = manifest_of(&[("core/a", a)], tweak);
        craft(
            &path,
            &[
                (b"core/a", EntryKind::File, a),
                (b"manifest.json", EntryKind::File, &manifest),
            ],
        );
        refused(&fixture, &host, &path, DAMAGED, &outside).await;
    }

    // Two roots that both hold `server` can't merge into one.
    write(&fixture.roots.desktop.join("server/notes.json"), b"{}");
    let backup = make_backup(&fixture, "two-roots").await;
    let one = other_roots(&fixture, "one-root", true);
    let on_one = host_on(one.clone(), &fixture);
    let (id, state) = open_with(&on_one, &backup, PASSWORD).await;
    assert_eq!(failure(state), ROOTS_OVERLAP);
    assert_eq!(staged_ids(&one), Vec::<String>::new());
    no_seal_key(&fixture.store, &id);
}

#[tokio::test]
async fn one_root_and_two_root_backups_land_in_the_right_roots() {
    // One root to two: the desktop's entries go to the desktop root (D11).
    let fixture = fixture(true);
    let backup = make_backup(&fixture, "one-root").await;
    let two = other_roots(&fixture, "two-roots", false);
    let (id, state) = open_with(&host_on(two.clone(), &fixture), &backup, PASSWORD).await;
    preview(state);
    let core = staging_dir(&two, RootKey::Core, &id);
    let desktop = staging_dir(&two, RootKey::Desktop, &id);
    assert_eq!(
        std::fs::read(desktop.join("desktop-state.json")).unwrap(),
        b"{\"open\":[]}"
    );
    assert!(!core.join("desktop-state.json").exists());
    assert!(core.join("server/notes.json").is_file());

    // Two roots to one: the desktop's part merges into the one root.
    let two_roots = self::fixture(false);
    let backup = make_backup(&two_roots, "two-roots").await;
    let one = other_roots(&two_roots, "one-root", true);
    let (id, state) = open_with(&host_on(one.clone(), &two_roots), &backup, PASSWORD).await;
    preview(state);
    let core = staging_dir(&one, RootKey::Core, &id);
    assert!(core.join("desktop-state.json").is_file());
    assert!(core.join("server/notes.json").is_file());
}

#[tokio::test]
async fn a_full_disk_stops_before_writing() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let size = std::fs::metadata(&backup).unwrap().len();
    let host = host(&fixture).with_free_space(Arc::new(|_| 1 << 30));
    let (id, state) = open_with(&host, &backup, PASSWORD).await;
    assert_eq!(failure(state), no_space(size + FREE_MARGIN - (1 << 30)));
    assert!(!fixture
        .roots
        .backups(RootKey::Core)
        .join("staging")
        .exists());
    assert!(!fixture
        .roots
        .backups(RootKey::Desktop)
        .join("staging")
        .exists());
    no_seal_key(&fixture.store, &id);
}

#[tokio::test]
async fn discard_removes_everything() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let host = host(&fixture);
    let (id, state) = open_with(&host, &backup, PASSWORD).await;
    preview(state);
    let core = staging_dir(&fixture.roots, RootKey::Core, &id);
    let desktop = staging_dir(&fixture.roots, RootKey::Desktop, &id);
    assert!(core.is_dir() && desktop.is_dir());
    assert!(read_seal_key(&fixture.store, &format!("staged:{id}")).is_ok());

    host.discard_staged(&id).unwrap();
    assert!(!core.exists() && !desktop.exists());
    no_seal_key(&fixture.store, &id);
    assert_eq!(host.staged(), None);
    assert_eq!(
        host.discard_staged(&id).unwrap_err().message(),
        GONE,
        "only once"
    );
    assert_eq!(host.discard_staged("../x").unwrap_err().message(), GONE);
}

/// Final review A minor: what an open leaves to remove (its own staging
/// when it fails, the restore staged before it when it works) is removed
/// off the async workers: here, off the test's own runtime thread.
#[tokio::test]
async fn staging_is_removed_off_the_async_workers() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let host = host(&fixture);
    let (first, state) = open_with(&host, &backup, PASSWORD).await;
    preview(state);
    let wrong = open_with(&host, &backup, "not the password at all").await;
    assert_eq!(failure(wrong.1), WRONG_PASSWORD);
    let (second, state) = open_with(&host, &backup, PASSWORD).await;
    preview(state);
    let ids = staged_ids(&fixture.roots);
    assert!(!ids.contains(&first) && !ids.contains(&wrong.0), "{ids:?}");
    assert!(ids.contains(&second));
    no_seal_key(&fixture.store, &first);
    assert_eq!(super::open::FORGOT_HERE.with(std::cell::Cell::get), 0);
}

#[tokio::test]
async fn one_job_at_a_time_and_none_while_a_restore_waits() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let host = host(&fixture);
    let first = host
        .start_open(backup.clone(), Zeroizing::new(PASSWORD.into()), false)
        .unwrap();
    let again = host
        .start_open(backup.clone(), Zeroizing::new(PASSWORD.into()), false)
        .unwrap_err();
    assert_eq!(again.message(), BACKUP_BUSY);
    let making = host
        .start_create(CreateRequest {
            password: Zeroizing::new(PASSWORD.into()),
            options: BackupOptions::default(),
            app: BackupApp::Desktop,
            web_part: None,
            web_db_version: None,
            output: Output::Outgoing,
        })
        .await
        .unwrap_err();
    assert_eq!(making.message(), BACKUP_BUSY);
    assert_eq!(
        host.discard_staged(&first.id).unwrap_err().message(),
        BACKUP_BUSY
    );

    // A cancelled open stages nothing.
    assert!(host.cancel(&first.id));
    assert_eq!(failure(finished(&host, &first.id).await), BACKUP_STOPPED);
    assert_eq!(staged_ids(&fixture.roots), Vec::<String>::new());

    // A second open replaces the first staged one.
    let (old, state) = open_with(&host, &backup, PASSWORD).await;
    preview(state);
    let (new, state) = open_with(&host, &backup, PASSWORD).await;
    preview(state);
    assert_eq!(staged_ids(&fixture.roots), vec![new.clone(), new.clone()]);
    no_seal_key(&fixture.store, &old);

    // While a restore waits on it, it can't be opened over or discarded.
    let pending = fixture.roots.backups(RootKey::Core).join("pending.json");
    std::fs::write(
        &pending,
        serde_json::to_vec(&PendingRestore {
            source: RestoreSource::Staged { id: new.clone() },
            created_at_ms: EpochMs::default(),
        })
        .unwrap(),
    )
    .unwrap();
    let waiting = host
        .start_open(backup, Zeroizing::new(PASSWORD.into()), false)
        .unwrap_err();
    assert_eq!(waiting.message(), RESTORE_PENDING);
    assert_eq!(
        host.discard_staged(&new).unwrap_err().message(),
        RESTORE_PENDING
    );
}

/// Final review B minor: a staged restore's key that won't go when its
/// staging is swept is listed, and a later sweep removes it.
#[tokio::test]
async fn a_staged_key_that_wont_go_is_tried_again() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let host = host(&fixture);
    let (id, state) = open_with(&host, &backup, PASSWORD).await;
    preview(state);
    let preview_file = staging_dir(&fixture.roots, RootKey::Core, &id).join("meta/preview.json");
    File::options()
        .write(true)
        .open(&preview_file)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(25 * 60 * 60))
        .unwrap();
    fixture.store.refuse_changes_after(Some(0));
    host.sweep();
    fixture.store.refuse_changes_after(None);
    assert_eq!(staged_ids(&fixture.roots), Vec::<String>::new());
    assert!(read_seal_key(&fixture.store, &format!("staged:{id}")).is_ok());
    let listed = std::fs::read_to_string(
        fixture
            .roots
            .backups(RootKey::Core)
            .join("keys-to-forget.json"),
    )
    .unwrap();
    assert!(listed.contains(&format!("staged:{id}")), "{listed}");
    host.sweep();
    no_seal_key(&fixture.store, &id);
}

#[tokio::test]
async fn the_sweep_removes_old_and_unfinished_staging() {
    let fixture = fixture(false);
    let backup = make_backup(&fixture, "good").await;
    let host = host(&fixture);
    let (id, state) = open_with(&host, &backup, PASSWORD).await;
    preview(state);

    // An unfinished staging folder in each root goes; a finished one
    // stays within 24 hours.
    let unfinished = "0123456789abcdef0123456789abcdef";
    for root in [RootKey::Core, RootKey::Desktop] {
        write(
            &staging_dir(&fixture.roots, root, unfinished).join("server/x.json"),
            b"{}",
        );
    }
    host.sweep();
    assert_eq!(staged_ids(&fixture.roots), vec![id.clone(), id.clone()]);
    assert!(host.staged().is_some());

    // Past 24 hours it goes with its key, unless a restore waits on it.
    let preview_file = staging_dir(&fixture.roots, RootKey::Core, &id).join("meta/preview.json");
    File::options()
        .write(true)
        .open(&preview_file)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(25 * 60 * 60))
        .unwrap();
    let pending = fixture.roots.backups(RootKey::Core).join("pending.json");
    std::fs::write(
        &pending,
        serde_json::to_vec(&PendingRestore {
            source: RestoreSource::Staged { id: id.clone() },
            created_at_ms: EpochMs::default(),
        })
        .unwrap(),
    )
    .unwrap();
    host.sweep();
    assert_eq!(staged_ids(&fixture.roots), vec![id.clone(), id.clone()]);
    std::fs::remove_file(&pending).unwrap();
    host.sweep();
    assert_eq!(staged_ids(&fixture.roots), Vec::<String>::new());
    no_seal_key(&fixture.store, &id);
    assert_eq!(host.staged(), None);

    // A preview dated ahead stays within 24 hours of now, and goes past
    // that.
    let (id, state) = open_with(&host, &backup, PASSWORD).await;
    preview(state);
    let preview_file = staging_dir(&fixture.roots, RootKey::Core, &id).join("meta/preview.json");
    let date = |ahead: u64| {
        File::options()
            .write(true)
            .open(&preview_file)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(ahead * 60 * 60))
            .unwrap();
    };
    date(1);
    host.sweep();
    assert_eq!(staged_ids(&fixture.roots), vec![id.clone(), id.clone()]);
    date(25);
    host.sweep();
    assert_eq!(staged_ids(&fixture.roots), Vec::<String>::new());
    no_seal_key(&fixture.store, &id);
}

#[tokio::test]
async fn a_web_part_waits_in_meta() {
    let fixture = fixture(false);
    let host = host(&fixture);
    // A kind 2 part, uploaded the way the browser does.
    let header = Header::new(Kind::WebPart, Kdf::Argon2id(KdfParams::FLOOR), [5; 23]);
    let key = Key::derive("web password!", &header).unwrap();
    let mut part = Encryptor::new(Vec::new(), &header, &key).unwrap();
    part.write_all(b"{\"version\":1}").unwrap();
    let part = part.finish().unwrap();
    let mut upload = host.begin_upload(UploadKind::WebPart).unwrap();
    upload.write(&part).unwrap();
    let id = upload.finish().unwrap();
    let path = fixture.out.join("web.bruhsbak");
    let job = host
        .start_create(CreateRequest {
            password: Zeroizing::new(PASSWORD.into()),
            options: BackupOptions::default(),
            app: BackupApp::Web,
            web_part: host.uploaded(UploadKind::WebPart, &id),
            web_db_version: Some(13),
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

    let (staged, state) = open_with(&host, &path, PASSWORD).await;
    let preview = preview(state);
    assert!(preview.has_web_part);
    assert_eq!(preview.made_on, BackupApp::Web);
    let meta = staging_dir(&fixture.roots, RootKey::Core, &staged).join("meta");
    assert_eq!(std::fs::read(meta.join("web-part.bin")).unwrap(), part);
    assert_eq!(mode(&meta.join("web-part.bin")), 0o600);
}

/// B Task 7 I1: the entry and manifest limits are the writer's. A GNU long
/// name doesn't count as an entry; a name of exactly 100 bytes needs none.
#[tokio::test]
async fn limits_count_entries_but_not_long_names() {
    let fixture = fixture(false);
    let a = b"alpha".as_slice();
    let long = format!("core/{}", "l".repeat(120));
    let hundred = format!("core/{}", "n".repeat(95));
    assert_eq!(hundred.len(), 100);
    let manifest = manifest_of(&[(long.as_str(), a), (hundred.as_str(), a)], |_| {});
    let bytes = u64::try_from(manifest.len()).unwrap();
    let mut tar = Vec::new();
    tar_long_name(&mut tar, long.as_bytes());
    tar_entry(&mut tar, &long.as_bytes()[..100], EntryKind::File, a);
    tar_entry(&mut tar, hundred.as_bytes(), EntryKind::File, a);
    tar_entry(&mut tar, b"manifest.json", EntryKind::File, &manifest);
    tar.extend_from_slice(&[0; 1024]);
    let path = fixture.out.join("limits.bruhsbak");
    seal_tar(&path, &tar);

    let limited = |entries, manifest_bytes| {
        host(&fixture).with_limits(Limits {
            entries,
            manifest_bytes,
            ..Limits::BACKUP
        })
    };
    // Three entries: two files and the manifest.
    let at_limits = limited(3, bytes);
    let (id, state) = open_with(&at_limits, &path, PASSWORD).await;
    preview(state);
    let core = staging_dir(&fixture.roots, RootKey::Core, &id);
    assert_eq!(std::fs::read(core.join(&long[5..])).unwrap(), a);
    assert_eq!(std::fs::read(core.join(&hundred[5..])).unwrap(), a);
    at_limits.discard_staged(&id).unwrap();

    let outside: [PathBuf; 0] = [];
    refused(&fixture, &limited(2, bytes), &path, UNSAFE_ENTRY, &outside).await;
    refused(&fixture, &limited(3, bytes - 1), &path, DAMAGED, &outside).await;

    // `keychain.json` at its limit opens; one byte over is refused (B Task
    // 7 N1).
    let json = format!(
        r#"{{"items":[{{"service":"{SECRETS_SERVICE}","account":"vault:global:{GLOBAL_VALUE}","value":"c2VjcmV0"}}]}}"#
    );
    let manifest = manifest_of(&[("keychain.json", json.as_bytes())], |manifest| {
        manifest.options.keychain = true;
        manifest.keychain_items = 1;
    });
    craft(
        &path,
        &[
            (b"keychain.json", EntryKind::File, json.as_bytes()),
            (b"manifest.json", EntryKind::File, &manifest),
        ],
    );
    let keychain = |keychain_bytes| {
        host(&fixture).with_limits(Limits {
            keychain_bytes,
            ..Limits::BACKUP
        })
    };
    let json_bytes = u64::try_from(json.len()).unwrap();
    let at_limit = keychain(json_bytes);
    let (id, state) = open_with(&at_limit, &path, PASSWORD).await;
    assert!(preview(state).has_keychain);
    at_limit.discard_staged(&id).unwrap();
    refused(
        &fixture,
        &keychain(json_bytes - 1),
        &path,
        DAMAGED,
        &outside,
    )
    .await;
}

/// Final review A-I1: every folder above a file comes first as an entry of
/// its own, as the writer puts them, so each folder a restore makes counts
/// toward the entry limit. A deep file without them is `DAMAGED`, even
/// with a manifest that lists it.
#[tokio::test]
async fn files_without_their_folder_entries_are_refused() {
    use EntryKind::{File as F, Folder};
    let fixture = fixture(false);
    let host = host(&fixture);
    let a = b"alpha".as_slice();
    let deep = "core/a/b/c/d";
    let listing = manifest_of(&[(deep, a)], |manifest| {
        manifest.skipped_names = 2;
        manifest.changed_files = 1;
    });
    let path = fixture.out.join("folders.bruhsbak");
    let outside: [PathBuf; 0] = [];

    craft(
        &path,
        &[
            (b"core/a", Folder, b""),
            (b"core/a/b", Folder, b""),
            (b"core/a/b/c", Folder, b""),
            (deep.as_bytes(), F, a),
            (b"manifest.json", F, &listing),
        ],
    );
    let (id, state) = open_with(&host, &path, PASSWORD).await;
    // Final review A minor: what the maker was warned about reaches the
    // preview.
    let shown = preview(state);
    assert_eq!((shown.skipped_names, shown.changed_files), (2, 1));
    host.discard_staged(&id).unwrap();

    for entries in [
        vec![(deep.as_bytes(), F, a)],
        vec![
            (b"core/a".as_slice(), Folder, b"".as_slice()),
            (deep.as_bytes(), F, a),
        ],
        vec![
            (b"core/a".as_slice(), Folder, b"".as_slice()),
            (b"core/a/b", Folder, b""),
            (deep.as_bytes(), F, a),
            (b"core/a/b/c", Folder, b""),
        ],
    ] {
        let mut entries = entries;
        entries.push((b"manifest.json", F, &listing));
        craft(&path, &entries);
        refused(&fixture, &host, &path, DAMAGED, &outside).await;
    }
}

/// Final review A-I1: the entry paths together are bounded, the writer's
/// limit: at it a backup opens, one byte under it is refused. A long name
/// counts with its full length.
#[tokio::test]
async fn entry_paths_are_bounded_together() {
    use EntryKind::{File as F, Folder};
    let fixture = fixture(false);
    let a = b"alpha".as_slice();
    let long = format!("core/n/{}", "l".repeat(120));
    let listing = manifest_of(&[(long.as_str(), a)], |_| {});
    let mut tar = Vec::new();
    tar_entry(&mut tar, b"core/n", Folder, b"");
    tar_long_name(&mut tar, long.as_bytes());
    tar_entry(&mut tar, &long.as_bytes()[..100], F, a);
    tar_entry(&mut tar, b"manifest.json", F, &listing);
    tar.extend_from_slice(&[0; 1024]);
    let path = fixture.out.join("paths.bruhsbak");
    seal_tar(&path, &tar);
    let total = u64::try_from("core/n".len() + long.len() + "manifest.json".len()).unwrap();

    let limited = |path_bytes| {
        host(&fixture).with_limits(Limits {
            path_bytes,
            ..Limits::BACKUP
        })
    };
    let at_limit = limited(total);
    let (id, state) = open_with(&at_limit, &path, PASSWORD).await;
    preview(state);
    at_limit.discard_staged(&id).unwrap();
    let outside: [PathBuf; 0] = [];
    refused(&fixture, &limited(total - 1), &path, UNSAFE_ENTRY, &outside).await;
}

/// B Task 7 ruling: `extract` checks every keychain item before anything
/// sees `Staged`: one of Bruhs' own, never a seal key, none twice.
#[tokio::test]
async fn keychain_items_are_checked_before_extract_answers() {
    let fixture = fixture(false);
    let path = fixture.out.join("items.bruhsbak");
    let item = |service: &str, account: &str| {
        format!(r#"{{"service":"{service}","account":"{account}","value":"c2VjcmV0"}}"#)
    };
    let account = format!("vault:global:{GLOBAL_VALUE}");
    let good = item(SECRETS_SERVICE, &account);
    let seal_key = item("bruhs-ai-backup", "staged:0123456789abcdef0123456789abcdef");
    for (number, (items, sound)) in [
        (vec![good.clone()], true),
        (vec![item("login", "someone")], false),
        (vec![seal_key], false),
        (vec![good.clone(), good], false),
    ]
    .into_iter()
    .enumerate()
    {
        let json = format!(r#"{{"items":[{}]}}"#, items.join(","));
        let manifest = manifest_of(&[("keychain.json", json.as_bytes())], |manifest| {
            manifest.options.keychain = true;
            manifest.keychain_items = u32::try_from(items.len()).unwrap();
        });
        craft(
            &path,
            &[
                (b"keychain.json", EntryKind::File, json.as_bytes()),
                (b"manifest.json", EntryKind::File, &manifest),
            ],
        );
        let id = format!("{number:032x}");
        let extracted = extract(
            &fixture.roots,
            &id,
            &path,
            PASSWORD,
            &CancelFlag::default(),
            &|_, _| {},
        );
        match (extracted, sound) {
            (Ok(staged), true) => {
                assert_eq!(staged.items.len(), 1);
                assert_eq!(staged.items[0].value.as_str(), "secret");
                host(&fixture).discard_staged(&id).unwrap();
            }
            (Err(error), false) => assert_eq!(error.message(), DAMAGED, "{json}"),
            (Ok(_), false) => panic!("{json} was taken"),
            (Err(error), true) => panic!("{error}"),
        }
        assert_eq!(staged_ids(&fixture.roots), Vec::<String>::new(), "{json}");
    }
}
