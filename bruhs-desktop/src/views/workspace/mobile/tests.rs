use super::{
    AppView, Connection, DesktopState, Message, MessageRole, MobileAuthority, MobileHost,
    MobileHostConfig, MobileLaunch, MobileTransport, PermissionMode, TurnState, Workspace,
};
use bruhs_models::mobile::*;
use gpui::{Entity, TestAppContext, VisualTestContext};
use serde_json::Value;
use std::{
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};

async fn runtime<T: Send + 'static>(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    future: impl Future<Output = T> + Send + 'static,
) -> T {
    let receive = workspace.update_in(cx, |_, _, cx| {
        let (send, receive) = tokio::sync::oneshot::channel();
        cx.global::<DesktopState>().runtime.spawn(async move {
            let _ = send.send(future.await);
        });
        receive
    });
    receive.await.unwrap()
}
fn message(text: &str, streaming: bool) -> Message {
    Message {
        role: MessageRole::Assistant,
        content: text.into(),
        model: None,
        thinking: "PRIVATE_PHONE_THINKING_CANARY".into(),
        usage: None,
        duration_ms: None,
        started_at_ms: Some(1),
        completed_at_ms: None,
        tool_calls: vec![],
        files: vec![],
        state: if streaming {
            TurnState::Streaming
        } else {
            TurnState::Finished
        },
        checkpoint: None,
    }
}
fn seed(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    path: &Path,
) -> (String, String) {
    workspace.update_in(cx, |w, window, cx| {
        w.threads.clear();
        for name in ["Native phone thread", "Second native thread"] {
            let thread = Workspace::create_thread_with_title(
                "Mobile fixture".into(),
                path.to_string_lossy().into(),
                "gpt-5.4".into(),
                PermissionMode::Default,
                name,
                window,
                cx,
            );
            thread.chat.update(cx, |chat, cx| {
                chat.messages = (0..60)
                    .map(|i| message(&format!("Completed native message {i}"), false))
                    .collect();
                chat.messages
                    .push(message("Actual native live output", true));
                chat.seed_mobile_fixture(window, cx);
                assert!(chat.has_private_input());
                let projected = chat
                    .mobile_snapshot(
                        MobileThread {
                            id: thread.id.clone(),
                            title: name.into(),
                            project: "Mobile fixture".into(),
                            updated_at_ms: 1,
                            state: chat.mobile_state(),
                        },
                        None,
                    )
                    .unwrap();
                assert!(!serde_json::to_string(&projected)
                    .unwrap()
                    .contains("PRIVATE_PHONE_"));
            });
            w.threads.push(thread);
        }
        let private = Workspace::create_thread_with_title(
            String::new(),
            path.to_string_lossy().into(),
            "gpt-5.4".into(),
            PermissionMode::Default,
            "PRIVATE_NONPROJECT_CHAT_CANARY",
            window,
            cx,
        );
        w.threads.push(private);
        w.active_view = AppView::Automations;
        (w.threads[0].id.clone(), w.threads[1].id.clone())
    })
}
async fn connect(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    root: &Path,
) -> (String, MobileAuthority, reqwest::Client) {
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bruhs-core/tests/fixtures/mobile");
    let path = root.join("mobile/devices.json");
    let (host, requests) = runtime(workspace, cx, async move {
        let authority = MobileAuthority::open(path, MobileLaunch::isolated())
            .await
            .unwrap();
        MobileHost::start(
            MobileHostConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                origin: "https://localhost:0/".parse().unwrap(),
                transport: MobileTransport::DirectTls {
                    certificate: fixtures.join("localhost.pem"),
                    private_key: fixtures.join("localhost-test-key.pem"),
                },
            },
            authority,
        )
        .await
        .unwrap()
    })
    .await;
    let origin = host.origin().to_string().trim_end_matches('/').to_owned();
    let authority = host.authority();
    workspace.update_in(cx, |w, _, cx| {
        w.attach_mobile(uuid::Uuid::new_v4(), host, requests, cx)
    });
    let ca = reqwest::Certificate::from_pem(include_bytes!(
        "../../../../../bruhs-core/tests/fixtures/mobile/ca.pem"
    ))
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(ca)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    (origin, authority, client)
}
async fn read(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    path: &str,
) -> Value {
    let request = client.get(format!("{origin}{path}")).bearer_auth(token);
    runtime(workspace, cx, async move {
        let response = request.send().await.unwrap();
        assert!(
            response.status().is_success(),
            "{}",
            response.text().await.unwrap()
        );
        response.json().await.unwrap()
    })
    .await
}
async fn send(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    sequence: u64,
    action: MobileAction,
) -> MobileCommandReceipt {
    let command = MobileCommand {
        id: uuid::Uuid::new_v4(),
        sequence,
        action,
    };
    let request = client
        .post(format!("{origin}/api/mobile/command"))
        .bearer_auth(token)
        .json(&command);
    runtime(workspace, cx, async move {
        request
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    })
    .await
}
#[gpui::test]
async fn mobile_real_https_broker_uses_native_messages_queue_and_exact_permission(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = super::super::performance_tests::fixture(cx);
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let (id, _) = seed(&workspace, cx, root.path());
    let (origin, authority, client) = connect(&workspace, cx, root.path()).await;
    let offer = authority
        .begin_pairing(
            &origin.parse().unwrap(),
            [
                MobileScope::Read,
                MobileScope::Prompt,
                MobileScope::Approve,
                MobileScope::Diff,
            ]
            .into(),
            Duration::from_secs(60),
        )
        .unwrap();
    let secret = reqwest::Url::parse(&offer.url)
        .unwrap()
        .fragment()
        .unwrap()
        .strip_prefix("pair=")
        .unwrap()
        .to_owned();
    let request = client
        .post(format!("{origin}/api/mobile/pair"))
        .json(&MobilePairInput {
            secret,
            name: "Native broker fixture".into(),
        });
    let issued: MobilePairResult = runtime(&workspace, cx, async move {
        request.send().await.unwrap().json().await.unwrap()
    })
    .await;
    let list = read(
        &workspace,
        cx,
        &client,
        &origin,
        &issued.token,
        "/api/mobile/threads",
    )
    .await;
    assert_eq!(list["value"]["threads"].as_array().unwrap().len(), 2);
    assert!(!list.to_string().contains("PRIVATE_NONPROJECT"));
    let snapshot = read(
        &workspace,
        cx,
        &client,
        &origin,
        &issued.token,
        &format!("/api/mobile/thread?thread_id={id}"),
    )
    .await;
    let raw = snapshot.to_string();
    assert!(raw.contains("Actual native live output"));
    assert!(!raw.contains("PRIVATE_PHONE_"));
    assert!(snapshot["value"]["laptop_action_required"]
        .as_bool()
        .unwrap());
    assert!(snapshot["value"]["next_before"].as_u64().unwrap() > 0);
    assert!(raw.len() <= RESPONSE_BYTES);
    let permission: MobilePermission =
        serde_json::from_value(snapshot["value"]["permission"].clone()).unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.threads[0]
            .chat
            .update(cx, |chat, _| chat.mobile_fixture_change_permission(false))
    });
    let stale = send(
        &workspace,
        cx,
        &client,
        &origin,
        &issued.token,
        1,
        MobileAction::Permission {
            thread_id: id.clone(),
            turn_generation: permission.turn_generation,
            request_id: permission.request_id.clone(),
            review_hash: permission.review_hash.clone(),
            decision: MobileDecision::Allow,
        },
    )
    .await;
    assert!(matches!(
        stale.outcome,
        MobileCommandOutcome::Rejected { .. }
    ));
    let current = read(
        &workspace,
        cx,
        &client,
        &origin,
        &issued.token,
        &format!("/api/mobile/thread?thread_id={id}"),
    )
    .await;
    let permission: MobilePermission =
        serde_json::from_value(current["value"]["permission"].clone()).unwrap();
    let allowed = send(
        &workspace,
        cx,
        &client,
        &origin,
        &issued.token,
        2,
        MobileAction::Permission {
            thread_id: id.clone(),
            turn_generation: permission.turn_generation,
            request_id: permission.request_id,
            review_hash: permission.review_hash,
            decision: MobileDecision::Allow,
        },
    )
    .await;
    assert!(matches!(
        allowed.outcome,
        MobileCommandOutcome::Accepted { .. }
    ));
    let queued = send(
        &workspace,
        cx,
        &client,
        &origin,
        &issued.token,
        3,
        MobileAction::Prompt {
            thread_id: id.clone(),
            text: "Queue from the phone".into(),
        },
    )
    .await;
    assert!(matches!(
        queued.outcome,
        MobileCommandOutcome::Accepted { queued: true }
    ));
    workspace.update_in(cx, |w, _, cx| {
        let chat = w.threads[0].chat.read(cx);
        assert!(!chat.has_pending_permission());
        assert_eq!(chat.prompt_queue.for_thread(&id).len(), 1);
        assert_eq!(chat.mobile_fixture_draft(cx), "laptop unsent draft");
    });
    authority.revoke_now(issued.device.id).unwrap();
    let persist = authority.clone();
    runtime(&workspace, cx, async move {
        persist.persist_revocation().await.unwrap();
    })
    .await;
    let request = client
        .get(format!("{origin}/api/mobile/session"))
        .bearer_auth(&issued.token);
    assert_eq!(
        runtime(&workspace, cx, async move {
            request.send().await.unwrap().status()
        })
        .await,
        reqwest::StatusCode::UNAUTHORIZED
    );
    workspace.update_in(cx, |w, _, cx| w.stop_mobile(cx));
    cx.condition(&workspace, |w, _| {
        matches!(w.mobile.connection, Connection::Off)
    })
    .await;
}

#[gpui::test]
fn mobile_native_projection_pages_live_text_and_rejects_local_racing_or_incomplete_approvals(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = super::super::performance_tests::fixture(cx);
    let root = tempfile::tempdir().unwrap();
    let (id, _) = seed(&workspace, cx, root.path());
    workspace.update_in(cx, |w, _, cx| {
        let summary = Workspace::mobile_summary(&w.threads[0], cx);
        w.threads[0].chat.update(cx, |chat, cx| {
            let permission = chat
                .mobile_snapshot(summary.clone(), None)
                .unwrap()
                .permission
                .unwrap();
            let old = MobileAction::Permission {
                thread_id: id.clone(),
                turn_generation: permission.turn_generation,
                request_id: permission.request_id.clone(),
                review_hash: permission.review_hash,
                decision: MobileDecision::Allow,
            };
            chat.mobile_fixture_local_answer(cx);
            chat.mobile_fixture_change_permission(false);
            assert!(chat.mobile_action(&old, cx).is_err());
            assert!(chat.has_pending_permission());
            let fresh = chat
                .mobile_snapshot(summary.clone(), None)
                .unwrap()
                .permission
                .unwrap();
            let fresh_action = MobileAction::Permission {
                thread_id: id.clone(),
                turn_generation: fresh.turn_generation,
                request_id: fresh.request_id,
                review_hash: fresh.review_hash,
                decision: MobileDecision::Allow,
            };
            chat.mobile_fixture_advance_turn();
            assert!(chat.mobile_action(&fresh_action, cx).is_err());
            assert!(chat.has_pending_permission());
            chat.mobile_fixture_change_permission(true);
            let large = chat
                .mobile_snapshot(summary.clone(), None)
                .unwrap()
                .permission
                .unwrap();
            assert!(!large.can_allow);
            let mut action = MobileAction::Permission {
                thread_id: id.clone(),
                turn_generation: large.turn_generation,
                request_id: large.request_id,
                review_hash: large.review_hash,
                decision: MobileDecision::Allow,
            };
            assert!(chat.mobile_action(&action, cx).is_err());
            if let MobileAction::Permission { decision, .. } = &mut action {
                *decision = MobileDecision::Deny;
            }
            assert!(matches!(
                chat.mobile_action(&action, cx),
                Ok(MobileCommandOutcome::Accepted { .. })
            ));
            assert!(!chat.has_pending_permission());
            let content = format!("{}TAIL_MARKER", "東京\n".repeat(TEXT_BYTES));
            chat.messages.last_mut().unwrap().content = content.clone();
            let snapshot = chat.mobile_snapshot(summary, None).unwrap();
            assert!(serde_json::to_vec(&snapshot).unwrap().len() <= RESPONSE_BYTES);
            let row = snapshot.messages.last().unwrap();
            assert!(row.text.ends_with("TAIL_MARKER"));
            assert!(row.byte_offset > 0);
            assert!(row.streaming);
            let first = chat
                .mobile_message(row.index, &row.reference, 0, None)
                .unwrap();
            let mut restored = first.text.clone();
            let mut next = first.next_offset;
            while let Some(offset) = next {
                let page = chat
                    .mobile_message(row.index, &row.reference, offset, Some(&first.revision))
                    .unwrap();
                restored.push_str(&page.text);
                next = page.next_offset;
            }
            assert_eq!(restored, content);
            chat.messages
                .last_mut()
                .unwrap()
                .content
                .replace_range(0..6, "大阪");
            assert!(chat
                .mobile_message(
                    row.index,
                    &row.reference,
                    first.next_offset.unwrap(),
                    Some(&first.revision)
                )
                .is_err());
            assert_eq!(chat.mobile_fixture_draft(cx), "laptop unsent draft");
        });
    });
}

#[gpui::test]
#[ignore = "requires a pinned Chromium/Playwright installation; runs a real phone client against the native workspace"]
async fn mobile_phone_browser_drives_actual_native_workspace_over_https(cx: &mut TestAppContext) {
    let (workspace, cx) = super::super::performance_tests::fixture(cx);
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("checkout");
    std::fs::create_dir(&project).unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .current_dir(&project)
            .args([
                "-c",
                "user.name=Mobile fixture",
                "-c",
                "user.email=mobile@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    };
    git(&["init", "--quiet"]);
    std::fs::write(project.join("source.rs"), "native fixture before\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "Mobile fixture"]);
    std::fs::write(
        project.join("source.rs"),
        format!(
            "native fixture changed\n{}",
            "native changed line 東京\n".repeat(1000)
        ),
    )
    .unwrap();
    let (first, _) = seed(&workspace, cx, &project);
    let (origin, authority, _) = connect(&workspace, cx, root.path()).await;
    let offer = authority
        .begin_pairing(
            &origin.parse().unwrap(),
            [
                MobileScope::Read,
                MobileScope::Prompt,
                MobileScope::Approve,
                MobileScope::Diff,
            ]
            .into(),
            Duration::from_secs(300),
        )
        .unwrap();
    let fixture_path = root.path().join("fixture.json");
    let control = root.path().join("controls");
    std::fs::create_dir(&control).unwrap();
    let certificate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../bruhs-core/tests/fixtures/mobile/localhost.pem");
    std::fs::write(&fixture_path,serde_json::to_vec(&serde_json::json!({"origin":origin,"pairing":offer.url.as_str(),"first":first,"control":control,"certificate":certificate})).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fixture_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let watched_control = control.clone();
    let watched_authority = authority.clone();
    let watched_origin = origin.clone();
    let watch=workspace.update_in(cx,|_,_,cx|cx.spawn(async move|this,cx|{
        let mut live=false;let mut offered=false;let mut revoked=false;
        loop {
            cx.background_executor().timer(Duration::from_millis(REFRESH_MS/4)).await;
            if !live && watched_control.join("live").exists(){
                if this.update(cx,|w,cx|w.threads[0].chat.update(cx,|chat,cx|{chat.messages.last_mut().unwrap().content="Live update from actual native ChatView <img src=x onerror=window.MOBILE_XSS=1>".into();cx.notify();})).is_err(){break;}live=true;
            }
            if !offered && watched_control.join("offer-readonly").exists(){
                let offer=watched_authority.begin_pairing(&watched_origin.parse().unwrap(),[MobileScope::Read].into(),Duration::from_secs(300)).unwrap();
                std::fs::write(watched_control.join("readonly-offer.json"),serde_json::to_vec(&serde_json::json!({"url":offer.url.as_str()})).unwrap()).unwrap();offered=true;
            }
            if !revoked && watched_control.join("revoke").exists(){
                let device=watched_authority.devices().unwrap().into_iter().find(|d|d.name=="Touch phone").unwrap();watched_authority.revoke_now(device.id).unwrap();
                let authority=watched_authority.clone();let _=this.update(cx,|_,cx|{cx.global::<DesktopState>().runtime.spawn(async move{authority.persist_revocation().await.unwrap();});});revoked=true;
            }
        }
    }));
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mobile_browser.mjs");
    let output = runtime(&workspace, cx, async move {
        tokio::time::timeout(
            Duration::from_secs(8 * 15),
            tokio::process::Command::new("node")
                .env(
                    "NODE_EXTRA_CA_CERTS",
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../bruhs-core/tests/fixtures/mobile/ca.pem"),
                )
                .arg(script)
                .arg(fixture_path)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap()
    })
    .await;
    drop(watch);
    assert!(
        output.status.success(),
        "phone browser failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report["passed"].as_u64().unwrap() >= 25);
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(
            w.threads[0]
                .chat
                .read(cx)
                .prompt_queue
                .for_thread(&first)
                .len(),
            4
        );
        assert!(!w.threads[0].chat.read(cx).has_pending_permission());
        assert_eq!(
            w.threads[0].chat.read(cx).mobile_fixture_draft(cx),
            "laptop unsent draft"
        );
        assert!(w.threads[1].chat.read(cx).has_pending_permission());
    });
    assert_eq!(
        authority
            .devices()
            .unwrap()
            .into_iter()
            .filter(|d| d.revoked)
            .count(),
        2
    );
    workspace.update_in(cx, |w, _, cx| w.stop_mobile(cx));
    cx.condition(&workspace, |w, _| {
        matches!(w.mobile.connection, Connection::Off)
    })
    .await;
}
