use super::*;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

fn folder_source(root: &Path, settle_ms: u64) -> EventSource {
    EventSource {
        id: "scans".into(),
        name: "Paper scans".into(),
        enabled: true,
        kind: SourceKind::Folder {
            path: root.to_str().unwrap().into(),
            pattern: "*.pdf".into(),
            recursive: true,
            settle_ms,
            poll_ms: 1000,
        },
    }
}

fn scan(root: &Path) -> Vec<ScannedFile> {
    scan_folder(root, "*.pdf", true).unwrap()
}

fn unbounded_folder_batch(
    cursor: Option<Value>,
    files: Vec<ScannedFile>,
    source: &EventSource,
    now: u64,
    settle_ms: u64,
) -> Result<PollBatch, ReactiveError> {
    folder_batch(cursor, files, source, now, settle_ms, usize::MAX)
}

#[test]
fn folder_reconciles_files_added_during_downtime_without_reemitting_completed_work() {
    let directory = tempfile::tempdir().unwrap();
    let source = folder_source(directory.path(), 50);
    let initial = unbounded_folder_batch(None, scan(directory.path()), &source, 100, 50).unwrap();
    assert!(initial.events.is_empty());

    // A scanner writes while the source process is not running.
    std::fs::write(directory.path().join("bill.pdf"), b"a complete bill").unwrap();
    let restarted = unbounded_folder_batch(
        Some(initial.cursor),
        scan(directory.path()),
        &source,
        1000,
        50,
    )
    .unwrap();
    assert!(restarted.events.is_empty());
    let settled = unbounded_folder_batch(
        Some(restarted.cursor),
        scan(directory.path()),
        &source,
        1050,
        50,
    )
    .unwrap();
    assert_eq!(settled.events.len(), 1);
    assert_eq!(settled.events[0].payload["file"], "bill.pdf");

    // A durable checkpoint remains meaningful after serialization and another restart.
    let persisted = serde_json::to_string(&settled.cursor).unwrap();
    let next = unbounded_folder_batch(
        Some(serde_json::from_str(&persisted).unwrap()),
        scan(directory.path()),
        &source,
        2000,
        50,
    )
    .unwrap();
    assert!(next.events.is_empty());
}

#[test]
fn folder_waits_for_growth_to_stop_and_preserves_settling_observation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bill.pdf");
    let source = folder_source(directory.path(), 50);
    std::fs::write(&path, b"partial").unwrap();
    let partial = unbounded_folder_batch(None, scan(directory.path()), &source, 100, 50).unwrap();
    std::fs::write(&path, b"partial plus the rest of the bill").unwrap();
    let growing = unbounded_folder_batch(
        Some(partial.cursor),
        scan(directory.path()),
        &source,
        140,
        50,
    )
    .unwrap();
    assert!(growing.events.is_empty());
    let waiting = unbounded_folder_batch(
        Some(growing.cursor),
        scan(directory.path()),
        &source,
        170,
        50,
    )
    .unwrap();
    assert!(waiting.events.is_empty());
    let complete = unbounded_folder_batch(
        Some(waiting.cursor),
        scan(directory.path()),
        &source,
        190,
        50,
    )
    .unwrap();
    assert_eq!(complete.events.len(), 1);
    assert_eq!(complete.events[0].payload["size"], 33);
}

#[test]
fn folder_reappearance_is_a_new_event_even_when_its_content_fingerprint_matches() {
    let directory = tempfile::tempdir().unwrap();
    let source = folder_source(directory.path(), 0);
    let make_file = || ScannedFile {
        path: "/scans/bill.pdf".into(),
        relative: "bill.pdf".into(),
        size: 100,
        fingerprint: "same".into(),
    };
    let first = unbounded_folder_batch(None, vec![make_file()], &source, 100, 0).unwrap();
    let gone = unbounded_folder_batch(Some(first.cursor), Vec::new(), &source, 200, 0).unwrap();
    let again =
        unbounded_folder_batch(Some(gone.cursor), vec![make_file()], &source, 300, 0).unwrap();
    assert_eq!(again.events.len(), 1);
    assert_ne!(first.events[0].key, again.events[0].key);
}

#[test]
fn folder_backlog_commits_only_available_capacity_and_drains_after_review() {
    let directory = tempfile::tempdir().unwrap();
    let source = folder_source(directory.path(), 0);
    let files = || {
        (0..100)
            .map(|index| ScannedFile {
                path: format!("/scans/bill-{index}.pdf"),
                relative: format!("bill-{index}.pdf"),
                size: 100,
                fingerprint: format!("fingerprint-{index}"),
            })
            .collect()
    };
    let first = folder_batch(None, files(), &source, 100, 0, 50).unwrap();
    assert_eq!(first.events.len(), 50);
    assert!(first.backpressured);
    let full = folder_batch(Some(first.cursor), files(), &source, 200, 0, 0).unwrap();
    assert!(full.events.is_empty());
    assert!(full.backpressured);
    let after_review = folder_batch(Some(full.cursor), files(), &source, 300, 0, 50).unwrap();
    assert_eq!(after_review.events.len(), 50);
    assert!(!after_review.backpressured);
    assert!(after_review.events.iter().all(|event| first
        .events
        .iter()
        .all(|previous| previous.key != event.key)));
    let idle = folder_batch(Some(after_review.cursor), files(), &source, 400, 0, 0).unwrap();
    assert!(idle.events.is_empty());
    assert!(!idle.backpressured);
}

#[cfg(unix)]
#[test]
fn folder_skips_symlinks_and_dependency_trees() {
    let directory = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("private.pdf"), b"private").unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), directory.path().join("escape")).unwrap();
    std::os::unix::fs::symlink(
        elsewhere.path().join("private.pdf"),
        directory.path().join("link.pdf"),
    )
    .unwrap();
    std::fs::create_dir(directory.path().join("node_modules")).unwrap();
    std::fs::write(
        directory.path().join("node_modules/ignored.pdf"),
        b"ignored",
    )
    .unwrap();
    std::fs::create_dir(directory.path().join("clients")).unwrap();
    std::fs::write(directory.path().join("clients/bill.pdf"), b"bill").unwrap();
    let files = scan(directory.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].relative, "clients/bill.pdf");
}

#[test]
fn filename_masks_are_anchored_and_handle_unicode_questions() {
    assert!(wildcard_match("*.pdf", "bill.pdf"));
    assert!(wildcard_match("*.pdf", "BILL.PDF"));
    assert!(!wildcard_match("*.pdf", "bill.pdf.tmp"));
    assert!(wildcard_match("bill-?.pdf", "bill-日.pdf"));
    assert!(!wildcard_match("bill-?.pdf", "bill-ab.pdf"));
    assert!(wildcard_match("a*b*c", "axybzc"));
    assert!(!wildcard_match("a*b*c", "axybzd"));
}

#[tokio::test]
async fn http_changes_emit_again_after_a_value_cycles_back() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("first"))
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let bytes = read_http(&client, &server.uri(), 1000).await.unwrap();
    let first = body_batch(None, "http", &server.uri(), &bytes, json!({})).unwrap();
    let same = body_batch(Some(first.cursor), "http", &server.uri(), &bytes, json!({})).unwrap();
    assert_eq!(first.events.len(), 1);
    assert!(same.events.is_empty());
    let second = body_batch(
        Some(same.cursor),
        "http",
        &server.uri(),
        b"second",
        json!({}),
    )
    .unwrap();
    let first_again = body_batch(
        Some(second.cursor),
        "http",
        &server.uri(),
        &bytes,
        json!({}),
    )
    .unwrap();
    assert_eq!(first_again.events.len(), 1);
    assert_ne!(first.events[0].key, first_again.events[0].key);
}

#[tokio::test]
async fn http_failures_and_oversized_bodies_are_explicit_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let error = read_http(&client, &server.uri(), 1000).await.unwrap_err();
    assert!(error.to_string().contains("503"));
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(vec![b'a'; MAX_SOURCE_BODY_BYTES + 1]),
        )
        .mount(&server)
        .await;
    let error = read_http(&client, &server.uri(), 1000).await.unwrap_err();
    assert!(error.to_string().contains("body budget"));
}

#[cfg(unix)]
#[tokio::test]
async fn command_returns_output_and_stops_a_timed_out_process_group() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        read_command("printf '%s' '{\"sensor\":true}'", directory.path(), 1000)
            .await
            .unwrap(),
        br#"{"sensor":true}"#
    );
    let started = tokio::time::Instant::now();
    let error = read_command("sleep 10", directory.path(), 20)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timeout"));
    assert!(started.elapsed() < Duration::from_secs(2));
    let oversized = read_command("yes x", directory.path(), 1000)
        .await
        .unwrap_err();
    assert!(oversized.to_string().contains("body budget"));
}

#[test]
fn schedule_restarts_catch_up_once_and_keep_the_next_aligned_deadline() {
    let first = schedule_batch(None, 100, 100).unwrap();
    assert!(first.events.is_empty());
    let after_downtime = schedule_batch(Some(first.cursor), 1250, 100).unwrap();
    assert_eq!(after_downtime.events.len(), 1);
    assert_eq!(after_downtime.events[0].payload["scheduled_at_ms"], 200);
    assert_eq!(after_downtime.events[0].payload["missed_intervals"], 10);
    assert_eq!(after_downtime.cursor["next_due_ms"], 1300);
    let next = schedule_batch(Some(after_downtime.cursor), 1251, 100).unwrap();
    assert!(next.events.is_empty());
}

#[test]
fn invalid_checkpoints_fail_instead_of_resetting_and_replaying() {
    assert!(schedule_batch(Some(json!({"invalid":true})), 100, 100).is_err());
    assert!(body_batch(
        Some(json!({"digest":12})),
        "http",
        "test",
        b"text",
        json!({})
    )
    .is_err());
}

#[tokio::test]
async fn restarted_weekly_source_wakes_for_its_persisted_deadline() {
    use bruhs_models::{
        reactive::ReactiveChange, reactive::ReactiveCommand, weekly_reports::WEEK_MS,
    };
    let dir = tempfile::tempdir().unwrap();
    let runtime = ReactiveRuntime::open(dir.path().join("weekly-clock.sqlite3"))
        .await
        .unwrap();
    let every_ms = u64::try_from(WEEK_MS).unwrap();
    let source = EventSource {
        id: "weekly-clock".into(),
        name: "Weekly clock".into(),
        enabled: true,
        kind: SourceKind::Schedule { every_ms },
    };
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSource {
                source: source.clone(),
            },
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: bruhs_models::reactive::ReactiveAgent {
                    id: "weekly-reader".into(),
                    name: "Weekly reader".into(),
                    instructions: String::new(),
                    memory: String::new(),
                    model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
                    project_path: dir.path().display().to_string(),
                    enabled: true,
                },
            },
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveSubscription {
                subscription: bruhs_models::reactive::Subscription {
                    id: "weekly-subscription".into(),
                    source_id: source.id.clone(),
                    agent_id: "weekly-reader".into(),
                    instructions: String::new(),
                    enabled: true,
                    auto_run: false,
                    debounce_ms: 0,
                    throttle_ms: 0,
                    on_busy: Default::default(),
                },
            },
        })
        .await
        .unwrap();
    let due = now_ms().unwrap() + 1000;
    runtime
        .ingest(
            &source,
            vec![],
            "schedule",
            json!({"every_ms":every_ms,"next_due_ms":due,"sequence":0}),
        )
        .await
        .unwrap();
    let mut changes = runtime.watch_changes();
    let worker = tokio::spawn(drive(runtime.clone(), source, reqwest::Client::new()));
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = runtime.snapshot().await.unwrap();
            if let Some(event) = snapshot
                .events
                .into_iter()
                .find(|event| event.source_id == "weekly-clock")
            {
                break event;
            }
            changes.changed().await.unwrap();
        }
    })
    .await;
    worker.abort();
    let _ = worker.await;
    let event=event.expect("A resumed weekly source must wake for its saved due time rather than sleep another seven days");
    assert_eq!(event.payload["scheduled_at_ms"], due);
}
