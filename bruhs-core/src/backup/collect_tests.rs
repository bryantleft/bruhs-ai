//! The file inventory, SQLite snapshots, free space and the roots' own
//! rules, over temporary roots only.
#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use bruhs_models::backup::BackupOptions;
use rusqlite::{Connection, OpenFlags};

use super::*;

/// A temp home with two data roots (`core`, `desktop`) and the config root
/// equal to the core root, as on macOS.
fn two_roots(dir: &Path) -> Roots {
    let home = dir.join("home");
    let core = home.join("core");
    Roots {
        home: home.clone(),
        core: core.clone(),
        desktop: home.join("desktop"),
        config: core,
    }
}

/// A temp home whose core and desktop roots are one folder, as on Linux.
fn one_root(dir: &Path) -> Roots {
    let home = dir.join("home");
    let root = home.join("data");
    Roots {
        home,
        core: root.clone(),
        desktop: root.clone(),
        config: root,
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Every file the first test lays out, with its size, split by root.
const CORE_FILES: [(&str, usize); 15] = [
    ("server/reactive.sqlite3", 11),
    ("server/reactive.sqlite3.lock", 1),
    ("credentials/vault.json", 7),
    ("audio/m/a.wav", 300),
    ("audio/m.jsonl", 13),
    ("audio/m/centroids.json", 5),
    ("logs/x.log", 9),
    ("models/m.bin", 9),
    ("worktrees/w/f", 9),
    ("service/r/bin", 9),
    ("capture/daemon-token", 9),
    ("server/bruhs/a1/work/n.md", 17),
    ("server/bruhs/a1/browser/Default/Cookies", 40),
    ("server/bruhs/a1/browser/Default/Cache/c", 900),
    ("server/bruhs/a1/browser-cache/c", 900),
];
const DESKTOP_FILES: [(&str, usize); 3] = [
    ("desktop-state.json", 23),
    ("knowledge-attachments/k.pdf", 29),
    ("logs/desktop.log", 31),
];

fn lay_out(core: &Path, desktop: &Path) {
    for (rel, size) in CORE_FILES {
        write(&core.join(rel), &vec![b'c'; size]);
    }
    for (rel, size) in DESKTOP_FILES {
        write(&desktop.join(rel), &vec![b'd'; size]);
    }
}

fn listed(inventory: &Inventory) -> Vec<(RootKey, &str)> {
    inventory
        .files
        .iter()
        .map(|file| (file.root, file.rel.as_str()))
        .collect()
}

#[test]
fn the_inventory_takes_data_and_leaves_caches() {
    let dir = tempfile::tempdir().unwrap();
    let roots = two_roots(dir.path());
    lay_out(&roots.core, &roots.desktop);

    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    assert_eq!(
        listed(&taken),
        [
            (RootKey::Core, "audio/m/a.wav"),
            (RootKey::Core, "audio/m/centroids.json"),
            (RootKey::Core, "audio/m.jsonl"),
            (RootKey::Core, "credentials/vault.json"),
            (RootKey::Core, "server/bruhs/a1/work/n.md"),
            (RootKey::Core, "server/reactive.sqlite3"),
            (RootKey::Desktop, "desktop-state.json"),
            (RootKey::Desktop, "knowledge-attachments/k.pdf"),
        ]
    );
    for file in &taken.files {
        let root = match file.root {
            RootKey::Core => &roots.core,
            RootKey::Desktop => &roots.desktop,
        };
        assert_eq!(file.path, root.join(&file.rel));
        assert_eq!(file.bytes, std::fs::metadata(&file.path).unwrap().len());
        assert!(!file.sqlite, "{}", file.rel);
    }
    assert_eq!(taken.audio_bytes, 300);
    // The profile, without its cache folder.
    assert_eq!(taken.browser_bytes, 40);
    assert_eq!(taken.data_bytes, 13 + 5 + 7 + 17 + 11 + 23 + 29);
    assert_eq!(taken.skipped_links, 0);

    // The switches change what is listed, never what is counted.
    let options = BackupOptions {
        audio: false,
        browser_profiles: true,
        ..BackupOptions::default()
    };
    let switched = inventory_of(&roots, &options);
    assert!(switched.contains(&(
        RootKey::Core,
        "server/bruhs/a1/browser/Default/Cookies".into()
    )));
    assert!(!switched.contains(&(RootKey::Core, "audio/m/a.wav".into())));
    assert!(switched.contains(&(RootKey::Core, "audio/m.jsonl".into())));
    assert!(!switched.iter().any(|(_, rel)| rel.contains("Cache")));
    let again = inventory(&roots, &options).unwrap();
    assert_eq!(
        (again.audio_bytes, again.browser_bytes, again.data_bytes),
        (taken.audio_bytes, taken.browser_bytes, taken.data_bytes)
    );

    // One root: each file once, under the core key.
    let dir = tempfile::tempdir().unwrap();
    let roots = one_root(dir.path());
    assert!(roots.same());
    lay_out(&roots.core, &roots.desktop);
    let single = inventory(&roots, &BackupOptions::default()).unwrap();
    let rels: Vec<_> = single.files.iter().map(|file| file.rel.as_str()).collect();
    assert_eq!(
        rels,
        [
            "audio/m/a.wav",
            "audio/m/centroids.json",
            "audio/m.jsonl",
            "credentials/vault.json",
            "desktop-state.json",
            "knowledge-attachments/k.pdf",
            "server/bruhs/a1/work/n.md",
            "server/reactive.sqlite3",
        ]
    );
    assert!(single.files.iter().all(|file| file.root == RootKey::Core));
}

fn inventory_of(roots: &Roots, options: &BackupOptions) -> Vec<(RootKey, String)> {
    inventory(roots, options)
        .unwrap()
        .files
        .into_iter()
        .map(|file| (file.root, file.rel))
        .collect()
}

/// Ruling F10 (a): on Linux the calendar state sits in the config root and
/// goes in as the core root's `calendar/`; nothing else of the config root
/// does.
#[test]
fn calendar_state_comes_from_the_config_root() {
    let dir = tempfile::tempdir().unwrap();
    let mut roots = one_root(dir.path());
    roots.config = dir.path().join("home").join("config");
    write(&roots.config.join("calendar/google/cursors.json"), b"{}");
    write(&roots.config.join("other.json"), b"{}");
    write(&roots.core.join("calendar/stale.json"), b"{}");
    write(&roots.core.join("server/mail.sqlite3"), b"x");

    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    assert_eq!(
        listed(&taken),
        [
            (RootKey::Core, "calendar/google/cursors.json"),
            (RootKey::Core, "server/mail.sqlite3"),
        ]
    );
    assert_eq!(
        taken.files[0].path,
        roots.config.join("calendar/google/cursors.json")
    );
}

#[test]
fn links_are_counted_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    let roots = two_roots(dir.path());
    let outside = dir.path().join("outside");
    write(&outside.join("vault.json"), b"theirs");
    write(&roots.core.join("server/bruhs/a1/work/n.md"), b"mine");
    std::os::unix::fs::symlink("/etc", roots.core.join("server/bruhs/a1/work/out")).unwrap();
    std::os::unix::fs::symlink(&outside, roots.core.join("credentials")).unwrap();
    // A link in a folder a backup leaves out isn't counted.
    std::fs::create_dir_all(roots.core.join("logs")).unwrap();
    std::os::unix::fs::symlink("/etc", roots.core.join("logs/etc")).unwrap();
    let fifo = roots.core.join("server/bruhs/a1/work/pipe");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let socket = std::os::unix::net::UnixListener::bind(roots.core.join("s")).unwrap();

    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    assert_eq!(taken.skipped_links, 2);
    assert_eq!(
        listed(&taken),
        [(RootKey::Core, "server/bruhs/a1/work/n.md")]
    );
    assert!(taken
        .files
        .iter()
        .all(|file| file.path.starts_with(&roots.core) && !file.path.starts_with("/etc")));
    drop(socket);
}

/// A name the restore would refuse fails the inventory instead of making a
/// backup that can't be restored.
#[test]
fn names_a_restore_refuses_stop_the_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let roots = two_roots(dir.path());
    write(&roots.core.join("server/bruhs/a1/work/a\\b"), b"x");
    let error = inventory(&roots, &BackupOptions::default()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(!error.to_string().contains("a1"), "{error}");
}

/// A database in WAL mode with an open writer that committed a row.
fn wal_database(path: &Path) -> Connection {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let writer = Connection::open(path).unwrap();
    writer
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))
        .unwrap();
    writer
        .execute_batch(
            "CREATE TABLE bruhs (name TEXT NOT NULL);
             INSERT INTO bruhs (name) VALUES ('kept');",
        )
        .unwrap();
    writer
}

fn digest(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn sqlite_files_are_marked_and_snapshotted() {
    let dir = tempfile::tempdir().unwrap();
    let roots = two_roots(dir.path());
    let live = roots.core.join("server/reactive.sqlite3");
    let writer = wal_database(&live);
    // Databases in the owner's own files are copied raw (ruling F15).
    for owner in [
        "server/bruhs/a1/work/app.sqlite3",
        "attachments/t/notes.sqlite3",
        "server/bruhs/a1/browser/Default/Cookies",
    ] {
        wal_database(&roots.core.join(owner));
    }
    let options = BackupOptions {
        browser_profiles: true,
        ..BackupOptions::default()
    };
    let taken = inventory(&roots, &options).unwrap();
    let marked: Vec<_> = taken
        .files
        .iter()
        .filter(|file| file.sqlite)
        .map(|file| file.rel.as_str())
        .collect();
    assert_eq!(marked, ["server/reactive.sqlite3"]);
    assert!(taken
        .files
        .iter()
        .any(|file| file.rel == "server/bruhs/a1/work/app.sqlite3" && !file.sqlite));

    let before = digest(&live);
    let into = dir.path().join("tmp").join("reactive.sqlite3");
    std::fs::create_dir_all(into.parent().unwrap()).unwrap();
    let bytes = snapshot_sqlite(&live, &into).unwrap();
    assert_eq!(bytes, std::fs::metadata(&into).unwrap().len());
    assert_eq!(
        std::fs::metadata(&into).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let snapshot = Connection::open_with_flags(&into, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let name: String = snapshot
        .query_row("SELECT name FROM bruhs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "kept");
    drop(snapshot);

    // The live file is unchanged and its writer still writes.
    assert_eq!(digest(&live), before);
    writer
        .execute("INSERT INTO bruhs (name) VALUES ('later')", [])
        .unwrap();

    // A snapshot never replaces a file, and never follows a link.
    assert!(snapshot_sqlite(&live, &into).is_err());
    let link = dir.path().join("link.sqlite3");
    std::os::unix::fs::symlink(&live, &link).unwrap();
    assert!(snapshot_sqlite(&link, &dir.path().join("tmp").join("b.sqlite3")).is_err());
    assert!(!dir.path().join("tmp").join("b.sqlite3").exists());
}

/// With no writer left open, a WAL database still snapshots, and the live
/// file stays as it was.
#[test]
fn a_closed_wal_database_snapshots_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("server").join("mail.sqlite3");
    drop(wal_database(&live));
    let before = digest(&live);
    let into = dir.path().join("mail.sqlite3");
    snapshot_sqlite(&live, &into).unwrap();
    assert_eq!(digest(&live), before);
    let snapshot = Connection::open_with_flags(&into, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let count: i64 = snapshot
        .query_row("SELECT count(*) FROM bruhs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn free_space_is_readable() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_bytes(dir.path()).unwrap() > 0);
    assert!(free_bytes(&dir.path().join("missing")).is_err());
}

#[test]
fn roots_name_their_backups_and_their_shape() {
    let roots = two_roots(Path::new("/h"));
    assert!(!roots.same());
    assert_eq!(
        roots.backups(RootKey::Core),
        PathBuf::from("/h/home/core/backups")
    );
    assert_eq!(
        roots.backups(RootKey::Desktop),
        PathBuf::from("/h/home/desktop/backups")
    );
    let info = roots.info();
    assert_eq!(
        (
            info.home.as_str(),
            info.core.as_str(),
            info.desktop.as_str()
        ),
        ("/h/home", "/h/home/core", "/h/home/desktop")
    );
    assert!(!info.same());
    assert!(one_root(Path::new("/h")).info().same());
}

/// D23 and ruling F10 (b): every override that points outside both roots,
/// or into a folder a backup leaves out, is named; one inside is not.
#[test]
fn overrides_outside_the_roots_are_named() {
    let roots = two_roots(Path::new("/h"));
    let env = |name: &str| -> Option<OsString> {
        match name {
            "BRUHS_REACTIVE_DB" => Some("/elsewhere/reactive.sqlite3".into()),
            "BRUHS_MEETING_DB" => Some("/h/home/core/server/meetings.sqlite3".into()),
            "BRUHS_MAIL_DB" => Some("/h/home/core/logs/mail.sqlite3".into()),
            "BRUHS_DESKTOP_STATE_FILE" => Some("/h/home/desktop/desktop-state.json".into()),
            "BRUHS_ATTACHMENTS_DIR" => Some("/h/home/core/../attachments".into()),
            "BRUHS_RICH_RESPONSE_DIR" => Some("relative/rich".into()),
            "BRUHS_WORKBENCH_DIR" => Some(OsString::new()),
            "BRUHS_DELIVERY_DIR" => Some("/h/home/desktop/delivery".into()),
            "BRUHS_WORKTREE_METER" => Some("/h/home/core/worktree-meter.json".into()),
            "HOME" => Some("/elsewhere".into()),
            _ => None,
        }
    };
    assert_eq!(
        roots.outside_overrides(env),
        [
            "BRUHS_REACTIVE_DB",
            "BRUHS_MAIL_DB",
            "BRUHS_ATTACHMENTS_DIR",
            "BRUHS_RICH_RESPONSE_DIR",
        ]
    );
    assert!(roots.outside_overrides(|_| None).is_empty());
}

#[test]
fn errors_answer_fixed_copy() {
    use bruhs_models::backup::{BACKUP_BUSY, DAMAGED, KEYCHAIN_NEEDS_YOU, NOT_A_BACKUP};
    let io = BackupError::from(std::io::Error::other("/secret/path/inside"));
    assert_eq!(io.message(), "Bruhs couldn't read or write its data.");
    assert_eq!(BackupError::Refused(NOT_A_BACKUP).message(), NOT_A_BACKUP);
    assert_eq!(BackupError::Busy(BACKUP_BUSY).message(), BACKUP_BUSY);
    assert_eq!(
        BackupError::Keychain(KEYCHAIN_NEEDS_YOU).message(),
        KEYCHAIN_NEEDS_YOU
    );
    assert_eq!(BackupError::Format(DAMAGED).message(), DAMAGED);
    assert_eq!(
        BackupError::Space("Free 2 GB.".into()).message(),
        "Free 2 GB."
    );
}
