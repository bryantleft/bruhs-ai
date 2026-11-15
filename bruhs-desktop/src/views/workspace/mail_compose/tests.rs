use super::super::Workspace;
use super::{key, Action, Job, MailComposer, Phase};
use crate::{app_state::DesktopState, server_client::ServerClient};
use bruhs_core::mail::device::{DeviceDraft, DeviceDraftFile, DeviceDraftScope, MailDeviceStore};
use bruhs_models::{connections::ServiceProvider, mail::*, AppView};
use gpui::{px, size, Modifiers, TestAppContext};
use gpui::{Entity, VisualTestContext};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn scope(origin: &str) -> DeviceDraftScope {
    DeviceDraftScope {
        origin: origin.into(),
        host_id: "host-a".into(),
        mailbox_id: "mailbox-a".into(),
    }
}
fn setup(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    super::super::performance_tests::fixture(cx)
}
fn attach(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    store: MailDeviceStore,
    client: ServerClient,
) -> Entity<MailComposer> {
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.active_view = AppView::Inbox;
        workspace.inbox_mail_visible = true;
        workspace.inbox_import_visible = false;
        let view = workspace.connected_inbox.update(cx, |mail, _| {
            mail.compose_test_setup(
                client.clone(),
                MailboxesSnapshot {
                    host_id: "host-a".into(),
                    mailboxes: vec![MailboxSummary {
                        mailbox_id: "mailbox-a".into(),
                        provider: ServiceProvider::Gmail,
                        last_connection_id: "cached-old-generation".into(),
                        account_label: "you@example.test".into(),
                        revision: 1,
                        threads: 0,
                        drafts: 0,
                    }],
                },
            )
        });
        view.update(cx, |view, cx| {
            view.client = client;
            view.store = Some(store);
            cx.notify();
        });
        cx.notify();
        view
    })
}

#[gpui::test]
async fn native_compose_keeps_latest_device_text_off_route_after_server_failure(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    let directory = tempfile::tempdir().unwrap();
    let store = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(MailDeviceStore::open(
                directory.path().join("drafts.sqlite3"),
            ))
            .unwrap()
    });
    let client = ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap();
    let view = attach(&workspace, cx, store.clone(), client);
    cx.run_until_parked();
    let entry = cx
        .debug_bounds("desktop-compose-entry-New email")
        .expect("real native New email entry");
    cx.simulate_click(entry.center(), Modifiers::default());
    cx.condition(&view, |view, _| {
        view.active()
            .is_some_and(|job| job.phase == Phase::Error && !job.running)
    })
    .await;
    let client_id = view.read_with(cx, |view, _| view.active().unwrap().value.client_id.clone());
    view.update_in(cx, |view, window, cx| {
        view.inputs[0].update(cx, |input, cx| input.focus(window, cx))
    });
    cx.simulate_input("Unfinished <");
    view.update_in(cx, |view, window, cx| {
        view.inputs[4].update(cx, |input, cx| input.focus(window, cx))
    });
    cx.simulate_input("Latest private body 東京");

    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Today;
        cx.notify();
    });
    cx.condition(&view, |view, _| {
        view.active().is_some_and(|job| {
            job.saved == job.sequence
                && job.phase == Phase::Error
                && !job.running
                && job.value.content.body == "Latest private body 東京"
        })
    })
    .await;
    let record = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.load(scope("http://127.0.0.1:9"), client_id.clone()))
            .unwrap()
            .unwrap()
    });
    assert_eq!(record.value.content.to, "Unfinished <");
    assert_eq!(record.value.content.body, "Latest private body 東京");
    assert!(!view.read_with(cx, |view, _| view.pending()));
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Inbox;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-compose-review").is_some(),
        "compact footer remains rendered"
    );
    let selected = directory.path().join("résumé-東京.bin");
    std::fs::write(&selected, [0u8, 42, 255]).unwrap();
    let attach_button = cx
        .debug_bounds("desktop-compose-attach")
        .expect("real native attachment control");
    cx.simulate_click(attach_button.center(), Modifiers::default());
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.multiple && !options.directories);
        Some(vec![selected])
    });
    cx.condition(&view, |view, _| {
        view.active().is_some_and(|job| {
            job.value.files.len() == 1 && job.saved == job.sequence && !job.running
        })
    })
    .await;
    let file_id = view.read_with(cx, |view, _| {
        view.active().unwrap().value.files[0].id.clone()
    });
    let bytes = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.file(scope("http://127.0.0.1:9"), client_id.clone(), file_id))
            .unwrap()
    });
    assert_eq!(
        bytes,
        vec![0, 42, 255],
        "native chooser bytes persist before an unavailable server"
    );
    view.update_in(cx, |view, window, cx| {
        view.action(Action::Close, window, cx);
        view.show_drafts(
            scope("http://127.0.0.1:9"),
            ServiceProvider::Gmail,
            window,
            cx,
        );
    });
    cx.condition(&view, |view, _| {
        !view.loading && !view.local_rows.is_empty()
    })
    .await;
    view.update_in(cx, |view, window, cx| {
        view.open(client_id.clone(), window, cx)
    });
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.inputs[4].read(cx).value().as_str(),
            "Latest private body 東京"
        )
    });
}

#[gpui::test]
async fn restored_pending_send_only_reads_status_and_never_posts_again(cx: &mut TestAppContext) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let directory = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let (base,worker)=workspace.read_with(cx,|_,cx|{
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();let base=format!("http://{}",listener.local_addr().unwrap());let seen=Arc::clone(&requests);
        let worker=cx.global::<DesktopState>().runtime.spawn(async move{
            let listener=tokio::net::TcpListener::from_std(listener).unwrap();
            while let Ok((mut stream,_))=listener.accept().await{let seen=seen.clone();tokio::spawn(async move{
                let mut data=vec![0;8192];let n=stream.read(&mut data).await.unwrap();let line=String::from_utf8_lossy(&data[..n]).lines().next().unwrap().to_owned();seen.lock().unwrap().push(line.clone());
                let (status,body)=if line.contains("/sends/restored-operation?"){
                    ("200 OK",serde_json::json!({"host_id":"host-a","mailbox_id":"mailbox-a","operation_id":"restored-operation","draft_id":"draft-a","draft_revision":4,"state":"accepted","provider_message_id":"accepted-once","provider_thread_id":null,"problem":null}))
                }else if line.contains("/drafts?"){
                    ("200 OK",serde_json::json!({"host_id":"host-a","mailbox_id":"mailbox-a","revision":1,"page":0,"total_matches":0,"rows":[]}))
                }else{("404 Not Found",serde_json::json!({"error":"unknown synthetic request"}))};
                let body=body.to_string();let response=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());let _=stream.write_all(response.as_bytes()).await;
            });}
        });(base,worker)
    });
    let store = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(MailDeviceStore::open(
                directory.path().join("drafts.sqlite3"),
            ))
            .unwrap()
    });
    let mut draft = DeviceDraft::new(
        scope(&base),
        ServiceProvider::Gmail,
        None,
        MailComposeContent {
            to: "recipient@example.test".into(),
            body: "saved exact body".into(),
            ..Default::default()
        },
    );
    draft.server_revision = 4;
    draft.dirty = false;
    draft.pending_send = Some(MailSendRequest {
        host_id: "host-a".into(),
        connection_id: "old-generation".into(),
        operation_id: "restored-operation".into(),
        revision: 4,
        digest: "frozen-digest".into(),
    });
    let id = draft.client_id.clone();
    workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(store.save(draft, None, vec![]))
            .unwrap()
    });
    let view = attach(
        &workspace,
        cx,
        store.clone(),
        ServerClient::new(&base, &base).unwrap(),
    );
    view.update_in(cx, |view, window, cx| {
        view.show_drafts(scope(&base), ServiceProvider::Gmail, window, cx);
    });
    cx.condition(&view, |view, _| !view.loading).await;
    view.update_in(cx, |view, window, cx| view.open(id, window, cx));
    cx.condition(&view, |view, _| {
        view.active()
            .is_some_and(|job| job.value.consumed() && !job.running)
    })
    .await;
    view.read_with(cx, |view, _| {
        assert!(view.active().unwrap().value.pending_send.is_none());
        assert!(view.active().unwrap().locked());
    });
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.starts_with("GET ") && line.contains("/sends/restored-operation?")));
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .all(|line| !line.starts_with("POST ")),
        "restore must never replay Send"
    );
    worker.abort();
}

#[gpui::test]
async fn graceful_shutdown_flushes_latest_text_and_selected_bytes_without_server_io(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(MailDeviceStore::open(
                directory.path().join("drafts.sqlite3"),
            ))
            .unwrap()
    });
    let view = attach(
        &workspace,
        cx,
        store.clone(),
        ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
    );
    let mut value = DeviceDraft::new(
        scope("http://127.0.0.1:9"),
        ServiceProvider::Gmail,
        None,
        MailComposeContent {
            body: "last keystroke".into(),
            ..Default::default()
        },
    );
    let id = value.client_id.clone();
    value.files.push(DeviceDraftFile {
        id: "selected-file".into(),
        file_name: "private.bin".into(),
        size_bytes: 3,
        server: None,
    });
    let task = view.update(cx, |view, cx| {
        view.jobs.insert(
            key(&value.scope, &value.client_id),
            Job {
                value,
                revision: None,
                sequence: 1,
                saved: 0,
                running: false,
                selecting_files: false,
                phase: Phase::Ready,
                added: [("selected-file".into(), vec![0, 42, 255])].into(),
                review: None,
                error: None,
            },
        );
        view.flush_quit(cx)
    });
    task.await;
    workspace.read_with(cx, |_, cx| {
        let runtime = &cx.global::<DesktopState>().runtime;
        assert_eq!(
            runtime
                .block_on(store.load(scope("http://127.0.0.1:9"), id.clone()))
                .unwrap()
                .unwrap()
                .value
                .content
                .body,
            "last keystroke"
        );
        assert_eq!(
            runtime
                .block_on(store.file(scope("http://127.0.0.1:9"), id, "selected-file".into()))
                .unwrap(),
            vec![0, 42, 255]
        );
    });
}

#[gpui::test]
async fn pending_file_selection_blocks_review_and_survives_closing_a_clean_draft(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(MailDeviceStore::open(
                directory.path().join("drafts.sqlite3"),
            ))
            .unwrap()
    });
    let view = attach(
        &workspace,
        cx,
        store,
        ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
    );
    let mut value = DeviceDraft::new(
        scope("http://127.0.0.1:9"),
        ServiceProvider::Gmail,
        None,
        MailComposeContent {
            to: "recipient@example.test".into(),
            ..Default::default()
        },
    );
    value.server_revision = 1;
    value.dirty = false;
    let k = key(&value.scope, &value.client_id);
    view.update_in(cx, |view, window, cx| {
        view.jobs.insert(
            k.clone(),
            Job {
                value,
                revision: Some(1),
                sequence: 1,
                saved: 1,
                running: false,
                selecting_files: false,
                phase: Phase::Ready,
                added: Default::default(),
                review: None,
                error: None,
            },
        );
        view.active = Some(k.clone());
        view.action(Action::Attach, window, cx);
        assert!(view.active().unwrap().selecting_files);
        assert!(view.active().unwrap().locked());
        view.action(Action::Review, window, cx);
        assert!(view.active().unwrap().phase == Phase::Ready);
        view.action(Action::Close, window, cx);
        assert!(view.jobs.contains_key(&k));
        assert!(view.pending());
    });
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.condition(&view, |view, _| {
        view.jobs.get(&k).is_some_and(|job| !job.selecting_files)
    })
    .await;
    assert!(!view.read_with(cx, |view, _| view.pending()));
}

#[test]
fn saved_status_requires_an_acknowledged_device_sequence() {
    let job = Job {
        value: DeviceDraft::new(
            scope("http://127.0.0.1:9"),
            ServiceProvider::Gmail,
            None,
            Default::default(),
        ),
        revision: None,
        sequence: 0,
        saved: u64::MAX,
        running: false,
        selecting_files: false,
        phase: Phase::Ready,
        added: Default::default(),
        review: None,
        error: None,
    };
    assert_eq!(
        super::render::status_text(&job),
        "Waiting to save on this device…"
    );
}

struct SaveObservation {
    requests: Vec<String>,
    remote: MailComposeDraft,
}
struct SaveFixture {
    client: ServerClient,
    attempt: DeviceDraft,
    worker: tokio::task::JoinHandle<SaveObservation>,
}
async fn save_fixture(conflict: bool) -> SaveFixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut attempt = DeviceDraft::new(
        scope(&base),
        ServiceProvider::Gmail,
        None,
        MailComposeContent {
            to: "recipient@example.test".into(),
            body: "attempt B".into(),
            ..Default::default()
        },
    );
    attempt.server_revision = 1;
    let initial = MailComposeDraft {
        host_id: attempt.scope.host_id.clone(),
        mailbox_id: attempt.scope.mailbox_id.clone(),
        client_id: attempt.client_id.clone(),
        draft_id: "canonical-draft".into(),
        revision: 1,
        reply: None,
        content: Default::default(),
        uploads: Vec::new(),
        latest_send: None,
    };
    let worker = tokio::spawn(async move {
        let mut remote = initial;
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut buffer = [0; 4096];
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0 && bytes.len() < 65536);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(index) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let line = headers.lines().next().unwrap().to_owned();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < end + length {
                let mut buffer = [0; 4096];
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            requests.push(line.clone());
            if line.starts_with("PUT ") {
                let request: MailDraftSave =
                    serde_json::from_slice(&bytes[end..end + length]).unwrap();
                assert_eq!(request.expected_revision, 1);
                remote.revision = 2;
                if conflict {
                    remote.content.body = "external edit D".into();
                    let body = "{\"error\":\"synthetic revision conflict\"}";
                    let response=format!("HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                    stream.write_all(response.as_bytes()).await.unwrap();
                } else {
                    remote.content = request.content;
                } // Commit B, then lose its response.
            } else {
                assert!(line.starts_with("GET "));
                let body = serde_json::to_string(&remote).unwrap();
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        }
        SaveObservation { requests, remote }
    });
    SaveFixture {
        client: ServerClient::new(&base, &base).unwrap(),
        attempt,
        worker,
    }
}
#[tokio::test]
async fn lost_save_reply_recovers_only_the_exact_attempted_content() {
    let fixture = save_fixture(false).await;
    let saved = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        super::work::save_attempt(&fixture.client, &fixture.attempt),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.content, fixture.attempt.content);
    let observation = fixture.worker.await.unwrap();
    assert_eq!(observation.requests.len(), 2);
    assert!(observation.requests[0].starts_with("PUT "));
    assert!(observation.requests[1].starts_with("GET "));
}
#[tokio::test]
async fn recovery_does_not_rebase_or_overwrite_an_external_edit() {
    let fixture = save_fixture(true).await;
    assert!(tokio::time::timeout(
        std::time::Duration::from_secs(3),
        super::work::save_attempt(&fixture.client, &fixture.attempt)
    )
    .await
    .unwrap()
    .is_err());
    let observation = fixture.worker.await.unwrap();
    assert_eq!(observation.remote.content.body, "external edit D");
    assert_eq!(
        observation
            .requests
            .iter()
            .filter(|line| line.starts_with("PUT "))
            .count(),
        1
    );
}

async fn physical_ready_composer(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> (Entity<MailComposer>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let store = workspace.read_with(cx, |_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .block_on(MailDeviceStore::open(
                directory.path().join("drafts.sqlite3"),
            ))
            .unwrap()
    });
    let view = attach(
        workspace,
        cx,
        store,
        ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
    );
    cx.run_until_parked();
    let entry = cx.debug_bounds("desktop-compose-entry-New email").unwrap();
    cx.simulate_click(entry.center(), Modifiers::default());
    cx.condition(&view, |view, _| {
        view.active()
            .is_some_and(|job| !job.running && job.phase == Phase::Error)
    })
    .await;
    view.update_in(cx, |view, window, cx| {
        let k = view.active.clone().unwrap();
        let job = view.jobs.get_mut(&k).unwrap();
        job.value.content.to = "recipient@example.test".into();
        job.value.content.body = "First line\nSecond line\nThird line".into();
        job.value.server_revision = 1;
        job.value.dirty = false;
        job.phase = Phase::Ready;
        job.error = None;
        job.saved = job.sequence;
        view.hydrate(window, cx);
        cx.notify();
    });
    cx.run_until_parked();
    (view, directory)
}

#[gpui::test]
async fn physical_composer_message_editor_keeps_multiple_lines_visible(cx: &mut TestAppContext) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let (_view, _directory) = physical_ready_composer(&workspace, cx).await;
    let body = cx.debug_bounds("desktop-compose-field-4").unwrap();
    assert!(
        body.size.height >= px(160.0),
        "A multiline email must not collapse to a single-line input: {body:?}"
    );
}

#[gpui::test]
async fn physical_composer_command_enter_reviews_from_a_focused_attach_button(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = setup(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let (view, _directory) = physical_ready_composer(&workspace, cx).await;
    let attach = cx.debug_bounds("desktop-compose-attach").unwrap();
    cx.simulate_click(attach.center(), Modifiers::default());
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.condition(&view, |view, _| {
        view.active().is_some_and(|job| !job.selecting_files)
    })
    .await;
    assert!(!cx.did_prompt_for_paths());
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert!(
        !cx.did_prompt_for_paths(),
        "Command-Enter must review, even when Attach files holds focus"
    );
    assert!(view.read_with(cx, |view, _| view
        .active()
        .is_some_and(|job| matches!(job.phase, Phase::Review | Phase::Error))));
}
