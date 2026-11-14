use super::*;

#[tokio::test]
async fn revoked_and_cross_scope_grants_cannot_launch_a_tool() {
    let host = LocalToolsHost::new().unwrap();
    let grant = host
        .grant(GrantRequest {
            principal: "task-a".into(),
            scope: ToolScope::Browser,
        })
        .unwrap();
    assert!(host.require(&grant.id, ToolScope::Devices).is_err());
    assert!(host.require("unknown", ToolScope::Browser).is_err());
    let mut ongoing = host.require(&grant.id, ToolScope::Browser).unwrap();
    host.revoke(&grant.id).await.unwrap();
    ongoing.changed().await.unwrap();
    assert!(*ongoing.borrow());
    let error = host
        .start_browser(BrowserStart {
            grant_id: grant.id,
            url: "about:blank".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Revoked));
}

#[test]
fn android_status_preserves_offline_and_unauthorized_devices() {
    let devices = devices::parse_android(
        "List of devices attached\nemulator-5554 device product:a model:Pixel_9 device:a\nserial1 unauthorized\nserial2 offline\n* daemon started successfully *\n",
    );
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[0].name, "Pixel 9");
    assert_eq!(devices[1].state, "unauthorized");
}

#[test]
fn android_text_and_urls_are_quoted_for_the_device_shell() {
    let args = devices::android_args(
        "emulator-5554",
        &DeviceAction::Type {
            text: "hello'; touch /sdcard/pwned; '".into(),
        },
    )
    .unwrap();
    assert_eq!(
        args.last().unwrap(),
        "'hello'\\'';%stouch%s/sdcard/pwned;%s'\\'''"
    );
    assert!(devices::android_args("--help", &DeviceAction::Home).is_err());
    assert!(devices::android_args(
        "safe",
        &DeviceAction::OpenUrl {
            url: "file:///private/data".into()
        }
    )
    .is_err());
}

#[test]
fn simulator_inventory_validates_identifiers() {
    let devices = devices::parse_ios(br#"{"devices":{"runtime":[{"udid":"0a3361cf-9351-41d2-91e0-b4d947f0de42","name":"iPhone","state":"Shutdown","isAvailable":true}]}}"#).unwrap();
    assert_eq!(devices[0].state, "Shutdown");
    assert!(devices::parse_ios(
        br#"{"devices":{"runtime":[{"udid":"--help","isAvailable":true}]}}"#
    )
    .is_err());
}

#[tokio::test]
async fn process_output_is_bounded_and_failures_are_not_success() {
    let policy = ProcessPolicy {
        timeout: std::time::Duration::from_secs(5),
        max_output_bytes: 16,
    };
    let oversized = process::run("/usr/bin/printf", &["%100s".into(), "x".into()], policy).await;
    assert!(oversized
        .unwrap_err()
        .to_string()
        .contains("capture budget"));
    assert!(process::run("/usr/bin/false", &[], policy).await.is_err());
}

#[tokio::test]
async fn cdp_correlates_replies_and_retains_events() {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request: serde_json::Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        socket
            .send(Message::Text(
                json!({"method":"Runtime.consoleAPICalled","params":{"type":"error"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Text(
                json!({"id":request["id"],"result":{"ok":true}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    });
    let mut client = browser::Cdp::connect(&format!("ws://{addr}"), ProcessPolicy::default())
        .await
        .unwrap();
    assert_eq!(
        client.call("Runtime.enable", json!({})).await.unwrap()["ok"],
        true
    );
    server.await.unwrap();
}

#[tokio::test]
#[ignore = "opens an isolated visible Chrome window; run explicitly for local acceptance"]
#[expect(
    clippy::result_large_err,
    reason = "Tungstenite fixes the handshake callback error to an HTTP response; this accepting fixture cannot box that signature"
)]
async fn live_browser_fills_clicks_and_saves_screenshot_then_revokes() {
    use axum::routing::get;
    use futures_util::SinkExt;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio_tungstenite::tungstenite::{
        handshake::server::{Request, Response},
        Message,
    };
    let unmarked = Arc::new(AtomicUsize::new(0));
    let observed = unmarked.clone();
    let app = axum::Router::new()
        .route("/", get(|| async { axum::response::Html("<title>Verification fixture</title><label>Name<input id=name></label><button id=go onclick=\"document.body.dataset.clicked=document.getElementById('name').value;console.log('clicked')\">Save</button>") }))
        .route("/api/private", get(move |headers: axum::http::HeaderMap| {
            let observed = observed.clone();
            async move {
                if headers.contains_key("x-bruhs-verification-browser") {
                    (axum::http::StatusCode::FORBIDDEN, "blocked")
                } else {
                    observed.fetch_add(1, Ordering::SeqCst);
                    (axum::http::StatusCode::OK, "private life")
                }
            }
        }))
        .route("/popup", get(|| async { axum::response::Html("<script>fetch('/api/private').then(r=>r.text()).then(text=>opener.postMessage({fixture:'popup',text},location.origin))</script>") }))
        .route("/worker.js", get(|| async {
            ([(axum::http::header::CONTENT_TYPE, "text/javascript")], "fetch('/api/private').then(r=>r.text()).then(text=>postMessage(text));")
        }))
        .route("/nested.js", get(|| async {
            ([(axum::http::header::CONTENT_TYPE, "text/javascript")], "const child=new Worker('/worker.js');child.onmessage=e=>{postMessage(e.data);child.terminate()};")
        }))
        .route("/shared.js", get(|| async {
            ([(axum::http::header::CONTENT_TYPE, "text/javascript")], "onconnect=e=>fetch('/api/private').then(r=>r.text()).then(text=>e.ports[0].postMessage(text));")
        }))
        .route("/sw.js", get(|| async {
            ([(axum::http::header::CONTENT_TYPE, "text/javascript")], "self.addEventListener('install',e=>e.waitUntil(self.skipWaiting()));self.addEventListener('activate',e=>e.waitUntil(self.clients.claim()));self.addEventListener('message',e=>e.waitUntil(fetch('/api/private').then(r=>r.text()).then(text=>e.ports[0].postMessage(text))));")
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_address = ws_listener.local_addr().unwrap();
    let websocket_fixture = tokio::spawn(async move {
        let (stream, _) = ws_listener.accept().await.unwrap();
        let mut identity = serde_json::Value::Null;
        let mut socket = tokio_tungstenite::accept_hdr_async(stream, |request: &Request, response: Response| {
            identity = serde_json::json!({
                "user_agent":request.headers().get("user-agent").and_then(|value| value.to_str().ok()),
                "marker":request.headers().contains_key("x-bruhs-verification-browser")
            });
            Ok(response)
        }).await.unwrap();
        socket
            .send(Message::Text(identity.to_string().into()))
            .await
            .unwrap();
        socket.close(None).await.unwrap();
    });
    let host = LocalToolsHost::new().unwrap();
    let grant = host
        .grant(GrantRequest {
            principal: "local verification acceptance".into(),
            scope: ToolScope::Browser,
        })
        .unwrap();
    let browser = host
        .start_browser(BrowserStart {
            grant_id: grant.id.clone(),
            url: format!("http://{addr}/"),
        })
        .await
        .unwrap();
    let command = |action| BrowserCommand {
        grant_id: grant.id.clone(),
        session_id: browser.id.clone(),
        command: action,
    };
    host.browser_command(command(BrowserAction::Fill {
        selector: "#name".into(),
        text: "Verified locally".into(),
    }))
    .await
    .unwrap();
    host.browser_command(command(BrowserAction::Click {
        selector: "#go".into(),
    }))
    .await
    .unwrap();
    let result = host
        .browser_command(command(BrowserAction::Evaluate {
            expression: "document.body.dataset.clicked".into(),
        }))
        .await
        .unwrap();
    assert_eq!(result.value, "Verified locally");
    for (target, expression) in [
        (
            "popup",
            "new Promise((resolve,reject)=>{const listener=e=>{if(e.data?.fixture==='popup'){removeEventListener('message',listener);resolve(e.data.text)}};addEventListener('message',listener);const popup=window.open('/popup','fixture');if(!popup)reject(new Error('Fixture popup blocked'));})",
        ),
        (
            "worker",
            "new Promise((resolve,reject)=>{const worker=new Worker('/worker.js');worker.onmessage=e=>{resolve(e.data);worker.terminate()};worker.onerror=e=>reject(new Error(e.message));})",
        ),
        (
            "nested worker",
            "new Promise((resolve,reject)=>{const worker=new Worker('/nested.js');worker.onmessage=e=>{resolve(e.data);worker.terminate()};worker.onerror=e=>reject(new Error(e.message));})",
        ),
        (
            "shared worker",
            "new Promise((resolve,reject)=>{const worker=new SharedWorker('/shared.js');worker.port.onmessage=e=>{resolve(e.data);worker.port.close()};worker.onerror=e=>reject(new Error(e.message));})",
        ),
        (
            "service worker",
            "(async()=>{await navigator.serviceWorker.register('/sw.js');const registration=await navigator.serviceWorker.ready;const channel=new MessageChannel();const result=new Promise(resolve=>channel.port1.onmessage=e=>resolve(e.data));registration.active.postMessage('probe',[channel.port2]);const text=await result;channel.port1.close();await registration.unregister();return text})()",
        ),
        (
            "popup navigation",
            "new Promise((resolve,reject)=>{const popup=window.open('/api/private','navigation-fixture');if(!popup){reject(new Error('Fixture popup blocked'));return}const deadline=Date.now()+20000;const timer=setInterval(()=>{const text=popup.document.body?.innerText;if(text==='blocked'||text==='private life'){clearInterval(timer);popup.close();resolve(text)}else if(Date.now()>deadline){clearInterval(timer);popup.close();reject(new Error('Popup navigation timed out'))}},20)})",
        ),
    ] {
        let result = host
            .browser_command(command(BrowserAction::Evaluate {
                expression: expression.into(),
            }))
            .await
            .unwrap_or_else(|error| panic!("{target}: {error}"));
        assert_eq!(
            result.value, "blocked",
            "{target} escaped the verification marker"
        );
    }
    assert_eq!(
        unmarked.load(Ordering::SeqCst),
        0,
        "A new target reached a private API without protection"
    );
    let websocket = host.browser_command(command(BrowserAction::Evaluate {
        expression: format!("new Promise((resolve,reject)=>{{const ws=new WebSocket('ws://{ws_address}/api/synthetic-ws');ws.onmessage=e=>resolve(JSON.parse(e.data));ws.onerror=()=>reject(new Error('Synthetic WebSocket connection failed'))}})"),
    })).await.unwrap();
    assert!(
        websocket.value["user_agent"]
            .as_str()
            .unwrap()
            .contains("BruhsVerification/1"),
        "WebSocket handshake must retain the launch-wide browser identity"
    );
    let screenshot = host
        .browser_command(command(BrowserAction::Screenshot))
        .await
        .unwrap()
        .image
        .unwrap();
    assert!(crate::uploads::is_upload_path(&screenshot.path));
    assert!(std::fs::metadata(&screenshot.path).unwrap().len() > 0);
    host.revoke(&grant.id).await.unwrap();
    assert!(host.snapshot().await.unwrap().browsers.is_empty());
    assert!(matches!(
        host.browser_command(command(BrowserAction::Snapshot)).await,
        Err(ToolError::Revoked)
    ));
    std::fs::remove_file(screenshot.path).unwrap();
    websocket_fixture.await.unwrap();
    fixture.abort();
}
