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
    let message = input_handle(&workspace, cx, |w| &w.inbox_body_input);
    let reply_box = input_handle(&workspace, cx, |w| &w.inbox_reply_input);

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
    // Tab stays in the dialog: its close button, Cancel and Delete, then
    // back to the close button, as the web dialog's focus trap wraps.
    // (Opened from the keyboard, desktop focuses the dialog itself; the
    // web focuses its first control.)
    cx.simulate_keystrokes("tab");
    let close = focused(cx).expect("the dialog's first stop");
    let mut stops = 1;
    loop {
        cx.simulate_keystrokes("tab");
        assert!(
            in_dialog(&workspace, cx),
            "Tab left the dialog after {stops} stops"
        );
        if focused(cx).as_ref() == Some(&close) {
            break;
        }
        stops += 1;
        assert!(stops < 10, "Tab never wrapped inside the dialog");
    }
    assert_eq!(stops, 3, "close, Cancel and Delete");
    // Shift-Tab wraps back to Delete, the last stop.
    cx.simulate_keystrokes("shift-tab");
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
