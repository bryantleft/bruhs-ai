//! Chat view behavior: which turn's events apply, stop/resubmit, render
//! caching, git through bruhs-core, and the Claude login wait.

use super::{AgentUiEvent, ChatView, MessageRole};
use crate::{app_state::DesktopState, theme};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::task::JoinHandle;

mod prompt_suggestion;
mod rich_responses;
mod streaming_costs;
mod ui_audit;

pub(in crate::views::chat) fn chat_window(
    cx: &mut TestAppContext,
) -> (Entity<ChatView>, &mut VisualTestContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state should initialize"));
    });
    let cell: Rc<RefCell<Option<Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let capture = cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.start_thread("thread-stream", "Stream thread", cx);
            chat
        });
        *capture.borrow_mut() = Some(chat.clone());
        bruhs_ui::Root::new(chat, window, cx)
    });
    let chat = cell
        .borrow()
        .clone()
        .expect("chat entity should be captured");
    (chat, cx)
}

/// A sender task that never finishes on its own (only an abort ends it).
fn pending_sender(cx: &mut VisualTestContext) -> JoinHandle<()> {
    cx.update(|_, cx| {
        cx.global::<DesktopState>()
            .runtime
            .spawn(std::future::pending::<()>())
    })
}

fn start_turn(
    chat: &Entity<ChatView>,
    prompt: &str,
    cx: &mut VisualTestContext,
) -> UnboundedSender<AgentUiEvent> {
    let sender = pending_sender(cx);
    let (tx, rx) = unbounded_channel();
    chat.update(cx, |chat, cx| {
        chat.start_turn_for_test(prompt, sender, rx, cx)
    });
    tx
}

fn send(tx: &UnboundedSender<AgentUiEvent>, event: AgentUiEvent) {
    // Sending to a receiver that was dropped is exactly what a stale turn
    // does; the tests assert on what the view shows, not on delivery.
    let _delivered = tx.send(event).is_ok();
}

fn assistant_contents(chat: &Entity<ChatView>, cx: &mut VisualTestContext) -> Vec<String> {
    chat.read_with(cx, |chat, _| {
        chat.messages
            .iter()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .map(|message| message.content.clone())
            .collect()
    })
}

fn paused_rich_rendering(
    chat: &Entity<ChatView>,
    cx: &mut VisualTestContext,
) -> super::svg_cache::RenderGates {
    let gates = super::svg_cache::RenderGates {
        math: std::sync::Arc::new(tokio::sync::Semaphore::new(0)),
        diagrams: std::sync::Arc::new(tokio::sync::Semaphore::new(0)),
    };
    chat.update(cx, |chat, _| {
        chat.content_blocks.svg = super::svg_cache::SvgCache::with_gates(gates.clone());
    });
    gates
}

pub(in crate::views::chat) fn rich_message(content: &str) -> super::Message {
    let mut message = super::Message::assistant_streaming(1, None);
    message.content = content.into();
    message.state = super::TurnState::Finished;
    message
}

#[gpui::test]
fn cold_math_does_not_block_composing_or_rebuild_on_redraw(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let gates = paused_rich_rendering(&chat, cx);
    chat.update(cx, |chat, cx| {
        chat.messages = vec![rich_message("Formula: $$x^2 + 1$$ remains readable.")];
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-pending-math-0-1").is_some());
    chat.update_in(cx, |chat, window, cx| {
        chat.input_state
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.simulate_input("Still typing while rendering");
    cx.run_until_parked();
    for _ in 0..4 {
        chat.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    chat.read_with(cx, |chat, cx| {
        assert_eq!(
            chat.input_state.read(cx).value().as_str(),
            "Still typing while rendering"
        );
        assert_eq!(
            chat.content_blocks.svg.requests.get(),
            1,
            "redraws share the in-flight render"
        );
        assert_eq!(chat.content_blocks.svg.pending(), 1);
    });
    gates.math.add_permits(1);
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-pending-math-0-1").is_none());
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.pending(), 0)
    });
}

#[gpui::test]
fn streaming_reuses_diagrams_and_replacing_content_cancels_old_work(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let gates = paused_rich_rendering(&chat, cx);
    chat.update(cx, |chat, cx| {
        let mut message = super::Message::assistant_streaming(1, None);
        message.content = "Before\n\n```mermaid\ngraph TD\nA-->B".into();
        chat.messages = vec![message];
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(
            chat.content_blocks.svg.requests.get(),
            0,
            "an open fence stays source while streaming"
        )
    });
    chat.update(cx, |chat, cx| {
        chat.messages[0].content.push_str("\n```\n\nAfter");
        cx.notify();
    });
    cx.run_until_parked();
    for _ in 0..4 {
        chat.update(cx, |chat, cx| {
            chat.messages[0].content.push_str(" more prose");
            cx.notify();
        });
        cx.run_until_parked();
    }
    chat.read_with(cx, |chat, _| {
        assert_eq!(
            chat.content_blocks.svg.requests.get(),
            1,
            "prose tokens do not queue duplicate diagrams"
        )
    });
    chat.update(cx, |chat, cx| {
        chat.messages[0] = rich_message("Replacement: $$y^2$$");
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(
            chat.content_blocks.svg.pending(),
            1,
            "the obsolete diagram is cancelled"
        )
    });
    gates.diagrams.add_permits(1);
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-pending-math-0-1").is_some(),
        "a late diagram cannot replace the new math block"
    );
    gates.math.add_permits(1);
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-pending-math-0-1").is_none());
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.messages[0].content, "Replacement: $$y^2$$")
    });
}

#[gpui::test]
fn rewind_and_thread_reset_cancel_pending_rich_content(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let _gates = paused_rich_rendering(&chat, cx);
    chat.update(cx, |chat, cx| {
        chat.messages = vec![super::Message::user("Earlier"), rich_message("$$x^2$$")];
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.pending(), 1)
    });
    chat.update(cx, |chat, cx| {
        chat.messages.truncate(1);
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.pending(), 0)
    });
    chat.update(cx, |chat, cx| {
        chat.messages.push(rich_message("$$z^2$$"));
        chat.scroll_messages_to_bottom();
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.pending(), 1)
    });
    chat.update(cx, |chat, cx| {
        chat.start_thread("replacement", "New thread", cx)
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.pending(), 0)
    });
}

#[gpui::test]
fn rich_messages_larger_than_the_global_cache_keep_their_live_jobs(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let _gates = paused_rich_rendering(&chat, cx);
    let count = super::MATH_RENDER_CACHE_ENTRIES + 1;
    let content = (0..count)
        .map(|index| format!("$$x_{{{index}}}$$"))
        .collect::<Vec<_>>()
        .join("\n\n");
    chat.update(cx, |chat, cx| {
        chat.messages = vec![rich_message(&content)];
        cx.notify();
    });
    cx.run_until_parked();
    for _ in 0..3 {
        chat.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    chat.read_with(cx, |chat, _| {
        assert_eq!(
            chat.content_blocks.svg.requests.get(),
            count,
            "a message must not repeatedly evict and requeue its own live formulas"
        );
        assert_eq!(chat.content_blocks.svg.len(), count);
    });
    chat.update(cx, |chat, cx| {
        chat.messages[0].content = "Plain replacement".into();
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(
            chat.content_blocks.svg.len(),
            0,
            "replaced sources are released"
        )
    });
}

#[gpui::test]
fn completed_diagrams_survive_prose_tokens_and_release_obsolete_images(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let gates = paused_rich_rendering(&chat, cx);
    chat.update(cx, |chat, cx| {
        let mut message = super::Message::assistant_streaming(1, None);
        message.content = "Before\n\n```mermaid\ngraph TD\nA-->B\n```\n\nAfter".into();
        chat.messages = vec![message];
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-pending-diagram-0-1").is_some());
    gates.diagrams.add_permits(1);
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-pending-diagram-0-1").is_none());
    for _ in 0..3 {
        chat.update(cx, |chat, cx| {
            chat.messages[0].content.push_str(" more prose");
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-pending-diagram-0-1").is_none());
    }
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.requests.get(), 1)
    });
    chat.update(cx, |chat, cx| {
        chat.messages[0] = rich_message("Replacement without a diagram");
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.requests.get(), 1);
        assert_eq!(
            chat.content_blocks.svg.len(),
            0,
            "obsolete images are released"
        );
    });
}

#[gpui::test]
fn shared_formulas_keep_their_job_until_the_last_message_releases_them(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let gates = paused_rich_rendering(&chat, cx);
    chat.update(cx, |chat, cx| {
        chat.messages = vec![rich_message("$$x^2$$"), rich_message("$$x^2$$")];
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.requests.get(), 1)
    });
    chat.update(cx, |chat, cx| {
        chat.messages[0].content = "Changed first message".into();
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(
            chat.content_blocks.svg.pending(),
            1,
            "the second message still owns the formula"
        )
    });
    gates.math.add_permits(1);
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-pending-math-1-0").is_none());
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.content_blocks.svg.requests.get(), 1)
    });
    chat.update(cx, |chat, cx| {
        chat.messages[1].content = "Changed second message".into();
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| assert_eq!(chat.content_blocks.svg.len(), 0));
}

#[gpui::test]
#[ignore = "manual optimized foreground timing; cargo test -p bruhs-desktop --bin bruhs-desktop --release rich_content_preparation_cost -- --ignored --nocapture"]
fn rich_content_preparation_cost(cx: &mut TestAppContext) {
    use std::{hint::black_box, time::Instant};
    let (chat, cx) = chat_window(cx);
    let _gates = paused_rich_rendering(&chat, cx);
    let mut fixtures = vec![(
        "cold math".to_string(),
        r"Before $$\sum_{i=0}^{20}\frac{x_i^2 + y_i}{\sqrt{z}}$$ after.".to_string(),
    )];
    for count in [10, 100, 500] {
        let mut source = String::from("Before\n\n```mermaid\ngraph TD\n");
        for index in 0..count {
            source.push_str(&format!("N{index}[Step {index}] --> N{}\n", index + 1));
        }
        source.push_str("```\n\nAfter");
        fixtures.push((format!("cold diagram / {count} edges"), source));
    }
    for (label, content) in fixtures {
        let start = Instant::now();
        black_box(super::desktop_assistant_content_blocks_while(
            &content, false,
        ));
        let legacy = start.elapsed();
        let start = Instant::now();
        chat.update_in(cx, |chat, window, cx| {
            black_box(chat.render_assistant_content_blocks(0, &content, false, window, cx));
        });
        let queued = start.elapsed();
        let start = Instant::now();
        chat.update(cx, |chat, cx| {
            chat.content_blocks.clear();
            chat.messages = vec![rich_message(&content)];
            chat.scroll_messages_to_bottom();
            cx.notify();
        });
        cx.run_until_parked();
        let frame = start.elapsed();
        let marker = if label == "cold math" {
            "desktop-pending-math-0-1"
        } else {
            "desktop-pending-diagram-0-1"
        };
        assert!(
            cx.debug_bounds(marker).is_some(),
            "the pending source must actually be laid out"
        );
        println!(
            "{label}: legacy foreground SVG preparation {:.3} ms; queued foreground preparation {:.3} ms; initial pending update + test layout {:.3} ms",
            legacy.as_secs_f64() * 1000.0,
            queued.as_secs_f64() * 1000.0,
            frame.as_secs_f64() * 1000.0
        );
        chat.update(cx, |chat, _| chat.content_blocks.clear());
    }
}

#[gpui::test]
fn a_message_from_another_threads_agent_shows_its_sender_not_the_header(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.messages = vec![
            super::Message::user(bruhs_models::thread_link::delivery_prompt(
                "Research",
                "1a2b3c4d",
                1,
                6,
                false,
                "Report what broke.",
            )),
            super::Message::user("Typed by hand."),
        ];
        chat.scroll_messages_to_bottom();
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-user-message-0").is_some());
    assert!(
        cx.debug_bounds("desktop-user-from-thread-0").is_some(),
        "the sender caption is drawn above an agent-sent message"
    );
    assert!(
        cx.debug_bounds("desktop-user-from-thread-1").is_none(),
        "a typed message has no sender caption"
    );
}

#[gpui::test]
fn virtual_transcript_reaches_history_by_keyboard_and_find(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.messages = (0..1000)
            .map(|index| {
                super::Message::user(format!(
                    "Saved message {index}: {}",
                    "Text to read. ".repeat(5)
                ))
            })
            .collect();
        chat.scroll_messages_to_bottom();
        cx.notify();
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(
            chat.rendered_message_rows.get() < 100,
            "only a viewport of the 1,000 records is rendered"
        );
    });
    assert!(cx.debug_bounds("desktop-user-message-999").is_some());
    assert!(cx.debug_bounds("desktop-user-message-0").is_none());
    chat.update_in(cx, |chat, window, cx| chat.messages_focus.focus(window, cx));
    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-user-message-0").is_some());
    assert!(cx.debug_bounds("desktop-user-message-999").is_none());
    chat.update(cx, |chat, cx| {
        chat.find_open = true;
        chat.find_query = "Saved message 500:".into();
        chat.find_pinned = None;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-user-message-500").is_some(),
        "find reaches offscreen history"
    );
    chat.update(cx, |chat, cx| {
        chat.find_open = false;
        cx.notify();
    });
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| chat.messages_focus.focus(window, cx));
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-user-message-999").is_some());
    chat.update(cx, |chat, cx| {
        chat.messages.truncate(1);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-user-message-0").is_some(),
        "rewinding clamps the viewport"
    );
}

#[gpui::test]
fn tokens_keep_a_reader_in_history_and_tail_can_resume(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, _| {
        chat.messages = (0..1000)
            .map(|index| super::Message::user(format!("Saved message {index}")))
            .collect();
    });
    let turn = start_turn(&chat, "New question", cx);
    send(&turn, AgentUiEvent::Text("Live answer. ".repeat(100)));
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| chat.messages_focus.focus(window, cx));
    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    send(&turn, AgentUiEvent::Text("A later token.".into()));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-user-message-0").is_some(),
        "a token must not scroll a reader away"
    );
    chat.update_in(cx, |chat, window, cx| chat.messages_focus.focus(window, cx));
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(chat.messages_viewport.is_following_tail())
    });
    send(
        &turn,
        AgentUiEvent::Text("The answer continues. ".repeat(100)),
    );
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(chat.messages_viewport.is_following_tail())
    });
}

#[gpui::test]
fn shrinking_a_message_clamps_the_scroll_anchor(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.messages = (0..1000)
            .map(|index| super::Message::user(format!("Message {index}")))
            .collect();
        chat.messages[500].content = "A line of expanded text.\n".repeat(1000);
        cx.notify();
    });
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        chat.messages_viewport.scroll_to(gpui::ListOffset {
            item_ix: 500,
            offset_in_item: gpui::px(1000.0),
        });
        cx.notify();
    });
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        chat.messages[500].content = "Collapsed text".into();
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-user-message-501").is_some(),
        "shrinking a row must retain the adjacent history instead of skipping ahead"
    );
    assert!(cx.debug_bounds("desktop-user-message-0").is_none());
}

#[gpui::test]
fn events_from_a_stopped_turn_never_reach_the_next_turn(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);

    let first = start_turn(&chat, "first", cx);
    send(&first, AgentUiEvent::Text("first answer".into()));
    cx.run_until_parked();
    assert_eq!(assistant_contents(&chat, cx), ["first answer"]);

    chat.update(cx, |chat, cx| chat.stop_streaming(cx));
    // Buffered after the stop, before and after the next turn starts.
    send(&first, AgentUiEvent::Text(" stale".into()));
    let second = start_turn(&chat, "second", cx);
    send(&first, AgentUiEvent::Text(" stale again".into()));
    send(&first, AgentUiEvent::Done);
    send(&second, AgentUiEvent::Text("second answer".into()));
    cx.run_until_parked();

    assert_eq!(
        assistant_contents(&chat, cx),
        ["first answer", "second answer"]
    );
    chat.read_with(cx, |chat, _| {
        assert!(
            chat.messages[1].state.is_stopped(),
            "the first turn stays stopped"
        );
        assert!(chat.is_streaming, "a stale Done must not end the new turn");
        assert!(
            chat.agent_task.is_some(),
            "a stale Done must not drop the new turn's sender"
        );
    });

    // The new turn can still be stopped.
    chat.update(cx, |chat, cx| chat.stop_streaming(cx));
    send(&second, AgentUiEvent::Text(" after stop".into()));
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(!chat.is_streaming);
        let last = chat.messages.last().expect("the second answer");
        assert!(last.state.is_stopped());
        assert_eq!(last.content, "second answer");
    });
}

#[gpui::test]
fn a_stopped_turn_keeps_its_sender_for_the_next_submit_to_wait_on(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let first = start_turn(&chat, "first", cx);
    chat.update(cx, |chat, cx| chat.stop_streaming(cx));
    chat.read_with(cx, |chat, _| {
        assert!(
            chat.agent_task.is_some(),
            "stop must keep the aborted sender so the next turn awaits its teardown"
        );
    });
    drop(first);
}

#[gpui::test]
fn switching_threads_drops_the_running_turns_events(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let first = start_turn(&chat, "first", cx);
    chat.update(cx, |chat, cx| {
        chat.start_thread("thread-other", "Other", cx)
    });
    let _other = start_turn(&chat, "other", cx);
    send(&first, AgentUiEvent::Text("late".into()));
    send(&first, AgentUiEvent::Done);
    cx.run_until_parked();
    assert_eq!(assistant_contents(&chat, cx), [""]);
    chat.read_with(cx, |chat, _| assert!(chat.is_streaming));
}

#[gpui::test]
fn a_queued_follow_up_stays_held_when_the_turn_is_stopped(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let _turn = start_turn(&chat, "first", cx);
    chat.update(cx, |chat, cx| {
        assert!(chat.queue_follow_up("next", cx));
        assert_eq!(chat.prompt_queue.len(), 1);
        assert!(!chat.queue_held);
    });
    chat.update(cx, |chat, cx| chat.stop_streaming(cx));
    chat.read_with(cx, |chat, _| {
        assert!(chat.queue_held);
        let rows = chat.prompt_queue.for_thread("thread-stream");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "next");
        assert!(!chat.is_streaming);
    });
    chat.update(cx, |chat, cx| chat.drain_queue(cx));
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.prompt_queue.len(), 1, "a held queue does not drain");
        assert!(!chat.is_streaming);
    });
}

#[gpui::test]
fn frames_reuse_parsed_blocks_and_a_token_rebuilds_only_the_streaming_message(
    cx: &mut TestAppContext,
) {
    let (chat, cx) = chat_window(cx);
    let first = start_turn(&chat, "first", cx);
    send(
        &first,
        AgentUiEvent::Text("Some **prose** and $x^2$.".into()),
    );
    send(&first, AgentUiEvent::Done);
    cx.run_until_parked();
    let builds =
        |cx: &mut VisualTestContext| chat.read_with(cx, |chat, _| chat.content_blocks.builds.get());
    let settled = builds(cx);
    assert!(settled > 0, "the finished answer was rendered");

    for _ in 0..5 {
        chat.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(builds(cx), settled, "redraws reuse the parsed blocks");

    let second = start_turn(&chat, "second", cx);
    cx.run_until_parked();
    let streaming = builds(cx);
    for token in ["one ", "two ", "three"] {
        send(&second, AgentUiEvent::Text(token.into()));
        cx.run_until_parked();
    }
    assert_eq!(
        builds(cx),
        streaming + 3,
        "each token rebuilds the streaming message only"
    );
}

mod git {
    use super::chat_window;
    use gpui::{AppContext as _, TestAppContext};
    use std::path::Path;
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "user.email=t@example.invalid", "-c", "user.name=T"])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }

    fn repo_with_unstaged_change() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("notes.txt"), "committed\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        std::fs::write(dir.path().join("notes.txt"), "unsaved work\n").unwrap();
        dir
    }

    #[gpui::test]
    fn a_branch_named_dot_never_discards_unstaged_changes(cx: &mut TestAppContext) {
        let repo = repo_with_unstaged_change();
        let (chat, cx) = chat_window(cx);
        let path = repo.path().to_string_lossy().into_owned();
        chat.update(cx, |chat, cx| {
            chat.set_project_context(path, "repo", true, cx);
            chat.checkout_branch(".", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(),
            "unsaved work\n"
        );
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.git_current_branch.as_deref(), Some("main"));
            assert_eq!(chat.git_dirty_count, 1);
        });
    }

    #[gpui::test]
    fn a_new_worktree_stores_the_base_branch_and_leaves_the_checkout(cx: &mut TestAppContext) {
        let repo = repo_with_unstaged_change();
        let (chat, cx) = chat_window(cx);
        let path = repo.path().to_string_lossy().into_owned();
        chat.update(cx, |chat, cx| {
            chat.set_project_context(path, "repo", true, cx);
            chat.run_choice = bruhs_models::logic::run_place::WorkspaceChoice::NewWorktree;
            chat.git_current_branch = Some("main".into());
            chat.checkout_branch("main", cx);
        });
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.base_branch, "main");
            assert!(!chat.run_locked);
            assert!(chat.worktree_path.is_none());
        });
        assert_eq!(
            std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(),
            "unsaved work\n"
        );
    }

    #[gpui::test]
    fn previous_worktree_branch_picks_do_not_switch_the_checkout(cx: &mut TestAppContext) {
        let repo = repo_with_unstaged_change();
        let (chat, cx) = chat_window(cx);
        let path = repo.path().to_string_lossy().into_owned();
        chat.update(cx, |chat, cx| {
            chat.set_project_context(path, "repo", true, cx);
            chat.run_choice = bruhs_models::logic::run_place::WorkspaceChoice::PreviousWorktree;
            chat.previous_worktree = Some(bruhs_models::logic::run_place::RememberedWorktree {
                path: "/tmp/earlier".into(),
                branch: "bruhs/earlier".into(),
                label: "repo".into(),
            });
            chat.git_current_branch = Some("main".into());
            chat.checkout_branch("other", cx);
        });
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.git_current_branch.as_deref(), Some("main"));
            assert!(!chat.run_locked);
        });
        assert_eq!(
            std::fs::read_to_string(repo.path().join("notes.txt")).unwrap(),
            "unsaved work\n"
        );
    }

    #[gpui::test]
    fn sending_without_a_base_branch_fails_before_the_agent(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        chat.update(cx, |chat, cx| {
            chat.set_project_context(
                "/tmp/bruhs-slice10-desktop".to_string(),
                "repo".to_string(),
                true,
                cx,
            );
            chat.run_choice = bruhs_models::logic::run_place::WorkspaceChoice::NewWorktree;
            chat.git_current_branch = Option::<String>::None;
            chat.base_branch.clear();
            chat.submit_prompt("hello".into(), cx);
        });
        chat.read_with(cx, |chat, _| {
            assert!(!chat.run_locked);
            assert!(!chat.is_streaming);
            assert!(!chat.placing);
            assert_eq!(chat.messages.len(), 2);
            assert_eq!(chat.messages[0].content, "hello");
            let error = chat.messages[1].state.error().expect("failed turn");
            assert!(
                error.contains("Wait for the branch list"),
                "unexpected error: {error}"
            );
        });
    }

    #[gpui::test]
    fn restoring_a_project_chat_reads_git_only_once_it_is_drawn(cx: &mut TestAppContext) {
        let repo = repo_with_unstaged_change();
        cx.update(|cx| {
            bruhs_ui::init(cx);
            cx.set_global(crate::app_state::DesktopState::new().unwrap());
        });
        let path = repo.path().to_string_lossy().into_owned();
        // A chat that is built but never drawn (a saved, inactive thread).
        let window = cx.add_empty_window();
        let chat = window.update(|window, cx| {
            cx.new(|cx| {
                let mut chat = super::ChatView::new(window, cx);
                chat.set_project_context(path, "repo", true, cx);
                chat
            })
        });
        window.run_until_parked();
        chat.read_with(window, |chat, _| {
            assert!(chat.git_info_stale);
            assert!(chat.git_task.is_none());
            assert_eq!(chat.git_current_branch, None);
        });
    }
}

#[cfg(unix)]
mod claude_login {
    use super::super::{run_login_and_fetch_status_with, LoginTiming};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Duration;

    /// A fake `claude`: `auth login` records its pid and then waits (like a
    /// login left open in a browser); `auth status` reports `status_json`
    /// once that login has started, and signed-out before.
    fn fake_claude(dir: &Path, status_json: &str) -> String {
        let pid_file = dir.join("login.pid");
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\npid='{pid}'\ncase \"$2\" in\n  status) if [ -s \"$pid\" ]; then echo '{status_json}'; else echo '{{\"loggedIn\": false}}'; fi ;;\n  login) echo $$ > \"$pid\"; exec sleep 600 ;;\nesac\n",
                pid = pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    fn process_is_running(pid: &str) -> bool {
        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .expect("ps runs");
        let stat = String::from_utf8_lossy(&output.stdout);
        // Gone, or a zombie waiting to be reaped: either way not running.
        !stat.trim().is_empty() && !stat.trim().starts_with('Z')
    }

    #[test]
    fn a_successful_poll_ends_the_wait_and_kills_the_login() {
        let dir = tempfile::tempdir().unwrap();
        let program = fake_claude(
            dir.path(),
            r#"{"loggedIn": true, "email": "dev@example.com"}"#,
        );
        let timing = LoginTiming {
            poll_interval: Duration::from_millis(20),
            poll_attempts: 500,
            login_timeout: Duration::from_secs(600),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();

        let status = runtime.block_on(run_login_and_fetch_status_with(&program, timing));

        assert!(status.is_authenticated());
        assert_eq!(status.user(), Some("dev@example.com"));
        assert!(!status.login_in_flight());
        let pid =
            std::fs::read_to_string(dir.path().join("login.pid")).expect("the login was started");
        let pid = pid.trim().to_string();
        let mut running = process_is_running(&pid);
        for _ in 0..200 {
            if !running {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            running = process_is_running(&pid);
        }
        assert!(!running, "the abandoned login process {pid} must be killed");
    }
}

#[gpui::test]
fn model_slash_command_opens_the_model_picker_like_web(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let opened_settings = Rc::new(std::cell::Cell::new(false));
    let seen = opened_settings.clone();
    cx.update(|_, cx| {
        cx.subscribe(&chat, move |_, event: &super::ChatViewEvent, _| {
            if matches!(event, super::ChatViewEvent::OpenSettings) {
                seen.set(true);
            }
        })
        .detach();
    });
    chat.update_in(cx, |chat, window, cx| {
        chat.input_state.update(cx, |input, cx| {
            input.set_value("/model", window, cx);
            input.set_selected_range(6..6, cx);
        });
        chat.run_slash_command_for_test("/model", window, cx);
        // The composer's slash handlers clear the trigger text next.
        chat.clear_active_slash_trigger(window, cx);
    });
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        assert!(chat.model_picker_open, "/model opens the model picker");
        assert!(
            gpui::Focusable::focus_handle(&chat.model_picker_query, cx).is_focused(window),
            "the picker's search takes focus after the slash text clears"
        );
        assert!(chat.input_state.read(cx).value().is_empty());
        chat.close_model_picker(window, cx);
        chat.run_slash_command_for_test("/settings", window, cx);
    });
    cx.run_until_parked();
    assert!(opened_settings.get(), "/settings still opens Settings");
    chat.read_with(cx, |chat, _| assert!(!chat.model_picker_open));
}

mod expansion_state {
    use super::super::{Message, TurnState};
    use super::chat_window;
    use gpui::TestAppContext;

    fn failed_turn(chat: &mut super::ChatView) {
        chat.messages = vec![
            Message::user("prompt"),
            Message {
                state: TurnState::Failed {
                    error: "boom".into(),
                },
                thinking: "reasoning".into(),
                ..Message::assistant_streaming(1, None)
            },
        ];
        chat.expanded_thinking.insert(1);
        chat.expanded_folds.insert(1);
    }

    #[gpui::test]
    fn clearing_the_thread_forgets_open_thinking_blocks(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        chat.update_in(cx, |chat, window, cx| {
            failed_turn(chat);
            chat.run_slash_command_for_test("/clear", window, cx);
            assert!(chat.messages.is_empty());
            assert!(chat.expanded_thinking.is_empty());
            assert!(chat.expanded_folds.is_empty());
        });
    }

    #[gpui::test]
    fn a_retried_turn_does_not_inherit_the_failed_turns_open_thinking(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        chat.update(cx, |chat, _cx| {
            failed_turn(chat);
            assert_eq!(chat.drop_failed_turn(1).as_deref(), Some("prompt"));
            assert!(chat.messages.is_empty());
            assert!(chat.expanded_thinking.is_empty());
            assert!(chat.expanded_folds.is_empty());
        });
    }

    #[gpui::test]
    fn starting_another_thread_forgets_open_thinking_blocks(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        chat.update(cx, |chat, cx| {
            failed_turn(chat);
            chat.start_thread("thread-other", "Other", cx);
            assert!(chat.expanded_thinking.is_empty());
            assert!(chat.expanded_folds.is_empty());
        });
    }
}

#[gpui::test]
fn rewind_returns_the_prompt_and_starts_a_fresh_session(cx: &mut TestAppContext) {
    use super::Message;

    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, _cx| {
        chat.messages.push(Message::user("older"));
        chat.messages.push(Message::user("newer"));
        chat.messages.push(Message::assistant_streaming(1, None));
        chat.is_streaming = true;
        assert!(!chat.rewind_at(1, "draft"));
        assert_eq!(chat.messages.len(), 3);
        assert!(!chat.fresh_session);

        chat.is_streaming = false;
        assert!(!chat.rewind_at(2, "draft"));
        assert_eq!(chat.messages.len(), 3);

        assert!(chat.rewind_at(1, "draft"));
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].content, "older");
        assert_eq!(chat.pending_composer.as_deref(), Some("draft\n\nnewer"));
        assert!(chat.fresh_session);
        assert!(chat.focus_pending_composer);
        assert!(chat.rewind_confirm.is_none());
        assert!(chat.recall.is_none());
    });
}

#[gpui::test]
fn goal_commands_update_locally_without_starting_a_turn(cx: &mut TestAppContext) {
    use bruhs_models::logic::commands::ThreadGoal;

    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.submit_prompt("/goal".into(), cx);
        chat.submit_prompt("/goal pause".into(), cx);
        assert!(chat.goal.is_none());
        assert!(chat.messages.is_empty());
        assert!(!chat.is_streaming);

        chat.goal = Some(ThreadGoal {
            text: "ship it".into(),
            paused: false,
        });
        chat.submit_prompt("/goal clear".into(), cx);
        assert!(chat.goal.is_none());
        assert!(!chat.is_streaming);

        chat.model_id = "gpt-5.3-codex".into();
        chat.goal = Some(ThreadGoal {
            text: "ship it".into(),
            paused: false,
        });
        chat.submit_prompt("/goal pause".into(), cx);
        let goal = chat.goal.clone().expect("pause keeps the goal");
        assert!(goal.paused);
        assert_eq!(goal.text, "ship it");
        assert!(chat.messages.is_empty());
    });
}

fn begin(
    chat: &mut ChatView,
    prompt: &str,
    cx: &mut gpui::Context<'_, ChatView>,
) -> bruhs_core::AgentTurn {
    let turn = chat
        .begin_turn(
            prompt.to_string(),
            super::streaming::PreparedPastes::default(),
            Vec::new(),
            Vec::new(),
            None,
            cx,
        )
        .expect("turn");
    // The test plays the provider: the turn ends at once.
    chat.is_streaming = false;
    turn
}

#[test]
fn a_session_goes_only_to_the_provider_that_owns_it() {
    use super::session_for_provider;
    use bruhs_models::agent::AgentProvider::{Claude, Codex};

    assert_eq!(
        session_for_provider(Some("s"), Some(Claude), Claude).as_deref(),
        Some("s")
    );
    assert_eq!(session_for_provider(Some("s"), Some(Claude), Codex), None);
    assert_eq!(session_for_provider(Some("s"), None, Claude), None);
    assert_eq!(session_for_provider(Some(""), Some(Claude), Claude), None);
    assert_eq!(session_for_provider(None, Some(Codex), Codex), None);
}

#[test]
fn an_import_context_leads_the_prompt_but_not_a_slash_command() {
    use super::prompt_with_import_context;

    let prompt = prompt_with_import_context(Some("# Earlier"), "next".into());
    assert!(prompt.contains("# Earlier"));
    assert!(prompt.ends_with("next"));
    assert_eq!(
        prompt_with_import_context(Some("# Earlier"), "/compact".into()),
        "/compact"
    );
    assert_eq!(prompt_with_import_context(None, "next".into()), "next");
}

#[gpui::test]
fn turns_resume_the_saved_session_of_their_own_provider(cx: &mut TestAppContext) {
    use bruhs_models::agent::AgentProvider;

    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.model_id = "claude-sonnet-4-6".into();
        chat.session_id = Some("claude-sess".into());
        chat.session_provider = Some(AgentProvider::Claude);
        let turn = begin(chat, "one", cx);
        assert_eq!(turn.session_id.as_deref(), Some("claude-sess"));
        assert!(!turn.fresh_session);

        // Another provider cannot resume it, and keeps it for a switch back.
        // The conversation so far goes along instead.
        chat.model_id = "gpt-5.3-codex".into();
        let turn = begin(chat, "two", cx);
        assert_eq!(turn.session_id, None);
        assert!(!turn.fresh_session);
        assert!(turn.prompt.starts_with("This thread continues"));
        assert!(turn.prompt.contains("one"));
        assert!(turn.prompt.ends_with("two"));
        assert_eq!(chat.session_id.as_deref(), Some("claude-sess"));

        chat.model_id = "claude-sonnet-4-6".into();
        chat.fresh_session = true;
        let turn = begin(chat, "three", cx);
        assert!(turn.fresh_session);
        assert_eq!(turn.session_id, None);
    });
}

#[gpui::test]
fn a_reported_session_is_kept_and_a_rejected_one_dropped(cx: &mut TestAppContext) {
    use bruhs_models::agent::AgentProvider;

    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.model_id = "gpt-5.3-codex".into();
        chat.fresh_session = true;
        begin(chat, "hi", cx);
        chat.apply_agent_event(AgentUiEvent::Session("codex-1".into()));
        assert_eq!(chat.session_id.as_deref(), Some("codex-1"));
        assert_eq!(chat.session_provider, Some(AgentProvider::Codex));
        assert!(!chat.fresh_session);

        chat.apply_agent_event(AgentUiEvent::SessionRejected);
        assert_eq!(chat.session_id, None);
        assert_eq!(chat.session_provider, None);

        // Edit from here forgets the session.
        chat.session_id = Some("codex-2".into());
        chat.session_provider = Some(AgentProvider::Codex);
        assert!(chat.rewind_at(0, ""));
        assert_eq!(chat.session_id, None);
        assert!(chat.fresh_session);
    });
}

#[gpui::test]
fn an_import_context_rides_along_until_a_session_holds_it(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.import_context = Some("# Earlier\n\nhello from cursor".into());
        let turn = begin(chat, "next question", cx);
        assert!(turn.prompt.contains("hello from cursor"));
        assert!(turn.prompt.ends_with("next question"));
        // The transcript shows the user's own words only.
        assert_eq!(chat.messages[0].content, "next question");
        // No session yet (the turn failed): the next turn sends it again.
        assert!(chat.import_context.is_some());

        chat.apply_agent_event(AgentUiEvent::Session("s-1".into()));
        assert_eq!(chat.import_context, None);
        let turn = begin(chat, "again", cx);
        assert!(!turn.prompt.contains("hello from cursor"));
        assert_eq!(turn.session_id.as_deref(), Some("s-1"));
    });
}

#[gpui::test]
fn a_changed_role_starts_a_new_session_that_carries_the_conversation(cx: &mut TestAppContext) {
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::roles::{instructions_stamp, ResolvedRole};

    let role = |instructions: &str| ResolvedRole {
        id: "writer".into(),
        name: "Writer".into(),
        instructions: instructions.into(),
        ..ResolvedRole::default()
    };
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.model_id = "claude-sonnet-4-6".into();
        chat.session_id = Some("s-1".into());
        chat.session_provider = Some(AgentProvider::Claude);

        // No role when the session started and none now: it resumes.
        let turn = begin(chat, "one", cx);
        assert_eq!(turn.session_id.as_deref(), Some("s-1"));
        assert!(!turn.prompt.contains("This thread continues"));

        // `/role` gave the thread instructions the session does not hold.
        chat.role_profile = Some(role("Write plainly."));
        let turn = begin(chat, "two", cx);
        assert!(turn.fresh_session && turn.session_id.is_none());
        assert_eq!(turn.instructions.as_deref(), Some("Write plainly."));
        assert!(turn.prompt.starts_with("This thread continues"));
        assert!(turn.prompt.contains("one") && turn.prompt.ends_with("two"));
        chat.record_session("s-2".into());
        assert_eq!(
            chat.instructions_stamp,
            instructions_stamp(Some("Write plainly."))
        );

        // The new session holds them: the next turn resumes it.
        let turn = begin(chat, "three", cx);
        assert_eq!(turn.session_id.as_deref(), Some("s-2"));
        assert!(!turn.fresh_session && turn.prompt == "three");
        chat.record_session("s-2".into());
        assert!(chat.instructions_stamp.is_some(), "a resumed turn keeps it");

        // A handoff only writes a brief, so it keeps the session it has.
        chat.role_profile = Some(role("Be brief."));
        let turn = begin(chat, "/handoff ship it", cx);
        assert_eq!(turn.session_id.as_deref(), Some("s-2"));
        chat.handoff_pending = false;

        // Edited in Settings, then removed: each needs a new session.
        let turn = begin(chat, "four", cx);
        assert!(turn.fresh_session && turn.prompt.starts_with("This thread continues"));
        chat.record_session("s-3".into());
        chat.role_profile = None;
        let turn = begin(chat, "five", cx);
        assert!(turn.fresh_session && turn.instructions.is_none());
        chat.record_session("s-4".into());
        assert_eq!(chat.instructions_stamp, None);
        let turn = begin(chat, "six", cx);
        assert_eq!(turn.session_id.as_deref(), Some("s-4"));
    });
}
