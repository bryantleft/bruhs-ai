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

/// A page connection knows its target is gone: Chrome said it crashed, or
/// the socket ended.
#[tokio::test]
async fn a_page_connection_knows_when_its_target_is_gone() {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for crash in [true, false] {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let request: serde_json::Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            if crash {
                let crashed = json!({"method":"Inspector.targetCrashed","params":{}});
                socket
                    .send(Message::Text(crashed.to_string().into()))
                    .await
                    .unwrap();
                let reply = json!({"id":request["id"],"result":{}});
                socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .unwrap();
            }
            // Without a crash, the target ends the socket instead.
            drop(socket);
        }
    });
    let endpoint = format!("ws://{addr}");
    let connect = || browser::Cdp::connect(&endpoint, ProcessPolicy::default());
    let mut crashed = connect().await.unwrap();
    assert!(!crashed.closed());
    crashed.call("Runtime.enable", json!({})).await.unwrap();
    assert!(crashed.closed());
    let mut ended = connect().await.unwrap();
    assert!(ended.call("Runtime.enable", json!({})).await.is_err());
    assert!(ended.closed());
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
        text: Some("Verified locally".into()),
        secret: None,
        submit: false,
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
        // Back never restores a page a vault fill touched, value and all.
        "--disable-features=BackForwardCache",
    ] {
        assert!(args.iter().any(|arg| arg == expected), "{expected}");
    }
    assert_eq!(args.last().map(String::as_str), Some("about:blank"));
    let verification = launch_args(&LaunchOptions::verification(), Path::new("/tmp/x"), "UA");
    assert!(!verification.iter().any(|arg| arg.starts_with("--headless")
        || arg.starts_with("--proxy")
        || arg.starts_with("--disable-features")));
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
fn kept_profiles_keep_their_cache_in_the_bruh_tree() {
    use browser::{launch_args, LaunchOptions};
    use std::path::Path;
    // On macOS Chrome otherwise moves a profile's cache from Application
    // Support to ~/Library/Caches, where deleting the bruh would miss it.
    for headless in [true, false] {
        let args = launch_args(
            &LaunchOptions {
                profile: Some("/data/bruhs/a/browser".into()),
                headless,
                proxy: Some("127.0.0.1:4100".parse().unwrap()),
            },
            Path::new("/data/bruhs/a/browser"),
            "UA",
        );
        let caches: Vec<&String> = args
            .iter()
            .filter(|arg| arg.starts_with("--disk-cache-dir"))
            .collect();
        assert_eq!(caches, ["--disk-cache-dir=/data/bruhs/a/browser-cache"]);
    }
    let verification = launch_args(&LaunchOptions::verification(), Path::new("/tmp/x"), "UA");
    assert!(!verification
        .iter()
        .any(|arg| arg.starts_with("--disk-cache-dir")));
}

/// A kept profile saves no passwords and fills no forms, whatever an
/// earlier window set; its other prefs stay.
#[tokio::test]
async fn kept_profiles_launch_without_the_password_manager() {
    let home = tempfile::tempdir().unwrap();
    let profile = home.path().join("browser");
    let sealed = |prefs: &serde_json::Value| {
        prefs["credentials_enable_service"] == false
            && prefs["profile"]["password_manager_enabled"] == false
            && prefs["autofill"]["profile_enabled"] == false
            && prefs["autofill"]["credit_card_enabled"] == false
            // No earlier tabs reopen, by choice or after a crash.
            && prefs["session"]["restore_on_startup"] == 5
            && prefs["profile"]["exit_type"] == "Normal"
            && prefs["profile"]["exited_cleanly"] == true
    };
    let read = || -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(profile.join("Default/Preferences")).unwrap())
            .unwrap()
    };
    // A first launch has no Preferences yet.
    browser::seal_preferences(&profile).await.unwrap();
    assert!(sealed(&read()), "{}", read());
    // The owner turned saving back on in a window; other prefs are kept.
    std::fs::write(
        profile.join("Default/Preferences"),
        r#"{"credentials_enable_service":true,"profile":{"name":"Bruh","password_manager_enabled":true,"exit_type":"Crashed"},"autofill":"odd","session":{"restore_on_startup":1,"startup_urls":["https://a.test/"]}}"#,
    )
    .unwrap();
    browser::seal_preferences(&profile).await.unwrap();
    let prefs = read();
    assert!(sealed(&prefs), "{prefs}");
    assert_eq!(prefs["profile"]["name"], "Bruh");
    assert_eq!(prefs["session"]["startup_urls"][0], "https://a.test/");
    // A file that isn't JSON is replaced, as Chrome would reset it.
    std::fs::write(profile.join("Default/Preferences"), "not json").unwrap();
    browser::seal_preferences(&profile).await.unwrap();
    assert!(sealed(&read()));
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

/// A fake page target that answers each method from its own queue of
/// replies (`{"result": …}` or `{"error": …}`), and `{"result": {}}` once a
/// queue is empty. Records every message's method, params and raw text.
async fn fake_page_replying(
    replies: Vec<(&'static str, serde_json::Value)>,
) -> (
    browser::Cdp,
    tokio::task::JoinHandle<Vec<(String, serde_json::Value, String)>>,
) {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut replies: std::collections::VecDeque<_> = replies.into();
        let mut calls = Vec::new();
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let request: serde_json::Value = serde_json::from_str(&text).unwrap();
            let method = request["method"].as_str().unwrap().to_owned();
            let mut reply = replies
                .iter()
                .position(|(name, _)| *name == method)
                .and_then(|at| replies.remove(at))
                .map_or_else(|| json!({"result": {}}), |(_, reply)| reply);
            reply["id"] = request["id"].clone();
            calls.push((method, request["params"].clone(), text.to_string()));
            socket
                .send(Message::Text(reply.to_string().into()))
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

/// The replies of a sign-in page at `https://shop.test/in`, whose main
/// frame `F` has document `L1` on history entry 7 every time it is asked.
fn sign_in_page() -> Vec<(&'static str, serde_json::Value)> {
    sign_in_page_loading(["L1", "L1", "L1"])
}

/// [`sign_in_page`] whose main frame reports these loaders in turn.
fn sign_in_page_loading(loaders: [&str; 3]) -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::json;
    let history = json!({"result": {"currentIndex": 0, "entries": [{"id": 7, "url": "https://shop.test/in"}]}});
    let mut replies = vec![(
        "Page.createIsolatedWorld",
        json!({"result": {"executionContextId": 9}}),
    )];
    for loader in loaders {
        let tree = json!({"result": {"frameTree": {"frame": {"id": "F", "loaderId": loader}}}});
        replies.push(("Page.getNavigationHistory", history.clone()));
        replies.push(("Page.getFrameTree", tree));
    }
    replies
}

fn answer(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"result": {"result": {"type": "object", "value": value}}})
}

fn vault_canary() -> String {
    format!("pw-{}", uuid::Uuid::new_v4().simple())
}

#[tokio::test]
async fn only_the_value_call_carries_the_value() {
    use bruhs_models::vault::SecretField;
    use serde_json::json;
    let value = vault_canary();
    let mut replies = sign_in_page();
    replies.push(("Runtime.callFunctionOn", answer(json!({"ok": true}))));
    replies.push(("Runtime.callFunctionOn", answer(json!({"ok": true}))));
    let (mut cdp, server) = fake_page_replying(replies).await;
    let mut filled = computer::FilledDocuments::default();
    let reply = browser::fill_secret(
        &mut cdp,
        "#pw",
        SecretField::Password,
        "https://shop.test",
        &value,
        true,
        &mut filled,
    )
    .await
    .unwrap();
    assert_eq!(
        reply.value,
        json!({"filled": "password", "submitted": true})
    );
    assert!(filled.holds_any(&["loader:L1".into()]));
    assert!(filled.holds_any(&["entry:7".into()]));
    drop(cdp);
    let calls = server.await.unwrap();
    let methods: Vec<_> = calls.iter().map(|(method, ..)| method.as_str()).collect();
    assert_eq!(
        methods,
        [
            "Page.getNavigationHistory",
            "Page.getFrameTree",
            "Page.createIsolatedWorld",
            "Runtime.callFunctionOn",
            "Page.getNavigationHistory",
            "Page.getFrameTree",
            "Runtime.callFunctionOn",
            "Page.getNavigationHistory",
            "Page.getFrameTree",
            "Input.dispatchKeyEvent",
            "Input.dispatchKeyEvent",
        ]
    );
    assert_eq!(calls[2].1["frameId"], "F");
    let (check, write) = (&calls[3].1, &calls[6].1);
    for call in [check, write] {
        assert_eq!(call["executionContextId"], 9);
        // Fixed bodies: the value and selector travel only as arguments.
        assert!(!call["functionDeclaration"]
            .as_str()
            .unwrap()
            .contains("#pw"));
    }
    assert_eq!(check["arguments"][0]["value"], "#pw");
    assert_eq!(check["arguments"][1]["value"], "https://shop.test");
    assert_eq!(check["arguments"][2]["value"], json!(["password"]));
    assert_eq!(check["arguments"][3]["value"], true);
    // The submit rule (ruling A2 I1): submitting, and the autocomplete
    // tokens that mark a sign-in field.
    assert_eq!(check["arguments"][4]["value"], true);
    assert_eq!(
        check["arguments"][5]["value"],
        json!(["username", "email", "one-time-code"])
    );
    let arguments = write["arguments"].as_array().unwrap();
    assert_eq!(arguments.last().unwrap()["value"], value.as_str());
    assert_eq!(write["userGesture"], false);
    for (at, (method, _, text)) in calls.iter().enumerate() {
        assert_eq!(text.contains(&value), at == 6, "{method}");
    }
    assert_eq!(calls[9].1["key"], "Enter");
    assert_eq!(calls[10].1["type"], "keyUp");
}

/// The page loaded another document between reading its marks and making
/// the world: the world may sit on a document nobody marked, so no value
/// is sent and nothing is marked.
#[tokio::test]
async fn a_document_that_changed_under_the_check_gets_no_value() {
    use bruhs_models::crew::FILL_FAILED;
    use bruhs_models::vault::SecretField;
    use serde_json::json;
    let value = vault_canary();
    let mut replies = sign_in_page_loading(["L1", "L2", "L2"]);
    replies.push(("Runtime.callFunctionOn", answer(json!({"ok": true}))));
    let (mut cdp, server) = fake_page_replying(replies).await;
    let mut filled = computer::FilledDocuments::default();
    let error = browser::fill_secret(
        &mut cdp,
        "#pw",
        SecretField::Password,
        "https://shop.test",
        &value,
        false,
        &mut filled,
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, ToolError::Invalid(message) if message == FILL_FAILED));
    assert!(filled.is_empty());
    drop(cdp);
    let calls = server.await.unwrap();
    let functions = calls
        .iter()
        .filter(|(method, ..)| method == "Runtime.callFunctionOn")
        .count();
    assert_eq!(functions, 1);
    assert!(calls.iter().all(|(.., text)| !text.contains(&value)));
}

#[tokio::test]
async fn a_field_of_the_wrong_kind_is_refused_before_typing() {
    use bruhs_models::crew::{
        FILL_FAILED, FILL_GET_FORM, LOGIN_WRONG_ORIGIN, SECRET_FIELD_MISMATCH, SUBMIT_ONLY_SIGNS_IN,
    };
    use bruhs_models::vault::SecretField;
    use serde_json::json;
    for (why, refusal) in [
        ("kind", ToolError::Invalid(SECRET_FIELD_MISMATCH.into())),
        ("get", ToolError::Refused(FILL_GET_FORM.into())),
        ("origin", ToolError::Refused(LOGIN_WRONG_ORIGIN.into())),
        ("missing", ToolError::Invalid(FILL_FAILED.into())),
        ("editable", ToolError::Invalid(FILL_FAILED.into())),
        ("submit", ToolError::Refused(SUBMIT_ONLY_SIGNS_IN.into())),
    ] {
        let value = vault_canary();
        let mut replies = sign_in_page();
        replies.push((
            "Runtime.callFunctionOn",
            answer(json!({"ok": false, "why": why})),
        ));
        let (mut cdp, server) = fake_page_replying(replies).await;
        let mut filled = computer::FilledDocuments::default();
        let error = browser::fill_secret(
            &mut cdp,
            "#pw",
            SecretField::Password,
            "https://shop.test",
            &value,
            true,
            &mut filled,
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), refusal.to_string(), "{why}");
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&refusal)
        );
        drop(cdp);
        let calls = server.await.unwrap();
        let values = calls
            .iter()
            .filter(|(method, ..)| method == "Runtime.callFunctionOn")
            .count();
        assert_eq!(values, 1, "{why}: no value call");
        // Nothing was typed, so the page still takes screenshots.
        assert!(filled.is_empty(), "{why}");
        assert!(calls.iter().all(|(.., text)| !text.contains(&value)));
        assert!(calls
            .iter()
            .all(|(method, ..)| method != "Input.dispatchKeyEvent"));
    }
}

#[tokio::test]
async fn chrome_errors_after_typing_never_travel() {
    use bruhs_models::crew::FILL_FAILED;
    use bruhs_models::vault::SecretField;
    use serde_json::json;
    let value = vault_canary();
    let failures = [
        json!({"error": {"code": -32000, "message": format!("Cannot find context for {value}")}}),
        json!({"result": {"result": {"type": "undefined"}, "exceptionDetails": {"text": format!("Uncaught {value}")}}}),
        answer(json!({"ok": false, "why": value.clone()})),
    ];
    for failure in failures {
        let mut replies = sign_in_page();
        replies.push(("Runtime.callFunctionOn", answer(json!({"ok": true}))));
        replies.push(("Runtime.callFunctionOn", failure));
        let (mut cdp, _server) = fake_page_replying(replies).await;
        let mut filled = computer::FilledDocuments::default();
        let error = browser::fill_secret(
            &mut cdp,
            "#pw",
            SecretField::Password,
            "https://shop.test",
            &value,
            false,
            &mut filled,
        )
        .await
        .unwrap_err();
        assert!(matches!(&error, ToolError::Invalid(message) if message == FILL_FAILED));
        assert!(!format!("{error} {error:?}").contains(&value));
        // A failed write may still have typed: the document counts as filled.
        assert!(filled.holds_any(&["loader:L1".into()]));
    }
    // A Chrome error before typing reads the same.
    let (mut cdp, _server) = fake_page_replying(vec![(
        "Page.getNavigationHistory",
        json!({"error": {"code": -32000, "message": "Not attached"}}),
    )])
    .await;
    let error = browser::fill_secret(
        &mut cdp,
        "#pw",
        SecretField::Password,
        "https://shop.test",
        &value,
        false,
        &mut computer::FilledDocuments::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, ToolError::Invalid(message) if message == FILL_FAILED));
}

/// A bruh's page keeps no network or console events: a submitted sign-in
/// form's body would sit in a `Network.requestWillBeSent` event.
#[tokio::test]
async fn a_bruh_page_buffers_no_request_bodies() {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message;
    let value = vault_canary();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = format!("p={value}");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let request: serde_json::Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            for event in [
                json!({"method":"Network.requestWillBeSent","params":{"request":{"url":"https://shop.test/in","postData":body}}}),
                json!({"method":"Runtime.consoleAPICalled","params":{"args":[{"value":body}]}}),
            ] {
                socket
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .unwrap();
            }
            let reply = json!({"id":request["id"],"result":{}});
            socket
                .send(Message::Text(reply.to_string().into()))
                .await
                .unwrap();
        }
    });
    let endpoint = format!("ws://{addr}");
    let mut thread = browser::Cdp::connect(&endpoint, ProcessPolicy::default())
        .await
        .unwrap();
    thread.call("Runtime.enable", json!({})).await.unwrap();
    assert_eq!(thread.buffered_events(), 2);
    let mut bruh = browser::Cdp::connect(&endpoint, ProcessPolicy::default())
        .await
        .unwrap();
    bruh.forget_page_events();
    bruh.call("Runtime.enable", json!({})).await.unwrap();
    assert_eq!(bruh.buffered_events(), 0);
    server.await.unwrap();
}

/// A launch goes ahead only when the guard itself answered its probe: a
/// browser going direct (a name error) or through another proxy (its own
/// page) is refused before any real page loads.
#[tokio::test]
async fn a_launch_needs_the_guards_own_reply_to_its_probe() {
    use serde_json::json;
    let (mut cdp, server) = fake_page(vec![json!("Bruhs network guard\n")]).await;
    browser::probe_guard(&mut cdp).await.unwrap();
    drop(cdp);
    let calls = server.await.unwrap();
    assert_eq!(calls[0].0, "Page.navigate");
    assert_eq!(calls[0].1["url"], "http://bruhs-guard-probe.invalid/");
    let refused = |result: Result<(), ToolError>| {
        matches!(result, Err(ToolError::Unavailable(message))
            if message == bruhs_models::crew::PROXY_POLICY_OVERRIDE)
    };
    let (mut cdp, _server) = fake_page(vec![json!("Bad gateway from corp-proxy")]).await;
    assert!(refused(browser::probe_guard(&mut cdp).await));
    let (mut cdp, _server) = fake_page_answering(
        Vec::new(),
        vec![(
            "Page.navigate",
            json!({"frameId":"f","errorText":"net::ERR_NAME_NOT_RESOLVED"}),
        )],
    )
    .await;
    assert!(refused(browser::probe_guard(&mut cdp).await));
}

/// A thread browser never fills from the vault or lists vault logins, and
/// refuses them before anything reaches the page.
#[tokio::test]
async fn thread_browsers_refuse_vault_fills_and_logins() {
    use bruhs_models::crew::{FILL_NEEDS_VALUE, SUBMIT_NEEDS_SECRET, VAULT_FOR_BRUHS};
    use bruhs_models::vault::SecretField;
    let (mut cdp, server) = fake_page(Vec::new()).await;
    let secret = BrowserAction::Fill {
        selector: "#p".into(),
        text: None,
        secret: Some(SecretRef {
            login_id: "v1-a".into(),
            field: SecretField::Password,
        }),
        submit: true,
    };
    let text_submit = BrowserAction::Fill {
        selector: "#p".into(),
        text: Some("x".into()),
        secret: None,
        submit: true,
    };
    let empty = BrowserAction::Fill {
        selector: "#p".into(),
        text: None,
        secret: None,
        submit: false,
    };
    for (action, expected) in [
        (secret, VAULT_FOR_BRUHS),
        (BrowserAction::Logins, VAULT_FOR_BRUHS),
        (text_submit, SUBMIT_NEEDS_SECRET),
        (empty, FILL_NEEDS_VALUE),
    ] {
        match browser::act(&mut cdp, action, COMPUTER_VIEWPORT).await {
            Err(ToolError::Invalid(reason)) => assert_eq!(reason, expected),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
    drop(cdp);
    assert_eq!(server.await.unwrap(), Vec::new());
}

/// A snapshot is a read: it gives the page no user activation, which would
/// let it open a popup or start a download. Input still does.
#[tokio::test]
async fn a_snapshot_gives_the_page_no_user_gesture() {
    use serde_json::json;
    let (mut cdp, server) = fake_page(vec![json!({"title":"t"}), json!({"filled":true})]).await;
    browser::act(&mut cdp, BrowserAction::Snapshot, COMPUTER_VIEWPORT)
        .await
        .unwrap();
    browser::act(
        &mut cdp,
        BrowserAction::Fill {
            selector: "#q".into(),
            text: Some("hi".into()),
            secret: None,
            submit: false,
        },
        COMPUTER_VIEWPORT,
    )
    .await
    .unwrap();
    drop(cdp);
    let gestures: Vec<_> = server
        .await
        .unwrap()
        .into_iter()
        .filter(|(method, _)| method == "Runtime.evaluate")
        .map(|(_, params)| params["userGesture"].clone())
        .collect();
    assert_eq!(gestures, [json!(false), json!(true)]);
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
    // Chrome kept the sealed prefs when it wrote the profile.
    let prefs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join("Default/Preferences")).unwrap())
            .unwrap();
    assert_eq!(prefs["credentials_enable_service"], false, "{prefs}");
    assert_eq!(prefs["profile"]["password_manager_enabled"], false);
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

/// A real Chrome behind the guard passes its probe; one whose traffic goes
/// to another proxy, as an organization's policy would send it, is refused
/// and stopped before it loads a page.
#[tokio::test]
#[ignore = "launches headless Chrome twice; run explicitly for local acceptance"]
async fn live_a_launch_whose_traffic_misses_the_guard_is_refused() {
    use browser::{Browser, LaunchOptions};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let home = tempfile::tempdir().unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let start = || BrowserStart {
        grant_id: "live".into(),
        url: "about:blank".into(),
    };
    let guard = network::NetworkGuard::start(network::Upstream::default()).unwrap();
    let options = |proxy| LaunchOptions {
        profile: Some(home.path().join("bruhs").join("a").join("browser")),
        headless: true,
        proxy: Some(proxy),
    };
    let mut guarded = Browser::start(
        &client,
        start(),
        ProcessPolicy::default(),
        &options(guard.addr()),
    )
    .await
    .unwrap();
    let grace = std::time::Duration::from_millis(bruhs_models::crew::CLOSE_GRACE_MS);
    guarded.close(grace).await.unwrap();
    drop(guarded);
    // Another proxy: it answers every request with its own error page.
    let decoy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let decoy_addr = decoy.local_addr().unwrap();
    let decoy_task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = decoy.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 9\r\nConnection: close\r\n\r\ncorporate")
                    .await;
            });
        }
    });
    let refused = Browser::start(
        &client,
        start(),
        ProcessPolicy::default(),
        &options(decoy_addr),
    )
    .await;
    match refused {
        Err(ToolError::Unavailable(message)) => {
            assert_eq!(message, bruhs_models::crew::PROXY_POLICY_OVERRIDE);
        }
        Err(error) => panic!("refused for another reason: {error}"),
        Ok(_) => panic!("a browser that missed the guard opened"),
    }
    decoy_task.abort();
}

/// A real page that closes itself leaves Chrome running: the browser then
/// knows its page is gone instead of failing every step forever.
#[tokio::test]
#[ignore = "launches headless Chrome; run explicitly for local acceptance"]
async fn live_a_page_that_closes_itself_is_known_gone() {
    use browser::{Browser, LaunchOptions};
    let home = tempfile::tempdir().unwrap();
    let options = LaunchOptions {
        profile: Some(home.path().join("bruhs").join("a").join("browser")),
        headless: true,
        proxy: None,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let start = BrowserStart {
        grant_id: "live".into(),
        url: "about:blank".into(),
    };
    let policy = ProcessPolicy::default();
    let mut browser = Browser::start(&client, start, policy, &options)
        .await
        .unwrap();
    assert!(!browser.gone());
    let _ = browser
        .command(
            BrowserAction::Evaluate {
                expression: "setTimeout(() => window.close(), 0); true".into(),
            },
            policy,
        )
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(browser
        .command(BrowserAction::Snapshot, policy)
        .await
        .is_err());
    assert!(browser.gone(), "the closed page is known gone");
    assert!(
        browser.exit_status().await.is_none(),
        "Chrome itself lives on"
    );
    browser
        .close(std::time::Duration::from_millis(
            bruhs_models::crew::CLOSE_GRACE_MS,
        ))
        .await
        .unwrap();
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

/// Sites a live browser reaches through this fixture as its proxy, by
/// host: `shop.test`, a sign-in page posting its form, whose script copies
/// the password into its `value` attribute (as React's controlled inputs
/// do), plus sign-in forms that send with GET; and `evil.test`, with an
/// autofocused password field. Every request it got is recorded with its
/// query and body.
pub(super) async fn vault_sites() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    use axum::{extract::Request, response::Html};
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = std::sync::Arc::clone(&seen);
    let app = axum::Router::new().fallback(move |request: Request| {
        let log = std::sync::Arc::clone(&log);
        async move {
            let (parts, body) = request.into_parts();
            let host = parts
                .uri
                .host()
                .map(str::to_owned)
                .or_else(|| {
                    let header = parts.headers.get("host")?;
                    Some(header.to_str().ok()?.to_owned())
                })
                .unwrap_or_default();
            let path = parts
                .uri
                .path_and_query()
                .map_or_else(String::new, ToString::to_string);
            let body = axum::body::to_bytes(body, 1 << 16)
                .await
                .unwrap_or_default();
            log.lock()
                .unwrap()
                .push(format!("{host}{path} {}", String::from_utf8_lossy(&body)));
            let page = match (host.as_str(), parts.uri.path()) {
                ("bruhs-guard-probe.invalid", _) => network::GUARD_PROBE_REPLY,
                ("shop.test", "/get-login") => concat!(
                    "<title>Sign in</title><form action=/done>",
                    "<input id=p type=password name=p><button>Sign in</button></form>"
                ),
                ("shop.test", "/formmethod") => concat!(
                    "<title>Sign in</title><form method=post action=/done>",
                    "<input id=p type=password name=p>",
                    "<button formmethod=get>Sign in</button></form>"
                ),
                ("shop.test", "/get-explicit") => concat!(
                    "<title>Sign in</title><form method=GET action=/done>",
                    "<input id=p type=password name=p><button>Sign in</button></form>"
                ),
                ("shop.test", "/login") => concat!(
                    "<title>Sign in</title><form method=post action=/done>",
                    "<input id=u name=u><input id=p type=password name=p><input id=c name=c>",
                    "<button>Sign in</button></form><a id=next href=/other>Next</a><script>",
                    "const p=document.getElementById('p');",
                    "p.addEventListener('input',e=>{p.setAttribute('value',p.value);",
                    "document.body.dataset.trusted=String(e.isTrusted)})</script>"
                ),
                ("shop.test", "/other") => {
                    "<title>Other</title><button id=back onclick=history.back()>Back</button>"
                }
                ("shop.test", "/rename") => concat!(
                    "<title>Settings</title><form method=post action=/done>",
                    "<input id=n name=n><button>Rename</button></form>"
                ),
                ("shop.test", "/first-step") => concat!(
                    "<title>Sign in</title><form method=post action=/done>",
                    "<input id=u name=u autocomplete=username><button>Next</button></form>"
                ),
                ("shop.test", "/done") => "<title>Done</title>",
                ("evil.test", _) => "<title>Evil</title><input id=p type=password autofocus>",
                _ => "<title>Nothing here</title>",
            };
            Html(page)
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, seen, server)
}

/// A real vault fill lands in its field through untrusted events, refuses
/// the wrong kind of field, another origin and a GET form in the page
/// itself, and submits a POST form. A form that would send with GET never
/// submits the value. A navigation between the check and the write, to
/// another origin or to a new document on the same one, types nothing.
#[tokio::test]
#[ignore = "launches headless Chrome on a temp profile; run explicitly for local acceptance"]
async fn live_a_vault_fill_lands_and_a_navigation_mid_fill_types_nothing() {
    use browser::{Browser, LaunchOptions};
    use bruhs_models::crew::{
        FILL_FAILED, FILL_GET_FORM, LOGIN_WRONG_ORIGIN, SECRET_FIELD_MISMATCH, SUBMIT_ONLY_SIGNS_IN,
    };
    use bruhs_models::vault::SecretField;
    use serde_json::json;
    let (sites, seen, server) = vault_sites().await;
    let home = tempfile::tempdir().unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let options = LaunchOptions {
        profile: Some(home.path().join("bruhs").join("a").join("browser")),
        headless: true,
        proxy: Some(sites),
    };
    let policy = ProcessPolicy::default();
    let start = BrowserStart {
        grant_id: "live".into(),
        url: "http://shop.test/login".into(),
    };
    let mut browser = Browser::start(&client, start, policy, &options)
        .await
        .unwrap();
    let evaluate = |expression: String| BrowserAction::Evaluate { expression };
    let value = vault_canary();
    let origin = "http://shop.test";
    let mut filled = computer::FilledDocuments::default();
    let reply = browser
        .fill_secret(
            "#p",
            SecretField::Password,
            origin,
            &value,
            false,
            &mut filled,
        )
        .await
        .unwrap();
    assert_eq!(
        reply.value,
        json!({"filled": "password", "submitted": false})
    );
    assert!(!filled.is_empty());
    // It landed, the page mirrored it into the attribute a selector could
    // probe, and the page saw untrusted events.
    let landed = browser
        .command(
            evaluate(format!(
                "[document.querySelector('#p').value==='{value}',document.querySelector('#p[value=\"{value}\"]')!==null,document.body.dataset.trusted]"
            )),
            policy,
        )
        .await
        .unwrap();
    assert_eq!(landed.value, json!([true, true, "false"]));
    // The page itself refuses the wrong kind of field and another origin.
    let refusal = |result: Result<ToolReply, ToolError>| match result {
        Err(ToolError::Invalid(message) | ToolError::Refused(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    };
    for (selector, field, origin, expected) in [
        ("#u", SecretField::Password, origin, SECRET_FIELD_MISMATCH),
        ("#p", SecretField::Username, origin, SECRET_FIELD_MISMATCH),
        (
            "#p",
            SecretField::Password,
            "http://evil.test",
            LOGIN_WRONG_ORIGIN,
        ),
        ("#nope", SecretField::Password, origin, FILL_FAILED),
    ] {
        let other = vault_canary();
        let result = browser
            .fill_secret(selector, field, origin, &other, false, &mut filled)
            .await;
        assert_eq!(refusal(result), expected, "{selector}");
    }
    // A submit presses Enter, and the form posts to its own site.
    browser
        .fill_secret(
            "#p",
            SecretField::Password,
            origin,
            &value,
            true,
            &mut filled,
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !seen
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.starts_with("shop.test/done ") && request.contains(&value))
        {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the form was posted with the value");
    let go = |url: &str| BrowserAction::Navigate { url: url.into() };
    // Ruling A2 I1: submit only signs in. A form with no password field
    // and no sign-in autocomplete is refused before anything is typed; a
    // username step marked as one submits.
    browser
        .command(go("http://shop.test/rename"), policy)
        .await
        .unwrap();
    let renamed = vault_canary();
    let result = browser
        .fill_secret(
            "#n",
            SecretField::Username,
            origin,
            &renamed,
            true,
            &mut filled,
        )
        .await;
    assert_eq!(refusal(result), SUBMIT_ONLY_SIGNS_IN);
    let untouched = browser
        .command(
            evaluate("[document.querySelector('#n').value,location.pathname]".into()),
            policy,
        )
        .await
        .unwrap();
    assert_eq!(untouched.value, json!(["", "/rename"]));
    browser
        .command(go("http://shop.test/first-step"), policy)
        .await
        .unwrap();
    let username = vault_canary();
    browser
        .fill_secret(
            "#u",
            SecretField::Username,
            origin,
            &username,
            true,
            &mut filled,
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !seen
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.starts_with("shop.test/done ") && request.contains(&username))
        {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the username step was posted");
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|request| !request.contains(&renamed)));
    // A form that declares GET is refused before anything is typed.
    browser
        .command(go("http://shop.test/get-explicit"), policy)
        .await
        .unwrap();
    let declared = vault_canary();
    let result = browser
        .fill_secret(
            "#p",
            SecretField::Password,
            origin,
            &declared,
            true,
            &mut filled,
        )
        .await;
    assert_eq!(refusal(result), FILL_GET_FORM);
    // A form that sends with GET by default, or through its button, never
    // submits the value: the submit is cancelled.
    for page in ["/get-login", "/formmethod"] {
        browser
            .command(go(&format!("http://shop.test{page}")), policy)
            .await
            .unwrap();
        let held = vault_canary();
        browser
            .fill_secret(
                "#p",
                SecretField::Password,
                origin,
                &held,
                true,
                &mut filled,
            )
            .await
            .unwrap();
        let clicked = browser
            .command(
                BrowserAction::Click {
                    selector: "button".into(),
                },
                policy,
            )
            .await
            .unwrap();
        assert_eq!(clicked.value["clicked"], true);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let stayed = browser
            .command(
                evaluate("[location.pathname,location.search]".into()),
                policy,
            )
            .await
            .unwrap();
        assert_eq!(stayed.value, json!([page, ""]), "{page}");
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .all(|request| !request.contains(&held)),
            "{page}"
        );
    }
    // A new document on the same origin commits between the check and the
    // write: the world died with the old one.
    browser
        .command(go("http://shop.test/login"), policy)
        .await
        .unwrap();
    let reloaded = vault_canary();
    let error = browser
        .fill_across_a_navigation(
            "#p",
            SecretField::Password,
            origin,
            &reloaded,
            "http://shop.test/login",
            &mut filled,
        )
        .await
        .unwrap_err();
    assert!(matches!(&error, ToolError::Invalid(message) if message == FILL_FAILED));
    let fresh = browser
        .command(
            evaluate("document.querySelector('#p').value".into()),
            policy,
        )
        .await
        .unwrap();
    assert_eq!(fresh.value, "");
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|request| !request.contains(&reloaded)));
    // An in-flight navigation to another origin commits there.
    let moved = vault_canary();
    let error = browser
        .fill_across_a_navigation(
            "#p",
            SecretField::Password,
            origin,
            &moved,
            "http://evil.test/",
            &mut filled,
        )
        .await
        .unwrap_err();
    assert!(matches!(&error, ToolError::Invalid(message) if message == FILL_FAILED));
    assert!(!format!("{error} {error:?}").contains(&moved));
    let evil = browser
        .command(
            evaluate("[location.host,document.querySelector('#p').value]".into()),
            policy,
        )
        .await
        .unwrap();
    assert_eq!(evil.value, json!(["evil.test", ""]));
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|request| !request.contains(&moved)));
    browser
        .close(std::time::Duration::from_millis(
            bruhs_models::crew::CLOSE_GRACE_MS,
        ))
        .await
        .unwrap();
    server.abort();
}
