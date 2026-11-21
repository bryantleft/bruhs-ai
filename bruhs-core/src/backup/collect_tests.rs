//! The file inventory, SQLite snapshots, free space and the roots' own
//! rules, over temporary roots only.
#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use bruhs_models::backup::{BackupOptions, DATA_NAME_REFUSED};
use rusqlite::{Connection, OpenFlags};

use super::*;

/// A fresh temp folder and its resolved path (`/var` is a link on macOS,
/// and the inventory names files under resolved roots).
fn temp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    (dir, base)
}

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
    let (_dir, base) = temp();
    let roots = two_roots(&base);
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
    let (_dir, base) = temp();
    let roots = one_root(&base);
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
    let (_dir, base) = temp();
    let mut roots = one_root(&base);
    roots.config = base.join("home").join("config");
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

/// Secret provider references (`secret-sources.json`) sit in the config
/// root too: on Linux they go in as the core root's file, from the config
/// root, and the core root's own copy stays out (main merge follow-up).
#[test]
fn secret_sources_come_from_the_config_root() {
    let (_dir, base) = temp();
    let mut roots = one_root(&base);
    roots.config = base.join("home").join("config");
    write(&roots.config.join("secret-sources.json"), b"{\"a\":1}");
    write(&roots.config.join("secret-sources.lock"), b"");
    write(&roots.config.join("other.json"), b"{}");
    write(&roots.core.join("secret-sources.json"), b"stale");
    write(&roots.core.join("server/mail.sqlite3"), b"x");

    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    assert_eq!(
        listed(&taken),
        [
            (RootKey::Core, "secret-sources.json"),
            (RootKey::Core, "server/mail.sqlite3"),
        ]
    );
    assert_eq!(
        taken.files[0].path,
        roots.config.join("secret-sources.json")
    );
    assert_eq!(taken.files[0].bytes, 7);
}

/// On macOS the config root is the core root, so the file goes in from
/// there.
#[test]
fn secret_sources_go_in_from_the_core_root_on_macos() {
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    write(&roots.core.join("secret-sources.json"), b"{\"a\":1}");
    write(&roots.core.join("secret-sources.lock"), b"");

    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    assert_eq!(listed(&taken), [(RootKey::Core, "secret-sources.json")]);
    assert_eq!(taken.files[0].path, roots.core.join("secret-sources.json"));
}

#[test]
fn links_are_counted_never_followed() {
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    let outside = base.join("outside");
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

/// B Task 5 I2 ruling: a name the restore would refuse is left out and
/// counted in the owner's own files (a folder once, with its tree), and
/// refuses the backup in Bruhs' own folders. Names an option or the rules
/// leave out, and entries that aren't files or folders, never count.
#[test]
fn unrestorable_names_are_skipped_in_owner_files_and_refused_in_bruhs_own() {
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    let work = roots.core.join("server/bruhs/a1/work");
    write(&work.join("a\\b"), b"x");
    write(&work.join("bad\\dir/one.md"), b"x");
    write(&work.join("bad\\dir/two.md"), b"x");
    write(&work.join("fine.md"), b"x");
    write(
        &roots.core.join("attachments/t/zero\u{200b}width.pdf"),
        b"x",
    );
    write(
        &roots.desktop.join("knowledge-attachments/k\u{202e}fdp.exe"),
        b"x",
    );
    write(
        &roots.core.join("server/bruhs/a1/browser/Default/odd\\name"),
        b"x",
    );
    // Left out anyway, so never judged.
    write(&roots.core.join("logs/a\\b"), b"x");
    write(&roots.core.join("server/odd\\name.lock"), b"x");
    let fifo = std::process::Command::new("mkfifo")
        .arg(roots.core.join("server/pipe\\name"))
        .status()
        .unwrap();
    assert!(fifo.success());
    // Not UTF-8 (APFS refuses such names; Linux keeps them).
    let raw = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    let non_utf8 = std::fs::write(work.join(raw), b"x").is_ok();

    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    assert_eq!(
        listed(&taken),
        [(RootKey::Core, "server/bruhs/a1/work/fine.md")]
    );
    assert_eq!(taken.skipped_names, 4 + u32::from(non_utf8));
    let profiles = BackupOptions {
        browser_profiles: true,
        ..BackupOptions::default()
    };
    let with_profiles = inventory(&roots, &profiles).unwrap();
    assert_eq!(with_profiles.skipped_names, 5 + u32::from(non_utf8));

    // In Bruhs' own folders the backup is refused, with fixed copy.
    write(&roots.core.join("server/x\\y.json"), b"{}");
    let refused = inventory(&roots, &BackupOptions::default()).unwrap_err();
    assert!(matches!(refused, BackupError::Refused(DATA_NAME_REFUSED)));
    assert!(!refused.message().contains("x\\y"));
}

/// B Task 7 I2: the second of two names a case- or normalization-blind
/// volume would merge is skipped in owner content and refused in Bruhs'
/// own folders, root by root.
#[test]
fn names_one_volume_would_merge_are_skipped_or_refused() {
    use collect::{Names, Verdict};
    let mut names = Names::default();
    let mut judge = |key, rel: &str| {
        let name = rel.rsplit('/').next().unwrap();
        names.judge(key, rel, &std::ffi::CString::new(name).unwrap())
    };
    let nfd = "attachments/cafe\u{301}.txt";
    let nfc = "attachments/caf\u{e9}.txt";
    for (key, rel, verdict) in [
        (RootKey::Core, "attachments/Makefile", Verdict::Take),
        (RootKey::Core, "attachments/makefile", Verdict::Skip),
        (RootKey::Core, nfd, Verdict::Take),
        (RootKey::Core, nfc, Verdict::Skip),
        (RootKey::Core, "server/bruhs/a1/work/README", Verdict::Take),
        (RootKey::Core, "server/bruhs/a1/work/readme", Verdict::Skip),
        (RootKey::Core, "server/Makefile", Verdict::Take),
        (RootKey::Core, "server/makefile", Verdict::Refuse),
        (RootKey::Core, "server/notes", Verdict::Take),
        // Each root is its own volume.
        (RootKey::Desktop, "server/makefile", Verdict::Take),
        // A name a restore refuses outright, as before.
        (RootKey::Core, "attachments/a\\b", Verdict::Skip),
        (RootKey::Core, "server/a\\b", Verdict::Refuse),
    ] {
        assert_eq!(judge(key, rel), verdict, "{key:?} {rel}");
    }
}

/// B Task 7 N2: a top-level `meta` in either root is Bruhs' own name for
/// the staging's folder, so a backup that holds one is refused; deeper
/// down it is an ordinary name.
#[test]
fn a_top_level_meta_is_refused() {
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    write(&roots.core.join("attachments/meta/x.txt"), b"x");
    write(&roots.core.join("server/meta.json"), b"{}");
    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    assert_eq!(
        listed(&taken),
        [
            (RootKey::Core, "attachments/meta/x.txt"),
            (RootKey::Core, "server/meta.json")
        ]
    );
    for (root, name) in [(&roots.core, "meta"), (&roots.desktop, "Meta")] {
        let stray = root.join(name).join("notes.txt");
        write(&stray, b"x");
        let refused = inventory(&roots, &BackupOptions::default()).unwrap_err();
        assert!(
            matches!(refused, BackupError::Refused(DATA_NAME_REFUSED)),
            "{name}"
        );
        std::fs::remove_dir_all(stray.parent().unwrap()).unwrap();
    }
}

/// The same through the walk, where the temp volume keeps both names of a
/// pair (Linux, case-sensitive APFS). The default macOS volume merges them
/// itself, so there the pair can't exist and the case above covers it.
#[test]
fn the_walk_keeps_one_of_each_merged_pair() {
    let (_dir, base) = temp();
    let probe = base.join("probe");
    std::fs::create_dir_all(&probe).unwrap();
    std::fs::write(probe.join("Makefile"), b"a").unwrap();
    let case_sensitive = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(probe.join("makefile"))
        .is_ok();
    std::fs::write(probe.join("caf\u{e9}"), b"a").unwrap();
    let keeps_forms = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(probe.join("cafe\u{301}"))
        .is_ok();
    if !case_sensitive && !keeps_forms {
        return;
    }
    let roots = two_roots(&base);
    let attachments = roots.core.join("attachments");
    let mut pairs = Vec::new();
    if case_sensitive {
        pairs.push(("Makefile", "makefile"));
    }
    if keeps_forms {
        pairs.push(("cafe\u{301}", "caf\u{e9}"));
    }
    for (first, second) in &pairs {
        write(&attachments.join(first), b"one");
        write(&attachments.join(second), b"two");
    }
    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    let expected: Vec<String> = pairs
        .iter()
        .map(|(first, _)| format!("attachments/{first}"))
        .collect();
    let mut got: Vec<&str> = listed(&taken).into_iter().map(|(_, rel)| rel).collect();
    got.sort_unstable();
    let mut expected: Vec<&str> = expected.iter().map(String::as_str).collect();
    expected.sort_unstable();
    assert_eq!(got, expected);
    assert_eq!(taken.skipped_names, u32::try_from(pairs.len()).unwrap());

    let (first, second) = pairs[0];
    write(&roots.core.join("server").join(first), b"{}");
    write(&roots.core.join("server").join(second), b"{}");
    let refused = inventory(&roots, &BackupOptions::default()).unwrap_err();
    assert!(matches!(refused, BackupError::Refused(DATA_NAME_REFUSED)));
}

/// The length limits are measured with the root's own prefix: `desktop/`
/// is three bytes longer than `core/`.
#[test]
fn name_lengths_count_the_real_root_prefix() {
    let name = std::ffi::CString::new("f").unwrap();
    let folder = "a".repeat(200);
    let mut rel = String::from("knowledge-attachments");
    while rel.len() + 1 + folder.len() <= 4086 {
        rel = format!("{rel}/{folder}");
    }
    let pad = 4089 - rel.len() - 3;
    rel = format!("{rel}/{}/f", "b".repeat(pad));
    assert_eq!(rel.len(), 4089);
    assert!(super::collect::restorable(RootKey::Core, &rel, &name));
    assert!(!super::collect::restorable(RootKey::Desktop, &rel, &name));
}

/// B Task 5 I1 ruling: a folder swapped for a link between reading its
/// type and opening it is counted as a link and never followed, so a
/// private folder can't ride into a backup under a bruh's work folder.
#[test]
fn a_link_swapped_in_mid_walk_is_never_followed() {
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    write(&roots.core.join("credentials/vault.json"), b"{}");
    let work = roots.core.join("server/bruhs/a1/work");
    write(&work.join("x/n.md"), b"mine");
    let mut swapped = false;
    let taken = super::collect::inventory_with(&roots, &BackupOptions::default(), &mut |rel| {
        if rel == "server/bruhs/a1/work/x" {
            std::fs::rename(work.join("x"), work.join("x-old")).unwrap();
            std::os::unix::fs::symlink(roots.core.join("credentials"), work.join("x")).unwrap();
            swapped = true;
        }
    })
    .unwrap();
    assert!(swapped);
    assert_eq!(listed(&taken), [(RootKey::Core, "credentials/vault.json")]);
    assert_eq!(taken.skipped_links, 1);
}

/// A listed file is reopened through folder handles: one whose folder or
/// itself became a link since the walk fails to open.
#[test]
fn listed_files_reopen_without_following_links() {
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    write(&roots.core.join("credentials/vault.json"), b"secret");
    let work = roots.core.join("server/bruhs/a1/work");
    write(&work.join("y/f.md"), b"mine");
    write(&work.join("z.md"), b"mine too");
    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    let file = |rel: &str| {
        taken
            .files
            .iter()
            .find(|file| file.rel == rel)
            .unwrap()
            .clone()
    };
    let (in_y, z) = (
        file("server/bruhs/a1/work/y/f.md"),
        file("server/bruhs/a1/work/z.md"),
    );
    let mut text = String::new();
    std::io::Read::read_to_string(&mut open_source(&in_y).unwrap(), &mut text).unwrap();
    assert_eq!(text, "mine");

    std::fs::rename(work.join("y"), work.join("y-old")).unwrap();
    std::fs::create_dir_all(roots.core.join("credentials/y")).unwrap();
    std::fs::write(roots.core.join("credentials/y/f.md"), b"secret").unwrap();
    std::os::unix::fs::symlink(roots.core.join("credentials/y"), work.join("y")).unwrap();
    assert!(open_source(&in_y).is_err());
    std::fs::remove_file(work.join("z.md")).unwrap();
    std::os::unix::fs::symlink(roots.core.join("credentials/vault.json"), work.join("z.md"))
        .unwrap();
    assert!(open_source(&z).is_err());
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
    let (_dir, base) = temp();
    let roots = two_roots(&base);
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
    // An empty file is not a database, whatever its name.
    write(&roots.core.join("server/empty.sqlite3"), b"");
    let taken = inventory(&roots, &options).unwrap();
    assert!(taken
        .files
        .iter()
        .any(|file| file.rel == "server/empty.sqlite3" && !file.sqlite));
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
    let into = base.join("tmp").join("reactive.sqlite3");
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
    let link = base.join("link.sqlite3");
    std::os::unix::fs::symlink(&live, &link).unwrap();
    assert!(snapshot_sqlite(&link, &base.join("tmp").join("b.sqlite3")).is_err());
    assert!(!base.join("tmp").join("b.sqlite3").exists());
}

/// With no writer left open, a WAL database still snapshots, and the live
/// file stays as it was.
#[test]
fn a_closed_wal_database_snapshots_unchanged() {
    let (_dir, base) = temp();
    let live = base.join("server").join("mail.sqlite3");
    drop(wal_database(&live));
    let before = digest(&live);
    let into = base.join("mail.sqlite3");
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
    let (_dir, base) = temp();
    assert!(free_bytes(&base).unwrap() > 0);
    assert!(free_bytes(&base.join("missing")).is_err());
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
            "BRUHS_GENERATORS_DB" => Some("/elsewhere/text-generators.sqlite3".into()),
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
            "BRUHS_GENERATORS_DB",
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

/// B Task 5 N1 ruling: past `MAX_ENTRY_DEPTH` levels an owner tree is
/// left out and counted once, Bruhs' own folders refuse the backup, and a
/// measuring walk stops quietly.
#[test]
fn trees_deeper_than_the_cap_are_skipped_refused_or_left_unmeasured() {
    use bruhs_models::backup::MAX_ENTRY_DEPTH;
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    // `server/bruhs/a1/work` is 4 levels; folders reach level 70.
    let deep = |from: &Path, levels: usize| {
        let mut path = from.to_path_buf();
        for _ in 0..levels {
            path.push("d");
        }
        path
    };
    let work = roots.core.join("server/bruhs/a1/work");
    let bottom = deep(&work, 66);
    write(&bottom.join("f"), b"x");
    // A file at the cap still goes in.
    let at_cap = deep(&work, MAX_ENTRY_DEPTH - 5).join("kept.md");
    write(&at_cap, b"x");
    let profile = roots.core.join("server/bruhs/a1/browser");
    write(&deep(&profile, 70).join("f"), b"profile");
    write(&profile.join("Default/Cookies"), b"cookies");

    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    let kept = format!(
        "server/bruhs/a1/work/{}/kept.md",
        vec!["d"; MAX_ENTRY_DEPTH - 5].join("/")
    );
    assert_eq!(listed(&taken), [(RootKey::Core, kept.as_str())]);
    assert_eq!(kept.split('/').count(), MAX_ENTRY_DEPTH);
    assert_eq!(taken.skipped_names, 1);
    // Measured up to the cap only, and never counted as a name.
    assert_eq!(taken.browser_bytes, 7);

    let refused_tree = deep(&roots.core.join("server/state"), 70);
    write(&refused_tree.join("f.json"), b"{}");
    let refused = inventory(&roots, &BackupOptions::default()).unwrap_err();
    assert!(matches!(refused, BackupError::Refused(DATA_NAME_REFUSED)));
}

/// B Task 5 N3 and N4 rulings: a listed file's root is the same folder it
/// was when the walk opened it, or the file isn't opened; a database is
/// never opened with a plain descriptor.
#[test]
fn reopening_checks_the_root_and_refuses_databases() {
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    write(&roots.core.join("server/bruhs/a1/work/n.md"), b"mine");
    drop(wal_database(&roots.core.join("server/reactive.sqlite3")));
    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    let note = taken.files.iter().find(|file| !file.sqlite).unwrap();
    let database = taken.files.iter().find(|file| file.sqlite).unwrap();
    assert_eq!(RootId::of(&note.base).unwrap(), note.base_id);
    assert!(open_source(note).is_ok());
    assert_eq!(
        open_source(database).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );

    // The root is swapped for another folder at the same path.
    std::fs::rename(&roots.core, base.join("home/core-old")).unwrap();
    write(&roots.core.join("server/bruhs/a1/work/n.md"), b"other");
    assert_ne!(RootId::of(&note.base).unwrap(), note.base_id);
    assert_eq!(
        open_source(note).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    // A link in its place isn't followed.
    std::fs::remove_dir_all(&roots.core).unwrap();
    std::os::unix::fs::symlink(base.join("home/core-old"), &roots.core).unwrap();
    assert!(open_source(note).is_err());
    assert!(RootId::of(&note.base).is_err());
}

/// The child side of `detection_keeps_this_process_s_sqlite_locks`: run as
/// its own process, it tries to take the write lock on the database the
/// parent names and fails when the parent still holds it. Without the
/// variable it does nothing.
#[test]
#[ignore = "run as a child process by detection_keeps_this_process_s_sqlite_locks"]
fn sqlite_lock_probe_child() {
    let Some(path) = std::env::var_os(LOCK_PROBE_DB) else {
        return;
    };
    let other = Connection::open(PathBuf::from(path)).unwrap();
    other.busy_timeout(std::time::Duration::ZERO).unwrap();
    let taken = other.execute_batch("BEGIN IMMEDIATE; ROLLBACK;");
    assert!(
        matches!(
            taken.as_ref().map_err(rusqlite::Error::sqlite_error_code),
            Err(Some(rusqlite::ErrorCode::DatabaseBusy))
        ),
        "another process took the write lock: {taken:?}"
    );
}

const LOCK_PROBE_DB: &str = "BRUHS_TEST_LOCK_PROBE_DB";

/// Whether another process is kept from writing `db` right now.
fn locked_for_others(db: &Path) -> bool {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "backup::collect_tests::sqlite_lock_probe_child",
            "--test-threads=1",
            "-q",
        ])
        .env(LOCK_PROBE_DB, db)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    status.success()
}

/// Ruling B Task 5 C1: the inventory finds databases through SQLite, so
/// it never closes a descriptor of a live database this process holds
/// open (a POSIX close drops every lock the process holds on the file).
/// A rollback-mode writer mid-transaction keeps its lock through the
/// inventory and through a snapshot, which sees only committed rows; once
/// it commits, the probe sees the database free (N5: the probe can tell).
#[test]
fn detection_keeps_this_process_s_sqlite_locks() {
    let (_dir, base) = temp();
    let roots = two_roots(&base);
    let live = roots.core.join("server").join("mail.sqlite3");
    std::fs::create_dir_all(live.parent().unwrap()).unwrap();
    let writer = Connection::open(&live).unwrap();
    writer
        .execute_batch("CREATE TABLE t (x); BEGIN IMMEDIATE; INSERT INTO t VALUES (1);")
        .unwrap();
    assert!(locked_for_others(&live), "the probe sees the lock");

    let taken = inventory(&roots, &BackupOptions::default()).unwrap();
    let database = taken.files.iter().find(|file| file.sqlite).unwrap();
    assert!(locked_for_others(&live), "the inventory dropped the lock");

    let into = base.join("mail-snapshot.sqlite3");
    snapshot_sqlite(&database.path, &into).unwrap();
    assert!(locked_for_others(&live), "the snapshot dropped the lock");
    let snapshot = Connection::open_with_flags(&into, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let rows: i64 = snapshot
        .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "the snapshot holds committed rows only");

    writer.execute_batch("COMMIT").unwrap();
    assert!(!locked_for_others(&live), "the probe sees the lock go");
}

/// A rollback-mode database another connection commits to during the
/// stepped copy, then stops writing (B Task 6 C1): the copy starts again
/// and ends with every committed row.
#[test]
fn a_rollback_database_copies_through_writes_between_steps() {
    let (_dir, base) = temp();
    let live = base.join("mail.sqlite3");
    let setup = Connection::open(&live).unwrap();
    setup
        .execute_batch(
            "CREATE TABLE mail(body TEXT);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 4000)
             INSERT INTO mail SELECT printf('%0500d', i) FROM n;",
        )
        .unwrap();
    let mode: String = setup
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    let pages: i64 = setup
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    let single_pass = u32::try_from(pages / 64 + 1).unwrap();
    std::fs::create_dir_all(base.join("tmp")).unwrap();
    let into = base.join("tmp").join("mail.sqlite3");

    let mut steps = 0_u32;
    let mut writes = 0_u32;
    snapshot_sqlite_with(&live, &into, &mut || {
        steps += 1;
        // Commit on the first three steps, then stop writing.
        if steps <= 3 {
            setup
                .execute("INSERT INTO mail VALUES ('late')", [])
                .unwrap();
            writes += 1;
        }
    })
    .unwrap();
    assert_eq!(writes, 3);
    assert!(
        steps > single_pass,
        "each write restarts the copy: {steps} steps for {single_pass} a pass"
    );
    let copy = Connection::open_with_flags(&into, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let rows: i64 = copy
        .query_row("SELECT count(*) FROM mail", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 4003);
    let check: String = copy
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(check, "ok");
}
