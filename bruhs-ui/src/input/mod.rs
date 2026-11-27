//! Text fields: single-line [`Input`] and multi-line [`Textarea`], their
//! states, events and editing actions.
//!
//! A field's state ([`InputState`], [`TextareaState`]) holds the text, the
//! selection and the undo history; the component draws it. Typing, IME
//! composition and the platform's text services go through gpui's
//! [`gpui::EntityInputHandler`]. Edits the person makes report
//! [`InputEvent::Change`]; [`EditorState::set_value`] never does.

mod actions;
mod code_fence;
mod core;
mod element;
mod fence_header;
mod list;
mod list_layout;
mod markdown;
mod state;
mod text_layout;
mod view;

#[cfg(test)]
use gpui::Pixels;
use gpui::{App, FocusHandle, Global, WeakFocusHandle, Window};

pub use actions::{
    composer_format_action, Backspace, Copy, Cut, Delete, DeleteToBeginningOfLine,
    DeleteToEndOfLine, DeleteToNextWordEnd, DeleteToPreviousWordStart, Enter, Escape, IndentInline,
    MoveDown, MoveEnd, MoveHome, MoveLeft, MoveRight, MoveToEnd, MoveToNextWord,
    MoveToPreviousWord, MoveToStart, MoveUp, OutdentInline, Paste, Redo, SelectAll, SelectDown,
    SelectLeft, SelectRight, SelectToEnd, SelectToEndOfLine, SelectToNextWordEnd,
    SelectToPreviousWordStart, SelectToStart, SelectToStartOfLine, SelectUp, Undo, CONTEXT,
};
pub use state::{
    EditorState, FieldKind, InputContentType, InputEvent, InputState, MultiLine, SingleLine,
    TextareaState,
};
pub use view::{Input, Sizable, Size, Textarea};

pub(crate) fn highlight_snippet(
    lang_tag: &str,
    code: &str,
) -> Vec<(std::ops::Range<usize>, gpui::Hsla)> {
    let Some(lang) = code_fence::language(lang_tag) else {
        return Vec::new();
    };
    if lang.id.is_empty() || code.is_empty() {
        return Vec::new();
    }
    let mut raw = Vec::new();
    bruhs_models::syntax::highlight_code(lang.id, code, 0, &mut raw);
    raw.into_iter()
        .map(|(r, kind)| (r, code_fence::TokenColor::from(kind).color()))
        .collect()
}

/// Every field's focus handle, so the app can ask whether one has focus.
#[derive(Default)]
struct Fields(Vec<WeakFocusHandle>);

impl Global for Fields {}

pub(crate) fn register_field(focus: &FocusHandle, cx: &mut App) {
    let fields = cx.default_global::<Fields>();
    fields.0.retain(|weak| weak.upgrade().is_some());
    fields.0.push(focus.downgrade());
}

/// Window helpers for text fields.
pub trait WindowExt {
    /// Whether a text field has focus (so plain keys type rather than act).
    fn has_focused_input(&mut self, cx: &App) -> bool;
}

impl WindowExt for Window {
    fn has_focused_input(&mut self, cx: &App) -> bool {
        cx.try_global::<Fields>().is_some_and(|fields| {
            fields
                .0
                .iter()
                .filter_map(WeakFocusHandle::upgrade)
                .any(|focus| focus.is_focused(self))
        })
    }
}

/// Bind the fields' editing keys.
pub fn init(cx: &mut App) {
    actions::bind_keys(cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::prelude::*;
    use gpui::{
        div, px, size, Context, Entity, EntityInputHandler as _, Focusable as _, TestAppContext,
        VisualTestContext,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Form {
        field: Entity<InputState>,
        readonly: bool,
    }

    impl Render for Form {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .w(px(120.))
                .child(Input::new(&self.field).readonly(self.readonly))
        }
    }

    fn form(
        cx: &mut TestAppContext,
    ) -> (
        Entity<InputState>,
        Rc<RefCell<Vec<InputEvent>>>,
        &mut VisualTestContext,
    ) {
        form_with(cx, false)
    }

    fn form_with(
        cx: &mut TestAppContext,
        readonly: bool,
    ) -> (
        Entity<InputState>,
        Rc<RefCell<Vec<InputEvent>>>,
        &mut VisualTestContext,
    ) {
        cx.update(crate::init);
        let events = Rc::new(RefCell::new(Vec::new()));
        let seen = events.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            let field = cx.new(|cx| InputState::new(window, cx));
            cx.subscribe(&field, move |_, _, event: &InputEvent, _| {
                seen.borrow_mut().push(event.clone());
            })
            .detach();
            Form { field, readonly }
        });
        cx.simulate_resize(size(px(400.), px(200.)));
        let field = cx.read(|cx| view.read(cx).field.clone());
        cx.update(|window, cx| field.update(cx, |field, cx| field.focus(window, cx)));
        cx.run_until_parked();
        (field, events, cx)
    }

    #[gpui::test]
    fn source_textarea_scrolls_long_lines_horizontally(cx: &mut TestAppContext) {
        struct Code(Entity<TextareaState>);
        impl Render for Code {
            fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
                div().w(px(180.)).child(Textarea::new(&self.0).h(px(80.)))
            }
        }
        cx.update(crate::init);
        let (view, cx) = cx
            .add_window_view(|w, cx| Code(cx.new(|cx| TextareaState::new(w, cx).soft_wrap(false))));
        let field = cx.read(|cx| view.read(cx).0.clone());
        cx.update(|w, cx| {
            field.update(cx, |f, cx| {
                f.focus(w, cx);
                f.insert(format!("short\n{}", "x".repeat(100)), w, cx);
            })
        });
        cx.run_until_parked();
        cx.read(|cx| {
            let scroll = &field.read(cx).scroll;
            assert!(
                scroll.max_offset().x > px(100.),
                "long line should overflow horizontally"
            );
            assert!(
                scroll.offset().x < px(-100.),
                "caret should be visible at end of line"
            );
            assert_eq!(scroll.max_offset().y, px(0.));
        });
    }

    #[gpui::test]
    fn rich_list_continuations_share_the_body_column_for_editing(cx: &mut TestAppContext) {
        struct Draft(Entity<TextareaState>);
        impl Render for Draft {
            fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
                div().w(px(185.)).child(Textarea::new(&self.0).h(px(480.)))
            }
        }
        let source = "- Parent alpha beta gamma delta café encore\n  + Nested alpha beta gamma delta café encore\n- [x] Done alpha beta gamma delta café encore\n  - [ ] Subtask\n\n9. Nine alpha beta gamma delta café encore\n10. Ten alpha beta gamma delta café encore\n\nAfter.";
        cx.update(crate::init);
        let (view, cx) = cx.add_window_view(|w, cx| Draft(cx.new(|cx| TextareaState::new(w, cx))));
        let field = cx.read(|cx| view.read(cx).0.clone());
        cx.update(|w, cx| {
            field.update(cx, |f, cx| {
                f.set_markdown_enabled(true, cx);
                f.set_value(source, w, cx);
                f.focus(w, cx);
            })
        });
        cx.run_until_parked();
        let (click, at) = cx.read(|cx| {
            let field = field.read(cx);
            let drawn = field.drawn.as_ref().unwrap();
            let mut columns = Vec::new();
            let mut target = None;
            for word in ["Parent", "Nested", "Done", "Nine", "Ten"] {
                let start = source.find(word).unwrap();
                let end = start + source[start..].find('\n').unwrap();
                let first = drawn
                    .layout
                    .position_for_index(drawn.map.to_display(start))
                    .unwrap();
                columns.push(first.x);
                let mut y = first.y;
                let mut wraps = 0;
                for (n, _) in source[start..end].char_indices() {
                    let at = start + n;
                    let pt = drawn
                        .layout
                        .position_for_index(drawn.map.to_display(at))
                        .unwrap();
                    if pt.y > y {
                        assert!(
                            (pt.x - first.x).abs() < px(1.),
                            "{word}: {pt:?} vs {first:?}"
                        );
                        let click =
                            gpui::point(pt.x - px(3.), pt.y + drawn.layout.line_height() / 2.);
                        assert_eq!(field.offset_at(click), Some(at), "{word} wrap margin");
                        if target.is_none() {
                            target = Some((click, at));
                        }
                        wraps += 1;
                        y = pt.y;
                    }
                }
                assert!(wraps > 0, "{word} should wrap");
            }
            let child = source.find("Subtask").unwrap();
            let child_x = field.range_to_bounds(&(child..child)).unwrap().left();
            let rem = (columns[1] - columns[0]) / 1.25;
            assert!((child_x - columns[2] - rem * (1.45 + 1.375)).abs() < px(1.));
            assert!(columns[1] > columns[0]);
            assert_eq!(columns[3], columns[4]);
            target.unwrap()
        });
        cx.simulate_click(click, Default::default());
        cx.simulate_input("Ω");
        assert_eq!(
            cx.read(|cx| field.read(cx).value().to_string()),
            format!("{}Ω{}", &source[..at], &source[at..])
        );
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(cx.read(|cx| field.read(cx).value().to_string()), source);
        cx.simulate_click(click, Default::default());
        cx.simulate_keystrokes("shift-up");
        assert_eq!(
            cx.read(|cx| field.read(cx).selected_range()),
            source.find("Parent").unwrap()..at
        );
        let task = source.find("[x]").unwrap();
        let checkbox = cx.read(|cx| {
            field
                .read(cx)
                .range_to_bounds(&(task..task + 3))
                .unwrap()
                .center()
        });
        cx.simulate_click(checkbox, Default::default());
        assert_eq!(
            cx.read(|cx| field.read(cx).value().to_string()),
            source.replace("[x]", "[ ]")
        );
        cx.simulate_click(click, Default::default());
        cx.update(|w, cx| {
            field.update(cx, |f, cx| {
                f.replace_and_mark_text_in_range(None, "ni", None, w, cx);
                f.replace_text_in_range(None, "你", w, cx);
            })
        });
        assert_eq!(
            cx.read(|cx| field.read(cx).value().to_string()),
            format!("{}你{}", &source[..at], &source[at..]).replace("[x]", "[ ]")
        );
    }

    #[gpui::test]
    fn a_composition_undoes_in_one_step(cx: &mut TestAppContext) {
        let (field, _, cx) = form(cx);
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "hi ", window, cx);
                for step in ["n", "ni", "nih", "niha", "nihao"] {
                    field.replace_and_mark_text_in_range(None, step, None, window, cx);
                }
                field.replace_text_in_range(None, "你好", window, cx);
                assert_eq!(field.value(), "hi 你好");
                field.undo(&Undo, window, cx);
                assert_eq!(field.value(), "hi ");
            })
        });
    }

    #[gpui::test]
    fn empty_code_block_commits_or_cancels_an_ime_edit(cx: &mut TestAppContext) {
        let (_, _, cx) = form(cx);
        cx.update(|w, cx| {
            let field = cx.new(|cx| TextareaState::new(w, cx));
            field.update(cx, |f, cx| {
                f.set_markdown_enabled(true, cx);
                f.insert("```rust", w, cx);
                assert!(f.edit_markdown_key(
                    bruhs_models::logic::composer_text::ComposerKey::Enter,
                    w,
                    cx
                ));
                let empty = "```rust\n```";
                assert_eq!(f.value(), empty);
                for cancel_by_committing in [true, false] {
                    f.replace_and_mark_text_in_range(None, "mao", None, w, cx);
                    if cancel_by_committing {
                        f.replace_text_in_range(None, "", w, cx);
                    } else {
                        f.replace_and_mark_text_in_range(None, "", None, w, cx);
                    }
                    assert_eq!(f.value(), empty);
                }
                f.replace_and_mark_text_in_range(None, "mao", None, w, cx);
                f.replace_text_in_range(None, "猫", w, cx);
                assert_eq!(f.value(), "```rust\n猫\n```");
                f.undo(&Undo, w, cx);
                assert_eq!(f.value(), empty);
            });
        });
    }

    #[gpui::test]
    fn pending_format_survives_ime_commit_and_paste(cx: &mut TestAppContext) {
        let (_, _, cx) = form(cx);
        cx.update(|window, cx| {
            let field = cx.new(|cx| TextareaState::new(window, cx));
            field.update(cx, |f, cx| {
                f.set_markdown_enabled(true, cx);
                f.format_mark(markdown::Mark::Bold, window, cx);
                f.replace_and_mark_text_in_range(None, "mao", None, window, cx);
                f.replace_text_in_range(None, "猫", window, cx);
                assert_eq!(f.value(), "**猫**");
                f.undo(&Undo, window, cx);
                assert_eq!(f.value(), "");
                f.format_mark(markdown::Mark::Italic, window, cx);
                f.replace_and_mark_text_in_range(None, "犬", None, window, cx);
                f.unmark_text(window, cx);
                assert_eq!(f.value(), "*犬*");
                f.undo(&Undo, window, cx);
                f.format_mark(markdown::Mark::Code, window, cx);
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("one".to_owned()));
                f.paste(&Paste, window, cx);
                assert_eq!(f.value(), "`one`");
                f.undo(&Undo, window, cx);
                assert_eq!(f.value(), "");
            });
        });
    }

    #[gpui::test]
    fn an_edit_mid_composition_drops_its_mark(cx: &mut TestAppContext) {
        let (field, _, cx) = form(cx);
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "日本", window, cx);
                field.replace_and_mark_text_in_range(None, "nihao", None, window, cx);
                field.undo(&Undo, window, cx);
                assert!(field.marked_text_range(window, cx).is_none());
            })
        });
        // Drawing after the undo must not cut a run inside a character.
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    #[gpui::test]
    fn set_value_is_quiet_and_typing_is_not(cx: &mut TestAppContext) {
        let (field, events, cx) = form(cx);
        cx.update(|window, cx| field.update(cx, |field, cx| field.set_value("abc", window, cx)));
        assert!(!events.borrow().contains(&InputEvent::Change));
        cx.simulate_input("d");
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abcd");
        assert!(events.borrow().contains(&InputEvent::Change));
    }

    #[gpui::test]
    fn a_long_value_stays_on_one_line(cx: &mut TestAppContext) {
        let (field, _, cx) = form(cx);
        cx.simulate_input("a long line of text that is much wider than the field");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let (lines, scroll) = cx.read(|cx| {
            let state = field.read(cx);
            let drawn = state.drawn.as_ref().expect("drawn");
            (drawn.layout.logical_line_count(), state.scroll_x)
        });
        assert_eq!(lines, 1);
        assert!(scroll > Pixels::ZERO, "the field scrolls to the caret");
    }

    #[gpui::test]
    fn a_read_only_field_selects_but_never_edits(cx: &mut TestAppContext) {
        let (field, events, cx) = form_with(cx, true);
        cx.update(|window, cx| field.update(cx, |field, cx| field.set_value("abc", window, cx)));
        cx.run_until_parked();
        cx.simulate_input("d");
        cx.simulate_keystrokes("cmd-a");
        assert_eq!(cx.read(|cx| field.read(cx).core.selection.range()), 0..3);
        cx.simulate_keystrokes("backspace cmd-x cmd-v cmd-z");
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abc");
        assert!(!events.borrow().contains(&InputEvent::Change));
        assert!(cx.update(|window, cx| field.read(cx).focus_handle(cx).is_focused(window)));
    }

    /// Right click, then a menu item, as a person would.
    fn menu_item(cx: &mut VisualTestContext, field: &Entity<InputState>, item: &'static str) {
        let bounds =
            cx.update(|_, cx| field.read(cx).drawn.as_ref().map(|drawn| drawn.text_bounds));
        let at = bounds.expect("the field is drawn").center();
        cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::default());
        cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::default());
        cx.run_until_parked();
        let item = cx.debug_bounds(item).expect("the menu shows the item");
        cx.simulate_click(item.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    /// The edit menu can't change a read-only field either: Cut and Paste
    /// are drawn disabled, and the state refuses every edit, whatever asks.
    #[gpui::test]
    fn a_read_only_fields_menu_never_cuts_or_pastes(cx: &mut TestAppContext) {
        let (field, events, cx) = form_with(cx, true);
        cx.update(|window, cx| field.update(cx, |field, cx| field.set_value("abc", window, cx)));
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
        cx.run_until_parked();
        menu_item(cx, &field, "edit-paste");
        cx.simulate_keystrokes("cmd-a");
        menu_item(cx, &field, "edit-cut");
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abc");
        // Nor can a caller reach the state's edits around the menu.
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.paste(&Paste, window, cx);
                field.cut(&Cut, window, cx);
                field.replace_text_in_range(None, "x", window, cx);
                field.replace_and_mark_text_in_range(None, "y", None, window, cx);
            })
        });
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abc");
        assert!(!events.borrow().contains(&InputEvent::Change));
    }

    /// The public edits honour read-only too, and a composition range the
    /// platform hands a read-only field leaves no mark behind.
    #[gpui::test]
    fn a_read_only_field_refuses_its_public_edits(cx: &mut TestAppContext) {
        let (field, events, cx) = form_with(cx, true);
        cx.update(|window, cx| field.update(cx, |field, cx| field.set_value("abc", window, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.insert("x", window, cx);
                field.replace_all("y", window, cx);
                field.clean(window, cx);
                field.replace_text_in_range(Some(0..1), "z", window, cx);
                assert!(field.marked_text_range(window, cx).is_none());
            })
        });
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abc");
        assert!(!events.borrow().contains(&InputEvent::Change));
    }

    /// The same menu edits an editable field, so the test above reaches it.
    #[gpui::test]
    fn an_editable_fields_menu_pastes(cx: &mut TestAppContext) {
        let (field, _, cx) = form(cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
        menu_item(cx, &field, "edit-paste");
        assert_eq!(cx.read(|cx| field.read(cx).value()), "pasted");
    }

    #[gpui::test]
    fn single_line_input_uses_min_height_and_grows_for_tall_text(cx: &mut TestAppContext) {
        struct Inputs {
            normal: Entity<InputState>,
            tall: Entity<InputState>,
        }
        impl Render for Inputs {
            fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
                div()
                    .w(px(300.0))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(|| "normal-input-wrap".to_owned())
                            .child(Input::new(&self.normal)),
                    )
                    .child(
                        div().debug_selector(|| "tall-input-wrap".to_owned()).child(
                            Input::new(&self.tall)
                                .text_size(px(28.0))
                                .line_height(px(36.0)),
                        ),
                    )
            }
        }
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|w, cx| {
            let normal = cx.new(|cx| {
                let mut state = InputState::new(w, cx);
                state.set_value("Default", w, cx);
                state
            });
            let tall = cx.new(|cx| {
                let mut state = InputState::new(w, cx);
                state.set_value("Tall metrics", w, cx);
                state
            });
            Inputs { normal, tall }
        });
        cx.run_until_parked();
        let normal = cx
            .debug_bounds("normal-input-wrap")
            .expect("normal input bounds");
        let tall = cx
            .debug_bounds("tall-input-wrap")
            .expect("tall input bounds");
        assert_eq!(
            normal.size.height,
            px(crate::theme::CONTROL_PX),
            "default single-line input should preserve CONTROL_PX height"
        );
        assert!(
            tall.size.height > px(crate::theme::CONTROL_PX),
            "tall font metrics should grow single-line input beyond CONTROL_PX, got {:?}",
            tall.size.height
        );
    }
}
