//! Reading export archives, folders and JSON within fixed budgets
//! (Task 3). Every hostile archive here is built in code.

use super::archive::{entry_kind, walk, EntryKind, ExportInput, Limits, WalkReport};
use super::json_stream::{for_each_record, LoneSurrogates};
use super::test_support::zip_of;
use super::ImportError;
use bruhs_models::imports::{
    ARCHIVE_REFUSED, CONVERSATION_TOO_LARGE, DAMAGED_EXPORT, ENCRYPTED_EXPORT, NOT_AN_EXPORT,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write as _};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use zip::write::SimpleFileOptions;

const MIB: usize = 1 << 20;

/// One visited entry: its kind and every record it held.
type Visited = Vec<(EntryKind, Vec<Value>)>;

fn records(bytes: &[u8], kind: EntryKind, len: Option<u64>) -> Result<Vec<Value>, ImportError> {
    records_with(bytes, kind, len, &Limits::TEST)
}

fn records_with(
    bytes: &[u8],
    kind: EntryKind,
    len: Option<u64>,
    limits: &Limits,
) -> Result<Vec<Value>, ImportError> {
    let mut out = Vec::new();
    for_each_record(bytes, kind, len, limits, &mut |value| {
        out.push(value);
        Ok(())
    })?;
    Ok(out)
}

fn refused(result: Result<impl std::fmt::Debug, ImportError>) -> &'static str {
    match result {
        Err(ImportError::Refused(copy)) => copy,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A source that hands out one byte per read.
struct OneByte<'a>(&'a [u8]);

impl Read for OneByte<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match (self.0.split_first(), buf.first_mut()) {
            (Some((first, rest)), Some(slot)) => {
                *slot = *first;
                self.0 = rest;
                Ok(1)
            }
            _ => Ok(0),
        }
    }
}

fn read_all(
    inputs: &[ExportInput],
    scratch: &Path,
    limits: &Limits,
) -> Result<Visited, ImportError> {
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    walk(
        inputs,
        scratch,
        limits,
        &cancel,
        &mut |kind, reader, len| {
            let mut values = Vec::new();
            for_each_record(reader, kind, len, limits, &mut |value| {
                values.push(value);
                Ok(())
            })?;
            seen.push((kind, values));
            Ok(())
        },
    )?;
    Ok(seen)
}

/// A temp folder with `scratch/` made, and the export written under
/// `in/`.
struct Place {
    root: tempfile::TempDir,
}

impl Place {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("scratch")).unwrap();
        std::fs::create_dir_all(root.path().join("in")).unwrap();
        Self { root }
    }

    fn scratch(&self) -> PathBuf {
        self.root.path().join("scratch")
    }

    fn file(&self, name: &str, bytes: &[u8]) -> ExportInput {
        let path = self.root.path().join("in").join(name);
        std::fs::write(&path, bytes).unwrap();
        ExportInput::File(path)
    }

    fn listing(&self) -> BTreeSet<PathBuf> {
        let mut out = BTreeSet::new();
        let mut stack = vec![self.root.path().to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.symlink_metadata().unwrap().is_dir() {
                    stack.push(path.clone());
                }
                out.insert(path.strip_prefix(self.root.path()).unwrap().to_path_buf());
            }
        }
        out
    }

    fn scratch_is_empty(&self) -> bool {
        std::fs::read_dir(self.scratch()).unwrap().next().is_none()
    }
}

fn kinds(seen: &Visited) -> Vec<EntryKind> {
    seen.iter().map(|(kind, _)| *kind).collect()
}

fn conversations(shard: Option<u32>) -> EntryKind {
    EntryKind::Conversations { shard }
}

// ── LoneSurrogates and records ─────────────────────────────────────────

fn through_filter(json: &str, one_byte: bool) -> String {
    let refusal = Rc::default();
    if one_byte {
        serde_json::from_reader(LoneSurrogates::new(OneByte(json.as_bytes()), refusal)).unwrap()
    } else {
        serde_json::from_reader(LoneSurrogates::new(json.as_bytes(), refusal)).unwrap()
    }
}

#[test]
fn lone_surrogates_become_replacement_characters() {
    let cases = [
        (r#""a\ud800b""#, "a\u{fffd}b"),
        (r#""\udc00""#, "\u{fffd}"),
        (r#""\ud83d\ude00""#, "😀"),
        (r#""\\ud800""#, r"\ud800"),
        (r#""\uD800\uD83D\uDE00x""#, "\u{fffd}😀x"),
        (r#""end\ud800""#, "end\u{fffd}"),
        (r#""\u00e9\"\ud800""#, "é\"\u{fffd}"),
    ];
    for (json, want) in cases {
        assert_eq!(through_filter(json, false), want, "{json}");
        assert_eq!(
            through_filter(json, true),
            want,
            "one byte at a time: {json}"
        );
    }
}

#[test]
fn stray_control_characters_in_strings_are_read() {
    // ChatGPT exports have carried raw control bytes inside strings.
    let json = "[{\"text\":\"tab\there\u{1}bell\u{7}\",\"n\":\"line\nbreak\"}]";
    let got = records(json.as_bytes(), conversations(None), None).unwrap();
    assert_eq!(
        got,
        vec![json!({"text": "tab\there\u{1}bell\u{7}", "n": "line\nbreak"})]
    );
    // Outside a string a control byte is still damage.
    let bad = records(b"[\x01]", conversations(None), None);
    assert_eq!(refused(bad), DAMAGED_EXPORT);
}

#[test]
fn records_stream_from_arrays_and_objects() {
    let got = records(br#"[{"a":1},{},{"b":2}]"#, conversations(None), None).unwrap();
    assert_eq!(got, vec![json!({"a":1}), json!({}), json!({"b":2})]);

    let grok = br#"{"conversations":[{"x":1}],"media_posts":[1,2]}"#;
    assert_eq!(
        records(grok, EntryKind::GrokBackend, None).unwrap(),
        vec![json!({"x":1})]
    );

    let manifest = br#"{"version":1,"logical_files":{"conversations.json":{"files":[]}}}"#;
    assert_eq!(
        records(manifest, EntryKind::Manifest, None).unwrap(),
        vec![serde_json::from_slice::<Value>(manifest).unwrap()]
    );

    // A UTF-8 byte order mark is allowed.
    let bom = b"\xEF\xBB\xBF[{\"a\":1}]";
    assert_eq!(
        records(bom, conversations(None), None).unwrap(),
        vec![json!({"a":1})]
    );

    // A bare object read whole when small, streamed when large.
    let padding = "p".repeat(1000);
    let small = json!({"conversations":[{"x":1}],"pad":padding}).to_string();
    let len = u64::try_from(small.len()).unwrap();
    assert_eq!(
        records(small.as_bytes(), EntryKind::Bare, Some(len)).unwrap(),
        vec![serde_json::from_str::<Value>(&small).unwrap()]
    );
    let limits = Limits {
        small_json: 512,
        ..Limits::TEST
    };
    assert_eq!(
        records_with(small.as_bytes(), EntryKind::Bare, Some(len), &limits).unwrap(),
        vec![json!({"x":1})]
    );

    // A top-level scalar or trailing text is damage.
    assert_eq!(
        refused(records(b"42", conversations(None), None)),
        DAMAGED_EXPORT
    );
    assert_eq!(
        refused(records(b"[] []", conversations(None), None)),
        DAMAGED_EXPORT
    );
    assert_eq!(
        refused(records(b"[{\"a\":", conversations(None), None)),
        DAMAGED_EXPORT
    );
}

#[test]
fn a_record_over_the_cap_is_refused() {
    let big = format!("[{{\"t\":\"{}\"}}]", "x".repeat(2 * MIB));
    assert_eq!(
        refused(records(big.as_bytes(), conversations(None), None)),
        CONVERSATION_TOO_LARGE
    );
    // Each record has its own budget: many records under the cap pass.
    let one = format!("{{\"t\":\"{}\"}}", "x".repeat(MIB / 2));
    let many = format!("[{}]", vec![one; 6].join(","));
    assert_eq!(
        records(many.as_bytes(), conversations(None), None)
            .unwrap()
            .len(),
        6
    );
    // A whole manifest is capped at the small-file size.
    let limits = Limits {
        small_json: 1024,
        ..Limits::TEST
    };
    let manifest = json!({"pad": "m".repeat(2048)}).to_string();
    assert_eq!(
        refused(records_with(
            manifest.as_bytes(),
            EntryKind::Manifest,
            None,
            &limits
        )),
        DAMAGED_EXPORT
    );
}

#[test]
fn huge_strings_and_keys_stay_within_budgets() {
    // A huge value under a key that is skipped is never held.
    let skipped = format!(
        "{{\"media_posts\":\"{}\",\"conversations\":[{{\"x\":1}}]}}",
        "m".repeat(3 * MIB)
    );
    assert_eq!(
        records(skipped.as_bytes(), EntryKind::GrokBackend, None).unwrap(),
        vec![json!({"x":1})]
    );
    // A key longer than a record is refused.
    let key = format!("{{\"{}\":1}}", "k".repeat(2 * MIB));
    assert_eq!(
        refused(records(key.as_bytes(), EntryKind::GrokBackend, None)),
        DAMAGED_EXPORT
    );
}

#[test]
fn deep_nesting_is_refused() {
    let deep = "[".repeat(10_000);
    assert_eq!(
        refused(records(deep.as_bytes(), conversations(None), None)),
        DAMAGED_EXPORT
    );
    // Also inside a well-formed value that is skipped, whose depth serde
    // never counts.
    let skipped = format!(
        "{{\"media_posts\":{}{},\"conversations\":[{{\"x\":1}}]}}",
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    assert_eq!(
        refused(records(skipped.as_bytes(), EntryKind::GrokBackend, None)),
        DAMAGED_EXPORT
    );
    // Ordinary depth is fine.
    let fine = format!("[{}{}]", "[".repeat(60), "]".repeat(60));
    assert_eq!(
        records(fine.as_bytes(), conversations(None), None)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_failing_visitor_stops_the_stream_with_its_own_error() {
    let mut calls = 0;
    let result = for_each_record(
        &b"[{},{},{}]"[..],
        conversations(None),
        None,
        &Limits::TEST,
        &mut |_| {
            calls += 1;
            Err(ImportError::Gone)
        },
    );
    assert!(matches!(result, Err(ImportError::Gone)), "{result:?}");
    assert_eq!(calls, 1);
}

// ── The allow-list ─────────────────────────────────────────────────────

#[test]
fn entry_kinds_follow_the_allow_list() {
    let allowed = [
        ("conversations.json", conversations(None)),
        ("export/Conversations.JSON", conversations(None)),
        ("conversations-000.json", conversations(Some(0))),
        ("a\\b\\conversations-0042.json", conversations(Some(42))),
        ("prod-grok-backend.json", EntryKind::GrokBackend),
        (
            "ttl/30d/export_data/u/prod-grok-backend.json",
            EntryKind::GrokBackend,
        ),
        ("export_manifest.json", EntryKind::Manifest),
        ("manifest-abc-123.json", EntryKind::Manifest),
        ("memories.json", EntryKind::Memory),
        ("memories/account.json", EntryKind::Memory),
        ("x/memories/account.json", EntryKind::Memory),
        ("Conversations__2026.zip", EntryKind::Zip),
    ];
    for (name, kind) in allowed {
        assert_eq!(entry_kind(name), Some(kind), "{name}");
    }
    let refused = [
        "conversations-12345.json",
        "conversations-.json",
        "conversations-1a.json",
        "conversations.json/",
        "memories/deep/x.json",
        "memories/notes.txt",
        "chat.html",
        "file-abc.dat",
        "projects/p.json",
        "",
        "/",
    ];
    for name in refused {
        assert_eq!(entry_kind(name), None, "{name}");
    }
}

#[test]
fn personal_files_are_never_opened() {
    let canary = b"[{\"canary\":\"never-read-7f3a\"}]";
    let names = [
        "users.json",
        "user.json",
        "login_history.json",
        "user_settings.json",
        "ttl/30d/export_data/u/prod-mc-billing.json",
        ".env",
        ".env.local",
        "auth.json",
        "auth-profiles.json",
        "memories/auth.json",
        "memories/users.json",
        "credentials/conversations.json",
        "secrets/conversations.json",
        "state.db",
        "store.sqlite-wal",
        "config.yaml",
        "openclaw.json",
        ".env.zip",
    ];
    for name in names {
        assert_eq!(entry_kind(name), None, "{name}");
    }
    let mut entries: Vec<(&str, &[u8])> = names.iter().map(|name| (*name, &canary[..])).collect();
    entries.push(("conversations.json", br#"[{"ok":1}]"#));
    let place = Place::new();
    let input = place.file("export.zip", &zip_of(&entries, true));
    let seen = read_all(&[input], &place.scratch(), &Limits::TEST).unwrap();
    assert_eq!(seen, vec![(conversations(None), vec![json!({"ok":1})])]);
}

#[test]
fn shards_are_read_in_numeric_order() {
    let zip = zip_of(
        &[
            ("conversations-010.json", br#"[{"n":10}]"#),
            ("export_manifest.json", br#"{"m":1}"#),
            ("conversations-002.json", br#"[{"n":2}]"#),
            ("conversations.json", br#"[{"n":0}]"#),
        ],
        false,
    );
    let place = Place::new();
    let input = place.file("export.zip", &zip);
    let seen = read_all(&[input], &place.scratch(), &Limits::TEST).unwrap();
    assert_eq!(
        kinds(&seen),
        vec![
            conversations(Some(2)),
            conversations(Some(10)),
            EntryKind::Manifest,
            conversations(None),
        ]
    );
    assert_eq!(seen[0].1, vec![json!({"n":2})]);
    assert_eq!(seen[1].1, vec![json!({"n":10})]);
}

// ── Hostile archives ───────────────────────────────────────────────────

/// Runs one hostile case in a fresh place: the result, and a check that
/// nothing was written and scratch is empty.
fn hostile(name: &str, zip: &[u8]) -> Result<Visited, ImportError> {
    hostile_with(name, zip, &Limits::TEST)
}

fn hostile_with(name: &str, zip: &[u8], limits: &Limits) -> Result<Visited, ImportError> {
    let place = Place::new();
    let input = place.file("export.zip", zip);
    let before = place.listing();
    let result = read_all(&[input], &place.scratch(), limits);
    assert_eq!(place.listing(), before, "{name}: the folder changed");
    assert!(place.scratch_is_empty(), "{name}: scratch kept a file");
    result
}

/// The offset of the end-of-central-directory record of a ZIP with no
/// comment.
fn eocd(zip: &[u8]) -> usize {
    let at = zip.len() - 22;
    assert_eq!(&zip[at..at + 4], b"PK\x05\x06");
    at
}

fn put_u16(zip: &mut [u8], at: usize, value: u16) {
    zip[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(zip: &mut [u8], at: usize, value: u32) {
    zip[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// `zip` with a Zip64 end record and locator inserted before its end
/// record, which is saturated to point at them.
fn with_zip64(mut zip: Vec<u8>, entries: u64, locator_points_past: bool) -> Vec<u8> {
    let at = eocd(&zip);
    let size = u32::from_le_bytes(zip[at + 12..at + 16].try_into().unwrap());
    let offset = u32::from_le_bytes(zip[at + 16..at + 20].try_into().unwrap());
    let mut record = Vec::new();
    record.extend_from_slice(b"PK\x06\x06");
    record.extend_from_slice(&44u64.to_le_bytes());
    record.extend_from_slice(&45u16.to_le_bytes());
    record.extend_from_slice(&45u16.to_le_bytes());
    record.extend_from_slice(&0u32.to_le_bytes());
    record.extend_from_slice(&0u32.to_le_bytes());
    record.extend_from_slice(&entries.to_le_bytes());
    record.extend_from_slice(&entries.to_le_bytes());
    record.extend_from_slice(&u64::from(size).to_le_bytes());
    record.extend_from_slice(&u64::from(offset).to_le_bytes());
    let record_at = u64::try_from(at).unwrap();
    let mut locator = Vec::new();
    locator.extend_from_slice(b"PK\x06\x07");
    locator.extend_from_slice(&0u32.to_le_bytes());
    let pointed = if locator_points_past {
        record_at + 1_000
    } else {
        record_at
    };
    locator.extend_from_slice(&pointed.to_le_bytes());
    locator.extend_from_slice(&1u32.to_le_bytes());
    let end = zip.split_off(at);
    zip.extend_from_slice(&record);
    zip.extend_from_slice(&locator);
    zip.extend_from_slice(&end);
    let at = eocd(&zip);
    put_u16(&mut zip, at + 8, u16::MAX);
    put_u16(&mut zip, at + 10, u16::MAX);
    put_u32(&mut zip, at + 16, u32::MAX);
    zip
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A stored ZIP written by hand, so names may repeat (the writer refuses
/// that) or be any bytes. `utf8` sets the language encoding flag.
fn raw_zip(entries: &[(&[u8], &[u8])], utf8: bool) -> Vec<u8> {
    let flags: u16 = if utf8 { 1 << 11 } else { 0 };
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in entries {
        let offset = u32::try_from(out.len()).unwrap();
        let size = u32::try_from(bytes.len()).unwrap();
        let name_len = u16::try_from(name.len()).unwrap();
        let crc = crc32(bytes);
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&[0; 4]); // method, time
        out.extend_from_slice(&0u16.to_le_bytes()); // date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(bytes);
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&flags.to_le_bytes());
        central.extend_from_slice(&[0; 6]); // method, time, date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let cd_offset = u32::try_from(out.len()).unwrap();
    let count = u16::try_from(entries.len()).unwrap();
    let cd_size = u32::try_from(central.len()).unwrap();
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[test]
fn hostile_archives_are_refused_without_writing_outside() {
    // A deflate bomb: 16 MiB of spaces, far past the ratio budget.
    let spaces = vec![b' '; 16 * MIB];
    let bomb = zip_of(&[("conversations.json", &spaces)], true);
    let place = Place::new();
    let input = place.file("export.zip", &bomb);
    let before = place.listing();
    let cancel = AtomicBool::new(false);
    let mut read = 0_u64;
    let result = walk(
        &[input],
        &place.scratch(),
        &Limits::TEST,
        &cancel,
        &mut |_, reader, _| {
            let mut buf = [0_u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => return Ok(()),
                    Ok(n) => read += u64::try_from(n).unwrap(),
                    Err(_) => return Err(ImportError::Refused(DAMAGED_EXPORT)),
                }
            }
        },
    );
    assert_eq!(refused(result), ARCHIVE_REFUSED);
    assert!(read <= Limits::TEST.json_entry, "read {read} bytes");
    assert!(read < u64::try_from(spaces.len()).unwrap());
    assert_eq!(place.listing(), before);

    // The same bomb through the record reader.
    assert_eq!(refused(hostile("bomb", &bomb)), ARCHIVE_REFUSED);

    // The read-wide budget counts every entry's bytes together.
    let half = vec![b' '; 2_000];
    let mut doc = b"[".to_vec();
    doc.extend_from_slice(&half);
    doc.extend_from_slice(b"]");
    let two = zip_of(
        &[
            ("conversations-001.json", &doc),
            ("conversations-002.json", &doc),
        ],
        false,
    );
    let tight = Limits {
        inflated: 3_000,
        ..Limits::TEST
    };
    assert_eq!(
        refused(hostile_with("read-wide", &two, &tight)),
        ARCHIVE_REFUSED
    );
    assert_eq!(hostile("read-wide fits", &two).unwrap().len(), 2);

    // ZIP inside ZIP: two levels inside the upload are read, three are not.
    let inner = zip_of(&[("conversations.json", br#"[{"deep":1}]"#)], true);
    let middle = zip_of(&[("inner.zip", &inner)], true);
    let two_levels = zip_of(&[("middle.zip", &middle)], true);
    let seen = hostile("two levels", &two_levels).unwrap();
    assert_eq!(seen, vec![(conversations(None), vec![json!({"deep":1})])]);
    let three_levels = zip_of(&[("outer.zip", &two_levels)], true);
    assert_eq!(
        refused(hostile("three levels", &three_levels)),
        ARCHIVE_REFUSED
    );

    // Too many entries, counted from the end record before parsing.
    let many: Vec<(String, Vec<u8>)> = (0..1_001)
        .map(|n| (format!("file-{n}.dat"), Vec::new()))
        .collect();
    let many_refs: Vec<(&str, &[u8])> = many
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    assert_eq!(
        refused(hostile("1001 entries", &zip_of(&many_refs, false))),
        ARCHIVE_REFUSED
    );

    // A forged end record claiming 10 000 entries is refused before the
    // central directory is parsed (otherwise it would be damage).
    let mut forged = zip_of(&[("conversations.json", b"[]")], false);
    let at = eocd(&forged);
    put_u16(&mut forged, at + 8, 10_000);
    put_u16(&mut forged, at + 10, 10_000);
    assert_eq!(refused(hostile("forged count", &forged)), ARCHIVE_REFUSED);

    // A central directory said to be over 64 MiB.
    let mut huge_cd = zip_of(&[("conversations.json", b"[]")], false);
    let at = eocd(&huge_cd);
    put_u32(&mut huge_cd, at + 12, 65 << 20);
    assert_eq!(
        refused(hostile("huge directory", &huge_cd)),
        ARCHIVE_REFUSED
    );

    // Zip64 lies: a record claiming 10 000 entries, and a locator that
    // points past itself.
    let base = zip_of(&[("conversations.json", br#"[{"z":64}]"#)], false);
    let honest = with_zip64(base.clone(), 1, false);
    assert_eq!(
        hostile("honest zip64", &honest).unwrap(),
        vec![(conversations(None), vec![json!({"z":64})])]
    );
    assert_eq!(
        refused(hostile(
            "zip64 count",
            &with_zip64(base.clone(), 10_000, false)
        )),
        ARCHIVE_REFUSED
    );
    assert_eq!(
        refused(hostile("zip64 locator", &with_zip64(base, 1, true))),
        DAMAGED_EXPORT
    );

    // Names that climb out or start at a root are only matched.
    let named = zip_of(
        &[
            ("../../escape.json", br#"[{"n":"escape"}]"#),
            ("/abs/conversations.json", br#"[{"n":"abs"}]"#),
            ("C:\\x\\conversations.json", br#"[{"n":"drive"}]"#),
            (
                "../../conversations.zip",
                &zip_of(&[("conversations.json", br#"[{"n":"nested"}]"#)], false),
            ),
        ],
        false,
    );
    let place = Place::new();
    let input = place.file("export.zip", &named);
    let seen = read_all(&[input], &place.scratch(), &Limits::TEST).unwrap();
    assert_eq!(
        seen,
        vec![
            (conversations(None), vec![json!({"n":"abs"})]),
            (conversations(None), vec![json!({"n":"drive"})]),
            (conversations(None), vec![json!({"n":"nested"})]),
        ]
    );
    for path in place.listing() {
        for part in path.iter() {
            let part = part.to_string_lossy();
            assert!(!part.contains("escape") && part != "abs", "{path:?}");
        }
    }
    assert!(place.scratch_is_empty());

    // A password-protected entry (ZipCrypto; aes-crypto is off).
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::unstable::write::FileOptionsExt::with_deprecated_encryption(
        SimpleFileOptions::default(),
        b"pw",
    )
    .unwrap();
    writer.start_file("conversations.json", options).unwrap();
    writer.write_all(br#"[{"secret":1}]"#).unwrap();
    let encrypted = writer.finish().unwrap().into_inner();
    assert_eq!(refused(hostile("encrypted", &encrypted)), ENCRYPTED_EXPORT);

    // A symlink entry named conversations.json is skipped.
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink(
            "conversations.json",
            "/etc/passwd",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer
        .start_file("conversations-001.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(br#"[{"real":1}]"#).unwrap();
    let linked = writer.finish().unwrap().into_inner();
    assert_eq!(
        hostile("symlink", &linked).unwrap(),
        vec![(conversations(Some(1)), vec![json!({"real":1})])]
    );

    // Two entries with one name: the central directory is ambiguous.
    let twice = raw_zip(
        &[
            (b"conversations.json", br#"[{"first":1}]"#),
            (b"conversations.json", br#"[{"second":1}]"#),
        ],
        false,
    );
    assert_eq!(refused(hostile("duplicate names", &twice)), DAMAGED_EXPORT);
    let once = raw_zip(&[(b"conversations.json", br#"[{"only":1}]"#)], false);
    assert_eq!(
        hostile("raw", &once).unwrap(),
        vec![(conversations(None), vec![json!({"only":1})])]
    );
}

#[test]
fn not_an_export_is_said_plainly() {
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    let empty = zip_of(&[], false);
    let html = zip_of(&[("chat.html", b"<html></html>")], true);
    for (name, bytes) in [("png", &png[..]), ("empty zip", &empty), ("html", &html)] {
        assert_eq!(refused(hostile(name, bytes)), NOT_AN_EXPORT, "{name}");
    }
    let full = zip_of(&[("conversations.json", br#"[{"a":1}]"#)], true);
    let truncated = &full[..full.len() / 2];
    assert_eq!(refused(hostile("truncated", truncated)), DAMAGED_EXPORT);
    // A bare JSON file is classified by its bytes, never its name.
    let place = Place::new();
    let input = place.file("anything.bin", b"\xEF\xBB\xBF \n [{\"a\":1}]");
    let seen = read_all(&[input], &place.scratch(), &Limits::TEST).unwrap();
    assert_eq!(seen, vec![(EntryKind::Bare, vec![json!({"a":1})])]);
}

#[test]
fn several_inputs_are_read_in_order() {
    let place = Place::new();
    let first = place.file(
        "one",
        &zip_of(&[("conversations.json", br#"[{"n":1}]"#)], true),
    );
    let skipped = place.file("two", b"not json");
    let third = place.file("three", br#"{"conversations":[{"n":3}]}"#);
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let report: WalkReport = walk(
        &[first, skipped, third],
        &place.scratch(),
        &Limits::TEST,
        &cancel,
        &mut |kind, reader, len| {
            for_each_record(reader, kind, len, &Limits::TEST, &mut |value| {
                seen.push((kind, value));
                Ok(())
            })
        },
    )
    .unwrap();
    assert_eq!(report.files_read, 2);
    assert_eq!(
        seen,
        vec![
            (conversations(None), json!({"n":1})),
            (EntryKind::Bare, json!({"conversations":[{"n":3}]})),
        ]
    );
}

#[test]
fn cancelling_ends_the_walk_as_gone() {
    let zip = zip_of(
        &[
            ("conversations-001.json", br#"[{"n":1}]"#),
            ("conversations-002.json", br#"[{"n":2}]"#),
        ],
        false,
    );
    let place = Place::new();
    let input = place.file("export.zip", &zip);
    let cancel = AtomicBool::new(false);
    let mut visits = 0;
    let result = walk(
        &[input],
        &place.scratch(),
        &Limits::TEST,
        &cancel,
        &mut |_, _, _| {
            visits += 1;
            cancel.store(true, Ordering::SeqCst);
            Ok(())
        },
    );
    assert!(matches!(result, Err(ImportError::Gone)), "{result:?}");
    assert_eq!(visits, 1);
}

// ── Folders ────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn a_folder_walk_never_follows_links() {
    use std::os::unix::fs::symlink;
    let place = Place::new();
    let outside = place.root.path().join("canary.json");
    std::fs::write(&outside, br#"[{"canary":"outside-91c2"}]"#).unwrap();
    let export = place.root.path().join("in").join("export");
    std::fs::create_dir_all(export.join("real")).unwrap();
    symlink(&outside, export.join("conversations.json")).unwrap();
    symlink("/", export.join("sub")).unwrap();
    std::fs::create_dir_all(export.join("memories")).unwrap();
    symlink(&outside, export.join("memories").join("linked.json")).unwrap();
    std::fs::write(
        export.join("real").join("conversations-001.json"),
        br#"[{"n":1}]"#,
    )
    .unwrap();
    // A FIFO with an allowed name is skipped, never opened (it would hang).
    let fifo = export.join("conversations-002.json");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let seen = read_all(
        &[ExportInput::Folder(export)],
        &place.scratch(),
        &Limits::TEST,
    )
    .unwrap();
    assert_eq!(seen, vec![(conversations(Some(1)), vec![json!({"n":1})])]);
}

#[test]
fn folder_walks_keep_to_depth_and_entry_budgets() {
    let place = Place::new();
    let export = place.root.path().join("in").join("export");
    let four = export.join("a/b/c/d");
    std::fs::create_dir_all(four.join("e")).unwrap();
    std::fs::write(four.join("conversations.json"), br#"[{"depth":4}]"#).unwrap();
    std::fs::write(
        four.join("e").join("conversations.json"),
        br#"[{"depth":5}]"#,
    )
    .unwrap();
    std::fs::write(export.join("users.json"), br#"[{"canary":1}]"#).unwrap();
    std::fs::write(
        export.join("nested.zip"),
        zip_of(&[("conversations-003.json", br#"[{"zip":1}]"#)], true),
    )
    .unwrap();
    let seen = read_all(
        &[ExportInput::Folder(export)],
        &place.scratch(),
        &Limits::TEST,
    )
    .unwrap();
    assert_eq!(
        seen,
        vec![
            (conversations(None), vec![json!({"depth":4})]),
            (conversations(Some(3)), vec![json!({"zip":1})]),
        ]
    );

    // Every entry counts, even ones never opened.
    let wide = place.root.path().join("in").join("wide");
    std::fs::create_dir_all(&wide).unwrap();
    for n in 0..1_001 {
        std::fs::write(wide.join(format!("f{n}.txt")), b"").unwrap();
    }
    let result = read_all(
        &[ExportInput::Folder(wide)],
        &place.scratch(),
        &Limits::TEST,
    );
    assert_eq!(refused(result), ARCHIVE_REFUSED);
}

#[test]
fn default_limits_come_from_the_shared_caps() {
    use bruhs_models::imports as caps;
    let limits = Limits::DEFAULT;
    assert_eq!(limits.upload_bytes, caps::MAX_UPLOAD_BYTES);
    assert_eq!(limits.upload_files, caps::MAX_UPLOAD_FILES);
    assert_eq!(limits.entries, caps::MAX_ARCHIVE_ENTRIES);
    assert_eq!(usize::from(limits.nesting), caps::MAX_ZIP_NESTING);
    assert_eq!(limits.ratio, caps::MAX_INFLATE_RATIO);
    assert_eq!(limits.ratio_floor, caps::RATIO_FLOOR_BYTES);
    assert_eq!(limits.json_entry, caps::MAX_JSON_ENTRY_BYTES);
    assert_eq!(limits.inflated, caps::MAX_INFLATED_BYTES);
    assert_eq!(limits.small_json, caps::MAX_SMALL_JSON_BYTES);
    assert_eq!(limits.record, caps::MAX_RECORD_BYTES);
    assert_eq!(limits.conversations, caps::MAX_CONVERSATIONS);
    assert_eq!(limits.stored, caps::MAX_STORED_BYTES);
    assert_eq!(limits.folder_depth, caps::MAX_FOLDER_DEPTH);
    let test = Limits::TEST;
    assert_eq!(test.record, 1 << 20);
    assert_eq!(test.entries, 1_000);
    assert_eq!(test.nesting, limits.nesting);
}

// ── Fix round 1 ────────────────────────────────────────────────────────

/// `[[0,0,…]]` with `n` zeros: tiny per value, large as a `Value`.
fn dense(n: usize) -> String {
    format!("[[{}]]", vec!["0"; n].join(","))
}

#[test]
fn dense_records_are_refused_by_their_value_count() {
    // 400 000 values in 800 KB: under the 1 MiB byte budget, over the
    // value budget.
    let attack = dense(400_000);
    assert!(attack.len() < 1 << 20);
    assert_eq!(
        refused(records(attack.as_bytes(), conversations(None), None)),
        CONVERSATION_TOO_LARGE
    );
    // Through a ZIP and a walk the copy is the same.
    let zip = zip_of(&[("conversations.json", attack.as_bytes())], true);
    assert_eq!(refused(hostile("dense", &zip)), CONVERSATION_TOO_LARGE);
    // Commas inside strings aren't values.
    let text = format!("[{{\"t\":\"{}\"}}]", ",".repeat(400_000));
    assert_eq!(
        records(text.as_bytes(), conversations(None), None)
            .unwrap()
            .len(),
        1
    );
    // Each record has its own count.
    let many = format!(
        "[{}]",
        vec![format!("[{}]", vec!["0"; 150_000].join(",")); 3].join(",")
    );
    assert_eq!(
        records(many.as_bytes(), conversations(None), None)
            .unwrap()
            .len(),
        3
    );
    // A file read whole has one count for the whole file.
    let manifest = format!("{{\"a\":{}}}", dense(400_000));
    assert_eq!(
        refused(records(manifest.as_bytes(), EntryKind::Manifest, None)),
        DAMAGED_EXPORT
    );
    assert_eq!(Limits::DEFAULT.values, 4 << 20);
}

#[test]
fn a_swallowed_refusal_still_ends_the_walk() {
    let bomb = zip_of(&[("conversations.json", &vec![b' '; 16 * MIB])], true);
    let place = Place::new();
    let input = place.file("export.zip", &bomb);
    let cancel = AtomicBool::new(false);
    let result = walk(
        &[input],
        &place.scratch(),
        &Limits::TEST,
        &cancel,
        &mut |_, reader, _| {
            let _ = std::io::copy(reader, &mut std::io::sink());
            Ok(())
        },
    );
    assert_eq!(refused(result), ARCHIVE_REFUSED);
}

#[test]
fn space_says_how_much_to_free() {
    let copy = ImportError::Space(2_000_000_000).to_string();
    assert_eq!(
        copy,
        bruhs_models::imports::NO_SPACE_IMPORT
            .replace("{size}", &bruhs_models::backup::format_bytes(2_000_000_000))
    );
    assert!(!copy.contains("{size}"));
}

#[test]
fn scratch_copies_are_unique_and_stale_ones_are_swept() {
    let place = Place::new();
    let scratch = place.scratch();
    let old = scratch.join("nested-0000000000000000-1.zip");
    std::fs::write(&old, b"left by a crash").unwrap();
    let hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(hours_ago)
        .unwrap();
    // A copy another read might be using right now, and an unrelated file.
    let fresh = scratch.join("nested-1111111111111111-1.zip");
    std::fs::write(&fresh, b"in use").unwrap();
    let other = scratch.join("keep.txt");
    std::fs::write(&other, b"not ours").unwrap();
    let inner = zip_of(&[("conversations.json", br#"[{"n":1}]"#)], true);
    let input = place.file("export.zip", &zip_of(&[("inner.zip", &inner)], true));
    let seen = read_all(&[input], &scratch, &Limits::TEST).unwrap();
    assert_eq!(seen, vec![(conversations(None), vec![json!({"n":1})])]);
    assert!(!old.exists());
    assert!(fresh.exists() && other.exists());
    assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn unreadable_folders_and_files_are_skipped_and_counted() {
    use std::os::unix::fs::PermissionsExt as _;
    let place = Place::new();
    let export = place.root.path().join("in").join("export");
    let locked = export.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::write(locked.join("conversations-001.json"), br#"[{"n":1}]"#).unwrap();
    std::fs::write(export.join("conversations-002.json"), br#"[{"n":2}]"#).unwrap();
    let shut = export.join("conversations-003.json");
    std::fs::write(&shut, br#"[{"n":3}]"#).unwrap();
    std::fs::write(export.join("conversations.json"), br#"[{"n":0}]"#).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o000)).unwrap();
    let blocked = std::fs::read_dir(&locked).is_err() && std::fs::File::open(&shut).is_err();
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let report = walk(
        &[ExportInput::Folder(export)],
        &place.scratch(),
        &Limits::TEST,
        &cancel,
        &mut |kind, reader, len| {
            for_each_record(reader, kind, len, &Limits::TEST, &mut |value| {
                seen.push(value);
                Ok(())
            })
        },
    );
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o600)).unwrap();
    let report = report.unwrap();
    if blocked {
        // Running as a user the modes bind (not root).
        assert_eq!(seen, vec![json!({"n":2}), json!({"n":0})]);
        assert_eq!(report.skipped, 2);
    }
}

#[test]
fn device_names_and_picked_folders_add_nothing() {
    for name in [
        "memories/CON.json",
        "memories/nul.json",
        "aux",
        "PRN.zip",
        "COM1.json",
        "lpt9.zip",
        "x/Con/conversations.json",
    ] {
        assert_eq!(entry_kind(name), None, "{name}");
    }
    for name in [
        "memories/console.json",
        "com0.zip",
        "lpt10.zip",
        "comm1.zip",
    ] {
        assert!(entry_kind(name).is_some(), "{name}");
    }
    // A folder picked as a file is passed over, not an error.
    let place = Place::new();
    let folder = place.root.path().join("in").join("a-folder");
    std::fs::create_dir_all(&folder).unwrap();
    let result = read_all(
        &[ExportInput::File(folder)],
        &place.scratch(),
        &Limits::TEST,
    );
    assert_eq!(refused(result), NOT_AN_EXPORT);
}

#[test]
fn odd_names_and_case_never_open_what_is_refused() {
    // Case variants of never-opened names.
    for name in [
        "USERS.JSON",
        "Memories/Auth.JSON",
        "Login_History.json",
        ".ENV",
        "PROD-MC-billing.json",
        "Credentials/conversations.json",
        "STATE.DB",
        "Store.SQLite",
    ] {
        assert_eq!(entry_kind(name), None, "{name}");
    }
    // Nested ZIPs named after what is never opened (M8).
    for name in [
        "credentials.zip",
        "credentials-2026.zip",
        "Secrets.zip",
        "auth.json.zip",
        ".env.zip",
        "users.json.zip",
    ] {
        assert_eq!(entry_kind(name), None, "{name}");
    }
    let canary = br#"[{"canary":"inner-5d1e"}]"#;
    let inner = zip_of(
        &[
            ("users.json", canary),
            ("Memories/AUTH.json", canary),
            ("conversations.json", br#"[{"ok":1}]"#),
        ],
        true,
    );
    let secret = zip_of(&[("conversations.json", canary)], true);
    let outer = zip_of(&[("inner.zip", &inner), ("credentials.zip", &secret)], true);
    assert_eq!(
        hostile("never-opened inside", &outer).unwrap(),
        vec![(conversations(None), vec![json!({"ok":1})])]
    );

    // Names that aren't UTF-8 are matched as decoded (CP437 without the
    // flag) and never fail the read; a bad UTF-8 name is passed over.
    let raw = raw_zip(
        &[
            (b"conversations\xff.json", canary),
            (b"\xff/users.json", canary),
            (b"\xfe\xff/conversations.json", br#"[{"cp437":1}]"#),
        ],
        false,
    );
    assert_eq!(
        hostile("cp437 names", &raw).unwrap(),
        vec![(conversations(None), vec![json!({"cp437":1})])]
    );
    let flagged = raw_zip(
        &[
            (b"conversations\xff.json", canary),
            (b"conversations-001.json", br#"[{"utf8":1}]"#),
        ],
        true,
    );
    assert_eq!(
        hostile("bad utf-8 names", &flagged).unwrap(),
        vec![(conversations(Some(1)), vec![json!({"utf8":1})])]
    );
}

#[test]
fn a_zip_that_is_not_one_is_skipped_and_counted() {
    let place = Place::new();
    let outer = zip_of(
        &[
            ("photos.zip", b"not a zip at all"),
            ("tiny.zip", b"PK"),
            ("conversations.json", br#"[{"n":1}]"#),
        ],
        true,
    );
    let input = place.file("export.zip", &outer);
    let cancel = AtomicBool::new(false);
    let mut kinds_seen = Vec::new();
    let report = walk(
        &[input],
        &place.scratch(),
        &Limits::TEST,
        &cancel,
        &mut |kind, _, _| {
            kinds_seen.push(kind);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(kinds_seen, vec![conversations(None)]);
    assert_eq!(report.skipped, 2);
    assert!(place.scratch_is_empty());

    let export = place.root.path().join("in").join("export");
    std::fs::create_dir_all(&export).unwrap();
    std::fs::write(export.join("bad.zip"), b"<html>").unwrap();
    std::fs::write(export.join("conversations.json"), br#"[{"n":2}]"#).unwrap();
    let report = walk(
        &[ExportInput::Folder(export)],
        &place.scratch(),
        &Limits::TEST,
        &cancel,
        &mut |_, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!((report.files_read, report.skipped), (1, 1));
}
