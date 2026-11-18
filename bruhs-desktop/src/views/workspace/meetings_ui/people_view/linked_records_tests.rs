//! PEOPLE-06 on desktop (docs/people-completion-2026-11-14.md), as the web
//! People harness checks it: a profile opens the person's meeting, Inbox
//! conversation and follow up, each showing Back to People, which returns to
//! the open profile. Synthetic records; the hub talks to a closed loopback
//! port (9), never a bruhs-server running on this machine.

use super::super::super::{AppView, Workspace};
use crate::server_client::ServerClient;
use crate::{app_state::DesktopState, theme};
use bruhs_models::{
    people_copy, InboxConversation, InboxMessage, InboxParticipant, InboxSource, Meeting,
    PersonProfile, Speaker,
};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

fn workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let captured = cell.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *captured.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().expect("workspace");
    // Tall enough that an open profile's follow ups are inside the window.
    cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(2400.0)));
    workspace.update(cx, |workspace, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9")
                    .expect("a loopback client"),
            );
        });
    });
    (workspace, cx)
}

fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.debug_bounds(selector).is_some()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let target = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(target.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

fn person(id: &str, name: &str) -> PersonProfile {
    let mut person = PersonProfile::new(name);
    person.id = id.into();
    person
}

fn meeting(id: &str, title: &str, at: f64, speaker: &str) -> Meeting {
    let mut meeting = Meeting::new(title, at);
    meeting.id = id.into();
    meeting.speakers.push(Speaker {
        id: "them_1".into(),
        label: "Them".into(),
        person_id: Some(speaker.into()),
        color: None,
    });
    meeting
}

/// A conversation whose last message came in, so it is owed a reply.
fn unanswered(id: &str, subject: &str, person_id: &str, at: i64) -> InboxConversation {
    let mut conversation = InboxConversation::new(
        InboxSource::Email,
        subject,
        vec![InboxParticipant {
            id: format!("participant-{person_id}"),
            display_name: person_id.into(),
            address: format!("{person_id}@example.invalid"),
            person_id: Some(person_id.into()),
        }],
        at,
    );
    conversation.id = id.into();
    conversation.receive(InboxMessage::incoming(person_id, "Any update?", at));
    conversation
}

fn view(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> AppView {
    workspace.read_with(cx, |workspace, _| workspace.active_view.clone())
}

/// Back to People returns to People with the profile open.
fn back_to_ada(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    assert!(shown(cx, "desktop-back-to-people"));
    click(cx, "desktop-back-to-people");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.active_view, AppView::People);
        assert_eq!(workspace.expanded_person_id.as_deref(), Some("ada"));
    });
    assert!(shown(cx, "desktop-person-links"), "the profile is open");
}

#[gpui::test]
fn a_profile_opens_its_meeting_and_conversation_and_comes_back(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    let now = bruhs_models::EpochMs::now().as_millis();
    workspace.update(cx, |workspace, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.meetings = vec![
                meeting("planning", "Synthetic planning sync", 2.0, "ada"),
                meeting("untitled", "  ", 1.0, "ada"),
                meeting("grace-only", "Grace's review", 3.0, "grace"),
            ];
            hub.people = vec![person("ada", "Ada").into(), person("grace", "Grace").into()];
        });
        workspace.inbox_conversations = vec![
            unanswered("lunch", "Lunch plans", "ada", now - 60_000),
            unanswered("grace-question", "Grace's question", "grace", now - 30_000),
        ];
        workspace.active_view = AppView::People;
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-person-expand-0");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.expanded_person_id.as_deref(), Some("ada"));
    });

    // Ada's records only, newest meeting first; a blank title reads as the
    // web's "Untitled meeting".
    assert!(shown(cx, "desktop-person-meeting-0"));
    assert!(shown(cx, "desktop-person-meeting-1"));
    assert!(
        !shown(cx, "desktop-person-meeting-2"),
        "not Grace's meeting"
    );
    assert!(shown(cx, "desktop-person-conversation-0"));
    assert!(!shown(cx, "desktop-person-conversation-1"));
    workspace.update(cx, |workspace, cx| {
        let hub = workspace.meetings_hub.read(cx);
        let ada = hub.person("ada").expect("Ada").clone();
        let titles: Vec<&str> = bruhs_models::person_meetings(&ada, &hub.meetings)
            .into_iter()
            .map(|meeting| people_copy::meeting_link_title(&meeting.title))
            .collect();
        assert_eq!(titles, ["Synthetic planning sync", "Untitled meeting"]);
    });
    // The untitled meeting's link carries its label, not an empty button.
    let untitled = cx
        .debug_bounds("desktop-person-meeting-1")
        .expect("the untitled meeting's link");
    assert!(
        f32::from(untitled.size.width) > 80.0,
        "an empty title drew a bare button: {untitled:?}"
    );

    // Follow ups: Ada's reply only, never Grace's conversation.
    assert!(shown(cx, "desktop-follow-ups"));
    assert!(shown(cx, "desktop-follow-up-0"));
    assert!(!shown(cx, "desktop-follow-up-1"), "not Grace's reply");

    // A meeting, and back.
    click(cx, "desktop-person-meeting-0");
    assert_eq!(
        view(&workspace, cx),
        AppView::MeetingDetail {
            meeting_id: "planning".into()
        }
    );
    back_to_ada(&workspace, cx);

    // A conversation, and back.
    click(cx, "desktop-person-conversation-0");
    assert_eq!(
        view(&workspace, cx),
        AppView::ConversationDetail {
            conversation_id: "lunch".into()
        }
    );
    back_to_ada(&workspace, cx);

    // The follow up opens the conversation it is about, and back.
    click(cx, "desktop-follow-up-0");
    assert_eq!(
        view(&workspace, cx),
        AppView::ConversationDetail {
            conversation_id: "lunch".into()
        }
    );
    back_to_ada(&workspace, cx);

    // Leaving another way forgets the way back, as on the web: the same
    // meeting opened again from Meetings has no Back to People.
    click(cx, "desktop-person-meeting-0");
    assert!(shown(cx, "desktop-back-to-people"));
    workspace.update(cx, |workspace, cx| {
        workspace.open_meetings_list(cx);
    });
    cx.run_until_parked();
    workspace.update(cx, |workspace, cx| {
        workspace.open_meeting_detail("planning".into(), cx);
    });
    cx.run_until_parked();
    assert!(!shown(cx, "desktop-back-to-people"));
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.people_page.return_to.is_none());
    });
}
