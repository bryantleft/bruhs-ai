use super::*;
use std::collections::{BTreeMap, HashSet};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
mod batch;
mod orphan;
mod retired;

struct Server {
    base: String,
    mode: Arc<AtomicU8>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: tokio::task::JoinHandle<()>,
}

impl Server {
    fn new(runtime: &tokio::runtime::Runtime, redirect: PersonMergeResolution) -> Self {
        Self::new_batch(runtime, vec![redirect])
    }

    fn new_batch(runtime: &tokio::runtime::Runtime, redirects: Vec<PersonMergeResolution>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("owned listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let base = format!("http://{}", listener.local_addr().expect("owned address"));
        let mode = Arc::new(AtomicU8::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (state, captured) = (mode.clone(), requests.clone());
        let worker = runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("async listener");
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let (body_at, length) = loop {
                    let mut buffer = [0; 4096];
                    let size = stream.read(&mut buffer).await.expect("request headers");
                    assert!(size > 0 && bytes.len() < 65536);
                    bytes.extend_from_slice(&buffer[..size]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let length: usize = headers.lines().find_map(|line| line.strip_prefix("content-length:")).unwrap_or("0").trim().parse().expect("content length");
                        break (end + 4, length);
                    }
                };
                while bytes.len() < body_at + length {
                    let mut buffer = [0; 4096];
                    let size = stream.read(&mut buffer).await.expect("request body");
                    assert!(size > 0 && bytes.len() < 65536);
                    bytes.extend_from_slice(&buffer[..size]);
                }
                let headers = String::from_utf8_lossy(&bytes[..body_at]);
                let request = headers.lines().next().expect("request line").to_string();
                captured.lock().expect("request log").push(request.clone());
                let mode = state.load(Ordering::SeqCst);
                let (status, body) = if request.starts_with("GET /api/people ") {
                    let sources: HashSet<_> = redirects.iter().map(|r| r.source_id.as_str()).collect();
                    let targets: BTreeMap<_, _> = redirects.iter().filter(|r| !sources.contains(r.target.id.as_str())).map(|r| (&r.target.id, &r.target)).collect();
                    ("200 OK", serde_json::to_string(&targets.into_values().collect::<Vec<_>>()).expect("directory response"))
                } else if request.starts_with("POST /api/people/resolve-merges ") {
                    let body: serde_json::Value = serde_json::from_slice(&bytes[body_at..body_at + length]).expect("lookup JSON");
                    let ids = body["ids"].as_array().expect("lookup IDs");
                    assert!(!ids.is_empty() && ids.len() <= 64);
                    if mode == 2 { ("503 Service Unavailable", "{}".into()) }
                    else {
                        let mut reply: Vec<_> = redirects.iter().filter(|r| ids.contains(&serde_json::json!(r.source_id))).cloned().collect();
                        if mode == 1 { for redirect in &mut reply { redirect.target.id.clone_from(&redirect.source_id); } }
                        reply.reverse();
                        ("200 OK", serde_json::to_string(&reply).expect("resolution response"))
                    }
                } else if request.starts_with("POST /api/people/source/merge_into/target ") {
                    ("200 OK", serde_json::to_string(&redirects[0].target).expect("merge response"))
                } else if request.starts_with("GET /api/meetings ") {
                    ("200 OK", "[]".into())
                } else { ("409 Conflict", "{}".into()) };
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(reply.as_bytes()).await.expect("HTTP reply");
            }
        });
        Self {
            base,
            mode,
            requests,
            worker,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[gpui::test]
async fn explicit_native_merge_response_uses_the_same_private_context_commit(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let server = workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        let server = Server::new(
            &cx.global::<super::super::super::DesktopState>().runtime,
            redirect,
        );
        w.meetings_hub.update(cx, |hub, cx| {
            hub.set_people_test_client(ServerClient::new(&server.base, &server.base).unwrap());
            hub.merge_people("source".into(), "target".into(), cx);
        });
        server
    });
    cx.condition(&workspace, |w, cx| {
        w.meetings_hub.read(cx).person_is_retired("source")
    })
    .await;
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.relationships[0].person_id, "target");
        assert!(w.relationships[0].notes.contains("Private source notes"));
        assert_eq!(w.captures[0].target_id.as_deref(), Some("target"));
    });
    assert!(server.requests.lock().unwrap()[0]
        .starts_with("POST /api/people/source/merge_into/target "));
}

#[gpui::test]
async fn refresh_resolves_retired_pending_id_before_any_create_and_preserves_unconfirmed_missing_contact(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let server = workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        let server = Server::new(
            &cx.global::<super::super::super::DesktopState>().runtime,
            redirect,
        );
        w.meetings_hub.update(cx, |hub, cx| {
            let mut unrelated = PersonProfile::new("Missing is not deletion evidence");
            unrelated.id = "unrelated".into();
            hub.people.push(unrelated);
            hub.set_people_test_client(ServerClient::new(&server.base, &server.base).unwrap());
            hub.refresh_people(cx);
        });
        server
    });
    cx.condition(&workspace, |w, cx| {
        w.meetings_hub.read(cx).person_is_retired("source")
    })
    .await;
    workspace.update_in(cx, |w, _, cx| {
        let hub = w.meetings_hub.read(cx);
        assert_eq!(hub.pending_people_count(), 0);
        assert!(hub.people.iter().any(|p| p.id == "unrelated"));
        assert_eq!(
            w.inbox_conversations[0].participants[0]
                .person_id
                .as_deref(),
            Some("target")
        );
    });
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].starts_with("GET /api/people "));
    assert!(requests[1].starts_with("POST /api/people/resolve-merges "));
    assert!(!requests.iter().any(|r| r.starts_with("POST /api/people ")));
    drop(requests);
}

#[gpui::test]
async fn malformed_or_failed_merge_lookup_preserves_local_snapshot_and_blocks_create_until_retry(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let (server, before) = workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        let server = Server::new(
            &cx.global::<super::super::super::DesktopState>().runtime,
            redirect,
        );
        w.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(ServerClient::new(&server.base, &server.base).unwrap())
        });
        (server, serde_json::to_value(w.snapshot(cx)).unwrap())
    });
    for mode in [1, 2] {
        server.mode.store(mode, Ordering::SeqCst);
        workspace.update_in(cx, |w, _, cx| {
            w.meetings_hub.update(cx, |hub, cx| {
                hub.notice = None;
                hub.retry_pending_people(cx);
            })
        });
        cx.condition(&workspace, |w, cx| w.meetings_hub.read(cx).notice.is_some())
            .await;
        workspace.update_in(cx, |w, _, cx| {
            assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), before)
        });
    }
    assert!(!server
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|r| r.starts_with("POST /api/people ")));
    server.mode.store(0, Ordering::SeqCst);
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub
            .update(cx, |hub, cx| hub.retry_pending_people(cx))
    });
    cx.condition(&workspace, |w, cx| {
        w.meetings_hub.read(cx).person_is_retired("source")
    })
    .await;
}
