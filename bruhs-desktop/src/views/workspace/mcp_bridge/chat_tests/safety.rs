use super::*;
use crate::views::chat::Message;

#[gpui::test]
async fn chat_undo_preserves_messages_unsent_input_stashes_active_threads_and_changed_metadata(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    let id = started(&workspace, cx, &files).await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    click(&workspace, cx, "desktop-run-workflow-step").await;
    workspace.update_in(cx, |w, window, cx| {
        let target = w.automation_state.workflows[0].steps[0]
            .target_id
            .clone()
            .unwrap();
        let chat = w
            .threads
            .iter()
            .find(|thread| thread.id == target)
            .unwrap()
            .chat
            .clone();
        assert!(w.automation_rollback_targets(&id, cx).is_ok());
        w.threads
            .iter_mut()
            .find(|thread| thread.id == target)
            .unwrap()
            .pinned = true;
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        w.threads
            .iter_mut()
            .find(|thread| thread.id == target)
            .unwrap()
            .pinned = false;
        chat.update(cx, |chat, _| chat.is_streaming = true);
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        chat.update(cx, |chat, _| chat.is_streaming = false);
        chat.update(cx, |chat, _| {
            chat.messages.push(Message::user("Keep this conversation"))
        });
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        chat.update(cx, |chat, _| chat.messages.clear());
        let previous = w.active_thread_id.replace(target);
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        w.active_thread_id = previous;
        chat.update(cx, |chat, cx| {
            chat.input_state
                .update(cx, |input, cx| input.set_value("Unsent draft", window, cx))
        });
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        chat.update(cx, |chat, cx| chat.stash_current_draft(window, cx));
        assert!(chat.read(cx).input_state.read(cx).value().is_empty());
        assert!(w.automation_rollback_targets(&id, cx).is_err());
        assert_eq!(w.work_items.len(), 1);
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn scheduled_chat_step_rechecks_revoked_grants_and_preview_before_creation(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let files = Files::new();
    started(&workspace, cx, &files).await;
    let original = workspace.update_in(cx, |w, _, _| w.automation_state.clone());
    let count = workspace.update_in(cx, |w, _, _| w.threads.len());
    for changed_payload in [false, true] {
        workspace.update_in(cx, |w, _, cx| {
            w.automation_state = original.clone();
            w.automation_error = None;
            w.run_latest_automation_step(cx);
            w.run_latest_automation_step(cx);
            assert!(w.automation_pending);
            if changed_payload {
                w.automation_state.workflows[0].steps[0].operation =
                    WorkflowOperation::CreateChatThread {
                        title: "Not reviewed".into(),
                    };
            } else {
                w.automation_state
                    .permissions
                    .set(AgentDomain::Chat, AgentAccess::Write, false);
            }
        });
        cx.condition(&workspace, |w, _| {
            !w.automation_pending && w.automation_error.is_some()
        })
        .await;
        workspace.update_in(cx, |w, _, _| {
            assert_eq!(w.threads.len(), count);
            assert_eq!(w.automation_state.jobs[0].next_step, 0);
        });
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
