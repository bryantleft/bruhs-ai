use super::super::bruh_page::BruhTab;
use super::input::{dom_key, modifiers};
use super::{ComputerView, ReactiveAutomations};
use crate::server_client::ScreencastEvent;
use crate::theme;
use crate::views::workspace::StopAll;
use bruhs_models::crew::{
    computer_status_line, AccessLevel, BruhAccess, BruhLook, BruhState, BruhStatus, CREW_PROTOCOL,
    LIVE_VIEW_HINT, USE_THE_WINDOW,
};
use bruhs_models::local_tools::{
    ComputerInput, ComputerRequest, ComputerState, Control, KeyKind, MouseButton, MouseKind,
    MODIFIER_META, MODIFIER_SHIFT,
};
use bruhs_models::reactive::{ReactiveAgent, ReactiveSnapshot};
use gpui::{
    AppContext as _, ClipboardItem, Entity, Focusable as _, KeyBinding, KeyUpEvent, Keystroke,
    Modifiers, VisualTestContext,
};
use std::{cell::Cell, cell::RefCell, rc::Rc};

fn snapshot() -> ReactiveSnapshot {
    ReactiveSnapshot {
        agents: vec![ReactiveAgent {
            id: "a".into(),
            name: "Ada".into(),
            instructions: String::new(),
            memory: String::new(),
            model: "claude-sonnet-4-6".into(),
            project_path: String::new(),
            enabled: true,
        }],
        bruhs: vec![BruhState {
            agent_id: "a".into(),
            name: "Ada".into(),
            look: BruhLook::default(),
            access: BruhAccess {
                level: AccessLevel::Full,
                ..BruhAccess::default()
            },
            effort: None,
            status: BruhStatus::Asleep,
            next_wake_ms: None,
            unread: 0,
            computer: true,
        }],
        protocol: CREW_PROTOCOL,
        ..ReactiveSnapshot::default()
    }
}

fn browser(control: Control, headed: bool) -> ComputerState {
    ComputerState {
        agent_id: "a".into(),
        session_id: Some("s".into()),
        url: Some("https://example.com/".into()),
        control,
        headed,
        ..ComputerState::default()
    }
}

/// A standalone panel window (F13) on the bruh's Computer tab.
fn open(
    cx: &mut gpui::TestAppContext,
    width: f32,
    height: f32,
) -> (
    Entity<ReactiveAutomations>,
    Entity<ComputerView>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        theme::apply_bruhs_theme(cx);
    });
    let holder = Rc::new(RefCell::new(None));
    let capture = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let panel = cx.new(ReactiveAutomations::new);
        *capture.borrow_mut() = Some(panel.clone());
        bruhs_ui::Root::new(panel, window, cx)
    });
    cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
    let panel = holder.borrow().clone().unwrap();
    panel.update(cx, |panel, cx| {
        panel.loaded = true;
        panel.set_snapshot(snapshot(), cx);
        panel.open_bruh_for_test("a", BruhTab::Computer, cx);
    });
    cx.run_until_parked();
    let view = panel
        .read_with(cx, |panel, _| panel.computer_view_for_test())
        .expect("the Computer tab shows its view");
    (panel, view, cx)
}

fn apply(view: &Entity<ComputerView>, cx: &mut VisualTestContext, event: ScreencastEvent) {
    view.update_in(cx, |view, window, cx| {
        view.apply_event_for_test(event, window, cx);
    });
}

fn inputs(panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext) -> Vec<ComputerInput> {
    panel.read_with(cx, |panel, _| panel.screencast_inputs_for_test().to_vec())
}

fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.debug_bounds(selector).is_some()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).expect(selector);
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

/// A 2×2 JPEG.
fn jpeg() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode_image(&image::RgbImage::new(2, 2))
        .unwrap();
    bytes
}

#[gpui::test]
fn a_closed_browser_offers_open_and_sends_it(cx: &mut gpui::TestAppContext) {
    let (panel, view, cx) = open(cx, 900.0, 1200.0);
    let status = view.read_with(cx, |view, cx| view.status(cx));
    assert_eq!(status, computer_status_line(None, true, "Ada"));
    assert!(shown(cx, "desktop-computer-status"));
    assert!(!shown(cx, "desktop-computer-take"), "no browser to take");
    click(cx, "desktop-computer-open");
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.computer_requests_for_test(),
            [("a".to_string(), ComputerRequest::Open)]
        );
    });
}

#[gpui::test]
fn frames_draw_and_are_acked_after_decode(cx: &mut gpui::TestAppContext) {
    let (_panel, view, cx) = open(cx, 900.0, 1200.0);
    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Bruh, false)),
    );
    cx.run_until_parked();
    assert!(
        !shown(cx, "desktop-computer-frame"),
        "nothing until a frame"
    );
    apply(&view, cx, ScreencastEvent::Frame(jpeg()));
    assert_eq!(
        view.read_with(cx, |view, _| view.acks),
        0,
        "acked after decode"
    );
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.acks), 1);
    assert!(shown(cx, "desktop-computer-frame"));
    let first = view.read_with(cx, |view, _| view.frame.clone()).unwrap();
    let painted = cx.update(|window, _| window.has_image_atlas_entry(&first));
    assert!(painted, "the frame is drawn");
    // Two more before the next decode ends: neither is acked early.
    apply(&view, cx, ScreencastEvent::Frame(jpeg()));
    apply(&view, cx, ScreencastEvent::Frame(jpeg()));
    assert_eq!(view.read_with(cx, |view, _| view.acks), 1);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.acks), 3);
    // The frame it replaced leaves the atlas.
    let kept = cx.update(|window, _| window.has_image_atlas_entry(&first));
    assert!(!kept, "the old frame is dropped");
    // A frame that can't be decoded is still acked, so the next comes.
    apply(&view, cx, ScreencastEvent::Frame(vec![1, 2, 3]));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.acks), 4);
    assert!(
        shown(cx, "desktop-computer-frame"),
        "the last good frame stays"
    );
}

#[gpui::test]
fn owner_keys_go_to_the_page_except_app_shortcuts(cx: &mut gpui::TestAppContext) {
    // The app's own modifier.
    let (stop, paste) = if cfg!(target_os = "macos") {
        ("cmd-.", "cmd-v")
    } else {
        ("ctrl-.", "ctrl-v")
    };
    let stops = Rc::new(Cell::new(0));
    let counted = stops.clone();
    cx.update(|cx| {
        cx.bind_keys([KeyBinding::new(stop, StopAll, None)]);
        cx.on_action(move |_: &StopAll, _| counted.set(counted.get() + 1));
    });
    let (panel, view, cx) = open(cx, 900.0, 1200.0);
    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Owner, false)),
    );
    cx.run_until_parked();
    let focus = |view: &Entity<ComputerView>, cx: &mut VisualTestContext| {
        view.update_in(cx, |view, window, cx| view.focus.focus(window, cx));
        cx.run_until_parked();
    };
    focus(&view, cx);
    let hint = view.update_in(cx, |view, window, cx| view.hint(window, cx));
    assert_eq!(hint, Some(LIVE_VIEW_HINT), "the hint shows while focused");
    cx.simulate_keystrokes("a");
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("a").unwrap(),
    });
    let key = |kind, key: &str, code: &str, modifiers| ComputerInput::Key {
        kind,
        key: key.into(),
        code: code.into(),
        modifiers,
    };
    assert_eq!(
        inputs(&panel, cx),
        [
            key(KeyKind::Down, "a", "KeyA", 0),
            key(KeyKind::Up, "a", "KeyA", 0)
        ]
    );
    // Tab goes to the page, not to the next control.
    cx.simulate_keystrokes("tab");
    assert_eq!(
        inputs(&panel, cx).last(),
        Some(&key(KeyKind::Down, "Tab", "Tab", 0))
    );
    assert!(view.update_in(cx, |view, window, _| view.focus.is_focused(window)));
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("tab").unwrap(),
    });
    let sent = inputs(&panel, cx).len();
    // ⌘. stays with the app and reaches its action.
    cx.simulate_keystrokes(stop);
    assert_eq!(inputs(&panel, cx).len(), sent, "no input");
    assert_eq!(stops.get(), 1, "the stop action ran");
    // ⌘V pastes the clipboard as text.
    cx.write_to_clipboard(ClipboardItem::new_string("pw".into()));
    cx.simulate_keystrokes(paste);
    assert_eq!(
        inputs(&panel, cx).last(),
        Some(&ComputerInput::Text { text: "pw".into() })
    );
    // F6 leaves for the address, releasing a key still held.
    cx.simulate_keystrokes("shift-b");
    assert_eq!(
        inputs(&panel, cx).last(),
        Some(&key(KeyKind::Down, "B", "KeyB", MODIFIER_SHIFT))
    );
    cx.simulate_keystrokes("f6");
    assert_eq!(
        inputs(&panel, cx).last(),
        Some(&key(KeyKind::Up, "B", "KeyB", 0)),
        "the held key comes up"
    );
    let at_address = view.update_in(cx, |view, window, cx| {
        view.address.focus_handle(cx).is_focused(window)
    });
    assert!(at_address, "focus is on the address");
    let hint = view.update_in(cx, |view, window, cx| view.hint(window, cx));
    assert_eq!(hint, None, "the hint goes");
}

#[gpui::test]
fn switching_apps_or_losing_input_releases_what_the_owner_holds(cx: &mut gpui::TestAppContext) {
    let (panel, view, cx) = open(cx, 900.0, 1200.0);
    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Owner, false)),
    );
    cx.run_until_parked();
    cx.update(|window, _| window.activate_window());
    view.update_in(cx, |view, window, cx| view.focus.focus(window, cx));
    cx.run_until_parked();
    let up = |key: &str, code: &str| ComputerInput::Key {
        kind: KeyKind::Up,
        key: key.into(),
        code: code.into(),
        modifiers: 0,
    };
    cx.simulate_keystrokes("shift-b");
    cx.deactivate_window();
    assert_eq!(
        inputs(&panel, cx).last(),
        Some(&up("B", "KeyB")),
        "the key comes up when the window goes"
    );
    // A window opens while a key is down: input turns off, but the owner
    // still holds control, so the release still goes.
    cx.update(|window, _| window.activate_window());
    view.update_in(cx, |view, window, cx| view.focus.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("c");
    let sent = inputs(&panel, cx).len();
    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Owner, true)),
    );
    cx.run_until_parked();
    let after = inputs(&panel, cx);
    assert_eq!(after.len(), sent + 1, "{after:?}");
    assert_eq!(after.last(), Some(&up("c", "KeyC")));
}

#[gpui::test]
fn hover_moves_over_the_view_go_once_per_frame(cx: &mut gpui::TestAppContext) {
    let (panel, view, cx) = open(cx, 640.0, 480.0);
    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Owner, false)),
    );
    cx.run_until_parked();
    let bounds = cx.debug_bounds("desktop-computer-live").unwrap();
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    assert!(inputs(&panel, cx).is_empty(), "moves wait for the frame");
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    assert_eq!(
        inputs(&panel, cx),
        [ComputerInput::Mouse {
            kind: MouseKind::Move,
            x: 640.0,
            y: 400.0,
            button: MouseButton::None,
            clicks: 0,
            modifiers: 0,
        }]
    );
    // Off the view, a hover sends nothing.
    let away = gpui::point(bounds.left() - gpui::px(40.0), bounds.center().y);
    cx.simulate_mouse_move(away, None, Modifiers::default());
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    assert_eq!(inputs(&panel, cx).len(), 1);
}

#[gpui::test]
fn a_frame_decoding_when_the_browser_changes_is_dropped(cx: &mut gpui::TestAppContext) {
    let (_panel, view, cx) = open(cx, 900.0, 1200.0);
    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Bruh, false)),
    );
    cx.run_until_parked();
    apply(&view, cx, ScreencastEvent::Frame(jpeg()));
    let mut next = browser(Control::Bruh, false);
    next.session_id = Some("s2".into());
    apply(&view, cx, ScreencastEvent::State(next));
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |view, _| view.frame.is_none()),
        "never shown"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.acks),
        0,
        "and never acked on the new socket"
    );
    assert!(!shown(cx, "desktop-computer-frame"));
}

#[gpui::test]
fn clicks_map_through_the_letterbox(cx: &mut gpui::TestAppContext) {
    let (panel, view, cx) = open(cx, 640.0, 480.0);
    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Owner, false)),
    );
    cx.run_until_parked();
    let bounds = cx.debug_bounds("desktop-computer-live").unwrap();
    cx.simulate_mouse_down(
        bounds.center(),
        gpui::MouseButton::Left,
        Modifiers::default(),
    );
    assert_eq!(
        inputs(&panel, cx),
        [ComputerInput::Mouse {
            kind: MouseKind::Down,
            x: 640.0,
            y: 400.0,
            button: MouseButton::Left,
            clicks: 1,
            modifiers: 0,
        }]
    );
    // A drag out past the left edge, released there, ends inside the page.
    let outside = gpui::point(bounds.left() - gpui::px(40.0), bounds.center().y);
    cx.simulate_mouse_move(outside, gpui::MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    cx.simulate_mouse_up(outside, gpui::MouseButton::Left, Modifiers::default());
    let sent = inputs(&panel, cx);
    assert!(
        matches!(
            sent[1],
            ComputerInput::Mouse { kind: MouseKind::Move, x, button: MouseButton::Left, .. } if x == 0.0
        ),
        "{sent:?}"
    );
    assert!(
        matches!(
            sent[2],
            ComputerInput::Mouse { kind: MouseKind::Up, x, button: MouseButton::Left, clicks: 1, .. } if x == 0.0
        ),
        "{sent:?}"
    );
    assert_eq!(sent.len(), 3);
    // A press on the frame focuses the view.
    assert!(view.update_in(cx, |view, window, _| view.focus.is_focused(window)));
}

#[gpui::test]
fn controls_follow_the_state(cx: &mut gpui::TestAppContext) {
    let (panel, view, cx) = open(cx, 900.0, 1200.0);
    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Bruh, false)),
    );
    cx.run_until_parked();
    assert!(shown(cx, "desktop-computer-take"));
    assert!(shown(cx, "desktop-computer-window"));
    assert!(shown(cx, "desktop-computer-close"));
    assert!(!shown(cx, "desktop-computer-hand-back"));
    assert!(!shown(cx, "desktop-computer-back"));
    assert!(!view.read_with(cx, |view, cx| view.address_editable(cx)));
    click(cx, "desktop-computer-window");
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.computer_requests_for_test(),
            [("a".to_string(), ComputerRequest::OpenWindow)]
        );
    });

    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Owner, false)),
    );
    cx.run_until_parked();
    assert!(shown(cx, "desktop-computer-hand-back"));
    assert!(shown(cx, "desktop-computer-back"));
    assert!(shown(cx, "desktop-computer-reload"));
    assert!(!shown(cx, "desktop-computer-take"));
    assert!(view.read_with(cx, |view, cx| view.address_editable(cx)));
    let url = view.read_with(cx, |view, cx| view.address.read(cx).value().to_string());
    assert_eq!(url, "https://example.com/");

    apply(
        &view,
        cx,
        ScreencastEvent::State(browser(Control::Owner, true)),
    );
    cx.run_until_parked();
    assert!(shown(cx, "desktop-computer-hand-back"));
    assert!(!shown(cx, "desktop-computer-take"));
    assert!(!shown(cx, "desktop-computer-window"));
    assert!(!shown(cx, "desktop-computer-back"));
    let (status, hint) = view.update_in(cx, |view, window, cx| {
        (view.status(cx), view.hint(window, cx))
    });
    assert_eq!(
        status,
        computer_status_line(Some(&browser(Control::Owner, true)), true, "Ada")
    );
    assert_eq!(hint, Some(USE_THE_WINDOW));

    // A failed request's state error shows on the status line.
    let mut failed = browser(Control::Bruh, false);
    failed.error = Some("The window couldn't open.".into());
    apply(&view, cx, ScreencastEvent::State(failed));
    cx.run_until_parked();
    let status = view.read_with(cx, |view, cx| view.status(cx));
    assert_eq!(status, "The window couldn't open.");
}

#[test]
fn gpui_keys_become_dom_keys() {
    let dom = |source: &'static str| {
        let keystroke = Keystroke::parse(source).unwrap().with_simulated_ime();
        dom_key(&keystroke)
    };
    let pair = |key: &str, code: &str| Some((key.to_string(), code.to_string()));
    assert_eq!(dom("enter"), pair("Enter", "Enter"));
    assert_eq!(dom("backspace"), pair("Backspace", "Backspace"));
    assert_eq!(dom("tab"), pair("Tab", "Tab"));
    assert_eq!(dom("escape"), pair("Escape", "Escape"));
    assert_eq!(dom("space"), pair(" ", "Space"));
    assert_eq!(dom("left"), pair("ArrowLeft", "ArrowLeft"));
    assert_eq!(dom("down"), pair("ArrowDown", "ArrowDown"));
    assert_eq!(dom("pageup"), pair("PageUp", "PageUp"));
    assert_eq!(dom("pagedown"), pair("PageDown", "PageDown"));
    assert_eq!(dom("home"), pair("Home", "Home"));
    assert_eq!(dom("end"), pair("End", "End"));
    assert_eq!(dom("delete"), pair("Delete", "Delete"));
    assert_eq!(dom("f1"), pair("F1", "F1"));
    assert_eq!(dom("f12"), pair("F12", "F12"));
    assert_eq!(dom("a"), pair("a", "KeyA"));
    assert_eq!(dom("shift-a"), pair("A", "KeyA"));
    assert_eq!(dom("7"), pair("7", "Digit7"));
    assert_eq!(dom("."), pair(".", "Period"));
    assert_eq!(dom("cmd-v"), pair("v", "KeyV"));
    assert_eq!(dom("f13"), None, "keys the page view doesn't know stay");
    let all = Modifiers {
        control: true,
        alt: true,
        shift: true,
        platform: true,
        function: false,
    };
    assert_eq!(modifiers(&all), 15);
    assert_eq!(modifiers(&Modifiers::command()), MODIFIER_META);
}
