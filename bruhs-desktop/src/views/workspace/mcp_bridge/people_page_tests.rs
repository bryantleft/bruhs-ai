//! People page GPUI checks (PEOPLE-01, 02, 04, 07, 08, 09, 10 and the Inbox
//! auto-link rule). Synthetic records only. The hub talks to a dead loopback
//! port, so no test reaches a real server; server replies are simulated by
//! emitting the hub event the reply would produce.

use super::super::meetings_ui::PersonMergePanel;
use super::super::{AppView, Workspace};
use super::test_fixture::fixture;
use crate::server_client::ServerClient;
use crate::views::meetings_hub::HubEvent;
use bruhs_models::{
    people_copy, InboxConversation, InboxParticipant, InboxSource, KnowledgeDocument,
    KnowledgeLink, KnowledgeLinkKind, PeopleUndo, PersonProfile, RelationshipItem,
    RelationshipItemKind, RelationshipProfile,
};
use gpui::{
    point, px, Bounds, Entity, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext,
    VisualTestContext,
};
use std::sync::Arc;

fn offline(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9")
                    .unwrap_or_else(|error| ServerClient::unavailable(&error)),
            );
        });
        w.active_view = AppView::People;
        cx.notify();
    });
    cx.run_until_parked();
}

fn person(id: &str, name: &str, aliases: &[&str]) -> PersonProfile {
    let mut person = PersonProfile::new(name);
    person.id = id.into();
    person.aliases = aliases.iter().map(|alias| (*alias).to_string()).collect();
    person
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

/// The pager sits below a full page of cards, past the bottom of the
/// window's viewport. Scroll the People page to its end the way a user would,
/// then re-read the page and the control from the frame the click will hit
/// and require the control's centre to be inside both the visible People page
/// and the measured window viewport.
fn scroll_people_page_to(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let page = cx
        .debug_bounds("desktop-people-page")
        .expect("Missing desktop-people-page");
    cx.simulate_event(ScrollWheelEvent {
        position: page.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), px(-100_000.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    let page = cx
        .debug_bounds("desktop-people-page")
        .expect("Missing desktop-people-page after scrolling");
    let target = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    let viewport =
        cx.update(|window, _| Bounds::new(point(px(0.0), px(0.0)), window.viewport_size()));
    assert!(
        page.contains(&target.center()),
        "{selector} is inside the visible People page after scrolling"
    );
    assert!(
        viewport.contains(&target.center()),
        "{selector} is inside the measured window viewport after scrolling"
    );
}

#[gpui::test]
async fn people_page_create_refuses_blank_names_and_keeps_unicode_names(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    offline(&workspace, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.people_page
            .create_input
            .update(cx, |input, cx| input.set_value("   ", window, cx));
        w.create_person_from_page(window, cx);
        assert_eq!(w.people_page.create_error, Some(people_copy::CREATE_BLANK));
        assert_eq!(
            w.people_page.create_input.read(cx).value().as_ref(),
            "   ",
            "a refused name stays in the input"
        );
        assert!(w.meetings_hub.read(cx).people.is_empty());
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-people-create-error").is_some());

    let long = format!("Zoë Ångström {}", "Łukasiewicz ".repeat(12));
    let expected = long.trim().to_string();
    workspace.update_in(cx, |w, window, cx| {
        w.people_page
            .create_input
            .update(cx, |input, cx| input.set_value(long.clone(), window, cx));
        w.create_person_from_page(window, cx);
        assert_eq!(w.people_page.create_error, None);
        assert!(w.people_page.create_input.read(cx).value().is_empty());
        let hub = w.meetings_hub.read(cx);
        let created = hub
            .people
            .iter()
            .find(|p| p.display_name == expected)
            .expect("created profile");
        assert!(hub.person_is_pending(&created.id), "syncs when online");
        // The workspace file carries the local snapshot, so a reload keeps it.
        let snapshot = hub.people_snapshot();
        assert!(snapshot.pending_creates.contains(&created.id));
        assert!(snapshot.records.iter().any(|p| p.display_name == expected));
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-people-create-error").is_none());
    let card = cx.debug_bounds("desktop-person-card-0").expect("card");
    let page = cx.debug_bounds("desktop-people-page").expect("page");
    assert!(
        card.right() <= page.right(),
        "a long name stays inside its card"
    );
}

#[gpui::test]
async fn people_search_finds_an_alias_beyond_the_first_page(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    offline(&workspace, cx);
    workspace.update_in(cx, |w, _, cx| {
        let mut people = (0..60i64)
            .map(|n| {
                let mut p = person(&format!("p-{n:02}"), &format!("Person {n}"), &[]);
                p.updated_at = 100 + n;
                Arc::new(p)
            })
            .collect::<Vec<_>>();
        let mut late = person("late", "Late Arrival", &["Needle"]);
        late.updated_at = 1;
        people.push(Arc::new(late));
        w.meetings_hub.update(cx, |hub, _| hub.people = people);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-person-name-late").is_none());
    assert!(cx.debug_bounds("desktop-people-pager").is_some());

    workspace.update_in(cx, |w, window, cx| {
        w.people_page
            .search_input
            .update(cx, |input, cx| input.set_value("nEEDLE", window, cx));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-person-name-late").is_some());
    assert!(cx.debug_bounds("desktop-people-pager").is_none());

    workspace.update_in(cx, |w, window, cx| {
        w.people_page
            .search_input
            .update(cx, |input, cx| input.set_value("Nobody at all", window, cx));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-people-search-empty").is_some());

    // Paging without a search reaches the same person on the last page.
    workspace.update_in(cx, |w, window, cx| {
        w.people_page
            .search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        w.people_page.page.set(0);
        cx.notify();
    });
    workspace.update_in(cx, |w, _, _| assert_eq!(w.people_page.page.get(), 0));
    scroll_people_page_to(cx, "desktop-people-next");
    click(cx, "desktop-people-next");
    workspace.update_in(cx, |w, _, _| assert_eq!(w.people_page.page.get(), 1));
    scroll_people_page_to(cx, "desktop-people-next");
    click(cx, "desktop-people-next");
    workspace.update_in(cx, |w, _, _| assert_eq!(w.people_page.page.get(), 2));
    assert!(cx.debug_bounds("desktop-person-name-late").is_some());
    workspace.update_in(cx, |w, _, _| assert_eq!(w.people_page.page.get(), 2));
}

#[gpui::test]
async fn alias_that_another_profile_uses_is_refused_and_kept(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    offline(&workspace, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.people = vec![
                Arc::new(person("ada", "Ada", &["Countess"])),
                Arc::new(person("zoe", "Zoë", &[])),
            ];
        });
        w.expanded_person_id = Some("ada".into());
        w.people_page
            .alias_input
            .update(cx, |input, cx| input.set_value("zoË", window, cx));
        w.add_person_alias("ada", window, cx);
        let error = w.people_page.alias_error.clone().expect("refused");
        assert!(error.contains("Zoë already uses that name"), "{error}");
        assert_eq!(w.people_page.alias_input.read(cx).value().as_ref(), "zoË");
        assert_eq!(
            w.meetings_hub.read(cx).person("ada").unwrap().aliases,
            vec!["Countess".to_string()]
        );
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-person-aliases").is_some());
    assert!(cx.debug_bounds("desktop-person-alias-error").is_some());
}

#[gpui::test]
async fn merge_sends_nothing_until_confirmed(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    offline(&workspace, cx);
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.people = vec![
                Arc::new(person("source", "Sam", &[])),
                Arc::new(person("target", "Taylor", &[])),
            ];
        });
        w.merge = Some(PersonMergePanel {
            person_id: "source".into(),
            target_id: Some("target".into()),
            dropdown_open: false,
            confirming: false,
        });
        cx.notify();
    });
    click(cx, "desktop-people-merge-confirm");
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.merge.as_ref().is_some_and(|merge| merge.confirming));
        assert!(w.people_page.pending_merge.is_none());
        assert!(!w.meetings_hub.read(cx).person_merge_in_flight);
    });
    assert!(cx
        .debug_bounds("desktop-people-merge-confirm-text")
        .is_some());
    click(cx, "desktop-people-merge-commit");
    workspace.update_in(cx, |w, _, _| {
        assert!(w.merge.is_none());
        assert_eq!(
            w.people_page.pending_merge,
            Some(("source".to_string(), "target".to_string()))
        );
    });
}

#[gpui::test]
async fn delete_needs_confirmation_cascades_and_undo_restores(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    offline(&workspace, cx);
    let ada = person("ada", "Ada", &[]);
    let mut relationship = RelationshipProfile::new("ada", 1);
    relationship.add_item(
        RelationshipItem::new(RelationshipItemKind::Reminder, "Call back", 1),
        1,
    );
    let mut conversation = InboxConversation::new(
        InboxSource::Email,
        "Synthetic launch",
        vec![InboxParticipant {
            id: "participant".into(),
            display_name: "Ada".into(),
            address: "ada@example.invalid".into(),
            person_id: Some("ada".into()),
        }],
        1,
    );
    conversation.id = "conversation".into();
    let mut document = KnowledgeDocument::new("Synthetic notes", "", 1);
    document.links.push(KnowledgeLink {
        kind: KnowledgeLinkKind::Person,
        target_id: "ada".into(),
        label: "Ada".into(),
    });
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![Arc::new(ada.clone())]);
        w.relationships = vec![relationship.clone()];
        w.inbox_conversations = vec![conversation.clone()];
        w.knowledge_documents = vec![document.clone()];
        cx.notify();
    });
    click(cx, "desktop-person-delete-0");
    assert!(cx.debug_bounds("desktop-person-delete-confirm").is_some());
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.people_page.confirm_delete.as_deref(), Some("ada"));
        assert_eq!(
            w.meetings_hub.read(cx).people.len(),
            1,
            "nothing deleted yet"
        );
        // The server is unreachable here: simulate its confirmed delete.
        w.people_page.confirm_delete = None;
        w.meetings_hub.update(cx, |hub, cx| {
            hub.forget_person("ada", cx);
            cx.emit(HubEvent::PersonDeleted {
                person: Box::new(ada.clone()),
                local_only: true,
            });
        });
        cx.notify();
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.meetings_hub.read(cx).people.is_empty());
        assert!(w.relationships.is_empty(), "reminders go with the profile");
        assert_eq!(w.inbox_conversations.len(), 1, "conversations stay");
        assert_eq!(w.inbox_conversations[0].participants[0].person_id, None);
        assert_eq!(w.knowledge_documents.len(), 1, "documents stay");
        assert!(w.knowledge_documents[0].links.is_empty());
        assert!(matches!(
            w.people_page.undo,
            Some(PeopleUndo::Delete { .. })
        ));
    });
    assert!(cx.debug_bounds("desktop-people-undo-bar").is_some());
    click(cx, "desktop-people-undo");
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.meetings_hub.read(cx).person("ada").is_some());
        assert_eq!(w.relationships, vec![relationship.clone()]);
        assert_eq!(w.inbox_conversations, vec![conversation.clone()]);
        assert_eq!(w.knowledge_documents, vec![document.clone()]);
        assert!(w.people_page.undo.is_none());
        assert!(w.people_page.undo_in_flight.is_none());
    });
}

#[gpui::test]
async fn reminders_use_kind_and_due_options_and_can_be_edited_and_deleted(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    offline(&workspace, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.people = vec![Arc::new(person("ada", "Ada", &[]))]
        });
        w.expanded_person_id = Some("ada".into());
        w.relationship_item_kind = RelationshipItemKind::Reminder;
        w.people_page.due_tomorrow = true;
        w.relationship_item_input
            .update(cx, |input, cx| input.set_value("Call back", window, cx));
        w.add_relationship_item("ada", window, cx);
        let item = w.relationships[0].items[0].clone();
        assert_eq!(item.kind, RelationshipItemKind::Reminder);
        assert!(item.due_at_ms.is_some());
        assert!(!w.people_page.due_tomorrow);

        w.begin_item_edit("ada", &item.id, window, cx);
        w.people_page
            .item_edit_input
            .update(cx, |input, cx| input.set_value("   ", window, cx));
        w.save_item_edit(cx);
        assert_eq!(w.people_page.item_error, Some(people_copy::ITEM_BLANK));
        assert_eq!(w.relationships[0].items[0].text, "Call back");
        assert!(w.people_page.editing_item.is_some());

        w.people_page
            .item_edit_input
            .update(cx, |input, cx| input.set_value("Send the deck", window, cx));
        if let Some(edit) = w.people_page.editing_item.as_mut() {
            edit.kind = RelationshipItemKind::Commitment;
        }
        w.save_item_edit(cx);
        let edited = &w.relationships[0].items[0];
        assert_eq!(edited.text, "Send the deck");
        assert_eq!(edited.kind, RelationshipItemKind::Commitment);
        assert_eq!(edited.due_at_ms, item.due_at_ms, "the due time is kept");
        assert!(w.people_page.editing_item.is_none());
        cx.notify();
    });
    click(cx, "desktop-relationship-delete-0");
    workspace.update_in(cx, |w, _, _| assert!(w.relationships[0].items.is_empty()));
}

#[gpui::test]
async fn inbox_import_links_by_alias_and_refuses_ambiguous_names(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    offline(&workspace, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.people = vec![
                Arc::new(person("ada", "Ada Lovelace", &["Countess"])),
                Arc::new(person("zoe", "Zoë", &[])),
                Arc::new(person("zoe-2", "zoë", &[])),
            ];
        });
        for (sender, expected) in [("COUNTESS", Some("ada")), ("ZOË", None)] {
            w.inbox_sender_input
                .update(cx, |input, cx| input.set_value(sender, window, cx));
            w.inbox_body_input.update(cx, |input, cx| {
                input.set_value("Synthetic hello", window, cx)
            });
            w.save_inbox_import(window, cx);
            assert_eq!(
                w.inbox_conversations[0].participants[0]
                    .person_id
                    .as_deref(),
                expected,
                "{sender}"
            );
        }
    });
}
