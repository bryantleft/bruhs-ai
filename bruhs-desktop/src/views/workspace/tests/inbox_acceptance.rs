//! Inbox acceptance on desktop (docs/inbox-completion-2026-11-13.md), through
//! GPUI test frames and a real state file, checked the way the web Inbox
//! harness checks it: a reply draft survives leaving the conversation and a
//! restart (INBOX-06), deleting asks first and stays deleted (INBOX-09),
//! the whole flow works from the keyboard with focus returning to the
//! control that opened a dialog (INBOX-11), and empty views use the web's
//! wording and next step (INBOX-13).

use super::keys::{focus_handle_of, focus_page, focused, in_dialog, press, tab_to};
use super::launch::{init, launch, quit, saved};
use super::*;
use crate::views::workspace::inbox_ui::inbox_empty_state;
use crate::views::workspace::save_status::SaveStatus;
use bruhs_models::{
    InboxConversation, InboxFilter, InboxMessage, InboxParticipant, InboxSource, MessageDirection,
    MessageSendState,
};

fn conversation(id: &str, subject: &str, sender: &str, at: i64) -> InboxConversation {
    let mut conversation = InboxConversation::new(
        InboxSource::Email,
        subject,
        vec![InboxParticipant {
            id: format!("participant-{id}"),
            display_name: sender.into(),
            address: String::new(),
            person_id: None,
        }],
        at,
    );
    conversation.id = id.into();
    conversation.receive(InboxMessage::incoming(sender, format!("{subject}?"), at));
    conversation
}

fn card(id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-inbox-card-{}",
        crate::views::workspace::element_key(id)
    ))
}

fn show_inbox(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_inbox_view(window, cx);
    });
    cx.run_until_parked();
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    click_debug_selector(cx, selector);
    cx.run_until_parked();
}

fn reply(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> String {
    workspace.read_with(cx, |workspace, cx| {
        workspace.inbox_reply_input.read(cx).value().to_string()
    })
}

fn conversation_view(id: &str) -> AppView {
    AppView::ConversationDetail {
        conversation_id: id.into(),
    }
}

fn input_handle<T: gpui::Focusable>(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    input: impl Fn(&Workspace) -> &Entity<T>,
) -> gpui::FocusHandle {
    workspace.read_with(cx, |workspace, cx| {
        input(workspace).read(cx).focus_handle(cx)
    })
}

/// `input_handle` for a multi-line field.
fn textarea_handle(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    input: impl Fn(&Workspace) -> &Entity<gpui_component::input::TextareaState>,
) -> gpui::FocusHandle {
    workspace.read_with(cx, |workspace, cx| {
        input(workspace).read(cx).focus_handle(cx)
    })
}

/// INBOX-06: a saved reply draft comes back when the conversation is
/// reopened, another conversation's reply box stays empty, and after a
/// restart the draft is there to continue and log; the logged reply and
/// the cleared draft survive another restart.
#[gpui::test]
async fn a_reply_draft_survives_leaving_and_a_restart(cx: &mut gpui::TestAppContext) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let mut second_launch = cx.clone();
    let mut third_launch = cx.clone();
    let (workspace, cx) = launch(cx, &path);
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.inbox_conversations = vec![
            conversation("lunch", "Lunch plans", "Ada", 2),
            conversation("brief", "The brief", "Grace", 1),
        ];
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;
    show_inbox(&workspace, cx);
    click(cx, card("lunch"));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .inbox_reply_input
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.run_until_parked();
    cx.simulate_input("Friday works");
    click(cx, "desktop-inbox-save-draft");
    saved(&workspace, cx).await;

    // Leave, open another conversation, come back.
    click(cx, "desktop-inbox-back");
    assert_eq!(
        workspace.read_with(cx, |w, _| w.active_view.clone()),
        AppView::Inbox
    );
    click(cx, card("brief"));
    assert_eq!(reply(&workspace, cx), "", "Grace's reply box is empty");
    click(cx, "desktop-inbox-back");
    click(cx, card("lunch"));
    assert_eq!(reply(&workspace, cx), "Friday works");
    saved(&workspace, cx).await;
    quit(&workspace, cx);

    // A restart keeps it; continue it and log the reply.
    let (restarted, cx) = launch(&mut second_launch, &path);
    show_inbox(&restarted, cx);
    click(cx, card("lunch"));
    assert_eq!(reply(&restarted, cx), "Friday works");
    restarted.update_in(cx, |workspace, window, cx| {
        workspace.inbox_reply_input.update(cx, |input, cx| {
            input.set_value("Friday works, at noon", window, cx);
        });
    });
    cx.run_until_parked();
    click(cx, "desktop-inbox-log-reply");
    saved(&restarted, cx).await;
    assert_eq!(reply(&restarted, cx), "");
    quit(&restarted, cx);

    let (third, cx) = launch(&mut third_launch, &path);
    third.read_with(cx, |workspace, _| {
        let lunch = workspace
            .inbox_conversations
            .iter()
            .find(|conversation| conversation.id == "lunch")
            .expect("the conversation");
        assert_eq!(lunch.draft_body(), "", "the logged draft is gone");
        let logged = lunch.messages.last().expect("the logged reply");
        assert_eq!(logged.body, "Friday works, at noon");
        assert_eq!(logged.direction, MessageDirection::Outgoing);
        assert_eq!(logged.send_state, MessageSendState::Sent);
        let brief = workspace
            .inbox_conversations
            .iter()
            .find(|conversation| conversation.id == "brief")
            .expect("the other conversation");
        assert_eq!(brief.draft_body(), "");
    });
}

/// INBOX-06, failed persistence: a reply draft saved while the state file's
/// folder is read-only stays in the reply box and the conversation, and
/// "Not saved" reports the failure with Retry save. The file keeps its
/// last good copy. Once the folder is writable again, Retry save stores the
/// draft, the strip goes, and a restart has the draft.
#[gpui::test]
#[cfg(unix)]
async fn a_reply_draft_survives_a_failed_save_and_retry(cx: &mut gpui::TestAppContext) {
    use std::os::unix::fs::PermissionsExt;

    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let folder = files.path().join("state");
    std::fs::create_dir(&folder).expect("the state folder");
    let path = folder.join("state.json");
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &path);
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.inbox_conversations = vec![conversation("lunch", "Lunch plans", "Ada", 2)];
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;
    show_inbox(&workspace, cx);
    click(cx, card("lunch"));
    saved(&workspace, cx).await;
    let last_good = std::fs::read(&path).expect("the saved state");

    // The folder goes read-only, then the draft is saved.
    let read_only = |mode: u32| {
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode))
            .expect("the folder's permissions");
    };
    read_only(0o555);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .inbox_reply_input
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.run_until_parked();
    cx.simulate_input("Friday works");
    click(cx, "desktop-inbox-save-draft");
    cx.condition(&workspace, |workspace, _| {
        matches!(workspace.save_status, SaveStatus::Failed(_))
    })
    .await;
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(&path).expect("the state file"),
        last_good,
        "the last good copy is untouched"
    );
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(workspace.inbox_reply_input.read(cx).value(), "Friday works");
        assert_eq!(
            workspace.inbox_conversations[0].draft_body(),
            "Friday works"
        );
        assert_eq!(
            workspace.save_status.alert(),
            Some("Local storage could not save this change. Keep this window open and retry.")
        );
    });
    assert_rendered_bounds(cx, "desktop-save-status");
    assert_rendered_bounds(cx, "desktop-retry-save");
    // Leaving and coming back keeps it too; that save fails as well and is
    // reported again.
    click(cx, "desktop-inbox-back");
    click(cx, card("lunch"));
    assert_eq!(reply(&workspace, cx), "Friday works");
    cx.condition(&workspace, |workspace, _| {
        matches!(workspace.save_status, SaveStatus::Failed(_))
    })
    .await;
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-save-status");

    // Writing works again: Retry save stores the draft.
    read_only(0o755);
    click(cx, "desktop-retry-save");
    saved(&workspace, cx).await;
    assert_no_rendered_bounds(cx, "desktop-save-status");
    let stored = load_desktop_state_from_path(&path)
        .expect("a readable state file")
        .expect("a state file");
    let lunch = stored
        .inbox_conversations
        .iter()
        .find(|conversation| conversation.id == "lunch")
        .expect("the conversation");
    assert_eq!(lunch.draft_body(), "Friday works");
    quit(&workspace, cx);

    let (restarted, cx) = launch(&mut relaunch, &path);
    show_inbox(&restarted, cx);
    click(cx, card("lunch"));
    assert_eq!(reply(&restarted, cx), "Friday works");
}

/// INBOX-09: Delete asks first with the shared wording. Cancel and Escape
/// keep the conversation; Delete removes it and returns to the list, and a
/// restart does not bring it back. Like the web, there is no undo after
/// the confirmation; the other conversation is untouched.
#[gpui::test]
async fn deleting_a_conversation_asks_first_and_stays_deleted(cx: &mut gpui::TestAppContext) {
    init(cx);
    cx.update(|cx| cx.bind_keys([gpui::KeyBinding::new("escape", CloseOverlays, None)]));
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &path);
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.inbox_conversations = vec![
            conversation("lunch", "Lunch plans", "Ada", 2),
            conversation("brief", "The brief", "Grace", 1),
        ];
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;
    show_inbox(&workspace, cx);
    click(cx, card("lunch"));
    saved(&workspace, cx).await;
    let before = workspace.read_with(cx, |workspace, _| workspace.inbox_conversations.clone());

    let delete_dialog = |workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
        workspace.read_with(cx, |workspace, _| {
            matches!(workspace.overlay, Some(Overlay::RecordDeleteConfirm { .. }))
        })
    };
    // Cancel keeps it.
    click(cx, "desktop-inbox-delete");
    assert!(delete_dialog(&workspace, cx));
    assert_rendered_bounds(cx, "desktop-record-delete-confirm");
    click(cx, "desktop-record-delete-cancel");
    assert!(!delete_dialog(&workspace, cx));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.active_view, conversation_view("lunch"));
        assert_eq!(workspace.inbox_conversations, before);
    });
    // Escape keeps it too.
    click(cx, "desktop-inbox-delete");
    assert!(delete_dialog(&workspace, cx));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!delete_dialog(&workspace, cx));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.inbox_conversations, before);
    });

    // Delete removes it and returns to the list.
    click(cx, "desktop-inbox-delete");
    click(cx, "desktop-record-delete-confirm");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.active_view, AppView::Inbox);
        let ids: Vec<&str> = workspace
            .inbox_conversations
            .iter()
            .map(|conversation| conversation.id.as_str())
            .collect();
        assert_eq!(ids, ["brief"]);
    });
    assert_no_rendered_bounds(cx, card("lunch"));
    assert_rendered_bounds(cx, card("brief"));
    saved(&workspace, cx).await;
    quit(&workspace, cx);

    let (restarted, cx) = launch(&mut relaunch, &path);
    restarted.read_with(cx, |workspace, _| {
        assert_eq!(workspace.inbox_conversations.len(), 1);
        assert_eq!(workspace.inbox_conversations[0], before[1], "untouched");
    });
    show_inbox(&restarted, cx);
    assert_no_rendered_bounds(cx, card("lunch"));
}

/// INBOX-11: import, triage, draft, navigation, search and the delete
/// dialog by keyboard alone. Each control is a Tab stop, Enter or Space
/// runs it, focus stays on a triage action after it runs, and closing a
/// dialog returns focus to the control that opened it, as the web dialog
/// does.
#[gpui::test]
async fn the_inbox_works_from_the_keyboard(cx: &mut gpui::TestAppContext) {
    init(cx);
    cx.update(|cx| cx.bind_keys([gpui::KeyBinding::new("escape", CloseOverlays, None)]));
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let (workspace, cx) = launch(cx, &path);
    show_inbox(&workspace, cx);
    let search = input_handle(&workspace, cx, |w| &w.inbox_search_input);
    let sender = input_handle(&workspace, cx, |w| &w.inbox_sender_input);
    let subject = input_handle(&workspace, cx, |w| &w.inbox_subject_input);
    let message = textarea_handle(&workspace, cx, |w| &w.inbox_body_input);
    let reply_box = textarea_handle(&workspace, cx, |w| &w.inbox_reply_input);

    // Import: the action just before the search field opens the form with
    // the sender focused. An empty import is refused and keeps its place.
    tab_to(cx, &search, "the search field");
    cx.simulate_keystrokes("shift-tab");
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| assert!(workspace.inbox_import_visible));
    assert!(
        focused(cx).as_ref() == Some(&sender),
        "the sender has focus"
    );
    cx.simulate_keystrokes("tab");
    assert!(focused(cx).as_ref() == Some(&subject));
    cx.simulate_keystrokes("tab");
    assert!(focused(cx).as_ref() == Some(&message));
    cx.simulate_keystrokes("tab");
    let import = focused(cx).expect("the Import button");
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.persistence_error.as_deref(),
            Some("A sender and message are required.")
        );
        assert!(workspace.inbox_conversations.is_empty());
        assert!(workspace.inbox_import_visible);
    });
    assert!(
        focused(cx).as_ref() == Some(&import),
        "focus stays on Import"
    );
    for _ in 0..3 {
        cx.simulate_keystrokes("shift-tab");
    }
    assert!(focused(cx).as_ref() == Some(&sender));
    cx.simulate_input("Ada Lovelace");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("Lunch plans");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("Friday works?");
    cx.simulate_keystrokes("tab");
    press(cx, "enter");
    let id = workspace.read_with(cx, |workspace, _| {
        let imported = workspace
            .inbox_conversations
            .first()
            .expect("the imported conversation");
        assert_eq!(imported.subject, "Lunch plans");
        assert_eq!(imported.participants[0].display_name, "Ada Lovelace");
        assert_eq!(imported.messages[0].body, "Friday works?");
        assert_eq!(workspace.active_view, conversation_view(&imported.id));
        imported.id.clone()
    });

    // Triage: Enter and Space run each action, and focus stays on it.
    let read = focus_handle_of(cx, "desktop-inbox-read");
    let important = focus_handle_of(cx, "desktop-inbox-important");
    let archive = focus_handle_of(cx, "desktop-inbox-archive");
    focus_page(&workspace, cx);
    tab_to(cx, &read, "Mark unread");
    let triaged = |workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
        workspace.read_with(cx, |workspace, _| workspace.inbox_conversations[0].clone())
    };
    press(cx, "enter");
    assert!(triaged(&workspace, cx).unread, "Mark unread ran");
    assert!(focused(cx).as_ref() == Some(&read));
    tab_to(cx, &important, "Important");
    press(cx, "space");
    assert!(triaged(&workspace, cx)
        .labels
        .iter()
        .any(|label| label == "important"));
    tab_to(cx, &archive, "Archive");
    press(cx, "enter");
    assert!(focused(cx).as_ref() == Some(&archive));
    let archived = triaged(&workspace, cx);
    assert!(archived.archived);
    assert!(!archived.unread, "archiving marks it read, as on the web");

    // Draft: Tab reaches the reply box, and Tab again Save draft.
    tab_to(cx, &reply_box, "the reply box");
    cx.simulate_input("Friday at noon");
    cx.simulate_keystrokes("tab");
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.inbox_conversations[0].draft_body(),
            "Friday at noon"
        );
    });

    // Back to the list by keyboard.
    let back = focus_handle_of(cx, "desktop-inbox-back");
    focus_page(&workspace, cx);
    tab_to(cx, &back, "Back to Inbox");
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.active_view, AppView::Inbox)
    });

    // Search: the archived conversation is not in Inbox; All finds it.
    tab_to(cx, &search, "the search field");
    cx.simulate_input("Lunch");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-inbox-empty");
    let all = focus_handle_of(cx, "desktop-inbox-filter-All");
    tab_to(cx, &search, "the search field");
    tab_to(cx, &all, "the All filter");
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.inbox_filter, InboxFilter::All);
    });
    let row = focus_handle_of(cx, card(&id));
    focus_page(&workspace, cx);
    tab_to(cx, &row, "the conversation row");
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.active_view, conversation_view(&id));
    });

    // Delete asks first: focus moves into the dialog, Escape closes it and
    // returns focus to Delete; Tab reaches Delete in the dialog.
    let delete = focus_handle_of(cx, "desktop-inbox-delete");
    focus_page(&workspace, cx);
    tab_to(cx, &delete, "Delete");
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert!(matches!(
            workspace.overlay,
            Some(Overlay::RecordDeleteConfirm { .. })
        ));
    });
    assert!(in_dialog(&workspace, cx), "focus moved into the dialog");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| assert!(workspace.overlay.is_none()));
    assert!(
        focused(cx).as_ref() == Some(&delete),
        "focus returns to Delete, as the web dialog restores its invoker"
    );
    press(cx, "enter");
    // Opened from the keyboard, the dialog focuses its first control,
    // Cancel, as the web dialog does. Tab stays in the dialog: Delete, the
    // close button, then Cancel again, as the web dialog's focus trap wraps.
    let cancel = focused(cx).expect("the dialog's first control");
    assert!(in_dialog(&workspace, cx), "focus moved into the dialog");
    let mut stops = 1;
    loop {
        cx.simulate_keystrokes("tab");
        assert!(
            in_dialog(&workspace, cx),
            "Tab left the dialog after {stops} stops"
        );
        if focused(cx).as_ref() == Some(&cancel) {
            break;
        }
        stops += 1;
        assert!(stops < 10, "Tab never wrapped inside the dialog");
    }
    assert_eq!(stops, 3, "Cancel, Delete and the close button");
    // Shift-Tab from Cancel passes the close button and wraps to Delete,
    // the last stop.
    cx.simulate_keystrokes("shift-tab shift-tab");
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.inbox_conversations.is_empty(), "deleted");
        assert_eq!(workspace.active_view, AppView::Inbox);
    });
    let at_page = workspace.update_in(cx, |workspace, window, _cx| {
        workspace.workspace_focus_handle.is_focused(window)
    });
    assert!(
        at_page,
        "the deleted record's page is gone; focus is on the page"
    );
    saved(&workspace, cx).await;
}

/// INBOX-11: the import form's Cancel, and Escape, close it and return
/// focus to Import conversation, as the web dialog restores its invoker.
/// Opened from the command bar, closing it leaves focus on the page, as on
/// the web, whose invoker is gone by then.
#[gpui::test]
async fn closing_the_import_form_returns_focus_to_its_opener(cx: &mut gpui::TestAppContext) {
    init(cx);
    cx.update(|cx| cx.bind_keys([gpui::KeyBinding::new("escape", CloseOverlays, None)]));
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let (workspace, cx) = launch(cx, &path);
    show_inbox(&workspace, cx);
    let open_by_enter = |cx: &mut gpui::VisualTestContext| {
        let import = focus_handle_of(cx, "desktop-import-conversation");
        focus_page(&workspace, cx);
        tab_to(cx, &import, "Import conversation");
        press(cx, "enter");
        workspace.read_with(cx, |workspace, _| assert!(workspace.inbox_import_visible));
    };
    let import_has_focus = |focus: Option<gpui::FocusHandle>, cx: &mut gpui::VisualTestContext| {
        focus.is_some() && focus == Some(focus_handle_of(cx, "desktop-import-conversation"))
    };

    // Cancel by Enter.
    open_by_enter(cx);
    let cancel = focus_handle_of(cx, "desktop-inbox-import-cancel");
    assert!(focused(cx).as_ref() == Some(&cancel));
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| assert!(!workspace.inbox_import_visible));
    let after_cancel = focused(cx);
    assert!(
        import_has_focus(after_cancel, cx),
        "Cancel returns focus to Import conversation"
    );

    // Escape, from the sender field.
    open_by_enter(cx);
    let sender = input_handle(&workspace, cx, |w| &w.inbox_sender_input);
    assert!(focused(cx).as_ref() == Some(&sender));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert!(!workspace.inbox_import_visible, "Escape closes the form")
    });
    let after_escape = focused(cx);
    assert!(
        import_has_focus(after_escape, cx),
        "Escape returns focus to Import conversation"
    );

    // From the command bar, which has focus when its action runs: the page.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.command_bar_visible());
        workspace.execute_command_action("import-conversation", window, cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert!(!workspace.command_bar_visible(), "the command bar closed");
        assert!(workspace.inbox_import_visible);
    });
    assert!(
        focused(cx).as_ref() == Some(&sender),
        "the sender has focus"
    );
    let cancel = focus_handle_of(cx, "desktop-inbox-import-cancel");
    assert!(focused(cx).as_ref() == Some(&cancel));
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| assert!(!workspace.inbox_import_visible));
    let on_page = workspace.update_in(cx, |workspace, window, _cx| {
        workspace.workspace_focus_handle.is_focused(window)
    });
    assert!(on_page, "focus is on the page");
}

/// INBOX-13: before the first import, each filter's empty view, a search
/// with no match and an empty Inbox beside a full All say what the web says
/// and offer its next step.
#[gpui::test]
async fn empty_inbox_views_use_the_web_wording(cx: &mut gpui::TestAppContext) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let (workspace, cx) = launch(cx, &path);
    show_inbox(&workspace, cx);
    let shown = |workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
        workspace.read_with(cx, |workspace, cx| {
            inbox_empty_state(
                &workspace.inbox_search_input.read(cx).value(),
                workspace.inbox_filter,
                workspace.saved_records_loaded,
            )
        })
    };
    const IMPORT_NEXT: &str = "Import a message or choose another view.";

    // Before the first import: the Inbox is clear, Import is offered.
    assert_rendered_bounds(cx, "desktop-inbox-empty");
    assert_rendered_bounds(cx, "desktop-import-conversation");
    assert_eq!(shown(&workspace, cx), ("Your inbox is clear.", IMPORT_NEXT));
    for (filter, message) in [
        ("Unread", "No unread conversations."),
        ("Snoozed", "No snoozed conversations."),
        ("Archived", "No archived conversations."),
        ("All", "No conversations yet."),
        ("Inbox", "Your inbox is clear."),
    ] {
        click(
            cx,
            leaked_debug_selector(format!("desktop-inbox-filter-{filter}")),
        );
        assert_rendered_bounds(cx, "desktop-inbox-empty");
        assert_eq!(shown(&workspace, cx), (message, IMPORT_NEXT), "{filter}");
    }

    // A search with no match is not an empty view.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.inbox_search_input.update(cx, |input, cx| {
            input.set_value("Definitely absent Ω", window, cx)
        });
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-inbox-empty");
    assert_eq!(
        shown(&workspace, cx),
        (
            "No matching conversations.",
            "Try another search or choose All."
        )
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .inbox_search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    });
    cx.run_until_parked();

    // The first import opens its conversation; the list then shows it.
    click(cx, "desktop-import-conversation");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .inbox_sender_input
            .update(cx, |input, cx| input.set_value("Ada", window, cx));
        workspace
            .inbox_body_input
            .update(cx, |input, cx| input.set_value("First message", window, cx));
    });
    click(cx, "desktop-inbox-import-submit");
    let id = workspace.read_with(cx, |workspace, _| {
        let id = workspace.inbox_conversations[0].id.clone();
        assert_eq!(workspace.active_view, conversation_view(&id));
        id
    });
    click(cx, "desktop-inbox-back");
    assert_rendered_bounds(cx, card(&id));
    assert_no_rendered_bounds(cx, "desktop-inbox-empty");

    // Archived: the Inbox is clear again while All lists it.
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.inbox_conversations[0].set_archived(true);
        workspace.persist_and_notify(cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-inbox-empty");
    assert_eq!(shown(&workspace, cx), ("Your inbox is clear.", IMPORT_NEXT));
    click(cx, "desktop-inbox-filter-All");
    assert_rendered_bounds(cx, card(&id));
    assert_no_rendered_bounds(cx, "desktop-inbox-empty");
    saved(&workspace, cx).await;
}

/// INBOX-13, storage failure: a state file that cannot be decoded leaves the
/// list saying what the web says when its records fail to load, instead of
/// an empty Inbox. "Not saved" says why, and the original file is kept:
/// nothing is saved over it, even after an import in this window.
#[gpui::test]
async fn an_undecodable_state_file_reads_as_a_load_failure(cx: &mut gpui::TestAppContext) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let original = b"{ \"inbox_conversations\": [ not json".to_vec();
    std::fs::write(&path, &original).expect("an undecodable state file");
    let (workspace, cx) = launch(cx, &path);
    workspace.read_with(cx, |workspace, _| {
        assert!(matches!(
            workspace.save_status,
            SaveStatus::RecoveryRequired(_)
        ));
        assert!(workspace.inbox_conversations.is_empty());
    });
    show_inbox(&workspace, cx);
    assert_rendered_bounds(cx, "desktop-inbox-empty");
    assert_rendered_bounds(cx, "desktop-save-status");
    let shown = |workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
        workspace.read_with(cx, |workspace, cx| {
            inbox_empty_state(
                &workspace.inbox_search_input.read(cx).value(),
                workspace.inbox_filter,
                workspace.saved_records_loaded,
            )
        })
    };
    // Not the loading line: these records will not appear by waiting.
    let failed = (
        "Conversations could not be loaded",
        "Stored conversations were left unchanged.",
    );
    assert_eq!(shown(&workspace, cx), failed);
    click(cx, "desktop-inbox-filter-All");
    assert_eq!(shown(&workspace, cx), failed, "every view says so");
    click(cx, "desktop-inbox-filter-Inbox");

    // An import still opens, in this window only; the original stays.
    click(cx, "desktop-import-conversation");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .inbox_sender_input
            .update(cx, |input, cx| input.set_value("Ada", window, cx));
        workspace
            .inbox_body_input
            .update(cx, |input, cx| input.set_value("Kept here", window, cx));
    });
    click(cx, "desktop-inbox-import-submit");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.inbox_conversations.len(), 1);
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).expect("the state file"), original);
}
