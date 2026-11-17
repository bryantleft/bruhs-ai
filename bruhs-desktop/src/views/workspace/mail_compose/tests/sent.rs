//! Synthetic loopback controls only. These tests are written, not yet run.
use super::*;
use bruhs_core::mail::device::{DeviceDraftRecord, DeviceFileBytes};
use bruhs_models::connections::{ConnectionState, ServiceConnection};
use gpui::AppContext as _;
use gpui_component::zoom::px;
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt as _;

#[derive(Clone)]
struct Request {
    method: String,
    path: String,
    body: Value,
}

struct Fixture {
    base: String,
    provider: ServiceProvider,
    requests: Arc<Mutex<Vec<Request>>>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    worker: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

fn send_status(state: MailDispatchState) -> MailSendStatus {
    MailSendStatus {
        host_id: "host-a".into(),
        mailbox_id: "mailbox-a".into(),
        operation_id: "uncertain-operation".into(),
        draft_id: "reviewed-draft".into(),
        draft_revision: 4,
        state,
        provider_message_id: None,
        provider_thread_id: None,
        problem: (state == MailDispatchState::Uncertain)
            .then(|| MailSendProblem::new(MailSendProblemKind::Unconfirmed)),
    }
}

fn snapshot(provider: ServiceProvider) -> MailboxesSnapshot {
    MailboxesSnapshot {
        host_id: "host-a".into(),
        mailboxes: vec![MailboxSummary {
            mailbox_id: "mailbox-a".into(),
            provider,
            last_connection_id: "stale-cached-generation".into(),
            account_label: "you@example.test".into(),
            revision: 1,
            threads: 0,
            drafts: 1,
        }],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hold {
    None,
    Connection,
    Reconcile,
}

impl Fixture {
    fn new(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
        provider: ServiceProvider,
        reply: MailDispatchState,
        deny: bool,
        hold: Hold,
    ) -> Self {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        assert!(![3001, 9444].contains(&socket.local_addr().unwrap().port()));
        let base = format!("http://{}", socket.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (seen, arrived, resume) = (
            Arc::clone(&requests),
            Arc::clone(&entered),
            Arc::clone(&release),
        );
        let worker = workspace.read_with(cx, |_,cx| cx.global::<DesktopState>().runtime.spawn(async move {
            let socket = tokio::net::TcpListener::from_std(socket).unwrap();
            while let Ok((stream,_)) = socket.accept().await {
                let mut input = tokio::io::BufReader::new(stream);
                let mut first = String::new();
                if input.read_line(&mut first).await.unwrap_or(0) == 0 { continue; }
                let mut parts = first.split_whitespace();
                let method = parts.next().unwrap().to_owned();
                let path = parts.next().unwrap().to_owned();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    assert_ne!(input.read_line(&mut line).await.unwrap(), 0, "complete fixture request headers");
                    if line == "\r\n" { break; }
                    if let Some((name,value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") { length = value.trim().parse::<usize>().unwrap(); }
                    }
                }
                // Same request ceiling as the actual compose router.
                assert!(length <= 8 << 20);
                let mut bytes = vec![0;length];
                input.read_exact(&mut bytes).await.unwrap();
                let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
                seen.lock().unwrap().push(Request {method:method.clone(),path:path.clone(),body});
                let connection_path = match provider {ServiceProvider::Gmail=>"/api/connections/gmail",ServiceProvider::Outlook=>"/api/connections/outlook",_=>unreachable!()};
                let (code,body) = if method == "GET" && path == "/api/mail" {
                    ("200 OK",serde_json::to_value(snapshot(provider)).unwrap())
                } else if method == "GET" && path == connection_path {
                    if hold == Hold::Connection { arrived.notify_one(); resume.notified().await; }
                    ("200 OK",serde_json::to_value(ServiceConnection {id:Some("renewed-generation".into()),provider,configured:true,account:Some("you@example.test".into()),scopes:vec![],state:ConnectionState::Connected}).unwrap())
                } else if method == "GET" && path.contains("/sends/uncertain-operation?") {
                    ("200 OK",serde_json::to_value(send_status(MailDispatchState::Uncertain)).unwrap())
                } else if method == "GET" && path.contains("/drafts?") {
                    ("200 OK",json!({"host_id":"host-a","mailbox_id":"mailbox-a","revision":1,"page":0,"total_matches":0,"rows":[]}))
                } else if method == "POST" && path == "/api/mail/mailboxes/mailbox-a/sends/uncertain-operation/reconcile" {
                    if hold == Hold::Reconcile { arrived.notify_one(); resume.notified().await; }
                    if deny { ("403 Forbidden",json!({"error":"private provider diagnostics must not be displayed"})) }
                    else { ("200 OK",serde_json::to_value(send_status(reply)).unwrap()) }
                } else { ("404 Not Found",json!({"error":"unexpected synthetic request"})) };
                let body = body.to_string();
                let response = format!("HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                let _ = input.get_mut().write_all(response.as_bytes()).await;
            }
        }));
        Self {
            base,
            provider,
            requests,
            entered,
            release,
            worker,
        }
    }
}

fn install(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    fixture: &Fixture,
) -> (
    Entity<MailComposer>,
    MailDeviceStore,
    String,
    tempfile::TempDir,
) {
    let directory = tempfile::tempdir().unwrap();
    let mut value = DeviceDraft::new(
        scope(&fixture.base),
        fixture.provider,
        None,
        MailComposeContent {
            to: "recipient@example.test".into(),
            subject: "Newer saved version".into(),
            body: "Newer retained text 東京".into(),
            attachments: vec!["server-file".into()],
            ..Default::default()
        },
    );
    value.server_revision = 9;
    value.dirty = false;
    value.pending_send = Some(MailSendRequest {
        host_id: "host-a".into(),
        connection_id: "old-generation".into(),
        operation_id: "uncertain-operation".into(),
        revision: 4,
        digest: "a".repeat(64),
    });
    value.latest_send = Some(send_status(MailDispatchState::Uncertain));
    value.files = vec![DeviceDraftFile {
        id: "saved-file".into(),
        file_name: "retained.bin".into(),
        size_bytes: 2,
        server: Some(MailDraftFile {
            id: "server-file".into(),
            file_name: "retained.bin".into(),
            size_bytes: 2,
            sha256: "06eb7d6a69ee19e5fbdf749018d3d2abfa04bcbd1365db312eb86dc7169389b8".into(),
        }),
    }];
    let id = value.client_id.clone();
    let store = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>().runtime.block_on(async {
            let store = MailDeviceStore::open(directory.path().join("drafts.sqlite3"))
                .await
                .unwrap();
            store
                .save(
                    value,
                    None,
                    vec![DeviceFileBytes {
                        id: "saved-file".into(),
                        bytes: vec![0, 255],
                    }],
                )
                .await
                .unwrap();
            store
        })
    });
    let client = ServerClient::new(&fixture.base, &fixture.base).unwrap();
    let view = attach(workspace, cx, store.clone(), client.clone());
    workspace.update(cx, |workspace, cx| {
        workspace.connected_inbox.update(cx, |mail, _| {
            mail.compose_test_setup(client, snapshot(fixture.provider));
        });
        cx.notify();
    });
    (view, store, id, directory)
}

async fn open_saved(view: &Entity<MailComposer>, cx: &mut VisualTestContext, id: String) {
    cx.run_until_parked();
    // The both-provider case reuses this window. Leave its previous composer
    // through the same keyboard action a user can use before opening Drafts.
    if cx.debug_bounds("desktop-compose-close").is_some() {
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
    }
    let entry = cx
        .debug_bounds("desktop-compose-entry-Drafts")
        .expect("real Drafts entry");
    cx.simulate_click(entry.center(), Modifiers::default());
    cx.condition(view, |view, _| !view.loading).await;
    view.update_in(cx, |view, window, cx| view.open(id, window, cx));
    cx.condition(view, |view, _| {
        view.active().is_some_and(|job| {
            job.phase == Phase::Ready && !job.running && job.saved == job.sequence
        })
    })
    .await;
}

// Draw the currently selected draft and reveal the real control before reading
// its click position. Navigation can change both the frame and the scroll offset.
fn check_sent_button(cx: &mut VisualTestContext) -> gpui::Bounds<gpui::Pixels> {
    cx.run_until_parked();
    let feedback = cx
        .debug_bounds("desktop-compose-feedback")
        .expect("real draft feedback");
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: feedback.origin + gpui::point(gpui::px(8.0), gpui::px(8.0)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-100_000.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    let button = cx
        .debug_bounds("desktop-compose-check-sent")
        .expect("real Check Sent control");
    let feedback = cx
        .debug_bounds("desktop-compose-feedback")
        .expect("real draft feedback after scrolling");
    let viewport =
        cx.update(|window, _| gpui::Bounds::new(gpui::Point::default(), window.viewport_size()));
    assert!(
        feedback.contains(&button.center()),
        "Check Sent centre is inside the feedback bounds"
    );
    assert!(
        viewport.contains(&button.center()),
        "Check Sent centre is inside the measured viewport"
    );
    button
}

fn device_record(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    store: &MailDeviceStore,
    fixture: &Fixture,
    id: &str,
) -> DeviceDraftRecord {
    workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.load(scope(&fixture.base), id.into()))
            .unwrap()
            .unwrap()
    })
}

#[gpui::test]
async fn native_check_sent_pending_restore_keeps_newer_edits_and_bytes_on_device(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let fixture = Fixture::new(
        &workspace,
        cx,
        ServiceProvider::Gmail,
        MailDispatchState::Uncertain,
        false,
        Hold::None,
    );
    let (view, store, id, _directory) = install(&workspace, cx, &fixture);
    let initial = device_record(&workspace, cx, &store, &fixture, &id);
    let mut value = initial.value;
    value.dirty = true;
    value.content.body.push_str("\nNewer device-only text");
    workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.save(value, Some(initial.revision), vec![]))
            .unwrap()
    });
    let before = device_record(&workspace, cx, &store, &fixture, &id);
    open_saved(&view, cx, id.clone()).await;
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.method == "GET"),
        "pending status recovery must not resume server saves, uploads or deletes"
    );
    let after = device_record(&workspace, cx, &store, &fixture, &id);
    assert_eq!(
        serde_json::to_value(&after.value).unwrap(),
        serde_json::to_value(&before.value).unwrap()
    );
    assert!(after.value.dirty);
    assert!(after.value.pending_send.is_some());
    let bytes = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.file(scope(&fixture.base), id, "saved-file".into()))
            .unwrap()
    });
    assert_eq!(bytes, vec![0, 255]);
}

#[gpui::test]
async fn native_check_sent_compact_close_preserves_the_accepted_newer_draft(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let fixture = Fixture::new(
        &workspace,
        cx,
        ServiceProvider::Outlook,
        MailDispatchState::Accepted,
        false,
        Hold::None,
    );
    let (view, store, id, _directory) = install(&workspace, cx, &fixture);
    let initial = device_record(&workspace, cx, &store, &fixture, &id);
    let mut value = initial.value;
    value.dirty = true;
    workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.save(value, Some(initial.revision), vec![]))
            .unwrap()
    });
    open_saved(&view, cx, id.clone()).await;
    let button = check_sent_button(cx);
    cx.simulate_click(button.center(), Modifiers::default());
    cx.condition(&view, |view, _| {
        view.active().is_some_and(|job| {
            !job.running
                && job.phase == Phase::Ready
                && job.value.pending_send.is_none()
                && job
                    .value
                    .latest_send
                    .as_ref()
                    .is_some_and(|status| status.state == MailDispatchState::Accepted)
        })
    })
    .await;
    let accepted = device_record(&workspace, cx, &store, &fixture, &id);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    cx.run_until_parked();
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    let close = cx
        .debug_bounds("desktop-compose-close")
        .expect("visible compact Close editor");
    let viewport =
        cx.update(|window, _| gpui::Bounds::new(gpui::Point::default(), window.viewport_size()));
    assert!(viewport.contains(&close.center()));
    cx.simulate_click(close.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.active.is_none()));
    assert!(cx.debug_bounds("desktop-compose-close").is_none());
    let after = device_record(&workspace, cx, &store, &fixture, &id);
    assert_eq!(
        serde_json::to_value(&after.value).unwrap(),
        serde_json::to_value(&accepted.value).unwrap()
    );
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method != "GET")
            .count(),
        1,
        "closing must not save, upload or dispatch the newer draft"
    );
}

#[gpui::test]
async fn native_check_sent_is_explicit_once_and_preserves_newer_text_files_and_revision(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    for provider in [ServiceProvider::Gmail, ServiceProvider::Outlook] {
        let fixture = Fixture::new(
            &workspace,
            cx,
            provider,
            MailDispatchState::Accepted,
            false,
            Hold::None,
        );
        let (view, store, id, _directory) = install(&workspace, cx, &fixture);
        open_saved(&view, cx, id.clone()).await;
        let before = device_record(&workspace, cx, &store, &fixture, &id);
        assert!(
            fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.method == "GET"),
            "restoration must not trigger reconciliation or send"
        );
        let start = fixture.requests.lock().unwrap().len();
        let button = check_sent_button(cx);
        cx.simulate_click(button.center(), Modifiers::default());
        cx.simulate_click(button.center(), Modifiers::default());
        cx.condition(&view, |view, _| {
            view.active().is_some_and(|job| {
                job.value
                    .latest_send
                    .as_ref()
                    .is_some_and(|s| s.state == MailDispatchState::Accepted)
                    && job.phase == Phase::Ready
                    && !job.running
                    && job.saved == job.sequence
            })
        })
        .await;
        let after = device_record(&workspace, cx, &store, &fixture, &id);
        assert_eq!(after.value.content, before.value.content);
        assert_eq!(after.value.server_revision, 9);
        assert!(!after.value.dirty);
        assert_eq!(
            serde_json::to_value(&after.value.files).unwrap(),
            serde_json::to_value(&before.value.files).unwrap()
        );
        assert!(after.value.pending_send.is_none());
        let bytes = workspace.read_with(cx, |_, cx| {
            cx.global::<DesktopState>()
                .runtime
                .block_on(store.file(scope(&fixture.base), id, "saved-file".into()))
                .unwrap()
        });
        assert_eq!(bytes, vec![0, 255]);
        let requests = fixture.requests.lock().unwrap()[start..].to_vec();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, "/api/mail");
        assert!(requests[1].path.starts_with("/api/connections/"));
        assert_eq!(requests[2].method, "POST");
        assert!(requests[2].path.ends_with("/reconcile"));
        assert_eq!(
            requests[2].body,
            json!({"host_id":"host-a","connection_id":"renewed-generation"})
        );
        view.read_with(cx, |view, _| {
            assert!(super::super::render::status_text(view.active().unwrap())
                .contains("Newer edits remain a draft"))
        });
    }
}

#[gpui::test]
async fn native_check_sent_unconfirmed_keeps_the_exact_device_copy_and_original_problem(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let fixture = Fixture::new(
        &workspace,
        cx,
        ServiceProvider::Gmail,
        MailDispatchState::Uncertain,
        false,
        Hold::None,
    );
    let (view, store, id, _directory) = install(&workspace, cx, &fixture);
    open_saved(&view, cx, id.clone()).await;
    let before = device_record(&workspace, cx, &store, &fixture, &id);
    let button = check_sent_button(cx);
    cx.simulate_click(button.center(), Modifiers::default());
    cx.condition(&view, |view, _| {
        view.active()
            .is_some_and(|job| job.phase == Phase::SentUnconfirmed && !job.running)
    })
    .await;
    let after = device_record(&workspace, cx, &store, &fixture, &id);
    assert_eq!(after.revision, before.revision);
    assert_eq!(
        serde_json::to_value(&after.value).unwrap(),
        serde_json::to_value(&before.value).unwrap()
    );
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
}

#[gpui::test]
async fn native_check_sent_auth_failure_retains_uncertainty_without_a_rejection_or_resend(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let fixture = Fixture::new(
        &workspace,
        cx,
        ServiceProvider::Gmail,
        MailDispatchState::Accepted,
        true,
        Hold::None,
    );
    let (view, store, id, _directory) = install(&workspace, cx, &fixture);
    open_saved(&view, cx, id.clone()).await;
    let before = device_record(&workspace, cx, &store, &fixture, &id);
    let button = check_sent_button(cx);
    cx.simulate_click(button.center(), Modifiers::default());
    cx.condition(&view, |view, _| {
        view.active()
            .is_some_and(|job| job.phase == Phase::SentError && !job.running)
    })
    .await;
    let after = device_record(&workspace, cx, &store, &fixture, &id);
    assert_eq!(after.revision, before.revision);
    assert_eq!(
        serde_json::to_value(&after.value).unwrap(),
        serde_json::to_value(&before.value).unwrap()
    );
    view.read_with(cx, |view, _| {
        assert!(!view
            .active()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("private provider diagnostics"))
    });
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
}

#[gpui::test]
async fn native_check_sent_context_change_before_post_never_starts_the_lookup(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let fixture = Fixture::new(
        &workspace,
        cx,
        ServiceProvider::Gmail,
        MailDispatchState::Accepted,
        false,
        Hold::Connection,
    );
    let (view, store, id, _directory) = install(&workspace, cx, &fixture);
    open_saved(&view, cx, id.clone()).await;
    let before = device_record(&workspace, cx, &store, &fixture, &id);
    let arrived = Arc::clone(&fixture.entered);
    let wait = workspace.read_with(cx, |_, cx| {
        cx.background_spawn(async move {
            arrived.notified().await;
        })
    });
    let button = check_sent_button(cx);
    cx.simulate_click(button.center(), Modifiers::default());
    wait.await;
    let k = key(&scope(&fixture.base), &id);
    view.update_in(cx, |view, window, cx| {
        view.action(Action::Close, window, cx)
    });
    fixture.release.notify_one();
    cx.condition(&view, |view, _| {
        view.jobs
            .get(&k)
            .is_some_and(|job| job.phase == Phase::SentError && !job.running)
    })
    .await;
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"),
        "changed context must fence the POST after preflight"
    );
    let after = device_record(&workspace, cx, &store, &fixture, &id);
    assert_eq!(after.revision, before.revision);
    assert_eq!(
        serde_json::to_value(after.value).unwrap(),
        serde_json::to_value(before.value).unwrap()
    );
}

#[gpui::test]
async fn native_check_sent_late_reply_never_overwrites_another_selected_draft(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let fixture = Fixture::new(
        &workspace,
        cx,
        ServiceProvider::Gmail,
        MailDispatchState::Accepted,
        false,
        Hold::Reconcile,
    );
    let (view, store, id, _directory) = install(&workspace, cx, &fixture);
    open_saved(&view, cx, id.clone()).await;
    let before = device_record(&workspace, cx, &store, &fixture, &id);
    let arrived = Arc::clone(&fixture.entered);
    let wait = workspace.read_with(cx, |_, cx| {
        cx.background_spawn(async move {
            arrived.notified().await;
        })
    });
    let button = check_sent_button(cx);
    cx.simulate_click(button.center(), Modifiers::default());
    wait.await;
    let mut other = DeviceDraft::new(
        scope(&fixture.base),
        ServiceProvider::Gmail,
        None,
        MailComposeContent {
            body: "Another selected draft".into(),
            ..Default::default()
        },
    );
    other.server_revision = 1;
    other.dirty = false;
    let other_id = other.client_id.clone();
    workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.save(other, None, vec![]))
            .unwrap()
    });
    view.update_in(cx, |view, window, cx| {
        view.open(other_id.clone(), window, cx)
    });
    fixture.release.notify_one();
    let k = key(&scope(&fixture.base), &id);
    cx.condition(&view, |view, _| {
        view.jobs
            .get(&k)
            .is_some_and(|job| job.phase == Phase::SentError)
            && view
                .active()
                .is_some_and(|job| job.value.client_id == other_id && !job.running)
    })
    .await;
    view.read_with(cx, |view, _| {
        let value = &view.active().unwrap().value;
        assert_eq!(value.content.body, "Another selected draft");
        assert!(value.latest_send.is_none());
        assert!(value.pending_send.is_none());
    });
    let after = device_record(&workspace, cx, &store, &fixture, &id);
    assert_eq!(after.revision, before.revision);
    assert_eq!(
        serde_json::to_value(after.value).unwrap(),
        serde_json::to_value(before.value).unwrap()
    );
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
}

#[gpui::test]
async fn native_check_sent_cached_draft_switch_before_post_cannot_reuse_a_generation(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let fixture = Fixture::new(
        &workspace,
        cx,
        ServiceProvider::Gmail,
        MailDispatchState::Accepted,
        false,
        Hold::Connection,
    );
    let (view, store, id, _directory) = install(&workspace, cx, &fixture);
    open_saved(&view, cx, id.clone()).await;
    let before = device_record(&workspace, cx, &store, &fixture, &id);
    let mut other = DeviceDraft::new(
        scope(&fixture.base),
        ServiceProvider::Gmail,
        None,
        MailComposeContent {
            body: "Cached other draft".into(),
            ..Default::default()
        },
    );
    other.server_revision = 1;
    other.dirty = false;
    let other_id = other.client_id.clone();
    workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.save(other, None, vec![]))
            .unwrap()
    });
    view.update_in(cx, |view, window, cx| {
        view.open(other_id.clone(), window, cx)
    });
    cx.condition(&view, |view, _| {
        view.active().is_some_and(|job| {
            job.value.client_id == other_id
                && job.phase == Phase::Ready
                && !job.running
                && job.sequence == job.saved
        })
    })
    .await;
    // Both jobs are now cached. This exercises the existing fast path without
    // changing that path or relying on a generation increment it does not make.
    view.update_in(cx, |view, window, cx| view.open(id.clone(), window, cx));
    let generation = view.read_with(cx, |view, _| view.generation);
    let arrived = Arc::clone(&fixture.entered);
    let wait = workspace.read_with(cx, |_, cx| {
        cx.background_spawn(async move {
            arrived.notified().await;
        })
    });
    let button = check_sent_button(cx);
    cx.simulate_click(button.center(), Modifiers::default());
    wait.await;
    view.update_in(cx, |view, window, cx| {
        view.open(other_id.clone(), window, cx)
    });
    assert_eq!(view.read_with(cx, |view, _| view.generation), generation);
    fixture.release.notify_one();
    let k = key(&scope(&fixture.base), &id);
    cx.condition(&view, |view, _| {
        view.jobs
            .get(&k)
            .is_some_and(|job| job.phase == Phase::SentError)
    })
    .await;
    view.read_with(cx, |view, _| {
        assert_eq!(view.active().unwrap().value.client_id, other_id);
        assert_eq!(
            view.active().unwrap().value.content.body,
            "Cached other draft"
        );
        assert!(view.active().unwrap().value.latest_send.is_none());
    });
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"),
        "cached navigation must fence the explicit POST"
    );
    let after = device_record(&workspace, cx, &store, &fixture, &id);
    assert_eq!(after.revision, before.revision);
    assert_eq!(
        serde_json::to_value(after.value).unwrap(),
        serde_json::to_value(before.value).unwrap()
    );
}
