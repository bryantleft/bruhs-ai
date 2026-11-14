//! Optimized, offscreen diagnostics for a token arriving in a long native reply.
use super::*;
use std::{hint::black_box, time::Instant};

#[gpui::test]
fn visible_inline_geometry_matches_complete_shaped_text(cx: &mut TestAppContext) {
    use gpui::{point, px, size, Bounds, SharedString, WrappedLineLayout};
    use gpui_component::text::inline_geometry_matches_full_scan;
    let (_, cx) = chat_window(cx);
    let lines: String = (0..48)
        .map(|index| format!("Line {index}: value = café + 👩🏽‍💻;\n"))
        .collect();
    let fixtures = [
        "Short café 👩‍👩‍👧‍👦 text\n\nTrailing newline\n".to_string(),
        lines,
        "Long wrapped words, e\u{301}, 中文 and 👩🏽‍💻. ".repeat(40),
        "مرحبا لا الله العربية עברית mixed text 123\n".repeat(12),
    ];
    let mut compared = 0;
    cx.update(|window, _| {
        for text in fixtures {
            for width in [px(160.), px(600.)] {
                let shaped = window.text_system().shape_text(
                    SharedString::from(text.clone()), px(14.),
                    &[window.text_style().to_run(text.len())], Some(width), None,
                ).unwrap();
                let layouts: Vec<_> = shaped.iter().map(|line| {
                    std::sync::Arc::new(WrappedLineLayout {
                        unwrapped_layout: line.unwrapped_layout.clone(),
                        wrap_boundaries: line.wrap_boundaries.clone(),
                        wrap_width: line.wrap_width,
                    })
                }).collect();
                let layout_height = px(19.25);
                let height: gpui::Pixels = layouts.iter().map(|line| line.size(layout_height).height).sum();
                for requested_height in [layout_height, px(28.)] {
                    for offset in [px(-20.), px(0.), layout_height, height / 2., height - px(40.)] {
                        let origin = point(px(120.), -offset);
                        for mask_width in [px(180.), px(900.)] {
                            let mask = Bounds::new(point(px(100.), px(0.)), size(mask_width, px(130.)));
                            assert!(inline_geometry_matches_full_scan(&text, &layouts, origin, layout_height, requested_height, mask), "clipped geometry differs: width={width:?}, offset={offset:?}, height={requested_height:?}, text={text:?}");
                            compared += 1;
                        }
                    }
                }
            }
        }
    });
    println!("Compared {compared} clipped shaped-text geometries with the complete scan");
}

#[gpui::test]
fn code_selection_keeps_its_text_while_the_streaming_tail_changes(cx: &mut TestAppContext) {
    use gpui::{point, px, Modifiers, MouseButton};
    use gpui_component::WindowExt as _;
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(gpui::size(px(1280.), px(800.)));
    chat.update(cx, |chat, cx| {
        let code: String = (0..250)
            .map(|index| format!("let value_{index} = compute({index});\n"))
            .collect();
        let mut message = super::super::Message::assistant_streaming(1, None);
        message.content = format!("```rust\n{code}```\n\nTail");
        chat.messages = vec![message];
        chat.is_streaming = true;
        chat.scroll_messages_to_bottom();
        cx.notify();
    });
    cx.run_until_parked();
    let viewport = cx.debug_bounds("desktop-chat-transcript").unwrap();
    let from = point(viewport.center().x - px(200.), viewport.bottom() - px(180.));
    let to = point(from.x + px(140.), from.y + px(40.));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let selected = cx.update(|window, cx| window.selected_text(cx));
    assert!(
        selected.contains("value_"),
        "the drag selects visible code: {selected:?}"
    );
    chat.update(cx, |chat, cx| {
        chat.apply_agent_event(AgentUiEvent::Text(" appended text".into()));
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|window, cx| window.selected_text(cx)),
        selected,
        "streaming preserves selected code"
    );
    chat.update(cx, |chat, cx| {
        chat.apply_agent_event(AgentUiEvent::Done);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|window, cx| window.selected_text(cx)),
        selected,
        "completion preserves selected code"
    );
}

#[gpui::test]
fn streaming_keeps_the_markdown_view_through_tokens_and_completion(cx: &mut TestAppContext) {
    use gpui_component::text::TextViewState;
    let created = Rc::new(RefCell::new(Vec::new()));
    let capture = Rc::clone(&created);
    let _subscription = cx.update(|cx| {
        cx.observe_new::<TextViewState>(move |_, _, cx| {
            capture.borrow_mut().push(cx.entity_id());
        })
    });
    let (chat, cx) = chat_window(cx);
    cx.run_until_parked();
    created.borrow_mut().clear();
    chat.update(cx, |chat, cx| {
        let mut message = super::super::Message::assistant_streaming(1, None);
        let code: String = (0..1_000)
            .map(|index| format!("let value_{index} = compute({index});\n"))
            .collect();
        message.content = format!("```rust\n{code}```\n\nTail");
        chat.messages = vec![message];
        chat.is_streaming = true;
        chat.scroll_messages_to_bottom();
        cx.notify();
    });
    cx.run_until_parked();
    let initial = created.borrow().len();
    for text in [" one", " two", " three"] {
        chat.update(cx, |chat, cx| {
            chat.apply_agent_event(AgentUiEvent::Text(text.into()));
            cx.notify();
        });
        cx.run_until_parked();
    }
    chat.update(cx, |chat, cx| {
        chat.apply_agent_event(AgentUiEvent::Done);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(initial, 1, "one prose block creates one Markdown view");
    assert_eq!(
        created.borrow().len(),
        initial,
        "tokens and completion keep the existing Markdown view"
    );
}

fn paragraphs(count: usize) -> String {
    (0..count)
        .map(|index| {
            format!(
                "Paragraph {index}: office affinity and **complete saved prose**, with a [link](https://example.invalid/path).\n\n"
            )
        })
        .collect()
}

#[gpui::test]
fn streamed_markdown_replacements_match_fresh_documents(cx: &mut TestAppContext) {
    use gpui_component::text::TextViewState;
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
    });
    let incremental = cx.update(|cx| cx.new(|cx| TextViewState::markdown("", cx)));
    let fresh = cx.update(|cx| cx.new(|cx| TextViewState::markdown("", cx)));
    let mut fixtures: Vec<String> = [
        "Stable **bold**.\n\nTail é 👩🏽‍💻\n---\n\nAnother paragraph.",
        "A [later][ref] link.\n\nTail\n\n[ref]: https://example.invalid \"Title\"\n",
        "First.\n\n[ref]: /old\n\n[ref] and new text\n",
        "- list\n  next\n\n  another\n\nOutside.\n\nTail\n",
        "```rust\nlet x = 1;\n```\n\nTail\n\n```mermaid\ngraph TD\n A --> B\n```\n",
        "<div>\nraw\n</div>\n\nParagraph with `code` and **bold**.",
        "😀 First\n\n   Indented paragraph\n   wrapped\n\nTail\n",
        "Table\n\na | b\n--- | ---\n1 | 2\n\nTail\n",
        "First\r\rTail\r\rLast [link](https://example.invalid).",
        "First\r\n \t\r\nTail\r\n\r\nLast.",
        "</div>`\t\n\n- item\n~~`",
        "- first\n\n- second\n\nTail",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let fragments = [
        "text ",
        "\n\n",
        "---\n",
        "> quote\n",
        "- item\n",
        "[x]: /url\n",
        "[x]",
        "![a](x)",
        "`",
        "```\n",
        "\t",
        "   ",
        "<div>",
        "</div>",
        "~~",
        "|a|b|\n|-|-|\n",
        "é",
        "👩🏽‍💻",
        "\r",
        "\r\n",
    ];
    let mut seed = 123_456_789_u64;
    for _ in 0..300 {
        let mut text = String::new();
        for _ in 0..8 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let index = usize::try_from(seed >> 32).unwrap() % fragments.len();
            text.push_str(fragments[index]);
        }
        fixtures.push(text);
    }
    let mut compared = 0;
    for text in fixtures {
        incremental.update(cx, |state, cx| state.set_text("", cx));
        for end in text.char_indices().map(|(at, c)| at + c.len_utf8()) {
            let prefix = &text[..end];
            incremental.update(cx, |state, cx| state.set_text(prefix, cx));
            fresh.update(cx, |state, cx| {
                state.set_text("", cx);
                state.set_text(prefix, cx);
            });
            incremental.read_with(cx, |state, cx| {
                assert!(
                    state.parsed_content_eq(fresh.read(cx)),
                    "streamed prefix must match a fresh document: {prefix:?}"
                );
            });
            compared += 1;
        }
    }
    println!("Compared {compared} streamed Markdown prefixes with fresh documents");
}

#[gpui::test]
fn markdown_background_appends_include_initial_and_replaced_content(cx: &mut TestAppContext) {
    use gpui_component::text::TextViewState;
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
    });
    let state = cx.update(|cx| cx.new(|cx| TextViewState::markdown("**Initial**", cx)));
    let expected = cx.update(|cx| cx.new(|cx| TextViewState::markdown("**Initial** suffix", cx)));
    state.update(cx, |state, cx| state.push_str(" suffix", cx));
    cx.run_until_parked();
    state.read_with(cx, |state, cx| {
        assert!(
            state.parsed_content_eq(expected.read(cx)),
            "append includes initial content"
        );
    });
    state.update(cx, |state, cx| {
        state.set_text("Replaced [link][ref].\n\n[ref]: /new\n\nTail", cx);
        state.push_str(" one", cx);
        state.push_str(" two", cx);
    });
    expected.update(cx, |state, cx| {
        state.set_text("Replaced [link][ref].\n\n[ref]: /new\n\nTail one two", cx)
    });
    cx.run_until_parked();
    state.read_with(cx, |state, cx| {
        assert!(
            state.parsed_content_eq(expected.read(cx)),
            "append includes replaced content and references"
        );
    });
    let mut burst = String::from("Burst **source**\n\nTail");
    state.update(cx, |state, cx| {
        state.set_text(&burst, cx);
        for index in 0..100 {
            let chunk = format!(" token-{index}");
            burst.push_str(&chunk);
            state.push_str(&chunk, cx);
        }
    });
    expected.update(cx, |state, cx| state.set_text(&burst, cx));
    cx.run_until_parked();
    state.read_with(cx, |state, cx| {
        assert!(
            state.parsed_content_eq(expected.read(cx)),
            "a coalesced burst retains every chunk"
        );
    });
    state.update(cx, |state, cx| {
        state.push_str(" stale", cx);
        state.set_text("Final synchronous replacement", cx);
    });
    expected.update(cx, |state, cx| {
        state.set_text("Final synchronous replacement", cx)
    });
    cx.run_until_parked();
    state.read_with(cx, |state, cx| {
        assert!(
            state.parsed_content_eq(expected.read(cx)),
            "an older background result cannot replace a newer value"
        );
    });
    let html = cx.update(|cx| cx.new(|cx| TextViewState::html("<p>First</p>", cx)));
    let complete = cx.update(|cx| {
        cx.new(|cx| TextViewState::html("<p>First</p><p><strong>Second</strong></p>", cx))
    });
    html.update(cx, |state, cx| {
        state.push_str("<p><strong>Second</strong></p>", cx)
    });
    cx.run_until_parked();
    html.read_with(cx, |state, cx| {
        assert!(
            state.parsed_content_eq(complete.read(cx)),
            "HTML appends retain earlier closed blocks"
        );
    });
}

#[gpui::test]
#[ignore = "manual optimized native streaming timing; cargo test -p bruhs-desktop --bin bruhs-desktop --release native_streaming_action_cost -- --ignored --nocapture"]
fn native_streaming_action_cost(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let _gates = paused_rich_rendering(&chat, cx);
    cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
    let prose = paragraphs(1_000);
    let code: String = (0..1_000)
        .map(|index| format!("let value_{index} = compute({index}); // a complete source line\n"))
        .collect();
    let fixtures = [
        ("prose / 100 paragraphs", paragraphs(100)),
        ("prose / 1000 paragraphs", prose.clone()),
        ("code / 1000 lines", format!("```rust\n{code}```\n\nTail")),
        (
            "mermaid word / 1000 paragraphs",
            format!("The mermaid word is prose.\n\n{prose}"),
        ),
        (
            "diagram / 1000 paragraphs",
            format!("```mermaid\ngraph TD\n A --> B\n```\n\n{prose}"),
        ),
    ];
    for (label, content) in fixtures {
        let mut parse = Vec::new();
        for _ in 0..21 {
            let started = Instant::now();
            black_box(crate::views::chat::parse_assistant_content(&content, true));
            parse.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let mut expected = content.clone();
        chat.update(cx, |chat, cx| {
            chat.content_blocks.clear();
            let mut message = super::super::Message::assistant_streaming(1, None);
            message.content = content.clone();
            chat.messages = vec![message];
            chat.is_streaming = true;
            chat.scroll_messages_to_bottom();
            cx.notify();
        });
        cx.run_until_parked();
        let mut reduce = Vec::new();
        let mut frame = Vec::new();
        for index in 0..24 {
            let chunk = format!(" streaming-{index} café");
            expected.push_str(&chunk);
            let started = Instant::now();
            let mut reduced = std::time::Duration::ZERO;
            chat.update(cx, |chat, cx| {
                let before = Instant::now();
                chat.apply_agent_event(AgentUiEvent::Text(chunk));
                reduced = before.elapsed();
                cx.notify();
            });
            cx.run_until_parked();
            let elapsed = started.elapsed();
            if index >= 3 {
                reduce.push(reduced.as_secs_f64() * 1000.0);
                frame.push(elapsed.as_secs_f64() * 1000.0);
            }
        }
        let started = Instant::now();
        chat.update(cx, |chat, cx| {
            chat.apply_agent_event(AgentUiEvent::Done);
            cx.notify();
        });
        cx.run_until_parked();
        let completion = started.elapsed().as_secs_f64() * 1000.0;
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.messages[0].content, expected);
            assert!(!chat.is_streaming);
        });
        println!(
            "NATIVE_STREAM_COST {}",
            serde_json::json!({
                "fixture": label, "bytes": content.len(), "parse_ms": parse,
                "reduce_ms": reduce, "frame_ms": frame, "completion_ms": completion,
            })
        );
    }
}
