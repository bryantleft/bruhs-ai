//! Import batches (Task 7): uploads and picked paths read into private,
//! short-lived batches, one read at a time. Every root is a temp folder.

use super::archive::Limits;
use super::batch::{ImportHost, ImportUpload, MemoryTexts};
use super::test_support::{assert_scales_linearly, fixture, zip_of};
use super::text::{clip_part, shown_name};
use super::ImportError;
use bruhs_models::imports::{
    cut_short_line, ImportBatch, ImportJob, ImportJobState, ARCHIVE_REFUSED, CLAUDE_MANIFEST,
    EXPORT_TOO_LARGE, FOLDER_MISSING, GROK_BEST_EFFORT, MAX_PART_CHARS, NOT_AN_EXPORT,
    TOO_MANY_FILES,
};
use bruhs_models::logic::portable::{title_from_prompt, ImportSource};
use bruhs_models::EpochMs;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const HOUR_MS: i64 = 60 * 60 * 1000;

/// A temp folder whose `core/imports` is the host's root.
struct Place {
    dir: tempfile::TempDir,
}

impl Place {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("core").join("imports")
    }

    /// A host with the test caps, and as much free space as it asks for.
    fn host(&self) -> ImportHost {
        let host = ImportHost::new(self.root()).with_limits(Limits::TEST);
        #[cfg(unix)]
        let host = host.with_free_space(Arc::new(|_: &Path| Ok(u64::MAX)));
        host
    }

    /// The batch folders under the root.
    fn batches(&self) -> BTreeSet<String> {
        std::fs::read_dir(self.root())
            .map(|listing| {
                listing
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Writes `bytes` to `name` in a folder beside the root.
    fn picked(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.path().join("picked").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

/// Uploads `files`, one part each, and starts the read.
fn upload(host: &ImportHost, files: &[&[u8]]) -> ImportJob {
    let mut upload = host.begin_upload().expect("the upload starts");
    for bytes in files {
        upload.next_file().unwrap();
        for chunk in bytes.chunks(4096) {
            upload.write(chunk).unwrap();
        }
    }
    upload.finish().expect("the read starts")
}

/// Polls `id` until it isn't reading, for at most 60 s (a hang guard,
/// generous for a loaded debug build).
fn wait(host: &ImportHost, id: &str) -> ImportJob {
    let start = Instant::now();
    loop {
        let job = host.job(id).expect("the job is known");
        if !matches!(job.state, ImportJobState::Reading { .. }) {
            return job;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "the read hangs");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ready(job: ImportJob) -> ImportBatch {
    match job.state {
        ImportJobState::Ready { batch } => *batch,
        other => panic!("expected a ready batch, got {other:?}"),
    }
}

fn failed(job: ImportJob) -> String {
    match job.state {
        ImportJobState::Failed { message } => message,
        other => panic!("expected a failed read, got {other:?}"),
    }
}

/// Uploads `files` and waits for the batch.
fn read(host: &ImportHost, files: &[&[u8]]) -> ImportBatch {
    let job = upload(host, files);
    ready(wait(host, &job.id))
}

fn refused<T: std::fmt::Debug>(result: Result<T, ImportError>) -> &'static str {
    match result {
        Err(ImportError::Refused(copy)) => copy,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// Waits until `condition` holds, for at most 10 s.
fn eventually(what: &str, condition: impl Fn() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ids(batch: &ImportBatch, source: ImportSource) -> BTreeSet<String> {
    batch
        .candidates
        .iter()
        .filter(|candidate| candidate.source == source)
        .map(|candidate| candidate.id.clone())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(ToString::to_string).collect()
}

fn chatgpt_records() -> Vec<Value> {
    match serde_json::from_slice(fixture("chatgpt-conversations.json")).unwrap() {
        Value::Array(records) => records,
        other => panic!("the fixture is an array, not {other}"),
    }
}

fn json_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

/// Every path under `dir`, with whether it is a folder.
fn tree(dir: &Path) -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            let is_dir = path.symlink_metadata().unwrap().is_dir();
            if is_dir {
                stack.push(path.clone());
            }
            out.push((path, is_dir));
        }
    }
    out
}

#[test]
fn an_upload_reads_into_a_batch_and_threads_load() {
    let place = Place::new();
    let host = place.host();
    let export = zip_of(
        &[
            ("conversations.json", fixture("chatgpt-conversations.json")),
            (
                "claude/conversations.json",
                fixture("claude-conversations.json"),
            ),
            ("memories.json", fixture("claude-memories-account.json")),
        ],
        true,
    );
    let batch = read(&host, &[&export]);

    assert_eq!(
        ids(&batch, ImportSource::ChatGpt),
        set(&[
            "conv-herbs-c",
            "conv-kitchen-d",
            "conv-lisbon-a",
            "conv-sourdough-b"
        ])
    );
    assert_eq!(
        ids(&batch, ImportSource::ClaudeAi),
        set(&[
            "conv-garden-h",
            "conv-loop-i",
            "conv-packing-j",
            "conv-sourdough-g"
        ])
    );
    let title = |id: &str| {
        batch
            .candidates
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|candidate| candidate.title.clone())
            .unwrap()
    };
    assert_eq!(title("conv-lisbon-a"), "Lisbon on a budget");
    assert_eq!(title("conv-sourdough-g"), "Sourdough for the weekend");
    // Newest first, as a local listing is.
    let times: Vec<_> = batch.candidates.iter().map(|c| c.updated_at).collect();
    assert!(times.windows(2).all(|pair| pair[0] >= pair[1]), "{times:?}");

    assert_eq!(batch.memory.len(), 1);
    assert_eq!(batch.memory[0].source, ImportSource::ClaudeAi);
    assert!(batch.memory[0]
        .text
        .contains("The owner likes short answers and weekend hikes."));
    let counts = batch.counts;
    assert_eq!(counts.conversations, 8);
    assert_eq!(
        counts.skipped_empty, 1,
        "conv-empty-e has only a system message"
    );
    assert_eq!(counts.skipped_unknown, 1, "the empty object");
    assert_eq!(counts.duplicates, 0);
    assert_eq!(counts.dropped_branches, 3, "one each in A, C and G");
    assert_eq!(counts.files_read, 3);
    assert_eq!((counts.skipped_files, counts.cut_short), (0, 0));
    assert!(batch.warnings.is_empty() && !batch.best_effort);
    assert!(batch.expires_at_ms.as_millis() > EpochMs::now().as_millis() + 23 * HOUR_MS);

    let thread = host
        .thread(&batch.id, ImportSource::ChatGpt, "conv-lisbon-a")
        .unwrap();
    assert_eq!(thread.candidate.title, "Lisbon on a budget");
    assert!(thread.context.as_deref().unwrap().starts_with("# "));
    let claude = host
        .thread(&batch.id, ImportSource::ClaudeAi, "conv-sourdough-g")
        .unwrap();
    assert!(!claude.messages.is_empty());
    // The same id under another source is another conversation.
    assert!(matches!(
        host.thread(&batch.id, ImportSource::Grok, "conv-lisbon-a"),
        Err(ImportError::NotFound { .. })
    ));

    let dir = place.root().join(&batch.id);
    assert!(!dir.join("incoming").exists() && !dir.join("scratch").exists());
    assert!(dir.join("batch.json").is_file() && !dir.join(".batch.json.tmp").exists());
    assert_eq!(place.batches(), set(&[&batch.id]));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |path: &Path| path.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&place.root()), 0o700);
        for (path, is_dir) in tree(&place.root()) {
            let want = if is_dir { 0o700 } else { 0o600 };
            assert_eq!(mode(&path), want, "{path:?}");
        }
    }
}

#[test]
fn two_shards_and_two_parts_dedupe_by_conversation() {
    let place = Place::new();
    let host = place.host();
    let records = chatgpt_records();
    let first = zip_of(
        &[
            ("conversations-000.json", &json_bytes(&json!(records[..2]))),
            ("conversations-001.json", &json_bytes(&json!(records[2..4]))),
        ],
        true,
    );
    let second = zip_of(
        &[("conversations.json", &json_bytes(&json!([records[1]])))],
        false,
    );
    let batch = read(&host, &[&first, &second]);
    let all: Vec<&str> = batch.candidates.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(all.len(), 4, "{all:?}");
    assert_eq!(
        ids(&batch, ImportSource::ChatGpt),
        set(&[
            "conv-herbs-c",
            "conv-kitchen-d",
            "conv-lisbon-a",
            "conv-sourdough-b"
        ])
    );
    assert_eq!(batch.counts.duplicates, 1);
    assert_eq!(batch.counts.files_read, 3);
}

#[test]
fn a_repeat_keeps_the_copy_from_the_first_shard() {
    let place = Place::new();
    let host = place.host();
    let records = chatgpt_records();
    let mut renamed = records[0].clone();
    renamed["title"] = json!("From the second shard");
    // Shard 001 comes first in the archive; shard 000 is still read first.
    let export = zip_of(
        &[
            ("conversations-001.json", &json_bytes(&json!([renamed]))),
            ("conversations-000.json", &json_bytes(&json!([records[0]]))),
        ],
        true,
    );
    let batch = read(&host, &[&export]);
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(batch.candidates[0].title, "Lisbon on a budget");
    assert_eq!(batch.counts.duplicates, 1);
}

#[test]
fn grok_batches_are_best_effort() {
    let place = Place::new();
    let host = place.host();
    let export = zip_of(
        &[(
            "prod-grok-backend.json",
            fixture("grok-prod-grok-backend.json"),
        )],
        true,
    );
    let batch = read(&host, &[&export]);
    assert!(!batch.candidates.is_empty());
    assert!(batch
        .candidates
        .iter()
        .all(|candidate| candidate.source == ImportSource::Grok));
    assert!(batch.best_effort);
    assert_eq!(batch.warnings, vec![GROK_BEST_EFFORT.to_string()]);

    // Uploaded bare, the same file is read by its structure alone.
    let bare = read(&host, &[fixture("grok-prod-grok-backend.json")]);
    assert_eq!(
        ids(&bare, ImportSource::Grok),
        ids(&batch, ImportSource::Grok)
    );
    assert!(bare.best_effort);

    // Grok's file name holding ChatGPT's records: never read as ChatGPT.
    let renamed = zip_of(
        &[(
            "prod-grok-backend.json",
            fixture("chatgpt-conversations.json"),
        )],
        true,
    );
    let odd = read(&host, &[&renamed]);
    assert!(odd.candidates.is_empty() && !odd.best_effort);
    assert_eq!(odd.counts.skipped_unknown, 6);
}

#[test]
fn a_manifest_alone_explains_itself() {
    let place = Place::new();
    let host = place.host();
    let batch = read(&host, &[fixture("claude-manifest.json")]);
    assert!(batch.candidates.is_empty());
    assert_eq!(batch.warnings, vec![CLAUDE_MANIFEST.to_string()]);
    assert_eq!(batch.counts.skipped_unknown, 0);

    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    let job = upload(&host, &[&png]);
    assert_eq!(failed(wait(&host, &job.id)), NOT_AN_EXPORT);
    eventually("the failed read's folder goes", || {
        !place.root().join(&job.id).exists()
    });
}

#[test]
fn a_refused_upload_leaves_nothing_behind() {
    let place = Place::new();
    let host = place.host();
    let earlier = read(&host, &[fixture("chatgpt-conversations.json")]);
    let before = tree(&place.root().join(&earlier.id));

    // Task 3's bomb: 16 MiB of spaces, far past the ratio budget.
    let bomb = zip_of(&[("conversations.json", &vec![b' '; 16 << 20])], true);
    let job = upload(&host, &[&bomb]);
    assert_eq!(failed(wait(&host, &job.id)), ARCHIVE_REFUSED);
    eventually("the refused read's folder goes", || {
        place.batches() == set(&[&earlier.id])
    });
    assert_eq!(tree(&place.root().join(&earlier.id)), before);
    assert_eq!(ready(host.job(&earlier.id).unwrap()), earlier);
    assert!(host
        .thread(&earlier.id, ImportSource::ChatGpt, "conv-lisbon-a")
        .is_ok());
}

#[test]
fn uploads_are_capped() {
    let place = Place::new();
    let host = place.host();
    let mut upload = host.begin_upload().unwrap();
    let cap = usize::try_from(Limits::TEST.upload_bytes).unwrap();
    upload.write(&vec![b'['; cap]).unwrap();
    assert_eq!(refused(upload.write(b"]")), EXPORT_TOO_LARGE);
    assert_eq!(place.batches().len(), 1);
    drop(upload);
    assert!(
        place.batches().is_empty(),
        "a dropped upload deletes its files"
    );

    let mut upload = host.begin_upload().unwrap();
    for _ in 0..Limits::TEST.upload_files {
        upload.next_file().unwrap();
        upload.write(b"[]").unwrap();
    }
    assert_eq!(refused(upload.next_file()), TOO_MANY_FILES);
    let parts = std::fs::read_dir(place.root().join(upload.id()).join("incoming"))
        .unwrap()
        .count();
    assert_eq!(parts, Limits::TEST.upload_files);
    drop(upload);
    assert!(place.batches().is_empty());

    // Nothing uploaded is nothing to read.
    let empty: ImportUpload = host.begin_upload().unwrap();
    assert_eq!(refused(empty.finish()), NOT_AN_EXPORT);
    assert!(place.batches().is_empty());
}

#[test]
fn paths_are_read_in_place_and_never_deleted() {
    let place = Place::new();
    let host = place.host();
    let zip = zip_of(
        &[("conversations.json", fixture("chatgpt-conversations.json"))],
        true,
    );
    let zip_path = place.picked("export.zip", &zip);
    let folder = place.picked(
        "claude-export/conversations.json",
        fixture("claude-conversations.json"),
    );
    let folder = folder.parent().unwrap().to_path_buf();
    let listing = tree(&place.dir.path().join("picked"));

    let job = host
        .read_paths(vec![zip_path.clone(), folder.clone()])
        .unwrap();
    let batch = ready(wait(&host, &job.id));
    assert_eq!(batch.candidates.len(), 8);
    assert_eq!(batch.counts.files_read, 2);
    assert_eq!(std::fs::read(&zip_path).unwrap(), zip);
    assert_eq!(
        std::fs::read(folder.join("conversations.json")).unwrap(),
        fixture("claude-conversations.json")
    );
    assert_eq!(tree(&place.dir.path().join("picked")), listing);

    // A picked file whose name is never opened adds nothing (F33).
    let users = place.picked("users.json", fixture("chatgpt-conversations.json"));
    let job = host.read_paths(vec![users]).unwrap();
    assert_eq!(failed(wait(&host, &job.id)), NOT_AN_EXPORT);

    let missing = place.dir.path().join("picked").join("gone.zip");
    assert_eq!(refused(host.read_paths(vec![missing])), FOLDER_MISSING);
    let too_many = vec![zip_path; Limits::TEST.upload_files + 1];
    assert_eq!(refused(host.read_paths(too_many)), TOO_MANY_FILES);
}

#[test]
fn one_read_at_a_time_and_old_batches_go() {
    let place = Place::new();
    let clock = Arc::new(AtomicI64::new(EpochMs::now().as_millis()));
    let ticking = Arc::clone(&clock);
    let host = place.host().with_clock(Arc::new(move || {
        EpochMs::saturating_from_i64(ticking.load(Ordering::SeqCst))
    }));

    let open = host.begin_upload().unwrap();
    assert!(matches!(host.begin_upload(), Err(ImportError::Busy)));
    let zip_path = place.picked("export.zip", fixture("chatgpt-conversations.json"));
    assert!(matches!(
        host.read_paths(vec![zip_path]),
        Err(ImportError::Busy)
    ));
    // Another handle on the host shares the slot.
    let other = host.clone();
    assert!(matches!(other.begin_upload(), Err(ImportError::Busy)));
    drop(open);

    let mut batches = Vec::new();
    for _ in 0..4 {
        clock.fetch_add(1_000, Ordering::SeqCst);
        batches.push(read(&host, &[fixture("chatgpt-conversations.json")]).id);
    }
    assert_eq!(
        place.batches(),
        set(&[&batches[1], &batches[2], &batches[3]])
    );
    assert!(host.job(&batches[0]).is_none());
    assert!(host.job(&batches[3]).is_some());
}

#[test]
fn discard_cancels_and_removes() {
    let place = Place::new();
    let (release, wait_here) = mpsc::channel::<()>();
    let wait_here = Mutex::new(wait_here);
    let host = place.host().with_record_hook(Arc::new(move || {
        let _ = wait_here
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
    }));
    let job = upload(&host, &[fixture("chatgpt-conversations.json")]);
    assert!(matches!(
        host.job(&job.id).unwrap().state,
        ImportJobState::Reading { .. }
    ));
    host.discard(&job.id).unwrap();
    assert!(host.job(&job.id).is_none());
    for _ in 0..8 {
        let _ = release.send(());
    }
    eventually("the discarded read's folder goes", || {
        place.batches().is_empty()
    });
    eventually("the slot is free again", || match host.begin_upload() {
        Ok(_) => true,
        Err(ImportError::Busy) => false,
        Err(other) => panic!("{other:?}"),
    });
    assert!(matches!(host.discard(&job.id), Err(ImportError::Gone)));

    // A ready batch is discarded at once.
    drop(release);
    let batch = read(&host, &[fixture("chatgpt-conversations.json")]);
    host.discard(&batch.id).unwrap();
    assert!(place.batches().is_empty() && host.job(&batch.id).is_none());
}

#[test]
fn expired_batches_are_swept() {
    let place = Place::new();
    let start = EpochMs::now().as_millis();
    let clock = Arc::new(AtomicI64::new(start));
    let ticking = Arc::clone(&clock);
    let host = place.host().with_clock(Arc::new(move || {
        EpochMs::saturating_from_i64(ticking.load(Ordering::SeqCst))
    }));
    let batch = read(&host, &[fixture("chatgpt-conversations.json")]);
    // A ready batch's leftovers go, the batch stays.
    let dir = place.root().join(&batch.id);
    std::fs::create_dir(dir.join("incoming")).unwrap();
    host.sweep();
    assert!(dir.join("batch.json").is_file() && !dir.join("incoming").exists());

    clock.store(start + 25 * HOUR_MS, Ordering::SeqCst);
    let at = |offset_ms: i64| {
        let ms = u64::try_from(start + offset_ms).unwrap();
        std::time::UNIX_EPOCH + Duration::from_millis(ms)
    };
    let orphan = |id: &str, modified| {
        let path = place.root().join(id);
        std::fs::create_dir(&path).unwrap();
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    };
    let stale = "a".repeat(32);
    let fresh = "b".repeat(32);
    orphan(&stale, at(23 * HOUR_MS));
    orphan(&fresh, at(25 * HOUR_MS));
    host.sweep();
    assert_eq!(place.batches(), set(&[&fresh]));
    assert!(matches!(
        host.thread(&batch.id, ImportSource::ChatGpt, "conv-lisbon-a"),
        Err(ImportError::Gone)
    ));
    assert!(host.job(&batch.id).is_none());
}

#[test]
fn ids_are_checked() {
    let place = Place::new();
    let host = place.host();
    let batch = read(&host, &[fixture("chatgpt-conversations.json")]);
    assert!(matches!(
        host.thread("../x", ImportSource::ChatGpt, "a"),
        Err(ImportError::Gone)
    ));
    assert!(matches!(
        host.thread(&batch.id, ImportSource::ChatGpt, "../x"),
        Err(ImportError::NotFound { .. })
    ));
    assert!(host.job(&"A1".repeat(16)).is_none());
    assert!(host.job(&batch.id.to_ascii_uppercase()).is_none());
    assert!(host.job("../x").is_none());
    assert!(matches!(host.discard("../x"), Err(ImportError::Gone)));
    assert!(matches!(
        host.discard(&"c".repeat(32)),
        Err(ImportError::Gone)
    ));
}

/// The free space check covers the upload and everything the read writes
/// (F14).
#[cfg(unix)]
#[test]
fn space_runs_out_with_fixed_copy() {
    use crate::backup::FREE_MARGIN;
    let place = Place::new();
    let low = place
        .host()
        .with_free_space(Arc::new(|_: &Path| Ok(FREE_MARGIN + 1)));
    let mut upload = low.begin_upload().unwrap();
    assert!(matches!(upload.write(b"[]"), Err(ImportError::Space(n)) if n > 0));
    drop(upload);
    assert!(place.batches().is_empty());

    let zip_path = place.picked("export.zip", fixture("chatgpt-conversations.json"));
    let job = low.read_paths(vec![zip_path]).unwrap();
    let message = failed(wait(&low, &job.id));
    assert!(
        message.starts_with("There isn't enough free disk space. Free "),
        "{message}"
    );
    eventually("the read's folder goes", || place.batches().is_empty());
}

#[test]
fn rows_show_clean_titles_and_attachment_names() {
    let place = Place::new();
    let host = place.host();
    let mut record = chatgpt_records().swap_remove(3);
    let long = "x".repeat(500);
    record["title"] = json!(format!("\u{202e}Kitchen\u{7}\n layout {long}"));
    record["mapping"]["u1"]["message"]["metadata"]["attachments"][0]["name"] =
        json!(format!("\u{202e}fdp.exe\u{200b}{long}"));
    let id = record["conversation_id"].as_str().unwrap().to_string();
    let batch = read(&host, &[&json_bytes(&json!([record]))]);
    let title = &batch.candidates[0].title;
    assert!(title.starts_with("Kitchen layout xxx"), "{title}");
    assert!(
        title.ends_with('…') && title.chars().count() <= 121,
        "{title}"
    );
    assert!(!title.contains('\u{202e}') && !title.contains('\u{7}'));
    let thread = host.thread(&batch.id, ImportSource::ChatGpt, &id).unwrap();
    assert_eq!(&thread.candidate.title, title);
    let text: String = thread.messages.iter().map(|m| m.text()).collect();
    assert!(text.contains("Attachment not imported: fdp.exe"), "{text}");
    assert!(!text.contains('\u{202e}') && !text.contains('\u{200b}'));
    assert!(!text.contains(&"x".repeat(200)));

    // A title of hidden characters only names the chat by its prompt (M7).
    let mut hidden = chatgpt_records().swap_remove(3);
    hidden["title"] = json!("\u{2066}\u{200b}\u{7}\u{2069}");
    let batch = read(&host, &[&json_bytes(&json!([hidden]))]);
    let thread = host.thread(&batch.id, ImportSource::ChatGpt, &id).unwrap();
    let prompt = thread.messages[0].text();
    assert_eq!(batch.candidates[0].title, title_from_prompt(&prompt));
    assert_ne!(batch.candidates[0].title, title_from_prompt(""));
}

/// Claude's attached and listed files and Grok's attachments are named
/// cleanly too (M8).
#[test]
fn claude_and_grok_attachment_names_are_clean() {
    let place = Place::new();
    let host = place.host();
    let hostile = format!("\u{2066}notes\u{202e}.txt\u{61c}{}", "y".repeat(300));
    let mut claude: Value = serde_json::from_slice(fixture("claude-conversations.json")).unwrap();
    claude[0]["chat_messages"][1]["attachments"][0]["file_name"] = json!(hostile);
    claude[0]["chat_messages"][1]["files"][0]["file_name"] = json!(format!("\u{200b}{hostile}"));
    let mut grok: Value = serde_json::from_slice(fixture("grok-prod-grok-backend.json")).unwrap();
    grok["conversations"][3]["responses"][0]["response"]["file_attachments"][0] =
        json!(format!("\u{2067}{hostile}"));
    let batch = read(&host, &[&json_bytes(&claude), &json_bytes(&grok)]);
    let text_of = |source, id: &str| -> String {
        let thread = host.thread(&batch.id, source, id).unwrap();
        thread.messages.iter().map(|m| m.text()).collect()
    };
    let claude_text = text_of(ImportSource::ClaudeAi, "conv-sourdough-g");
    assert!(
        claude_text.contains("Attached notes.txtyyy"),
        "{claude_text}"
    );
    assert!(
        claude_text.contains("Attachment not imported: notes.txtyyy"),
        "{claude_text}"
    );
    let grok_text = text_of(ImportSource::Grok, "grok-files-0004");
    assert!(
        grok_text.contains("Attachment not imported: notes.txtyyy"),
        "{grok_text}"
    );
    for text in [&claude_text, &grok_text] {
        let hidden =
            |c: char| bruhs_models::imports::is_hidden_char(c) && !matches!(c, '\n' | '\t');
        assert!(!text.chars().any(hidden), "{text:?}");
        assert!(!text.contains(&"y".repeat(200)));
    }
}

#[test]
fn shown_names_drop_every_hidden_character() {
    assert_eq!(
        shown_name(
            "\u{2066}a\u{2067}b\u{2068}c\u{2069}d\u{61c}e\u{ad}f\u{e0041}g",
            120
        ),
        "abcdefg"
    );
    assert_eq!(
        shown_name("one\u{2028}two\u{2029}three\n\tfour", 120),
        "one two three four"
    );
    assert_eq!(shown_name("abcdef", 3), "abc…");
    assert_eq!(shown_name("\u{202e}\u{7}", 10), "");
}

/// The same memory file in two parts of a split export is offered once
/// (M6).
#[test]
fn repeated_memory_is_offered_once() {
    let place = Place::new();
    let host = place.host();
    let part = |name: &str| {
        zip_of(
            &[
                (name, fixture("chatgpt-conversations.json")),
                ("memories.json", fixture("claude-memories-account.json")),
            ],
            true,
        )
    };
    let batch = read(
        &host,
        &[
            &part("conversations-000.json"),
            &part("conversations-001.json"),
        ],
    );
    assert_eq!(batch.memory.len(), 1);
    let text = &batch.memory[0].text;
    assert_eq!(text.matches("weekend hikes").count(), 1, "{text}");
    // A different memory file still joins it.
    let both = zip_of(
        &[
            ("memories.json", fixture("claude-memories-account.json")),
            ("memories/old.json", fixture("claude-memories.json")),
        ],
        true,
    );
    let batch = read(&host, &[&both]);
    let text = &batch.memory[0].text;
    assert!(
        text.contains("weekend hikes") && text.contains("sourdough on weekends"),
        "{text}"
    );
}

/// A discard that lands while a finished read still holds the slot
/// removes the batch; one during an upload stops it (M5).
#[test]
fn discard_never_misses_a_live_batch() {
    let place = Place::new();
    let host = place.host();
    let batch = read(&host, &[fixture("chatgpt-conversations.json")]);
    let hold = host.hold_slot_for_test(&batch.id);
    host.discard(&batch.id).unwrap();
    assert!(place.batches().is_empty() && host.job(&batch.id).is_none());
    drop(hold);

    let mut upload = host.begin_upload().unwrap();
    upload.write(b"[").unwrap();
    let id = upload.id().to_string();
    host.discard(&id).unwrap();
    assert!(matches!(upload.write(b"]"), Err(ImportError::Gone)));
    assert!(matches!(upload.finish(), Err(ImportError::Gone)));
    assert!(place.batches().is_empty());
    assert!(host.begin_upload().is_ok(), "the slot is free");
}

#[test]
fn skipped_files_and_unknown_records_are_counted() {
    let place = Place::new();
    let host = place.host();
    let export = zip_of(
        &[
            ("conversations.json", fixture("chatgpt-conversations.json")),
            ("nested.zip", b"not a zip at all"),
            ("users.json", br#"[{"email":"owner@example.com"}]"#),
        ],
        true,
    );
    let batch = read(&host, &[&export, br#"[{"what":"no idea"}]"#]);
    assert_eq!(batch.counts.skipped_files, 1, "the .zip that isn't one");
    // users.json is never opened; the bare record is unknown.
    assert_eq!(batch.counts.skipped_unknown, 2);
    assert_eq!(batch.counts.conversations, 4);
}

/// A 100 001-node chat fits the shipped caps (32 MiB, 4 Mi values per
/// record), and the node past `MAX_NODES` makes it count as cut short.
#[test]
fn a_hundred_thousand_node_chat_fits_the_shipped_caps() {
    let place = Place::new();
    let host = ImportHost::new(place.root());
    #[cfg(unix)]
    let host = host.with_free_space(Arc::new(|_: &Path| Ok(u64::MAX)));
    let nodes = bruhs_models::imports::MAX_NODES + 1;
    let mut doc = String::from(r#"[{"id":"big","title":"Big","mapping":{"#);
    for n in 0..nodes {
        let parent = if n == 0 {
            "null".to_string()
        } else {
            format!("\"n{}\"", n - 1)
        };
        let message = if n % 10 == 0 {
            format!(
                r#"{{"id":"m{n}","author":{{"role":"user"}},"content":{{"content_type":"text","parts":["p{n}"]}},"create_time":{n}}}"#
            )
        } else {
            "null".to_string()
        };
        if n > 0 {
            doc.push(',');
        }
        doc.push_str(&format!(
            r#""n{n}":{{"id":"n{n}","parent":{parent},"children":[],"message":{message}}}"#
        ));
    }
    doc.push_str(&format!(r#"}},"current_node":"n{}"}}]"#, nodes - 1));
    assert!(doc.len() < 32 << 20, "{} bytes", doc.len());
    let batch = read(&host, &[doc.as_bytes()]);
    assert_eq!(batch.counts.conversations, 1);
    assert_eq!(batch.counts.cut_short, 1);
    assert!(host.thread(&batch.id, ImportSource::ChatGpt, "big").is_ok());
}

/// Memory is held only up to the part cut, and the cut counts what was
/// left out exactly as cutting the whole join would (final review C2).
#[test]
fn memory_holds_one_part_and_counts_the_rest() {
    let texts: Vec<String> = (0..50)
        .map(|n| format!("{}-{n}", "m".repeat(100_000)))
        .collect();
    let mut memory = MemoryTexts::default();
    for text in texts.iter().chain(&texts) {
        memory.add(text.clone());
    }
    assert!(
        memory.held_chars() <= MAX_PART_CHARS + 100_010,
        "{} characters held",
        memory.held_chars()
    );
    let suggestion = memory.suggestion().expect("memory is offered");
    assert_eq!(
        suggestion.text,
        clip_part(texts.join("\n\n"), MAX_PART_CHARS)
    );
    assert_eq!(MemoryTexts::default().suggestion(), None);
}

/// Repeated memory is found by digest, so many records dedupe in linear
/// time, however alike their texts are (final review C2).
#[test]
fn memory_dedupes_in_linear_time() {
    assert_scales_linearly(
        "memory dedupe",
        2_000,
        |n| {
            (0..n)
                .map(|i| format!("{}-{i}", "m".repeat(64)))
                .collect::<Vec<_>>()
        },
        |texts| {
            let mut memory = MemoryTexts::default();
            for text in texts {
                memory.add(text.clone());
            }
            std::hint::black_box(memory.suggestion());
        },
    );
}

/// Many large memory files in one ZIP give one memory within a part
/// (final review C2's ZIP, scaled to the test caps).
#[test]
fn many_memory_files_give_one_part() {
    let place = Place::new();
    let host = place.host();
    let files: Vec<(String, Vec<u8>)> = (0..120)
        .map(|n| {
            let text = format!("{}-{n}", "m".repeat(100_000));
            (
                format!("memories/{n}.json"),
                json_bytes(&json!({ "conversations_memory": text })),
            )
        })
        .collect();
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let batch = read(&host, &[&zip_of(&entries, true)]);
    assert_eq!(batch.memory.len(), 1);
    let text = &batch.memory[0].text;
    let more = 120 * 100_004 + 2 * 119 - MAX_PART_CHARS - 10 * 2 - 90;
    assert!(
        text.ends_with(&cut_short_line(more)),
        "{}",
        &text[text.len() - 80..]
    );
    assert!(text.chars().count() <= MAX_PART_CHARS + 64);
}
