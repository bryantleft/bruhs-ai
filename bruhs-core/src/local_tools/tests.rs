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

#[test]
fn bruh_launches_use_a_persistent_headless_profile_behind_the_guard() {
    use browser::{launch_args, LaunchOptions};
    use std::path::Path;
    let options = LaunchOptions {
        profile: Some("/data/bruhs/a/browser".into()),
        headless: true,
        proxy: Some("127.0.0.1:4100".parse().unwrap()),
    };
    let args = launch_args(&options, Path::new("/data/bruhs/a/browser"), "UA");
    for expected in [
        "--headless=new",
        "--user-data-dir=/data/bruhs/a/browser",
        "--proxy-server=http://127.0.0.1:4100",
        "--proxy-bypass-list=<-loopback>",
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
        "--window-size=1280,800",
        "--use-mock-keychain",
        "--password-store=basic",
        "--user-agent=UA",
    ] {
        assert!(args.iter().any(|arg| arg == expected), "{expected}");
    }
    assert_eq!(args.last().map(String::as_str), Some("about:blank"));
    let verification = launch_args(&LaunchOptions::verification(), Path::new("/tmp/x"), "UA");
    assert!(!verification
        .iter()
        .any(|arg| arg.starts_with("--headless") || arg.starts_with("--proxy")));
    assert!(verification
        .iter()
        .any(|arg| arg == "--user-data-dir=/tmp/x"));
}

#[test]
fn headed_and_headless_bruh_launches_share_the_cookie_key() {
    use browser::{launch_args, LaunchOptions};
    use std::path::Path;
    let launch = |headless| {
        launch_args(
            &LaunchOptions {
                profile: Some("/data/bruhs/a/browser".into()),
                headless,
                proxy: Some("127.0.0.1:4100".parse().unwrap()),
            },
            Path::new("/data/bruhs/a/browser"),
            "UA",
        )
    };
    let keys = |args: Vec<String>| -> Vec<String> {
        args.into_iter()
            .filter(|arg| arg.contains("keychain") || arg.starts_with("--password-store"))
            .collect()
    };
    let headed = keys(launch(false));
    assert_eq!(headed, ["--use-mock-keychain", "--password-store=basic"]);
    assert_eq!(headed, keys(launch(true)));
    assert!(!launch(false)
        .iter()
        .any(|arg| arg.starts_with("--headless")));
}

#[test]
fn bruh_browsers_deny_downloads_in_a_window_too() {
    use browser::LaunchOptions;
    let bruh = |headless| LaunchOptions {
        profile: Some("/data/bruhs/a/browser".into()),
        headless,
        proxy: Some("127.0.0.1:4100".parse().unwrap()),
    };
    assert!(bruh(true).denies_downloads());
    assert!(bruh(false).denies_downloads());
    assert!(!LaunchOptions::verification().denies_downloads());
}

#[tokio::test]
async fn a_window_is_measured_from_its_layout_metrics() {
    use serde_json::json;
    let metrics = json!({
        "cssLayoutViewport": {"pageX": 0, "pageY": 0, "clientWidth": 1440, "clientHeight": 812.5},
        "cssVisualViewport": {"clientWidth": 1, "clientHeight": 1}
    });
    let (mut cdp, server) =
        fake_page_answering(vec![], vec![("Page.getLayoutMetrics", metrics)]).await;
    assert_eq!(
        browser::layout_viewport(&mut cdp).await.unwrap(),
        Some(Viewport {
            width: 1440,
            height: 812
        })
    );
    drop(cdp);
    let calls = server.await.unwrap();
    assert!(
        calls.iter().all(|(method, _)| method != "Runtime.evaluate"),
        "no page script measures the window"
    );
    let (mut cdp, _server) = fake_page(vec![]).await;
    assert_eq!(browser::layout_viewport(&mut cdp).await.unwrap(), None);
}

#[tokio::test]
async fn a_window_feed_follows_the_main_frame_and_ends_with_the_page() {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (go, started) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let Some(Ok(Message::Text(text))) = socket.next().await else {
            panic!("Page.enable first");
        };
        let request: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(request["method"], "Page.enable");
        socket
            .send(Message::Text(
                json!({"id": request["id"], "result": {}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let Some(Ok(Message::Text(text))) = socket.next().await else {
            panic!("then the history");
        };
        let request: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(request["method"], "Page.getNavigationHistory");
        let history = json!({"id": request["id"], "result": {"currentIndex": 0, "entries": [{"id": 1, "url": "https://a.test/0"}]}});
        socket
            .send(Message::Text(history.to_string().into()))
            .await
            .unwrap();
        // The test has read where the page already was.
        started.await.unwrap();
        let events = [
            json!({"method": "Page.frameNavigated", "params": {"frame": {"id": "main", "url": "https://a.test/1"}}}),
            json!({"method": "Page.navigatedWithinDocument", "params": {"frameId": "main", "url": "https://a.test/2"}}),
            json!({"method": "Page.frameNavigated", "params": {"frame": {"id": "ad", "parentId": "main", "url": "https://ads.test/"}}}),
            json!({"method": "Page.navigatedWithinDocument", "params": {"frameId": "ad", "url": "https://ads.test/x"}}),
        ];
        for event in events {
            socket
                .send(Message::Text(event.to_string().into()))
                .await
                .unwrap();
        }
        // The window closes.
        socket.close(None).await.unwrap();
    });
    let mut cdp = browser::Cdp::connect(&format!("ws://{addr}"), ProcessPolicy::default())
        .await
        .unwrap();
    let (urls, mut feed) = tokio::sync::watch::channel(None);
    let mut seen = Vec::new();
    let mut go = Some(go);
    let follow = browser::follow_window(&mut cdp, &urls);
    tokio::pin!(follow);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::select! {
                () = &mut follow => break,
                Ok(()) = feed.changed() => {
                    seen.extend(feed.borrow_and_update().clone());
                    if let Some(go) = go.take() {
                        go.send(()).unwrap();
                    }
                }
            }
        }
    })
    .await
    .expect("the feed ends with the page");
    seen.extend(feed.borrow_and_update().clone());
    assert_eq!(seen.first().map(String::as_str), Some("https://a.test/0"));
    assert_eq!(seen.last().map(String::as_str), Some("https://a.test/2"));
    assert!(!seen.iter().any(|url| url.contains("ads.test")), "{seen:?}");
    server.await.unwrap();
}

#[tokio::test]
async fn owner_back_goes_through_the_navigation_history() {
    use serde_json::json;
    let history = |index: u64| {
        json!({
            "currentIndex": index,
            "entries": [
                {"id": 7, "url": "https://a.test/1"},
                {"id": 9, "url": "https://a.test/2"}
            ]
        })
    };
    let (mut cdp, server) =
        fake_page_answering(vec![], vec![("Page.getNavigationHistory", history(1))]).await;
    browser::go_back(&mut cdp).await.unwrap();
    drop(cdp);
    let calls = server.await.unwrap();
    assert_eq!(
        calls,
        [
            ("Page.getNavigationHistory".to_owned(), json!({})),
            (
                "Page.navigateToHistoryEntry".to_owned(),
                json!({"entryId": 7})
            )
        ]
    );
    // On the first entry there is nowhere to go.
    let (mut cdp, server) =
        fake_page_answering(vec![], vec![("Page.getNavigationHistory", history(0))]).await;
    browser::go_back(&mut cdp).await.unwrap();
    drop(cdp);
    assert_eq!(server.await.unwrap().len(), 1);
}

#[test]
fn the_snapshot_never_reads_input_values() {
    assert!(!browser::SNAPSHOT_EXPRESSION.contains("value"));
}

/// A fake page target: records every CDP call and answers `Runtime.evaluate`
/// calls in turn from `evaluations`, everything else with `{}`.
async fn fake_page(
    evaluations: Vec<serde_json::Value>,
) -> (
    browser::Cdp,
    tokio::task::JoinHandle<Vec<(String, serde_json::Value)>>,
) {
    fake_page_answering(evaluations, Vec::new()).await
}

/// [`fake_page`], answering each method in `answers` with its result.
async fn fake_page_answering(
    evaluations: Vec<serde_json::Value>,
    answers: Vec<(&'static str, serde_json::Value)>,
) -> (
    browser::Cdp,
    tokio::task::JoinHandle<Vec<(String, serde_json::Value)>>,
) {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut evaluations = evaluations.into_iter();
        let mut calls = Vec::new();
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let request: serde_json::Value = serde_json::from_str(&text).unwrap();
            let method = request["method"].as_str().unwrap().to_owned();
            let answer = answers.iter().find(|(name, _)| *name == method);
            let result = if let Some((_, answer)) = answer {
                answer.clone()
            } else if method == "Runtime.evaluate" {
                json!({"result":{"value":evaluations.next().unwrap_or(serde_json::Value::Null)}})
            } else {
                json!({})
            };
            calls.push((method, request["params"].clone()));
            socket
                .send(Message::Text(
                    json!({"id":request["id"],"result":result})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
        calls
    });
    let cdp = browser::Cdp::connect(&format!("ws://{addr}"), ProcessPolicy::default())
        .await
        .unwrap();
    (cdp, server)
}

#[tokio::test]
async fn real_clicks_press_and_release_at_the_element_centre() {
    use serde_json::json;
    let (mut cdp, server) = fake_page(vec![json!({"x":100,"y":50,"hit":true,"w":10,"h":10})]).await;
    let reply = browser::act(
        &mut cdp,
        BrowserAction::Click {
            selector: "#buy".into(),
        },
        COMPUTER_VIEWPORT,
    )
    .await
    .unwrap();
    assert_eq!(
        reply.value,
        json!({"clicked":true,"via":"input","x":100.0,"y":50.0})
    );
    drop(cdp);
    let calls = server.await.unwrap();
    let methods: Vec<_> = calls.iter().map(|(method, _)| method.as_str()).collect();
    assert_eq!(
        methods,
        [
            "Runtime.evaluate",
            "Input.dispatchMouseEvent",
            "Input.dispatchMouseEvent",
            "Input.dispatchMouseEvent"
        ]
    );
    let kinds: Vec<_> = calls[1..]
        .iter()
        .map(|(_, params)| params["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["mouseMoved", "mousePressed", "mouseReleased"]);
    for (_, params) in &calls[1..] {
        assert_eq!(
            (params["x"].as_f64(), params["y"].as_f64()),
            (Some(100.0), Some(50.0))
        );
    }
    assert!(calls[0].1["expression"]
        .as_str()
        .unwrap()
        .contains("elementFromPoint"));
}

#[tokio::test]
async fn a_covered_or_boxless_element_falls_back_to_a_script_click() {
    use serde_json::json;
    for located in [
        json!({"x":100,"y":50,"hit":false,"w":10,"h":10}),
        json!({"x":0,"y":0,"hit":true,"w":0,"h":0}),
    ] {
        let (mut cdp, server) = fake_page(vec![located, json!(true)]).await;
        let reply = browser::act(
            &mut cdp,
            BrowserAction::Click {
                selector: "#buy".into(),
            },
            COMPUTER_VIEWPORT,
        )
        .await
        .unwrap();
        assert_eq!(reply.value, json!({"clicked":true,"via":"script"}));
        drop(cdp);
        let calls = server.await.unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(method, _)| method == "Runtime.evaluate"));
        assert!(calls[1].1["expression"]
            .as_str()
            .unwrap()
            .contains(".click()"));
    }
}

#[tokio::test]
async fn typing_focuses_then_inserts_and_scrolling_wheels_at_the_centre() {
    use serde_json::json;
    let (mut cdp, server) = fake_page(vec![json!(true)]).await;
    let reply = browser::act(
        &mut cdp,
        BrowserAction::Type {
            text: "hi".into(),
            selector: Some("#q".into()),
        },
        COMPUTER_VIEWPORT,
    )
    .await
    .unwrap();
    assert_eq!(reply.value, json!({"typed":2}));
    let reply = browser::act(
        &mut cdp,
        BrowserAction::Scroll { dx: 0.0, dy: 300.0 },
        COMPUTER_VIEWPORT,
    )
    .await
    .unwrap();
    assert_eq!(reply.value, json!({"scrolled":true}));
    drop(cdp);
    let calls = server.await.unwrap();
    let methods: Vec<_> = calls.iter().map(|(method, _)| method.as_str()).collect();
    assert_eq!(
        methods,
        [
            "Runtime.evaluate",
            "Input.insertText",
            "Input.dispatchMouseEvent"
        ]
    );
    assert!(calls[0].1["expression"]
        .as_str()
        .unwrap()
        .contains(".focus()"));
    assert_eq!(calls[1].1, json!({"text":"hi"}));
    let wheel = &calls[2].1;
    assert_eq!(wheel["type"], "mouseWheel");
    assert_eq!(
        (
            wheel["x"].as_f64(),
            wheel["y"].as_f64(),
            wheel["deltaY"].as_f64()
        ),
        (Some(640.0), Some(400.0), Some(300.0))
    );
}

#[tokio::test]
async fn agent_keys_and_points_are_checked_before_anything_reaches_the_page() {
    use serde_json::json;
    let (mut cdp, server) = fake_page(vec![]).await;
    let reply = browser::act(
        &mut cdp,
        BrowserAction::Key {
            key: "Enter".into(),
        },
        COMPUTER_VIEWPORT,
    )
    .await
    .unwrap();
    assert_eq!(reply.value, json!({"pressed":"Enter"}));
    let reply = browser::act(
        &mut cdp,
        BrowserAction::ClickAt { x: 10.0, y: 20.0 },
        COMPUTER_VIEWPORT,
    )
    .await
    .unwrap();
    assert_eq!(
        reply.value,
        json!({"clicked":true,"via":"input","x":10.0,"y":20.0})
    );
    for refused in [
        BrowserAction::Key { key: "F5".into() },
        BrowserAction::ClickAt { x: 1281.0, y: 0.0 },
        BrowserAction::Scroll {
            dx: 0.0,
            dy: 20_000.0,
        },
        BrowserAction::Type {
            text: "x".repeat(MAX_TYPE_CHARS + 1),
            selector: None,
        },
    ] {
        assert!(matches!(
            browser::act(&mut cdp, refused, COMPUTER_VIEWPORT).await,
            Err(ToolError::Invalid(_))
        ));
    }
    drop(cdp);
    let calls = server.await.unwrap();
    let methods: Vec<_> = calls.iter().map(|(method, _)| method.as_str()).collect();
    assert_eq!(
        methods,
        ["Input.dispatchKeyEvent"; 2]
            .iter()
            .chain(["Input.dispatchMouseEvent"; 3].iter())
            .copied()
            .collect::<Vec<_>>()
    );
    assert_eq!(calls[0].1["modifiers"], 0);
    assert_eq!(calls[0].1["type"], "keyDown");
    assert_eq!(calls[1].1["type"], "keyUp");
}

#[tokio::test]
#[ignore = "launches headless Chrome on a persistent temp profile; run explicitly for local acceptance"]
async fn live_bruh_profile_keeps_a_cookie_across_a_graceful_close() {
    use axum::{http::header, routing::get};
    use browser::{Browser, LaunchOptions};
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = seen.clone();
    let app = axum::Router::new()
        .route(
            "/set",
            get(|| async {
                (
                    [(header::SET_COOKIE, "bruh=kept; Max-Age=3600; Path=/")],
                    axum::response::Html("<title>Set</title>"),
                )
            }),
        )
        .route(
            "/check",
            get(move |headers: axum::http::HeaderMap| {
                let observed = observed.clone();
                async move {
                    let cookie = headers
                        .get(header::COOKIE)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("")
                        .to_owned();
                    observed.lock().unwrap().push(cookie);
                    axum::response::Html("<title>Check</title>")
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let home = tempfile::tempdir().unwrap();
    let profile = home.path().join("bruhs").join("a").join("browser");
    let options = LaunchOptions {
        profile: Some(profile.clone()),
        headless: true,
        proxy: None,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let start = |path: &str| BrowserStart {
        grant_id: "live".into(),
        url: format!("http://{addr}{path}"),
    };
    let grace = std::time::Duration::from_millis(bruhs_models::crew::CLOSE_GRACE_MS);
    let mut first = Browser::start(&client, start("/set"), ProcessPolicy::default(), &options)
        .await
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&profile).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    assert_eq!(
        first.current_url().await.unwrap(),
        format!("http://{addr}/set")
    );
    assert!(first.page_socket().starts_with("ws://127.0.0.1:"));
    assert!(first.page_socket().contains("/devtools/page/"));
    first.close(grace).await.unwrap();
    let status = first.exit_status().await.expect("Chrome exited");
    assert!(status.success(), "Chrome was killed, not closed: {status}");
    drop(first);
    assert!(
        profile.join("Default").exists(),
        "the profile outlives its browser"
    );
    let mut second = Browser::start(&client, start("/check"), ProcessPolicy::default(), &options)
        .await
        .unwrap();
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|cookie| cookie.contains("bruh=kept")),
        "the cookie did not survive the close: {:?}",
        seen.lock().unwrap()
    );
    second.close(grace).await.unwrap();
    assert!(second.exit_status().await.is_some());
    drop(second);
    assert!(profile.exists());
    fixture.abort();
}

#[tokio::test]
#[ignore = "launches headless Chrome; run explicitly for local acceptance"]
async fn live_browser_clicks_types_presses_and_scrolls_with_real_input() {
    use axum::routing::get;
    use browser::{Browser, LaunchOptions};
    let app = axum::Router::new()
        .route("/", get(|| async { axum::response::Html("<title>Input fixture</title><input id=q><div id=covered style=\"position:relative\"><button id=under onclick=\"document.body.dataset.under=event.isTrusted\">Under</button><div style=\"position:absolute;inset:0\"></div></div><button id=far style=\"margin-top:2000px\" onclick=\"document.body.dataset.far=event.isTrusted\">Far</button>") }))
        .route("/other", get(|| async { axum::response::Html("<title>Other</title>") }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let options = LaunchOptions {
        profile: None,
        headless: true,
        proxy: None,
    };
    let policy = ProcessPolicy::default();
    let mut browser = Browser::start(
        &client,
        BrowserStart {
            grant_id: "live".into(),
            url: format!("http://{addr}/"),
        },
        policy,
        &options,
    )
    .await
    .unwrap();
    let evaluate = |expression: &str| BrowserAction::Evaluate {
        expression: expression.into(),
    };
    assert_eq!(
        browser
            .command(evaluate("[innerWidth,innerHeight]"), policy)
            .await
            .unwrap()
            .value,
        serde_json::json!([1280, 800])
    );
    let typed = browser
        .command(
            BrowserAction::Type {
                text: "hey".into(),
                selector: Some("#q".into()),
            },
            policy,
        )
        .await
        .unwrap();
    assert_eq!(typed.value["typed"], 3);
    browser
        .command(
            BrowserAction::Key {
                key: "Backspace".into(),
            },
            policy,
        )
        .await
        .unwrap();
    assert_eq!(
        browser
            .command(evaluate("document.querySelector('#q').value"), policy)
            .await
            .unwrap()
            .value,
        "he"
    );
    let far = browser
        .command(
            BrowserAction::Click {
                selector: "#far".into(),
            },
            policy,
        )
        .await
        .unwrap();
    assert_eq!(far.value["via"], "input");
    let under = browser
        .command(
            BrowserAction::Click {
                selector: "#under".into(),
            },
            policy,
        )
        .await
        .unwrap();
    assert_eq!(under.value["via"], "script");
    assert_eq!(
        browser
            .command(
                evaluate("[document.body.dataset.far,document.body.dataset.under]"),
                policy
            )
            .await
            .unwrap()
            .value,
        serde_json::json!(["true", "false"])
    );
    let before = browser
        .command(evaluate("scrollY"), policy)
        .await
        .unwrap()
        .value;
    browser
        .command(BrowserAction::Scroll { dx: 0.0, dy: 300.0 }, policy)
        .await
        .unwrap();
    let after = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let now = browser
                .command(evaluate("scrollY"), policy)
                .await
                .unwrap()
                .value;
            if now != before {
                return now;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the wheel scrolled the page");
    assert!(after.as_f64().unwrap() > before.as_f64().unwrap());
    browser
        .command(
            BrowserAction::Navigate {
                url: format!("http://{addr}/other"),
            },
            policy,
        )
        .await
        .unwrap();
    assert_eq!(
        browser.current_url().await.unwrap(),
        format!("http://{addr}/other")
    );
    browser.owner_input(&ComputerInput::Reload).await.unwrap();
    browser.owner_input(&ComputerInput::Back).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while browser.current_url().await.unwrap() != format!("http://{addr}/") {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Back returned to the first page");
    // Owner navigation follows a bruh's URL rules: never this computer.
    for refused in [format!("http://{addr}/other"), "file:///etc/hosts".into()] {
        assert!(matches!(
            browser
                .owner_input(&ComputerInput::Navigate { url: refused })
                .await,
            Err(ToolError::Invalid(_))
        ));
    }
    assert_eq!(
        browser.current_url().await.unwrap(),
        format!("http://{addr}/")
    );
    browser
        .close(std::time::Duration::from_millis(
            bruhs_models::crew::CLOSE_GRACE_MS,
        ))
        .await
        .unwrap();
    fixture.abort();
}

#[tokio::test]
#[ignore = "launches Chrome headless and in a window on one temp profile; run explicitly for local acceptance"]
async fn live_cookies_survive_opening_and_closing_a_window() {
    use axum::{extract::Path, http::header, routing::get};
    use browser::{Browser, LaunchOptions};
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = seen.clone();
    let app = axum::Router::new()
        .route(
            "/set/{name}",
            get(|Path(name): Path<String>| async move {
                (
                    [(
                        header::SET_COOKIE,
                        format!("{name}=kept; Max-Age=3600; Path=/"),
                    )],
                    axum::response::Html("<title>Set</title>"),
                )
            }),
        )
        .route(
            "/check",
            get(move |headers: axum::http::HeaderMap| {
                let observed = observed.clone();
                async move {
                    let cookie = headers
                        .get(header::COOKIE)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("")
                        .to_owned();
                    observed.lock().unwrap().push(cookie);
                    axum::response::Html("<title>Check</title>")
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let home = tempfile::tempdir().unwrap();
    let profile = home.path().join("bruhs").join("a").join("browser");
    let options = |headless| LaunchOptions {
        profile: Some(profile.clone()),
        headless,
        proxy: None,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let policy = ProcessPolicy::default();
    let grace = std::time::Duration::from_millis(bruhs_models::crew::CLOSE_GRACE_MS);
    let open = |path: &str, headless| {
        let request = BrowserStart {
            grant_id: "live".into(),
            url: format!("http://{addr}{path}"),
        };
        let (client, options) = (client.clone(), options(headless));
        async move { Browser::start(&client, request, policy, &options).await }
    };
    let close = |mut browser: Browser| async move {
        browser.close(grace).await.unwrap();
        let status = browser.exit_status().await.expect("Chrome exited");
        assert!(status.success(), "Chrome was killed, not closed: {status}");
    };
    let last_check = |seen: &Mutex<Vec<String>>| seen.lock().unwrap().last().cloned();
    close(open("/set/headless", true).await.unwrap()).await;
    // Open in a window: the headless login is there.
    let mut window = open("/check", false).await.unwrap();
    let cookie = last_check(&seen).unwrap_or_default();
    assert!(cookie.contains("headless=kept"), "{cookie:?}");
    window
        .command(
            BrowserAction::Navigate {
                url: format!("http://{addr}/set/window"),
            },
            policy,
        )
        .await
        .unwrap();
    close(window).await;
    // Hand back: headless again, with the window's login too.
    let headless = open("/check", true).await.unwrap();
    let cookie = last_check(&seen).unwrap_or_default();
    assert!(
        cookie.contains("headless=kept") && cookie.contains("window=kept"),
        "{cookie:?}"
    );
    close(headless).await;
    fixture.abort();
}

#[tokio::test]
#[ignore = "launches Chrome in a window on a temp profile; run explicitly for local acceptance"]
async fn live_window_is_measured_by_the_browser_and_goes_back_through_history() {
    use axum::routing::get;
    use browser::{Browser, LaunchOptions};
    let app = axum::Router::new()
        .route("/", get(|| async { axum::response::Html("<title>One</title>") }))
        .route(
            "/two",
            get(|| async {
                // Page script can't trap the owner or fake the window's size.
                axum::response::Html("<title>Two</title><script>history.back=()=>{};Object.defineProperty(window,'innerWidth',{get:()=>5})</script>")
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let policy = ProcessPolicy::default();
    let mut browser = Browser::start(
        &client,
        BrowserStart {
            grant_id: "live".into(),
            url: format!("http://{addr}/"),
        },
        policy,
        &LaunchOptions::verification(),
    )
    .await
    .unwrap();
    let mut window = browser.watch_window();
    browser
        .command(
            BrowserAction::Navigate {
                url: format!("http://{addr}/two"),
            },
            policy,
        )
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        window.wait_for(|url| url.as_deref() == Some(&format!("http://{addr}/two"))),
    )
    .await
    .expect("the window feed sees page two")
    .unwrap();
    // Measured again on page two, whose innerWidth reads 5.
    browser.owner_input(&ComputerInput::Back).await.unwrap();
    let viewport = browser.viewport();
    let mut url = String::new();
    for _ in 0..100 {
        url = browser.current_url().await.unwrap();
        if url == format!("http://{addr}/") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(url, format!("http://{addr}/"));
    let size = browser
        .command(
            BrowserAction::Evaluate {
                expression:
                    "[document.documentElement.clientWidth,document.documentElement.clientHeight]"
                        .into(),
            },
            policy,
        )
        .await
        .unwrap()
        .value;
    assert_eq!(
        size,
        serde_json::json!([viewport.width, viewport.height]),
        "{viewport:?}"
    );
    // Closing the window's page by hand ends the feed.
    let page = url::Url::parse(browser.page_socket()).unwrap();
    let target = page
        .path_segments()
        .unwrap()
        .next_back()
        .unwrap()
        .to_owned();
    client
        .get(format!(
            "http://127.0.0.1:{}/json/close/{target}",
            page.port().unwrap()
        ))
        .send()
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while window.changed().await.is_ok() {}
    })
    .await
    .expect("the feed ends when the page closes");
    browser
        .close(std::time::Duration::from_millis(
            bruhs_models::crew::CLOSE_GRACE_MS,
        ))
        .await
        .unwrap();
    fixture.abort();
}

#[tokio::test]
async fn a_press_cut_short_is_released_once() {
    use serde_json::json;
    let mouse = "Input.dispatchMouseEvent";
    let (mut cdp, server) = fake_page(vec![]).await;
    cdp.release_pressed().await.unwrap();
    cdp.call(
        mouse,
        json!({"type":"mousePressed","x":10.0,"y":20.0,"button":"left","buttons":1,"clickCount":1,"modifiers":0}),
    )
    .await
    .unwrap();
    cdp.release_pressed().await.unwrap();
    cdp.release_pressed().await.unwrap();
    // A press with its release needs nothing more.
    for kind in ["mousePressed", "mouseReleased"] {
        cdp.call(
            mouse,
            json!({"type":kind,"x":1.0,"y":2.0,"button":"left","buttons":0,"clickCount":1,"modifiers":0}),
        )
        .await
        .unwrap();
    }
    cdp.release_pressed().await.unwrap();
    drop(cdp);
    let calls = server.await.unwrap();
    let kinds: Vec<_> = calls
        .iter()
        .map(|(_, params)| params["type"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        kinds,
        [
            "mousePressed",
            "mouseReleased",
            "mousePressed",
            "mouseReleased"
        ]
    );
    assert_eq!(calls[1].1["x"], 10.0);
    assert_eq!(calls[1].1["y"], 20.0);
    assert_eq!(calls[1].1["button"], "left");
    assert_eq!(calls[1].1["buttons"], 0);
}

#[test]
fn the_page_address_comes_from_its_navigation_history() {
    let history = serde_json::json!({
        "currentIndex": 1,
        "entries": [{"url": "https://a.test/1"}, {"url": "https://a.test/2"}],
    });
    assert_eq!(
        browser::current_entry_url(&history).as_deref(),
        Some("https://a.test/2")
    );
    assert_eq!(browser::current_entry_url(&serde_json::json!({})), None);
}
