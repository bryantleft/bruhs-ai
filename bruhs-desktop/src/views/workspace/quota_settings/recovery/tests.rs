use super::{AgentProvider, Context, EpochMs, Message, TurnState, Uuid, Window, Workspace};
use crate::views::workspace::tests::launch;

fn limited_thread(
    ws: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
) -> String {
    ws.new_general_thread(window, cx);
    ws.choose_quota_preference("settings-quota-resume", "on", cx);
    let thread = ws.active_thread().unwrap();
    let id = thread.id.clone();
    let chat = thread.chat.clone();
    let now = EpochMs::now().as_millis();
    let generation = chat.update(cx, |view, _| {
        view.model_id = "gpt-5.4".into();
        view.session_provider = Some(AgentProvider::Codex);
        view.session_id = Some("quota-test-session".into());
        view.fresh_session = false;
        view.run_locked = true;
        let mut response = Message::assistant_streaming(now - 100, Some(view.model_id.clone()));
        response.state = TurnState::Failed {
            error: "usage limit exceeded".into(),
        };
        response.completed_at_ms = Some(now);
        view.messages = vec![Message::user("Test quota recovery"), response];
        view.turn_generation()
    });
    ws.handle_quota_limit(&chat, Some(now + 600_000), generation, cx);
    assert!(ws.quota_recoveries.contains_key(&id));
    id
}

#[gpui::test]
async fn recovery_restores_and_a_stale_timer_or_receipt_cannot_dispatch(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("workspace.json");
    launch::init(cx);
    let mut next = cx.clone();
    let (ws, win) = launch::launch(cx, &file);
    let id = ws.update_in(win, limited_thread);
    launch::saved(&ws, win).await;
    let (saved, warning, _) = crate::views::workspace::load_desktop_state_with_backup(&file);
    assert!(warning.is_none(), "{warning:?}");
    assert!(saved.unwrap().quota_recoveries.contains_key(&id));
    launch::quit(&ws, win);
    let (ws, win) = launch::launch(&mut next, &file);
    ws.update_in(win, |ws, window, cx| {
        let ticket = ws.quota_recoveries[&id].clone();
        let chat = ws
            .recovery_chat(&id, &ticket, cx)
            .expect("restored owned session");
        ws.select_thread_by_id(&id, cx);
        assert!(ws.quota_timers.contains_key(&id));
        ws.claim_quota_recovery(&id, Uuid::new_v4(), cx);
        assert!(ws.quota_save_receipts().is_empty());
        assert_eq!(ws.quota_recoveries[&id], ticket);
        ws.claim_quota_recovery(&id, ticket.request, cx);
        assert!(ws.snapshot(cx).quota_recoveries.is_empty());
        ws.dispatch_saved_quota(vec![(id.clone(), Uuid::new_v4())], cx);
        assert_eq!(ws.quota_save_receipts(), vec![(id.clone(), ticket.request)]);
        // A draft entered while the claim is saving takes precedence over automation.
        chat.update(cx, |view, cx| {
            view.set_composer_draft("Keep this draft".into(), window, cx)
        });
    });
    launch::saved(&ws, win).await;
    ws.update_in(win, |ws, _, cx| {
        assert!(ws.quota_save_receipts().is_empty());
        assert_eq!(ws.active_thread().unwrap().chat.read(cx).messages.len(), 2);
        assert!(ws.snapshot(cx).quota_recoveries.is_empty());
    });
}

#[gpui::test]
async fn recovery_requires_the_original_failed_session_and_enabled_setting(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    launch::init(cx);
    let (ws, win) = launch::launch(cx, &dir.path().join("state.json"));
    ws.update_in(win, |ws, window, cx| {
        let id = limited_thread(ws, window, cx);
        let ticket = ws.quota_recoveries[&id].clone();
        let chat = ws.active_thread().unwrap().chat.clone();
        chat.update(cx, |view, _| view.session_id = Some("other-session".into()));
        assert!(ws.recovery_chat(&id, &ticket, cx).is_none());
        chat.update(cx, |view, _| view.session_id = Some(ticket.session.clone()));
        assert!(ws.recovery_chat(&id, &ticket, cx).is_some());
        ws.choose_quota_preference("settings-quota-resume", "off", cx);
        assert!(!ws.quota_timers.contains_key(&id));
        assert!(ws.snapshot(cx).quota_recoveries.is_empty());
        assert!(ws.recovery_chat(&id, &ticket, cx).is_none());
    });
    launch::saved(&ws, win).await;
}

#[gpui::test]
async fn recovery_does_not_dispatch_when_another_writer_claimed_it(cx: &mut gpui::TestAppContext) {
    use crate::views::workspace::save_status::SaveStatus;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state.json");
    launch::init(cx);
    let mut next = cx.clone();
    let (first, win) = launch::launch(cx, &file);
    let id = first.update_in(win, limited_thread);
    launch::saved(&first, win).await;
    let (second, other) = launch::launch(&mut next, &file);
    first.update_in(win, |ws, _, cx| {
        ws.choose_quota_preference("settings-quota-resume", "off", cx);
    });
    launch::saved(&first, win).await;
    second.update_in(other, |ws, _, cx| {
        let request = ws.quota_recoveries[&id].request;
        ws.claim_quota_recovery(&id, request, cx);
        assert_eq!(ws.quota_save_receipts().len(), 1);
    });
    other
        .condition(&second, |ws, _| ws.save_status == SaveStatus::Conflict)
        .await;
    second.update_in(other, |ws, _, cx| {
        assert_eq!(ws.quota_save_receipts().len(), 1);
        let thread = ws.threads.iter().find(|t| t.id == id).unwrap();
        assert_eq!(thread.chat.read(cx).messages.len(), 2);
    });
}
