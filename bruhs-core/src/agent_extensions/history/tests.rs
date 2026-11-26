use super::*;
use serde_json::json;
fn write_rows(path: &Path, rows: &[Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
}
fn request(source: UsageHistorySource, root: &Path) -> UsageHistoryRequest {
    UsageHistoryRequest {
        source,
        root: Some(root.to_string_lossy().into_owned()),
        max_files: 50,
        max_bytes: SCAN_BYTES,
        offset: 0,
        inventory: None,
    }
}
fn claude(output: u64) -> Value {
    json!({"type":"assistant","sessionId":"native-session","cwd":"/project","timestamp":"2026-11-13T00:00:00Z","message":{"id":"message-1","model":"claude-test","content":[{"text":"NEVER_COPY_TRANSCRIPT"}],"usage":{"input_tokens":10,"output_tokens":output,"cache_read_input_tokens":20,"cache_creation_input_tokens":5}}})
}
#[test]
fn preview_is_metadata_only_and_commit_deduplicates_stream_updates_and_retries() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let path = source.path().join("projects/p/session.jsonl");
    write_rows(&path, &[claude(3), claude(7), claude(7)]);
    let preview = preview_at(
        store.path(),
        request(UsageHistorySource::Claude, source.path()),
    )
    .unwrap();
    assert!(!store.path().join(LEDGER).exists());
    assert_eq!(preview.usage.records, 1);
    assert_eq!(preview.usage.by_model["claude-test"].output, 7);
    assert!(!std::fs::read_to_string(store.path().join(PREVIEW))
        .unwrap()
        .contains("NEVER_COPY_TRANSCRIPT"));
    for _ in 0..2 {
        let saved = commit_at(store.path(), &preview.id).unwrap();
        assert_eq!(saved.records, 1);
        assert_eq!(saved.by_model["claude-test"].input, 10);
    }
    assert!(!store.path().join(PREVIEW).exists());
    write_rows(&path, &[claude(12)]);
    let next = preview_at(
        store.path(),
        request(UsageHistorySource::Claude, source.path()),
    )
    .unwrap();
    assert_eq!(
        commit_at(store.path(), &next.id).unwrap().by_model["claude-test"].output,
        12
    );
    assert!(commit_at(store.path(), &preview.id).is_err());
    let snapshot = super::super::snapshot_at(store.path()).unwrap();
    assert!(snapshot.by_model.is_empty());
    assert_eq!(snapshot.imported.records, 1);
}
fn codex(input: u64, read: u64, output: u64) -> Value {
    json!({"type":"event_msg","timestamp":"2026-11-13T00:00:01Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":read,"output_tokens":output}}}})
}
#[test]
fn codex_cumulative_rows_count_only_new_usage_and_decreases_are_explicit() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let path = source
        .path()
        .join("sessions/2026/11/13/rollout-session.jsonl");
    let meta = json!({"type":"session_meta","payload":{"id":"session","cwd":"/project"}});
    let model = json!({"type":"turn_context","payload":{"model":"gpt-test"}});
    write_rows(
        &path,
        &[
            meta.clone(),
            model.clone(),
            codex(100, 40, 10),
            codex(100, 40, 10),
            codex(150, 50, 25),
        ],
    );
    let preview = preview_at(
        store.path(),
        request(UsageHistorySource::Codex, source.path()),
    )
    .unwrap();
    let total = &preview.usage.by_model["gpt-test"];
    assert_eq!((total.input, total.cache_read, total.output), (100, 50, 25));
    commit_at(store.path(), &preview.id).unwrap();
    let again = preview_at(
        store.path(),
        request(UsageHistorySource::Codex, source.path()),
    )
    .unwrap();
    assert_eq!(
        commit_at(store.path(), &again.id).unwrap().by_model["gpt-test"].input,
        100
    );
    write_rows(&path, &[meta, model, codex(100, 40, 10), codex(3, 0, 1)]);
    let broken = preview_at(
        store.path(),
        request(UsageHistorySource::Codex, source.path()),
    )
    .unwrap();
    assert_eq!(broken.usage.records, 0);
    assert!(broken
        .warnings
        .iter()
        .any(|message| message.contains("decreased")));
}

#[test]
fn overlapping_codex_logs_cannot_inflate_the_same_cumulative_identity() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let meta = json!({"type":"session_meta","payload":{"id":"session","cwd":"/project"}});
    let model = json!({"type":"turn_context","payload":{"model":"gpt-test"}});
    write_rows(
        &source.path().join("sessions/a.jsonl"),
        &[
            meta.clone(),
            model.clone(),
            codex(100, 40, 10),
            codex(150, 50, 25),
        ],
    );
    let preview = preview_at(
        store.path(),
        request(UsageHistorySource::Codex, source.path()),
    )
    .unwrap();
    commit_at(store.path(), &preview.id).unwrap();
    write_rows(
        &source.path().join("sessions/b.jsonl"),
        &[meta, model, codex(150, 50, 25)],
    );
    let error = preview_at(
        store.path(),
        request(UsageHistorySource::Codex, source.path()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("conflicting deltas"));
    assert_eq!(
        snapshot_at(store.path(), &ExtensionConfig::default())
            .unwrap()
            .by_model["gpt-test"]
            .input,
        100
    );
}
#[test]
fn malformed_logs_do_not_allocate_a_warning_for_each_row_and_forged_receipts_fail() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let path = source.path().join("projects/p/session.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json\n".repeat(1000)).unwrap();
    let preview = preview_at(
        store.path(),
        request(UsageHistorySource::Claude, source.path()),
    )
    .unwrap();
    assert_eq!(preview.warnings.len(), 1);
    assert_eq!(preview.usage.records, 0);
    assert!(commit_at(store.path(), "../native-usage").is_err());
    assert!(commit_at(store.path(), &uuid::Uuid::new_v4().to_string()).is_err());
}

#[test]
fn byte_budget_never_silently_skips_the_remainder_of_a_file() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let path = source.path().join("projects/p/session.jsonl");
    write_rows(&path, &[claude(7), claude(12)]);
    let mut small = request(UsageHistorySource::Claude, source.path());
    small.max_bytes = std::fs::metadata(&path).unwrap().len() - 1;
    let cut = preview_at(store.path(), small).unwrap();
    assert_eq!(cut.files_read, 0);
    assert_eq!(cut.usage.records, 0);
    assert_eq!(cut.next_offset, Some(0));
    let mut larger = request(UsageHistorySource::Claude, source.path());
    larger.max_bytes = cut.minimum_next_bytes.unwrap();
    let complete = preview_at(store.path(), larger).unwrap();
    assert_eq!(complete.files_read, 1);
    assert_eq!(complete.usage.by_model["claude-test"].output, 12);
    assert_eq!(complete.next_offset, None);
}
#[cfg(unix)]
#[test]
fn symlinked_log_files_are_not_followed() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("session.jsonl");
    write_rows(&path, &[claude(7)]);
    let project = source.path().join("projects/p");
    std::fs::create_dir_all(&project).unwrap();
    std::os::unix::fs::symlink(path, project.join("session.jsonl")).unwrap();
    let preview = preview_at(
        store.path(),
        request(UsageHistorySource::Claude, source.path()),
    )
    .unwrap();
    assert_eq!(preview.files_read, 0);
}

#[cfg(unix)]
#[test]
fn escaping_log_root_and_special_files_cannot_be_opened() {
    let source = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    write_rows(&outside.path().join("session.jsonl"), &[claude(7)]);
    std::os::unix::fs::symlink(outside.path(), source.path().join("projects")).unwrap();
    let error = preview_at(
        store.path(),
        request(UsageHistorySource::Claude, source.path()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("outside"));
    assert!(!store.path().join(PREVIEW).exists());
    std::fs::remove_file(source.path().join("projects")).unwrap();
    std::fs::create_dir(source.path().join("projects")).unwrap();
    let fifo = source.path().join("projects/hang.jsonl");
    use std::os::unix::ffi::OsStrExt as _;
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: name is a live NUL-terminated filename and mkfifo retains no pointer.
    let created = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
    assert_eq!(created, 0);
    let start = std::time::Instant::now();
    assert!(open_regular(&fifo).is_err());
    let preview = preview_at(
        store.path(),
        request(UsageHistorySource::Claude, source.path()),
    )
    .unwrap();
    assert_eq!(preview.files_read, 0);
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    let tree =
        LogTree::open(std::fs::canonicalize(source.path().join("projects")).unwrap()).unwrap();
    assert!(tree
        .file(
            &std::fs::canonicalize(source.path().join("projects"))
                .unwrap()
                .join("hang.jsonl")
        )
        .is_err());
}

#[test]
fn malformed_or_oversized_codex_context_excludes_the_file_instead_of_using_the_previous_model() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let path = source.path().join("sessions/session.jsonl");
    let first = [
        json!({"type":"session_meta","payload":{"id":"session","cwd":"/project"}}),
        json!({"type":"turn_context","payload":{"model":"gpt-a"}}),
        codex(100, 40, 10),
    ];
    write_rows(&path, &first);
    use std::io::Write as _;
    {
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{malformed turn_context").unwrap();
        writeln!(file, "{}", codex(200, 50, 20)).unwrap();
    }
    let malformed = preview_at(
        store.path(),
        request(UsageHistorySource::Codex, source.path()),
    )
    .unwrap();
    assert_eq!(malformed.usage.records, 0);
    assert!(malformed
        .warnings
        .iter()
        .any(|warning| warning.contains("file was excluded")));
    let mut oversized = first.to_vec();
    oversized.push(json!({"type":"turn_context","payload":{"model":"gpt-b","padding":"x".repeat(usize::try_from(FRAME_BYTES).unwrap())}}));
    oversized.push(codex(200, 50, 20));
    write_rows(&path, &oversized);
    let oversized = preview_at(
        store.path(),
        request(UsageHistorySource::Codex, source.path()),
    )
    .unwrap();
    assert_eq!(oversized.usage.records, 0);
    assert!(oversized
        .warnings
        .iter()
        .any(|warning| warning.contains("file was excluded")));
}

#[test]
fn later_pages_bind_to_the_first_inventory_and_replaced_previews_cannot_commit() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    write_rows(&source.path().join("projects/b.jsonl"), &[claude(7)]);
    let mut other = claude(9);
    other["message"]["id"] = json!("second-message");
    write_rows(&source.path().join("projects/c.jsonl"), &[other]);
    let mut first_request = request(UsageHistorySource::Claude, source.path());
    first_request.max_files = 1;
    let first = preview_at(store.path(), first_request.clone()).unwrap();
    assert_eq!(first.next_offset, Some(1));
    let mut next_request = first_request.clone();
    next_request.offset = 1;
    assert!(preview_at(store.path(), next_request.clone())
        .unwrap_err()
        .to_string()
        .contains("first"));
    next_request.inventory = Some(first.inventory.clone());
    let next = preview_at(store.path(), next_request.clone()).unwrap();
    assert!(commit_at(store.path(), &first.id)
        .unwrap_err()
        .to_string()
        .contains("replaced"));
    let imported = commit_at(store.path(), &next.id).unwrap();
    assert_eq!(imported.records, 1);
    assert!(!store.path().join(PREVIEW).exists());
    assert_eq!(commit_at(store.path(), &next.id).unwrap().records, 1);
    write_rows(&source.path().join("projects/a.jsonl"), &[claude(11)]);
    assert!(preview_at(store.path(), next_request)
        .unwrap_err()
        .to_string()
        .contains("inventory changed"));
    let restarted = preview_at(store.path(), first_request).unwrap();
    assert_ne!(first.inventory, restarted.inventory);
}

#[test]
fn a_process_lock_prevents_a_second_companion_writer() {
    let store = tempfile::tempdir().unwrap();
    let lock = process_lock(store.path()).unwrap();
    assert!(process_lock(store.path()).is_err());
    #[cfg(unix)]
    {
        let result=std::process::Command::new("python3").args(["-c","import fcntl,sys\nf=open(sys.argv[1],'rb')\ntry: fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)\nexcept BlockingIOError: sys.exit(2)",store.path().join("usage-history.lock").to_str().unwrap()]).status().unwrap();
        assert_eq!(result.code(), Some(2));
    }
    drop(lock);
    assert!(process_lock(store.path()).is_ok());
}

#[cfg(unix)]
#[test]
fn a_directory_replaced_by_a_symlink_cannot_redirect_the_open_log_tree() {
    let source = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write_rows(
        &source.path().join("projects/p/session.jsonl"),
        &[claude(7)],
    );
    write_rows(&outside.path().join("session.jsonl"), &[claude(11)]);
    let root = std::fs::canonicalize(source.path().join("projects")).unwrap();
    let tree = LogTree::open(root.clone()).unwrap();
    std::fs::rename(root.join("p"), root.join("original")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("p")).unwrap();
    assert!(tree.file(&root.join("p/session.jsonl")).is_err());
}

#[test]
fn committing_a_page_that_exceeds_the_cumulative_ledger_budget_preserves_both_inputs() {
    let source = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    write_rows(&source.path().join("projects/source.jsonl"), &[claude(7)]);
    let mut preview = preview_at(
        store.path(),
        request(UsageHistorySource::Claude, source.path()),
    )
    .unwrap();
    let project = "x".repeat(1 << 20);
    let record = |id: String| UsageRecord {
        id,
        thread_id: "fixture".into(),
        session_id: None,
        model: "fixture-model".into(),
        project: project.clone(),
        at: 0,
        input: 1,
        output: 1,
        cache_read: 0,
        cache_write: 0,
        requested_speed: Default::default(),
        cli_cost_usd: None,
    };
    let records = (0..49)
        .map(|index| {
            let row = record(format!("old-{index}"));
            (row.id.clone(), row)
        })
        .collect();
    atomic(
        store.path(),
        &store.path().join(LEDGER),
        &Ledger {
            records,
            imported_at: None,
            last_committed_preview: None,
        },
    )
    .unwrap();
    let original_size = std::fs::metadata(store.path().join(LEDGER)).unwrap().len();
    let records = (0..2)
        .map(|index| {
            let row = record(format!("new-{index}"));
            (row.id.clone(), row)
        })
        .collect::<BTreeMap<_, _>>();
    preview.usage = aggregate(&records, &ExtensionConfig::default(), None);
    let id = preview.id.clone();
    atomic(
        store.path(),
        &store.path().join(PREVIEW),
        &Frozen { preview, records },
    )
    .unwrap();
    assert!(commit_at(store.path(), &id)
        .unwrap_err()
        .to_string()
        .contains("50 MiB"));
    assert_eq!(
        std::fs::metadata(store.path().join(LEDGER)).unwrap().len(),
        original_size
    );
    assert_eq!(read_ledger(store.path()).unwrap().records.len(), 49);
    assert!(store.path().join(PREVIEW).exists());
}

#[test]
fn retained_metadata_is_bounded_before_publishing_a_new_ledger() {
    let store = tempfile::tempdir().unwrap();
    let path = store.path().join(LEDGER);
    std::fs::write(&path, r#"{"records":{},"imported_at":null}"#).unwrap();
    struct Large<'a>(&'a str);
    impl Serialize for Large<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq as _;
            let mut sequence = serializer.serialize_seq(Some(51))?;
            for _ in 0..51 {
                sequence.serialize_element(self.0)?;
            }
            sequence.end()
        }
    }
    let text = "x".repeat(1 << 20);
    let before = std::fs::read(&path).unwrap();
    assert!(atomic(store.path(), &path, &Large(&text))
        .unwrap_err()
        .to_string()
        .contains("50 MiB"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let file = OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(SCAN_BYTES + 1).unwrap();
    assert!(read_ledger(store.path())
        .err()
        .unwrap()
        .to_string()
        .contains("50 MiB"));
}
