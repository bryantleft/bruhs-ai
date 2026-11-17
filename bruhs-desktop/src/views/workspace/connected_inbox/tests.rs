//! UI fixtures use real HTTP through ServerClient. The server integration suite
//! separately exercises that client against the production router and mail store.
use super::{element_key, Action};
use crate::{app_state::DesktopState, server_client::ServerClient};
use bruhs_models::{
    connections::{ConnectionState, ServiceConnection, ServiceProvider},
    mail::*,
    AppView, InboxConversation, InboxMessage, InboxSource,
};
use gpui::{size, Modifiers, TestAppContext};
use gpui_component::zoom::px;
use serde_json::json;
use std::sync::{
    atomic::{AtomicU8, Ordering as AtomicOrdering},
    Arc, Mutex,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    provider: ServiceProvider,
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    sync_mode: Arc<AtomicU8>,
    release_page: Arc<tokio::sync::Notify>,
    page_started: Arc<tokio::sync::Notify>,
    worker: tokio::task::JoinHandle<()>,
}

fn thread_for(provider: ServiceProvider) -> ConnectedMailThread {
    let mut conversation =
        InboxConversation::new(InboxSource::Email, "Native mail fixture", vec![], 1);
    conversation.id = "canonical-thread".into();
    conversation.receive(InboxMessage::incoming("Zoë", "Older saved body", 2));
    conversation.receive(InboxMessage::incoming(
        "Zoë",
        "Newest body <script>not HTML</script>",
        3,
    ));
    let messages = conversation
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| MailMessageDetail {
            id: message.id.clone(),
            external_id: format!("message-{index}"),
            label_ids: vec!["INBOX".into()],
            envelope: MailEnvelope {
                from: vec![MailAddress {
                    name: "Zoë".into(),
                    address: "zoe@example.test".into(),
                }],
                to: vec![MailAddress {
                    name: String::new(),
                    address: "you@example.test".into(),
                }],
                ..Default::default()
            },
            attachments: vec![MailAttachment {
                id: format!("attachment-{index}"),
                part_id: "1".into(),
                external_id: Some("provider-part".into()),
                file_name: "../../設計.bin".into(),
                content_type: "application/octet-stream".into(),
                size_bytes: u64::try_from(b"Native attachment bytes".len()).unwrap(),
                content_id: None,
                inline: false,
            }],
            body_from_html: false,
        })
        .collect();
    ConnectedMailThread {
        conversation,
        connected_mail: ConnectedMailMetadata {
            provider,
            mailbox_id: "mailbox-a".into(),
            external_thread_id: "thread-a".into(),
            messages,
        },
    }
}

impl Server {
    fn new(runtime: &tokio::runtime::Runtime) -> Self {
        Self::for_provider(runtime, ServiceProvider::Gmail)
    }

    fn for_provider(runtime: &tokio::runtime::Runtime, provider: ServiceProvider) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let sync_mode = Arc::new(AtomicU8::new(0));
        let release_page = Arc::new(tokio::sync::Notify::new());
        let page_started = Arc::new(tokio::sync::Notify::new());
        let started = page_started.clone();
        let (captured, mode, release) = (requests.clone(), sync_mode.clone(), release_page.clone());
        let worker = runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            while let Ok((mut stream, _)) = listener.accept().await {
                let (captured, mode, release, started) = (captured.clone(), mode.clone(), release.clone(), started.clone());
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let header_end = loop {
                        let mut buffer = [0; 4096];
                        let size = stream.read(&mut buffer).await.unwrap();
                        assert!(size > 0 && bytes.len() < 65536, "bounded fixture request");
                        bytes.extend_from_slice(&buffer[..size]);
                        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") { break end + 4; }
                    };
                    let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
                    let line = headers.lines().next().unwrap().to_string();
                    let length: usize = headers.lines().find_map(|line| line.to_lowercase().strip_prefix("content-length:").map(str::to_owned)).unwrap_or_else(|| "0".into()).trim().parse().unwrap();
                    while bytes.len() < header_end + length {
                        let mut buffer = [0; 4096];
                        let size = stream.read(&mut buffer).await.unwrap();
                        assert!(size > 0);
                        bytes.extend_from_slice(&buffer[..size]);
                    }
                    captured.lock().unwrap().push(line.clone());
                    let path = line.split_whitespace().nth(1).unwrap();
                    let prefix=if provider==ServiceProvider::Outlook{"/api/mail/outlook"}else{"/api/mail"};
                    if path.starts_with(&format!("{prefix}/mailboxes/mailbox-a/threads/thread-a/attachments/")) {
                        let url = reqwest::Url::parse(&format!("http://fixture{path}")).unwrap();
                        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
                        assert_eq!(query["connection_id"], "current-generation");
                        assert_eq!(query["host_id"], "host-a");
                        let body = b"Native attachment bytes";
                        let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        stream.write_all(header.as_bytes()).await.unwrap();
                        stream.write_all(body).await.unwrap();
                        return;
                    }
                    let (status, body) = if path == "/api/mail" {
                        ("200 OK", serde_json::to_value(MailboxesSnapshot { host_id: "host-a".into(), mailboxes: vec![MailboxSummary { mailbox_id: "mailbox-a".into(), provider, last_connection_id: "old-generation".into(), account_label: "you@example.test".into(), revision: 1, threads: 1, drafts: 0 }] }).unwrap())
                    } else if path == format!("/api/connections/{}",provider.id()) {
                        ("200 OK", serde_json::to_value(ServiceConnection { id: Some("current-generation".into()), provider, configured: true, account: Some("you@example.test".into()), scopes: vec![], state: ConnectionState::Connected }).unwrap())
                    } else if path == format!("/api/mail/{}/sync",provider.id()) {
                        let request: GmailSyncRequest = serde_json::from_slice(&bytes[header_end..header_end+length]).unwrap();
                        assert_eq!(request.connection_id, "current-generation", "Never reuse cached authorization metadata");
                        assert_eq!(request.host_id, "host-a");
                        let sync_mode = mode.load(AtomicOrdering::Acquire);
                        started.notify_one();
                        if sync_mode == 1 { release.notified().await; }
                        if sync_mode == 2 { ("429 Too Many Requests", json!({"error":"synthetic private provider diagnostic"})) }
                        else { ("200 OK", serde_json::to_value(MailSyncResult { host_id: "host-a".into(), mailbox_id: "mailbox-a".into(), revision: 2, threads_updated: 1, full_scan_complete: true, caught_up: sync_mode != 1 }).unwrap()) }
                    } else if path.starts_with("/api/mail/mailboxes/mailbox-a/threads/thread-a?") {
                        ("200 OK", serde_json::to_value(thread_for(provider)).unwrap())
                    } else if path.starts_with("/api/mail/mailboxes/mailbox-a/threads?") {
                        let record=thread_for(provider);
                        let mut card = bruhs_models::mail::connected_mail_card(&record.conversation, &record.connected_mail);
                        card.id = "canonical-thread".into();
                        ("200 OK", serde_json::to_value(MailThreadsPage { host_id: "host-a".into(), mailbox_id: "mailbox-a".into(), revision: 1, page: 0, total_matches: 1, rows: vec![MailThreadRow { card, external_id: "thread-a".into() }] }).unwrap())
                    } else { ("404 Not Found", json!({"error":"Unknown fixture route"})) };
                    let body = serde_json::to_string(&body).unwrap();
                    let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    stream.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        Self {
            provider,
            base,
            requests,
            sync_mode,
            release_page,
            page_started,
            worker,
        }
    }

    fn sync_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| {
                request.starts_with(&format!("POST /api/mail/{}/sync ", self.provider.id()))
            })
            .count()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[gpui::test]
async fn native_mail_navigation_reads_searches_and_stops_between_committed_pages(
    cx: &mut TestAppContext,
) {
    // HTTP and held provider replies use the real Tokio clock. Do not advance
    // GPUI's timeout past them merely because its fake task queue is parked.
    cx.executor().allow_parking();
    let (workspace, cx) = super::super::performance_tests::fixture(cx);
    cx.simulate_resize(size(px(1600.0), px(1000.0)));
    let server = workspace.update_in(cx, |workspace, window, cx| {
        let server = Server::new(&cx.global::<DesktopState>().runtime);
        let client = ServerClient::new(&server.base, &server.base).unwrap();
        workspace
            .connected_inbox
            .update(cx, |view, _| view.client = client);
        workspace.inbox_search_input.update(cx, |input, cx| {
            input.set_value("keep imported query", window, cx)
        });
        workspace.open_inbox_view(window, cx);
        server
    });
    cx.run_until_parked();
    let email = cx
        .debug_bounds("desktop-inbox-email")
        .expect("real Inbox Email entry");
    cx.simulate_click(email.center(), Modifiers::default());
    let mail = workspace.read_with(cx, |workspace, _| workspace.connected_inbox.clone());
    cx.condition(&mail, |view, _| {
        view.rows.is_some() && !view.accounts_loading
    })
    .await;
    cx.run_until_parked();
    let row = cx
        .debug_bounds(Box::leak(
            format!("desktop-mail-row-{}", element_key("thread-a")).into_boxed_str(),
        ))
        .unwrap();
    cx.simulate_click(row.center(), Modifiers::default());
    cx.condition(&mail, |view, _| view.thread.is_some()).await;
    mail.read_with(cx, |view, _| {
        assert_eq!(view.message, 1);
        assert!(view.thread.as_ref().unwrap().conversation.messages[1]
            .body
            .contains("<script>"));
    });
    cx.run_until_parked();
    let older = cx.debug_bounds("desktop-mail-older-message").unwrap();
    cx.simulate_click(older.center(), Modifiers::default());
    mail.read_with(cx, |view, _| assert_eq!(view.message, 0));
    cx.run_until_parked();
    let save_selector =
        Box::leak(format!("mail-download-{}", element_key("attachment-0")).into_boxed_str());
    let save_button = cx
        .debug_bounds(save_selector)
        .expect("Real native attachment save control");
    cx.simulate_click(save_button.center(), Modifiers::default());
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|directory| {
        // The test platform exposes the directory; the OS-owned suggested-name
        // field is not part of this fixture's return value.
        assert!(directory.ends_with("Downloads"));
        None
    });
    cx.condition(&mail, |view, _| view.downloading.is_none())
        .await;
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains("/attachments/")),
        "Canceling the native destination review reads no provider bytes"
    );
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("selected.bin");
    cx.run_until_parked();
    let save_button = cx.debug_bounds(save_selector).unwrap();
    cx.simulate_click(save_button.center(), Modifiers::default());
    cx.simulate_new_path_selection(|_| Some(destination.clone()));
    cx.condition(&mail, |view, _| {
        view.downloading.is_none()
            && view
                .notice
                .as_ref()
                .is_some_and(|notice| notice.contains("Attachment saved"))
    })
    .await;
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"Native attachment bytes"
    );
    cx.run_until_parked();
    let feedback = cx
        .debug_bounds("desktop-mail-feedback")
        .expect("Download outcome is visible above the reader scroller");
    let reader = cx.debug_bounds("desktop-mail-scroll").unwrap();
    assert!(feedback.bottom() <= reader.top() + px(1.0));
    assert!(
        feedback.size.height <= px(64.0),
        "A short result must not reserve the full 128px feedback limit"
    );
    mail.update(cx, |view, cx| {
        view.notice = Some(format!(
            "{}END_RECOVERY",
            "Long saved result line.\n".repeat(70)
        ));
        cx.notify();
    });
    cx.simulate_resize(size(px(640.0), px(480.0)));
    cx.run_until_parked();
    let feedback = cx.debug_bounds("desktop-mail-feedback").unwrap();
    let reader = cx.debug_bounds("desktop-mail-scroll").unwrap();
    assert!(feedback.size.height <= px(128.0));
    assert!(feedback.bottom() <= reader.top() + px(1.0));
    assert!(reader.size.height >= px(120.0));
    mail.update_in(cx, |view, window, cx| view.feedback_focus.focus(window, cx));
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    mail.read_with(cx, |view, _| {
        let maximum = view.feedback_scroll.max_offset().y;
        assert!(maximum > px(0.0), "Long feedback must remain scrollable");
        assert_eq!(view.feedback_scroll.offset().y, -maximum);
    });
    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    mail.read_with(cx, |view, _| {
        assert_eq!(view.feedback_scroll.offset().y, px(0.0));
    });
    mail.update(cx, |view, cx| {
        view.notice = None;
        cx.notify();
    });
    cx.simulate_resize(size(px(1600.0), px(1000.0)));
    mail.update(cx, |view, cx| view.action(Action::Back, cx));
    mail.update_in(cx, |view, window, cx| {
        view.query.update(cx, |input, cx| {
            input.set_value("CAFÉ 世界 & older", window, cx)
        });
        view.action(Action::Search, cx);
    });
    cx.condition(&mail, |view, _| !view.rows_loading).await;
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains("query=CAF%C3%89+%E4%B8%96%E7%95%8C+%26+older")),
        "Search text is encoded through the actual native HTTP client"
    );
    server.sync_mode.store(1, AtomicOrdering::Release);
    mail.update(cx, |view, cx| view.sync(false, cx));
    server.page_started.notified().await;
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Today;
        cx.notify();
    });
    mail.read_with(cx, |view, _| assert!(view.sync_stop.is_some()));
    mail.update(cx, |view, cx| view.action(Action::Stop, cx));
    server.release_page.notify_one();
    cx.condition(&mail, |view, _| {
        view.sync_stop.is_none() && !view.accounts_loading && !view.rows_loading
    })
    .await;
    assert_eq!(
        server.sync_count(),
        1,
        "Stop prevents the next provider page after the current one is saved"
    );
    server.sync_mode.store(2, AtomicOrdering::Release);
    mail.update(cx, |view, cx| view.sync(false, cx));
    cx.condition(&mail, |view, _| {
        view.sync_stop.is_none()
            && view.sync_error.is_some()
            && !view.accounts_loading
            && !view.rows_loading
    })
    .await;
    mail.read_with(cx, |view, _| {
        assert!(view
            .sync_error
            .as_ref()
            .unwrap()
            .contains("limiting requests"));
        assert!(!view
            .sync_error
            .as_ref()
            .unwrap()
            .contains("private provider"));
        assert!(view.rows.is_some());
    });
    server.sync_mode.store(0, AtomicOrdering::Release);
    mail.update(cx, |view, cx| view.sync(false, cx));
    cx.condition(&mail, |view, _| {
        view.sync_stop.is_none()
            && view.sync_error.is_none()
            && view
                .notice
                .as_ref()
                .is_some_and(|notice| notice.contains("finished"))
    })
    .await;
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_inbox_view(window, cx);
        workspace.inbox_mail_visible = false;
    });
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.inbox_search_input.read(cx).value().as_str(),
            "keep imported query"
        )
    });
    mail.read_with(cx, |view, _| {
        assert_eq!(view.applied_query, "CAFÉ 世界 & older")
    });
}

#[gpui::test]
async fn native_outlook_control_uses_current_authorization_and_stops_off_route(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (workspace, cx) = super::super::performance_tests::fixture(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    let server = workspace.update(cx, |workspace, cx| {
        let server = Server::for_provider(
            &cx.global::<DesktopState>().runtime,
            ServiceProvider::Outlook,
        );
        let client = ServerClient::new(&server.base, &server.base).unwrap();
        workspace.connected_inbox.update(cx, |view, cx| {
            view.client = client;
            view.refresh(cx);
        });
        workspace.active_view = AppView::Inbox;
        workspace.inbox_mail_visible = true;
        cx.notify();
        server
    });
    let mail = workspace.read_with(cx, |workspace, _| workspace.connected_inbox.clone());
    cx.condition(&mail, |view, _| {
        view.snapshot.is_some() && !view.accounts_loading && !view.rows_loading
    })
    .await;
    server.sync_mode.store(1, AtomicOrdering::Release);
    cx.run_until_parked();
    let button = cx
        .debug_bounds("desktop-mail-sync-outlook")
        .expect("real Outlook sync control");
    cx.simulate_click(button.center(), Modifiers::default());
    server.page_started.notified().await;
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Today;
        cx.notify();
    });
    mail.update(cx, |view, cx| view.action(Action::Stop, cx));
    server.release_page.notify_one();
    cx.condition(&mail, |view, _| {
        view.sync_stop.is_none() && !view.accounts_loading && !view.rows_loading
    })
    .await;
    assert_eq!(
        server.sync_count(),
        1,
        "Stop saves the Outlook page without starting another page off-route"
    );
    let requests = server.requests.lock().unwrap();
    assert!(requests
        .iter()
        .any(|request| request.starts_with("GET /api/connections/outlook ")));
    assert!(requests.iter().all(|request| !request.contains("/gmail")));
    drop(requests);
    mail.read_with(cx, |view, _| {
        assert!(view.rows.is_some());
        assert_eq!(view.mailbox.as_deref(), Some("mailbox-a"));
    });
}
