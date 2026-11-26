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
                            needs_answer: chat.mobile_needs_answer(),
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
            let device = uuid::Uuid::new_v4();
            assert!(chat.mobile_action(&old, device, cx).is_err());
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
            assert!(chat.mobile_action(&fresh_action, device, cx).is_err());
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
            assert!(chat.mobile_action(&action, device, cx).is_err());
            if let MobileAction::Permission { decision, .. } = &mut action {
                *decision = MobileDecision::Deny;
            }
            assert!(matches!(
                chat.mobile_action(&action, device, cx),
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

#[gpui::test]
#[ignore = "requires the local Claude stdio fixture and Playwright"]
async fn mobile_phone_browser_delivers_a_structured_answer(cx: &mut TestAppContext) {
    let directory = PathBuf::from(std::env::var_os("BRUHS_MOBILE_QUESTION_FIXTURE").unwrap());
    let path = std::env::var_os("PATH").unwrap();
    assert_eq!(
        std::env::split_paths(&path).find(|dir| dir.join("claude").is_file()),
        Some(directory.join("bin")),
        "Run this test with the local Claude stdio fixture first on PATH."
    );
    let (workspace, cx) = super::super::performance_tests::fixture(cx);
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("answer-example.md"),
        format!(
            "First line before the anchor\nLinked project example\n{}\nLast example line\n",
            "x".repeat(5000)
        ),
    )
    .unwrap();
    std::fs::write(root.path().join(".env"), "FIXTURE_ONLY=private").unwrap();
    let state_path = root.path().join("workspace.json");
    let (id, chat) = workspace.update_in(cx, |w, window, cx| {
        w.persistence_test_path = Some(state_path.clone());
        w.threads.clear();
        let thread = Workspace::create_thread_with_title(
            "Phone fixture".into(),
            root.path().to_string_lossy().into(),
            "claude-sonnet-4-6".into(),
            PermissionMode::Default,
            "Phone question fixture",
            window,
            cx,
        );
        let chat = thread.chat.clone();
        let id = thread.id.clone();
        chat.update(cx, |chat, cx| {
            chat.set_composer_draft("Laptop draft".into(), window, cx)
        });
        w.threads.push(thread);
        w.active_view = AppView::Automations;
        (id, chat)
    });
    let (origin, authority, _) = connect(&workspace, cx, root.path()).await;
    let mut devices = Vec::new();
    for (name, scopes) in [
        (
            "Second fixture phone",
            [MobileScope::Read, MobileScope::Prompt].into(),
        ),
        ("Read-only fixture phone", [MobileScope::Read].into()),
    ] {
        let offer = authority
            .begin_pairing(&origin.parse().unwrap(), scopes, Duration::from_secs(90))
            .unwrap();
        let secret = reqwest::Url::parse(&offer.url)
            .unwrap()
            .fragment()
            .unwrap()
            .strip_prefix("pair=")
            .unwrap()
            .to_owned();
        let authority = authority.clone();
        let issued = runtime(&workspace, cx, async move {
            authority
                .pair(MobilePairInput {
                    secret,
                    name: name.into(),
                })
                .await
        })
        .await
        .unwrap();
        devices.push(issued.token);
    }
    let pair = authority
        .begin_pairing(
            &origin.parse().unwrap(),
            [MobileScope::Read, MobileScope::Prompt, MobileScope::Diff].into(),
            Duration::from_secs(90),
        )
        .unwrap();
    let certificate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../bruhs-core/tests/fixtures/mobile/localhost.pem");
    let path = root.path().join("fixture.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "origin": origin,
            "pairing": pair.url.as_str(),
            "thread": id,
            "certificate": certificate,
            "other_device": devices[0],
            "read_only_device": devices[1],
        }))
        .unwrap(),
    )
    .unwrap();
    chat.update(cx, |chat, cx| {
        chat.submit_prompt("Draft release notes.".into(), cx)
    });
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mobile_question_browser.mjs");
    let output = runtime(&workspace, cx, async move {
        tokio::time::timeout(
            Duration::from_secs(75),
            tokio::process::Command::new("node")
                .arg(script)
                .arg(path)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap()
    })
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_slice(&std::fs::read(directory.join("reply.json")).unwrap()).unwrap();
    let answers = &response["response"]["updatedInput"]["answers"];
    assert_eq!(
        answers["Which sections should I include?"],
        "Setup, Examples"
    );
    let answer = answers["How should the release notes read?"]
        .as_str()
        .unwrap();
    let file = answer
        .strip_prefix(
            "Concise\nMention the startup fix\n\nRead these attached files\nrelease-notes.txt ",
        )
        .unwrap();
    assert_eq!(std::fs::read(file).unwrap(), vec![b'n'; 70_000]);
    assert_eq!(
        chat.read_with(cx, |chat, cx| chat.mobile_fixture_draft(cx)),
        "Laptop draft"
    );
    cx.condition(&workspace, |w, _| {
        w.save_status.is_saved()
            && std::fs::read_to_string(&state_path).is_ok_and(|state| state.contains(file))
    })
    .await;
    chat.update(cx, |chat, cx| chat.confirm_rewind(0, cx));
    cx.condition(&workspace, |w, _| {
        w.save_status.is_saved()
            && std::fs::read_to_string(&state_path)
                .is_ok_and(|state| state.contains("retired_question_files"))
    })
    .await;
    assert!(
        Path::new(file).is_file(),
        "The previous durable workspace can still recover this answer"
    );
    workspace.update_in(cx, |w, _, cx| w.persist(cx));
    cx.condition(&workspace, |w, _| {
        w.save_status.is_saved() && !Path::new(file).exists()
    })
    .await;
    eprintln!("The rewind retired the submitted file when the recovery snapshot rotated");
    workspace.update_in(cx, |w, _, cx| w.stop_mobile(cx));
    cx.condition(&workspace, |w, _| {
        matches!(w.mobile.connection, Connection::Off)
    })
    .await;
}

#[cfg(unix)]
#[gpui::test]
async fn paired_link_resolves_project_reuses_pull_and_limits_scratch_to_its_device(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (workspace, cx) = super::super::performance_tests::fixture(cx);
    let root = tempfile::tempdir().unwrap();
    let (thread_id, _) = seed(&workspace, cx, root.path());
    let active_before = workspace.read_with(cx, |w, _| w.active_thread_id.clone());
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .current_dir(root.path())
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null"
            ])
            .args(args)
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(root.path().join("source.rs"), "fixture PR head\n").unwrap();
    git(&["add", "source.rs"]);
    git(&["commit", "-qm", "PR fixture"]);
    git(&["update-ref", "refs/pull/43/head", "HEAD"]);
    std::fs::write(root.path().join("source.rs"), "local edit\n").unwrap();
    git(&["config", "protocol.file.allow", "always"]);
    let local_remote = reqwest::Url::from_file_path(root.path()).unwrap();
    git(&[
        "config",
        &format!("url.{local_remote}.insteadOf"),
        "https://github.com/fixture/docs.git",
    ]);
    git(&["remote", "add", "origin", "git@github.com:fixture/docs.git"]);
    workspace.update_in(cx, |w, _, cx| {
        super::super::project_picker_ui::push_recent_project(
            &mut w.recent_projects,
            super::super::DesktopProjectLocation {
                name: "Mobile fixture".into(),
                path: root.path().to_string_lossy().into(),
            },
        );
        w.project_actions.insert(
            root.path().to_string_lossy().into_owned(),
            vec![bruhs_core::git::ProjectAction {
                id: "fixture-setup".into(),
                name: "Fixture setup".into(),
                command: "cp source.rs setup-proof".into(),
                shortcut: None,
                preview_url: None,
                auto_open_preview: false,
                lifecycle: bruhs_core::git::ProjectActionLifecycle {
                    run_on_worktree_create: true,
                    wait_for_setup: true,
                    ..Default::default()
                },
            }],
        );
        let thread = w
            .threads
            .iter_mut()
            .find(|thread| thread.id == thread_id)
            .unwrap();
        let branch = "bruhs/linked-phone-fixture".to_owned();
        let path = thread.project_path.clone();
        thread.branch = Some(branch.clone());
        assert!(thread.chat.read(cx).project_scoped);
        w.known_thread_pulls.insert(
            thread_id.clone(),
            super::super::pull_settlement::Observation::linked(path, branch, 42),
        );
    });
    let (origin, authority, client) = connect(&workspace, cx, root.path()).await;

    async fn pair(
        ws: &Entity<Workspace>,
        cx: &mut VisualTestContext,
        client: &reqwest::Client,
        origin: &str,
        authority: &MobileAuthority,
        scopes: MobileScopes,
        name: &str,
    ) -> MobilePairResult {
        let offer = authority
            .begin_pairing(&origin.parse().unwrap(), scopes, Duration::from_secs(60))
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
                name: name.into(),
            });
        runtime(ws, cx, async move {
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

    async fn status(
        ws: &Entity<Workspace>,
        cx: &mut VisualTestContext,
        client: &reqwest::Client,
        origin: &str,
        token: &str,
        link_id: uuid::Uuid,
    ) -> MobileLinkStatus {
        let url = format!("{origin}/api/mobile/linked-draft?link_id={link_id}");
        let request = client.get(url).bearer_auth(token);
        runtime(ws, cx, async move {
            tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let response = request
                        .try_clone()
                        .unwrap()
                        .send()
                        .await
                        .unwrap()
                        .error_for_status()
                        .unwrap()
                        .json::<MobileReadReply>()
                        .await
                        .unwrap();
                    let MobileReadReply::LinkedDraft(row) = response else {
                        panic!("unexpected response: {response:?}");
                    };
                    assert_eq!(row.link_id, link_id);
                    if row.state != MobileLinkStatus::Preparing {
                        return row.state;
                    }
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            })
            .await
            .unwrap()
        })
        .await
    }

    let opted = pair(
        &workspace,
        cx,
        &client,
        &origin,
        &authority,
        [MobileScope::Read, MobileScope::LinkedDrafts].into(),
        "Draft phone",
    )
    .await;
    let read_only = pair(
        &workspace,
        cx,
        &client,
        &origin,
        &authority,
        [MobileScope::Read].into(),
        "Read phone",
    )
    .await;
    let open =
        |pull_request: Option<&str>, repository: Option<&str>| MobileAction::OpenLinkedDraft {
            target: MobileLinkTarget {
                pull_request: pull_request.map(str::to_owned),
                repository: repository.map(str::to_owned),
                provider: None,
                model: None,
            },
        };
    let forbidden = client
        .post(format!("{origin}/api/mobile/command"))
        .bearer_auth(&read_only.token)
        .json(&MobileCommand {
            id: uuid::Uuid::new_v4(),
            sequence: 1,
            action: open(None, None),
        });
    assert_eq!(
        runtime(&workspace, cx, async move {
            forbidden.send().await.unwrap().status()
        })
        .await,
        reqwest::StatusCode::FORBIDDEN
    );
    let mut link_ids = Vec::new();
    for (sequence, action) in [
        (1, open(Some("fixture/docs#42"), None)),
        (2, open(None, Some("fixture/docs"))),
        (3, open(None, None)),
    ] {
        let ack = send(
            &workspace,
            cx,
            &client,
            &origin,
            &opted.token,
            sequence,
            action,
        )
        .await;
        assert!(matches!(
            ack.outcome,
            MobileCommandOutcome::Accepted { queued: true }
        ));
        let ready = status(&workspace, cx, &client, &origin, &opted.token, ack.id).await;
        let MobileLinkStatus::Ready { thread_id: id, .. } = ready else {
            panic!("draft did not resolve: {ready:?}");
        };
        link_ids.push(id);
    }
    assert_eq!(link_ids[0], thread_id);
    workspace.read_with(cx, |w, cx| {
        let repo = w
            .threads
            .iter()
            .find(|thread| thread.id == link_ids[1])
            .unwrap();
        assert_eq!(repo.project_name, "Mobile fixture");
        assert!(repo.chat.read(cx).messages.is_empty());
        assert_eq!(w.active_thread_id, active_before);
    });
    let scratch = format!("/api/mobile/thread?thread_id={}", link_ids[2]);
    let view = read(&workspace, cx, &client, &origin, &opted.token, &scratch).await;
    assert_eq!(view["value"]["thread"]["id"], link_ids[2]);
    assert_eq!(view["value"]["messages"].as_array().unwrap().len(), 0);
    let request = client
        .get(format!("{origin}{scratch}"))
        .bearer_auth(&read_only.token);
    assert_eq!(
        runtime(&workspace, cx, async move {
            request.send().await.unwrap().status()
        })
        .await,
        reqwest::StatusCode::CONFLICT
    );

    let checkout = send(
        &workspace,
        cx,
        &client,
        &origin,
        &opted.token,
        4,
        open(Some("fixture/docs#43"), None),
    )
    .await;
    assert!(matches!(
        status(&workspace, cx, &client, &origin, &opted.token, checkout.id).await,
        MobileLinkStatus::Checkout { error: None, .. }
    ));
    let project_path = root.path().canonicalize().unwrap();
    let branch = format!("bruhs/pr-43-{}", checkout.id);
    let dest = bruhs_core::git::worktree_dest(&project_path, &branch).unwrap();
    assert!(!dest.exists());
    let mismatched = send(
        &workspace,
        cx,
        &client,
        &origin,
        &opted.token,
        5,
        MobileAction::ContinueLinkedDraft {
            link_id: checkout.id,
            choice: MobileLinkChoice::Clone,
        },
    )
    .await;
    assert!(matches!(
        mismatched.outcome,
        MobileCommandOutcome::Rejected { .. }
    ));
    assert!(!dest.exists());
    let confirm = send(
        &workspace,
        cx,
        &client,
        &origin,
        &opted.token,
        6,
        MobileAction::ContinueLinkedDraft {
            link_id: checkout.id,
            choice: MobileLinkChoice::Checkout,
        },
    )
    .await;
    assert!(matches!(
        confirm.outcome,
        MobileCommandOutcome::Accepted { queued: true }
    ));
    let placed = status(&workspace, cx, &client, &origin, &opted.token, checkout.id).await;
    let MobileLinkStatus::Ready {
        thread_id: pull_thread,
        ..
    } = placed
    else {
        panic!("checkout failed: {placed:?}");
    };
    assert_eq!(
        std::fs::read_to_string(dest.join("setup-proof")).unwrap(),
        "fixture PR head\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("source.rs")).unwrap(),
        "local edit\n"
    );
    workspace.read_with(cx, |w, cx| {
        let thread = w
            .threads
            .iter()
            .find(|thread| thread.id == pull_thread)
            .unwrap();
        assert_eq!(thread.branch.as_deref(), Some(branch.as_str()));
        assert!(thread.chat.read(cx).messages.is_empty());
    });
    git(&["worktree", "remove", "--force", dest.to_str().unwrap()]);

    let path = workspace.update_in(cx, |w, _, cx| {
        w.threads
            .iter()
            .find(|thread| thread.id == link_ids[2])
            .unwrap()
            .chat
            .read(cx)
            .project_path
            .clone()
    });
    std::fs::remove_dir_all(path).unwrap();
    workspace.update_in(cx, |w, _, cx| w.stop_mobile(cx));
}
