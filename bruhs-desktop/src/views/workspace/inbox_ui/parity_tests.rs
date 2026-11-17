//! Web parity for an imported conversation: its Linked person.

use super::super::performance_tests::fixture;
use super::super::{AppView, InboxConversation, InboxMessage, InboxParticipant, InboxSource};
use bruhs_models::PersonProfile;
use gpui::VisualTestContext;
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
