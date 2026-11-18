use super::{ReactiveAutomations, ReactiveOpenRoomSettings};
use crate::theme;
use bruhs_models::crew::room::{
    Room, RoomAuthor, RoomMessage, RoomState, NO_MEMBERS, ROOM_GONE, ROOM_LEAD,
};
use bruhs_models::crew::{
    BruhAccess, BruhLook, BruhState, BruhStatus, ServiceNotice, CREW_PROTOCOL,
};
use bruhs_models::logic::context_chip::ChipKind;
use bruhs_models::reactive::{
    OutboxItem, OutboxSignal, ReactiveCommand, ReactiveRun, ReactiveSnapshot, RunTrigger,
};
use bruhs_models::EpochMs;
use gpui::{AppContext as _, Entity, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

fn named(id: &str, name: &str) -> BruhState {
    BruhState {
        agent_id: id.into(),
        name: name.into(),
        look: BruhLook::default(),
        access: BruhAccess::default(),
        effort: None,
        status: BruhStatus::Asleep,
        next_wake_ms: None,
        unread: 0,
    }
}

fn room(members: &[&str], lead: Option<&str>) -> RoomState {
    RoomState {
        room: Room {
            id: "r".into(),
            name: "Launch".into(),
            members: members.iter().map(|id| (*id).to_string()).collect(),
            lead: lead.map(str::to_string),
        },
        last_seq: 4,
        working: Vec::new(),
    }
}

fn snapshot(protocol: u32, bruhs: Vec<BruhState>, rooms: Vec<RoomState>) -> ReactiveSnapshot {
    ReactiveSnapshot {
        protocol,
        bruhs,
        rooms,
        ..ReactiveSnapshot::default()
    }
}

fn message(seq: u64, author: RoomAuthor, text: &str) -> RoomMessage {
    RoomMessage {
        id: seq.to_string(),
        room_id: "r".into(),
        seq,
        author,
        text: text.into(),
        run_id: None,
        outbox_id: None,
        failed: false,
        created_at: EpochMs::saturating_from_i64(1),
    }
}

fn by(agent_id: &str) -> RoomAuthor {
    RoomAuthor::Bruh {
        agent_id: agent_id.into(),
    }
}

fn approval(id: &str) -> OutboxItem {
    serde_json::from_value(serde_json::json!({
        "id": id, "agent_id": "a", "run_id": "ra", "kind": "ask",
        "title": "Approval", "message": "", "created_at": 1,
        "approval": {
            "tool": "SandboxNetworkAccess", "summary": "Reach example.com",
            "fingerprint": "f", "state": "waiting",
        },
    }))
    .unwrap()
}

fn room_run(id: &str, agent_id: &str) -> ReactiveRun {
    let mut run: ReactiveRun = serde_json::from_value(serde_json::json!({
        "id": id, "agent_id": agent_id, "source_id": null, "subscription_id": null,
        "event_id": null, "status": "running", "prompt": id, "thread_id": "t",
        "session_id": null, "created_at": 1, "updated_at": 1,
        "not_before": 1, "result": null, "error": null,
    }))
    .unwrap();
    run.trigger = Some(RunTrigger::Room {
        room_id: "r".into(),
        message_id: "1".into(),
        hop: 0,
        resumed: false,
    });
    run
}

/// A standalone panel window on room `r`'s page.
fn open(
    cx: &mut gpui::TestAppContext,
    snapshot: ReactiveSnapshot,
) -> (Entity<ReactiveAutomations>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::apply_bruhs_theme(cx);
    });
    let holder = Rc::new(RefCell::new(None));
    let capture = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let panel = cx.new(ReactiveAutomations::new);
        *capture.borrow_mut() = Some(panel.clone());
        gpui_component::Root::new(panel, window, cx)
    });
    cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(1200.0)));
    let panel = holder.borrow().clone().unwrap();
    panel.update(cx, |panel, cx| {
        panel.loaded = true;
        panel.set_snapshot(snapshot, cx);
        panel.show_room(Some("r".into()), cx);
    });
    cx.run_until_parked();
    (panel, cx)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).expect(selector);
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

/// A key press as the app gets it, without the newline the test
/// platform would also type.
fn press(cx: &mut VisualTestContext, key: &str) {
    let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

fn sent(panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext) -> Vec<ReactiveCommand> {
    panel.update(cx, |panel, _| panel.sent_for_test().to_vec())
}

/// Focus the composer and type `text` into it.
fn type_in(panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext, text: &str) {
    panel.update_in(cx, |panel, window, cx| {
        let composer = panel.room.as_ref().unwrap().composer.clone().unwrap();
        composer.update(cx, |input, cx| input.focus(window, cx));
    });
    cx.run_until_parked();
    cx.simulate_input(text);
    cx.run_until_parked();
}

fn draft(panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext) -> String {
    panel.update(cx, |panel, cx| {
        let composer = panel.room.as_ref().unwrap().composer.clone().unwrap();
        composer.read(cx).value().to_string()
    })
}

#[gpui::test]
fn a_room_shows_messages_faces_system_lines_and_cards(cx: &mut gpui::TestAppContext) {
    let mut shown = snapshot(
        CREW_PROTOCOL,
        vec![named("a", "Ada"), named("b", "Agent b")],
        vec![room(&["a", "b"], Some("a"))],
    );
    shown.outbox = vec![approval("i1")];
    shown.runs = vec![room_run("rb", "b")];
    let (panel, cx) = open(cx, shown);
    let mut asked = message(4, by("a"), "Reach example.com");
    asked.outbox_id = Some("i1".into());
    panel.update(cx, |panel, cx| {
        panel.show_room_messages_for_test(vec![
            asked,
            message(3, RoomAuthor::System, "Bo joined."),
            message(2, by("a"), "On it."),
            message(1, RoomAuthor::Owner, "@Ada ship it"),
        ]);
        cx.notify();
    });
    cx.run_until_parked();
    let owner = cx
        .debug_bounds("room-line-owner-1")
        .expect("the owner's line");
    let bruh = cx.debug_bounds("room-line-bruh-2").expect("Ada's line");
    let face = cx.debug_bounds("room-face-bruh-2").expect("Ada's face");
    assert!(bruh.contains(&face.center()), "the face sits in its line");
    assert_eq!(face.left(), bruh.left(), "the face leads the line");
    let system = cx
        .debug_bounds("room-line-system-3")
        .expect("the system line");
    assert!(owner.bottom() <= bruh.top() && bruh.bottom() <= system.top());
    let card = cx
        .debug_bounds("room-line-card-4")
        .expect("the approval card");
    let approve = cx.debug_bounds("desktop-bruh-approve-i1").expect("Approve");
    assert!(card.contains(&approve.center()), "Approve sits in the card");
    let working = cx
        .debug_bounds("room-line-working-rb")
        .expect("b's working line");
    assert!(card.bottom() <= working.top(), "working comes last");
    panel.update(cx, |panel, _| {
        assert_eq!(panel.room_working_text("b"), "Agent b is working");
    });
    // The card keeps the chat's card width beside the face.
    let header = cx.debug_bounds("desktop-room-header").expect("header");
    assert!(cx.debug_bounds("desktop-room-face-a").is_some());
    assert!(header.contains(
        &cx.debug_bounds("desktop-room-settings")
            .expect("Room settings")
            .center()
    ));
    click(cx, "desktop-bruh-approve-i1");
    assert_eq!(
        sent(&panel, cx).last(),
        Some(&ReactiveCommand::DecideApproval {
            id: "i1".into(),
            approve: true,
        })
    );
}

/// A bruh's message draws as the thread chat's Markdown; the owner's words
/// and a failed reply stay as written.
#[gpui::test]
fn a_bruhs_message_draws_as_markdown(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = open(
        cx,
        snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada")],
            vec![room(&["a"], Some("a"))],
        ),
    );
    let mut failed = message(3, by("a"), "**Ran out** of road");
    failed.failed = true;
    panel.update(cx, |panel, cx| {
        panel.show_room_messages_for_test(vec![
            failed,
            message(
                2,
                by("a"),
                "- **Food** and drinks\n- ![map](https://example.com/m.png)",
            ),
            message(1, RoomAuthor::Owner, "**pack** it"),
        ]);
        cx.notify();
    });
    cx.run_until_parked();
    let line = cx.debug_bounds("room-line-bruh-2").expect("Ada's line");
    let prose = cx.debug_bounds("room-md-2").expect("Ada's Markdown");
    assert!(line.contains(&prose.center()));
    assert!(cx.debug_bounds("room-md-1").is_none(), "the owner's words");
    assert!(cx.debug_bounds("room-md-3").is_none(), "a failed reply");
    panel.read_with(cx, |panel, _| {
        let page = panel.room.as_ref().unwrap();
        let markdown = page.prose.get("2").expect("built with the lines");
        assert!(
            !markdown.contains("!["),
            "a remote image is a link, never loaded: {markdown}"
        );
    });
}

#[gpui::test]
fn at_lists_members_and_enter_inserts_a_chip(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = open(
        cx,
        snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada"), named("b", "Bo")],
            vec![room(&["a", "b"], Some("b"))],
        ),
    );
    type_in(&panel, cx, "@b");
    panel.update(cx, |panel, _| {
        assert_eq!(panel.room.as_ref().unwrap().hits, ["b"]);
    });
    let list = cx
        .debug_bounds("desktop-room-mentions")
        .expect("the @ list");
    let row = cx.debug_bounds("desktop-room-mention-b").expect("Bo's row");
    assert!(list.contains(&row.center()));
    let lead = cx
        .debug_bounds("desktop-room-mention-lead-b")
        .expect("Bo's row says Lead");
    assert!(row.contains(&lead.center()));
    // Escape closes the list until the draft changes.
    press(cx, "escape");
    assert!(cx.debug_bounds("desktop-room-mentions").is_none());
    cx.simulate_input("o");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-room-mention-b").is_some());
    press(cx, "enter");
    assert_eq!(draft(&panel, cx), "@Bo ");
    panel.update(cx, |panel, _| {
        let page = panel.room.as_ref().unwrap();
        assert!(page.mention.is_none(), "picking closes the list");
        let [chip] = page.chips.as_slice() else {
            panic!("one chip, got {:?}", page.chips);
        };
        assert_eq!((chip.start, chip.end), (0, 3));
        assert_eq!(
            chip.kind,
            ChipKind::Bruh {
                agent_id: "b".into(),
                name: "Bo".into(),
            }
        );
    });
    assert!(sent(&panel, cx).is_empty(), "picking posts nothing");
    // Backspace at the chip's end takes the whole name out.
    press(cx, "backspace");
    press(cx, "backspace");
    assert_eq!(draft(&panel, cx), "");
    panel.update(cx, |panel, _| {
        assert!(panel.room.as_ref().unwrap().chips.is_empty());
    });
}

#[gpui::test]
fn posting_sends_post_room_and_clears(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = open(
        cx,
        snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada"), named("b", "Bo")],
            vec![room(&["a", "b"], None)],
        ),
    );
    assert!(cx.debug_bounds("desktop-room-hint").is_some(), "no lead");
    type_in(&panel, cx, "@Bo check");
    panel.update(cx, |panel, _| {
        assert!(panel.room.as_ref().unwrap().mention.is_none());
    });
    press(cx, "enter");
    assert_eq!(
        sent(&panel, cx).last(),
        Some(&ReactiveCommand::PostRoom {
            room_id: "r".into(),
            text: "@Bo check".into(),
        })
    );
    assert_eq!(draft(&panel, cx), "");
}

/// The service refused the post: its words come back to write again, unless
/// the owner has started a new one; a post that went through stays gone.
#[gpui::test]
fn a_refused_post_comes_back_into_the_composer(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = open(
        cx,
        snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada")],
            vec![room(&["a"], Some("a"))],
        ),
    );
    type_in(&panel, cx, "Pack the van");
    press(cx, "enter");
    assert_eq!(draft(&panel, cx), "");
    panel.update(cx, |panel, cx| {
        panel.command_failed_for_test("The service is busy.");
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(draft(&panel, cx), "Pack the van");
    // Sent again and taken: a later failure leaves the composer alone.
    press(cx, "enter");
    panel.update(cx, |panel, cx| {
        panel.room_posted_for_test(cx);
        panel.command_failed_for_test("The service is busy.");
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(draft(&panel, cx), "");
}

/// Web `px-3 pt-2.5` on the textarea: the text sits 12px in and 10px
/// down from the composer's ring, with no field padding on top.
#[gpui::test]
fn the_composer_text_starts_where_the_web_textarea_does(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = open(
        cx,
        snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada")],
            vec![room(&["a"], Some("a"))],
        ),
    );
    type_in(&panel, cx, "Hi");
    let surface = cx
        .debug_bounds("desktop-room-composer")
        .expect("the composer");
    let glyph = panel
        .update(cx, |panel, cx| {
            let composer = panel.room.as_ref().unwrap().composer.clone().unwrap();
            composer.read(cx).range_to_bounds(&(0..1))
        })
        .expect("the first letter is laid out");
    // The desktop ring is a 1px border inside the bounds.
    let left = f32::from(glyph.left() - surface.left()) - 1.0;
    let top = f32::from(glyph.top() - surface.top()) - 1.0;
    assert!((left - 12.0).abs() <= 1.0, "text starts {left}px in");
    assert!((top - 10.0).abs() <= 3.0, "text starts {top}px down");
    // Web `EDITOR_MIN_HEIGHT_PX` (52) for the field and 36 for the Send
    // row: 88px in all.
    let height = f32::from(surface.size.height);
    assert!(
        (height - 88.0).abs() <= 1.0,
        "the composer is {height}px tall"
    );
}

#[gpui::test]
fn the_composer_is_off_for_an_empty_room_or_an_older_service(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = open(
        cx,
        snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada")],
            vec![room(&[], None)],
        ),
    );
    panel.update(cx, |panel, _| {
        assert_eq!(panel.room_composer_reason(), Some(NO_MEMBERS));
    });
    assert!(cx.debug_bounds("desktop-room-blocked").is_some());
    assert!(cx.debug_bounds("desktop-room-hint").is_none());
    panel.update_in(cx, |panel, window, cx| {
        let composer = panel.room.as_ref().unwrap().composer.clone().unwrap();
        composer.update(cx, |input, cx| input.set_value("hello", window, cx));
        panel.post_room(window, cx);
    });
    click(cx, "desktop-room-send");
    // A room on a service from before rooms.
    panel.update(cx, |panel, cx| {
        panel.set_snapshot(
            snapshot(1, vec![named("a", "Ada")], vec![room(&["a"], None)]),
            cx,
        );
        cx.notify();
    });
    cx.run_until_parked();
    panel.update_in(cx, |panel, window, cx| {
        assert_eq!(
            panel.room_composer_reason(),
            Some(ServiceNotice::ServingOld.message())
        );
        panel.post_room(window, cx);
    });
    click(cx, "desktop-room-send");
    assert!(sent(&panel, cx).is_empty());
}

/// A redraw with nothing new reuses the lines instead of building them
/// again on every frame.
#[gpui::test]
fn the_lines_are_built_only_when_messages_or_the_snapshot_change(cx: &mut gpui::TestAppContext) {
    let crew = vec![named("a", "Ada")];
    let mut shown = snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]);
    shown.runs = vec![room_run("ra", "a")];
    let (panel, cx) = open(cx, shown);
    let lines = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
        panel.read_with(cx, |panel, _| {
            Rc::clone(&panel.room.as_ref().unwrap().lines)
        })
    };
    let first = lines(&panel, cx);
    assert_eq!(first.len(), 1, "the working line");
    panel.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert!(
        Rc::ptr_eq(&first, &lines(&panel, cx)),
        "rebuilt on a redraw"
    );
    panel.update(cx, |panel, cx| {
        panel.show_room_messages_for_test(vec![message(1, RoomAuthor::Owner, "hi")]);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(lines(&panel, cx).len(), 2, "new messages rebuild them");
    panel.update(cx, |panel, cx| {
        panel.set_snapshot(
            snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]),
            cx,
        );
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(lines(&panel, cx).len(), 1, "a new snapshot rebuilds them");
}

#[gpui::test]
fn load_earlier_leaves_the_reader_where_they_were(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = open(
        cx,
        snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada")],
            vec![room(&["a"], Some("a"))],
        ),
    );
    let page_of = |seqs: std::ops::RangeInclusive<u64>| -> Vec<RoomMessage> {
        seqs.rev()
            .map(|seq| message(seq, RoomAuthor::Owner, &format!("Message {seq}")))
            .collect()
    };
    let top = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
        panel.read_with(cx, |panel, _| {
            panel
                .room
                .as_ref()
                .unwrap()
                .list
                .state
                .logical_scroll_top()
                .item_ix
        })
    };
    panel.update(cx, |panel, cx| {
        panel.show_room_messages_for_test(page_of(51..=100));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(top(&panel, cx) > 1, "opens on the newest line");
    // The reader goes up to the first line under Load earlier.
    panel.update(cx, |panel, cx| {
        let page = panel.room.as_ref().unwrap();
        page.list.state.scroll_to(gpui::ListOffset {
            item_ix: 1,
            offset_in_item: gpui::px(0.0),
        });
        cx.notify();
    });
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        panel.show_earlier_room_messages_for_test(page_of(1..=50));
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.room.as_ref().unwrap().lines.len()),
        100
    );
    assert_eq!(
        top(&panel, cx),
        51,
        "Message 51 is still the first line in view, under the 50 that loaded"
    );
}

#[gpui::test]
fn room_settings_asks_the_workspace_and_a_deleted_room_hides_the_page(
    cx: &mut gpui::TestAppContext,
) {
    let crew = vec![named("a", "Ada")];
    let (panel, cx) = open(
        cx,
        snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]),
    );
    let asked = Rc::new(RefCell::new(Vec::new()));
    let seen = asked.clone();
    let _watch = cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &ReactiveOpenRoomSettings, _| {
            seen.borrow_mut().push(event.clone());
        })
    });
    click(cx, "desktop-room-settings");
    assert_eq!(
        *asked.borrow(),
        [ReactiveOpenRoomSettings {
            room_id: "r".into()
        }]
    );
    // The room goes while Room settings has focus.
    panel.update_in(cx, |panel, window, cx| {
        panel
            .room
            .as_ref()
            .unwrap()
            .settings_focus
            .focus(window, cx);
        panel.set_snapshot(snapshot(CREW_PROTOCOL, crew, Vec::new()), cx);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-room-gone").is_some(),
        "{ROOM_GONE}"
    );
    assert!(cx.debug_bounds("desktop-room-settings").is_none());
    assert!(cx.debug_bounds("desktop-room-composer").is_none());
    panel.update_in(cx, |panel, window, _| {
        assert!(
            panel.room.as_ref().unwrap().gone_focus.is_focused(window),
            "focus lands on the notice"
        );
    });
}

/// A new message moves the room's `last_seq`, from a snapshot or a live
/// crew frame: the page reads its newest messages again.
#[gpui::test]
fn a_new_last_seq_reads_the_newest_page_again(cx: &mut gpui::TestAppContext) {
    let crew = vec![named("a", "Ada")];
    let (panel, cx) = open(
        cx,
        snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]),
    );
    let read_for = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
        panel.read_with(cx, |panel, _| panel.room.as_ref().unwrap().read_for)
    };
    assert_eq!(read_for(&panel, cx), Some(4));
    let mut moved = room(&["a"], None);
    moved.last_seq = 6;
    panel.update(cx, |panel, cx| {
        // A snapshot read is already on its way.
        panel.reading = true;
        panel.apply_crew_signal(
            OutboxSignal {
                bruhs: crew,
                rooms: vec![moved],
                protocol: CREW_PROTOCOL,
                ..OutboxSignal::default()
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(read_for(&panel, cx), Some(6));
}

/// Like the web, a live crew frame reads the snapshot again only when its
/// bruhs or rooms changed.
#[gpui::test]
fn only_a_changed_crew_frame_reads_the_snapshot_again(cx: &mut gpui::TestAppContext) {
    let crew = vec![named("a", "Ada")];
    let (panel, cx) = open(
        cx,
        snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]),
    );
    let frame = |rooms: Vec<RoomState>| OutboxSignal {
        bruhs: crew.clone(),
        rooms,
        protocol: CREW_PROTOCOL,
        ..OutboxSignal::default()
    };
    let reads = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
        panel.read_with(cx, |panel, _| panel.last_refresh.is_some())
    };
    panel.update(cx, |panel, cx| {
        panel.reading = false;
        panel.last_refresh = None;
        panel.apply_crew_signal(frame(vec![room(&["a"], None)]), cx);
    });
    assert!(!reads(&panel, cx), "nothing changed");
    let mut working = room(&["a"], None);
    working.working = vec!["a".into()];
    panel.update(cx, |panel, cx| {
        panel.apply_crew_signal(frame(vec![working]), cx)
    });
    assert!(reads(&panel, cx), "the room changed");
}

/// Up and Down move through the `@` list and wrap; Enter picks the
/// selected member; Tab leaves the list alone and inserts nothing.
#[gpui::test]
fn up_and_down_wrap_and_tab_inserts_nothing(cx: &mut gpui::TestAppContext) {
    let (panel, cx) = open(
        cx,
        snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada"), named("b", "Bo")],
            vec![room(&["a", "b"], Some("b"))],
        ),
    );
    let selected = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
        panel.read_with(cx, |panel, _| panel.room.as_ref().unwrap().selected)
    };
    type_in(&panel, cx, "@");
    panel.update(cx, |panel, _| {
        assert_eq!(panel.room.as_ref().unwrap().hits, ["a", "b"]);
    });
    // Only the lead's row says Lead.
    assert!(cx.debug_bounds("desktop-room-mention-lead-b").is_some());
    assert!(cx.debug_bounds("desktop-room-mention-lead-a").is_none());
    assert_eq!(ROOM_LEAD, "Lead");
    assert_eq!(selected(&panel, cx), 0);
    press(cx, "down");
    assert_eq!(selected(&panel, cx), 1);
    press(cx, "down");
    assert_eq!(selected(&panel, cx), 0, "Down from the last wraps");
    press(cx, "up");
    assert_eq!(selected(&panel, cx), 1, "Up from the first wraps");
    press(cx, "enter");
    assert_eq!(draft(&panel, cx), "@Bo ");
    cx.simulate_input("@");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-room-mentions").is_some());
    press(cx, "tab");
    panel.update(cx, |panel, _| {
        let page = panel.room.as_ref().unwrap();
        assert_eq!(page.chips.len(), 1, "Tab inserts no chip");
    });
    assert!(!draft(&panel, cx).contains("@Ada"));
    assert!(sent(&panel, cx).is_empty());
}

/// Web `min-w-0 flex-1` on the title: a long members line truncates
/// and Room settings stays beside the name.
#[gpui::test]
fn a_long_members_line_keeps_room_settings_beside_the_name(cx: &mut gpui::TestAppContext) {
    let ids = ["a", "b", "c", "d", "e", "f"];
    let crew: Vec<BruhState> = ids
        .iter()
        .map(|id| named(id, &format!("Bruh with a long name {id}")))
        .collect();
    let (_, cx) = open(cx, snapshot(CREW_PROTOCOL, crew, vec![room(&ids, None)]));
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    cx.run_until_parked();
    let name = cx.debug_bounds("desktop-room-name").expect("the name");
    let settings = cx
        .debug_bounds("desktop-room-settings")
        .expect("Room settings");
    let line = cx.debug_bounds("desktop-room-members").expect("members");
    assert!(
        settings.top() < name.bottom(),
        "Room settings sits on the name's row: {settings:?} {name:?}"
    );
    assert!(line.right() <= settings.left(), "the line stops before it");
    assert!(
        line.size.height < gpui::px(20.0),
        "one line: {:?}",
        line.size.height
    );
}
