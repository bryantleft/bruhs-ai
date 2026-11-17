//! Web parity for an imported conversation: its Linked person, and the
//! page's layout (PM-41).

use super::super::performance_tests::fixture;
use super::super::{AppView, InboxConversation, InboxMessage, InboxParticipant, InboxSource};
use bruhs_models::{MessageDirection, MessageSendState, PersonProfile};
use gpui::{px, size, VisualTestContext};
use std::sync::Arc;

fn click(cx: &mut VisualTestContext, selector: &str) {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let target = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(target.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

fn person(id: &str, name: &str) -> Arc<PersonProfile> {
    let mut person = PersonProfile::new(name);
    person.id = id.into();
    Arc::new(person)
}

fn conversation(id: &str, sender: &str) -> InboxConversation {
    let mut conversation = InboxConversation::new(
        InboxSource::Slack,
        format!("From {sender}"),
        vec![InboxParticipant {
            id: format!("participant-{id}"),
            display_name: sender.into(),
            address: String::new(),
            person_id: None,
        }],
        1,
    );
    conversation.id = id.into();
    conversation.receive(InboxMessage::incoming(sender, "Hello", 1));
    conversation
}

/// PM-18: like the web's Linked person dropdown, an imported conversation
/// links its sender to any person in the directory, or to nobody, and saves;
/// the person then lists the conversation. Escape and opening another
/// conversation close the list.
#[gpui::test]
fn conversation_links_and_unlinks_a_person_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.people = vec![person("person-ada", "Ada"), person("person-grace", "Grace")];
        });
        workspace.inbox_conversations =
            vec![conversation("one", "Ada L"), conversation("two", "G")];
        workspace.select_inbox_conversation("one", window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-inbox-linked-person").is_some());
    assert!(cx
        .debug_bounds("desktop-inbox-linked-person-list")
        .is_none());

    click(cx, "desktop-inbox-linked-person");
    assert!(cx
        .debug_bounds("desktop-inbox-linked-person-list")
        .is_some());
    for option in [
        "desktop-inbox-linked-person-option-",
        "desktop-inbox-linked-person-option-person-ada",
        "desktop-inbox-linked-person-option-person-grace",
    ] {
        let option: &'static str = Box::leak(option.to_string().into_boxed_str());
        assert!(cx.debug_bounds(option).is_some(), "{option}");
    }
    let saves = workspace.read_with(cx, |workspace, _| workspace.persist_requests);
    click(cx, "desktop-inbox-linked-person-option-person-grace");
    workspace.read_with(cx, |workspace, _| {
        let linked = &workspace.inbox_conversations[0];
        assert_eq!(linked.linked_person_id(), Some("person-grace"));
        assert!(workspace.persist_requests > saves, "the link is saved");
        assert!(
            !workspace.inbox_person_menu_open,
            "choosing closes the list"
        );
        let listed =
            bruhs_models::person_conversations("person-grace", &workspace.inbox_conversations);
        assert_eq!(listed.len(), 1, "the person lists the conversation");
        assert_eq!(listed[0].id, "one");
    });
    assert!(cx
        .debug_bounds("desktop-inbox-linked-person-list")
        .is_none());

    click(cx, "desktop-inbox-linked-person");
    click(cx, "desktop-inbox-linked-person-option-");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.inbox_conversations[0].linked_person_id(), None);
    });

    click(cx, "desktop-inbox-linked-person");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert!(!workspace.inbox_person_menu_open, "Escape closes the list");
    });

    click(cx, "desktop-inbox-linked-person");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.select_inbox_conversation("two", window, cx);
        assert_eq!(
            workspace.active_view,
            AppView::ConversationDetail {
                conversation_id: "two".into()
            }
        );
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-inbox-linked-person-list")
            .is_none(),
        "another conversation opens with the list closed"
    );
}

/// PM-41: like the web, the title has its source and participants under it,
/// the actions are outlined buttons (Delete too, which still asks first),
/// messages sit in a centered column at 88% of its width, and the reply box
/// is three rows tall in that column with the ⌘↵ hint. ⌘↵ logs the reply
/// without the newline the text box would add.
#[gpui::test]
fn conversation_page_lays_out_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        let mut launch = conversation("launch", "Ada Lovelace");
        launch.source = InboxSource::Email;
        workspace.inbox_conversations = vec![launch];
        workspace.select_inbox_conversation("launch", window, cx);
    });
    cx.run_until_parked();

    let title = cx.debug_bounds("desktop-inbox-conversation-title").unwrap();
    let subtitle = cx
        .debug_bounds("desktop-inbox-conversation-subtitle")
        .unwrap();
    assert_eq!(subtitle.origin.x, title.origin.x);
    assert!(subtitle.origin.y >= title.origin.y + title.size.height);
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.inbox_conversations[0].source_and_participants(),
            "Email, Ada Lovelace"
        );
    });
    for action in [
        "desktop-inbox-read",
        "desktop-inbox-snooze",
        "desktop-inbox-important",
        "desktop-inbox-archive",
        "desktop-inbox-ask-agent",
        "desktop-inbox-delete",
    ] {
        let bounds = cx
            .debug_bounds(action)
            .unwrap_or_else(|| panic!("{action}"));
        assert!(bounds.size.height >= px(32.0), "{action} is a 32px control");
    }

    let column = cx.debug_bounds("desktop-inbox-message-column").unwrap();
    let reply = cx.debug_bounds("desktop-inbox-reply").unwrap();
    assert_eq!(column.size.width, px(768.0), "web max-w-3xl");
    assert_eq!(reply.size.width, column.size.width);
    let centered = (column.center().x - reply.center().x).abs();
    assert!(
        centered <= px(1.0),
        "the column and the reply box share a center"
    );
    let message = cx.debug_bounds("desktop-inbox-message-0").unwrap();
    let inner = column.size.width - px(32.0);
    assert!((message.size.width - inner * 0.88).abs() <= px(1.0));
    assert_eq!(
        message.origin.x,
        column.origin.x + px(16.0),
        "received: at the left"
    );
    let time = cx.debug_bounds("desktop-inbox-message-time-0").unwrap();
    assert!(
        time.origin.x + time.size.width >= message.origin.x + message.size.width - px(14.0),
        "the time sits at the message's right"
    );
    let reply_box = cx.debug_bounds("desktop-inbox-reply-box").unwrap();
    assert!(reply_box.size.height >= px(75.0), "three rows, not one");
    let hint = cx.debug_bounds("desktop-inbox-reply-hint").unwrap();
    assert_eq!(hint.origin.x, reply.origin.x);

    click(cx, "desktop-inbox-delete");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.inbox_conversations.len(), 1, "Delete asks first");
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, cx| {
        window.activate_window();
        workspace
            .inbox_reply_input
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.run_until_parked();
    cx.simulate_input("Thanks, see you Thursday");
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        let logged = workspace.inbox_conversations[0].messages.last().unwrap();
        assert_eq!(logged.body, "Thanks, see you Thursday");
        assert_eq!(logged.direction, MessageDirection::Outgoing);
        assert_eq!(logged.send_state, MessageSendState::Sent);
        assert!(workspace.inbox_reply_input.read(cx).value().is_empty());
    });
    let sent = cx.debug_bounds("desktop-inbox-message-1").unwrap();
    let column = cx.debug_bounds("desktop-inbox-message-column").unwrap();
    assert!(
        (sent.origin.x + sent.size.width - (column.origin.x + column.size.width - px(16.0))).abs()
            <= px(1.0),
        "sent: at the right"
    );
}

#[test]
fn command_enter_drops_only_the_newline_it_added() {
    assert_eq!(super::reply_without_submit_newline("Thanks\n", 7), "Thanks");
    assert_eq!(super::reply_without_submit_newline("Ta\nke", 3), "Take");
    assert_eq!(
        super::reply_without_submit_newline("Line\nTwo", 9),
        "Line\nTwo",
        "a newline the user typed stays"
    );
    assert_eq!(super::reply_without_submit_newline("", 0), "");
}
