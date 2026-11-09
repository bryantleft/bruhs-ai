use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use crate::views::chat::{DesktopToolCall, Message, TurnState};
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse};
use bruhs_models::{AgentAccess, AgentDomain};
use gpui::TestAppContext;
use serde_json::json;

fn summaries(query: &str, offset: usize) -> BridgeRequest {
    BridgeRequest::Read {
        domain: AgentDomain::Chat,
        query: query.into(),
        offset,
    }
}

#[gpui::test]
async fn chat_reads_require_read_even_with_write_and_recheck_before_id_lookup(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let id = workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Chat, AgentAccess::Read, false);
        w.automation_state
            .permissions
            .set(AgentDomain::Chat, AgentAccess::Write, true);
        w.threads[0].id.clone()
    });
    connect(&workspace, cx).await;
    for request in [
        summaries("", 0),
        BridgeRequest::ReadChatMessages {
            thread_id: id.clone(),
            offset: 0,
        },
    ] {
        assert!(matches!(
            call(&workspace, cx, request).await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Chat, AgentAccess::Read, true);
        w.automation_state
            .permissions
            .set(AgentDomain::Chat, AgentAccess::Write, false);
    });
    assert!(matches!(
        call(&workspace, cx, summaries("", 0)).await,
        BridgeResponse::Ok(_)
    ));
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::ReadChatMessages {
                thread_id: id.clone(),
                offset: 0
            }
        )
        .await,
        BridgeResponse::Ok(_)
    ));
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::Chat, AgentAccess::Read, false)
    });
    for thread_id in [id, "missing".into()] {
        let BridgeResponse::Error(message) = call(
            &workspace,
            cx,
            BridgeRequest::ReadChatMessages {
                thread_id,
                offset: 0,
            },
        )
        .await
        else {
            panic!("revoked read must fail")
        };
        assert_eq!(message, "Read access is not granted for this domain.");
    }
    assert!(matches!(
        call(&workspace, cx, summaries("", 0)).await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn chat_summary_reads_do_not_serialize_large_history_and_filter_current_titles(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let id = workspace.update_in(cx, |w, _, cx| {
        w.threads[0].chat.update(cx, |chat, _| {
            chat.thread_title = "Needle 🗒".into();
            chat.messages = vec![Message::user("private-body".repeat(100_000))];
        });
        w.threads[0].id.clone()
    });
    connect(&workspace, cx).await;
    let BridgeResponse::Ok(result) = call(&workspace, cx, summaries("NEEDLE", 0)).await else {
        panic!("summary read")
    };
    assert_eq!(result["records"][0]["id"], id);
    assert_eq!(result["records"][0]["title"], "Needle 🗒");
    assert!(result["records"][0].get("messages").is_none());
    assert!(!result.to_string().contains("private-body"));
    for (query, offset) in [("private-body", 0), ("NEEDLE", 1)] {
        let BridgeResponse::Ok(result) = call(&workspace, cx, summaries(query, offset)).await
        else {
            panic!("filtered summary")
        };
        assert_eq!(result["records"], json!([]));
    }
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::ReadChatMessages {
                thread_id: id,
                offset: 0
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, cx| {
        w.threads[0].chat.update(cx, |chat, _| {
            chat.thread_title = "Changed live title".into()
        })
    });
    let BridgeResponse::Ok(result) = call(&workspace, cx, summaries("CHANGED", 0)).await else {
        panic!("live summary")
    };
    assert_eq!(result["records"][0]["title"], "Changed live title");
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn chat_history_pages_preserve_completed_content_without_drafts_streams_or_mutations(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let (id, before) = workspace.update_in(cx, |w, window, cx| {
        w.threads[0].chat.update(cx, |chat, cx| {
            chat.messages = (0..123)
                .map(|i| Message::user(format!("message-{i}")))
                .collect();
            let mut completed = Message::assistant_streaming(100, Some("model".into()));
            completed.content = "Completed assistant 🗒".into();
            completed.thinking = "Stored reasoning text".into();
            completed.completed_at_ms = Some(200);
            completed.duration_ms = Some(100);
            completed.state = TurnState::Failed {
                error: "Stored interruption".into(),
            };
            completed.tool_calls.push(DesktopToolCall {
                id: "tool-1".into(),
                tool: "read".into(),
                input: json!({"path":"local"}),
                output: Some("Stored tool result".into()),
                is_error: false,
            });
            chat.messages[122] = completed;
            let mut streaming = Message::assistant_streaming(123, None);
            streaming.content = "unfinished-secret".into();
            chat.messages.insert(1, streaming);
            chat.input_state.update(cx, |input, cx| {
                input.set_value("stashed-secret", window, cx)
            });
            chat.stash_current_draft(window, cx);
            chat.input_state
                .update(cx, |input, cx| input.set_value("unsent-secret", window, cx));
        });
        (
            w.threads[0].id.clone(),
            serde_json::to_value(w.threads[0].snapshot(cx)).unwrap(),
        )
    });
    connect(&workspace, cx).await;
    let mut all = Vec::new();
    for offset in [0, 50, 100] {
        let BridgeResponse::Ok(result) = call(
            &workspace,
            cx,
            BridgeRequest::ReadChatMessages {
                thread_id: id.clone(),
                offset,
            },
        )
        .await
        else {
            panic!("history page")
        };
        assert_eq!(result["thread_id"], id);
        assert_eq!(
            result["next_offset"],
            if offset < 100 {
                json!(offset + 50)
            } else {
                json!(null)
            }
        );
        assert!(!result.to_string().contains("secret"));
        all.extend(result["records"].as_array().unwrap().iter().cloned());
    }
    assert_eq!(all.len(), 123);
    assert_eq!(json!(all), before["messages"]);
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(
            serde_json::to_value(w.threads[0].snapshot(cx)).unwrap(),
            before
        );
        assert_eq!(
            w.threads[0].chat.read(cx).input_state.read(cx).value(),
            "unsent-secret"
        );
        assert!(w.automation_state.workflows.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}
