use bruhs_models::logic::prompt_suggestion::{SuggestionKey, SuggestionReply};
use gpui::TestAppContext;

use super::chat_window;

fn offer(chat: &mut super::ChatView, cx: &mut gpui::Context<'_, super::ChatView>) {
    chat.task_models.prompt_suggestions = true;
    let key = SuggestionKey {
        thread: "thread-stream".into(),
        turn: "finished".into(),
        model: chat.model_id.clone(),
    };
    chat.prompt_suggestion.begin(key.clone());
    chat.prompt_suggestion.finish(
        &key,
        SuggestionReply {
            suggestion: "Run the tests".into(),
        },
    );
    chat.sync_prompt_suggestion(cx);
    cx.notify();
}

#[gpui::test]
fn prompt_suggestion_tab_fills_the_draft_without_sending_and_can_be_undone(
    cx: &mut TestAppContext,
) {
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.input_state
            .update(cx, |input, cx| input.focus(window, cx));
        offer(chat, cx);
    });
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        assert!(
            chat.input_state.read(cx).value().is_empty(),
            "ghost text is not draft text"
        );
    });
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        assert_eq!(chat.input_state.read(cx).value(), "Run the tests");
        assert!(
            chat.messages.is_empty() && !chat.is_streaming,
            "Tab never sends"
        );
    });
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        assert!(chat.input_state.read(cx).value().is_empty())
    });
}

#[gpui::test]
fn prompt_suggestion_escape_dismisses_without_changing_the_draft(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.input_state
            .update(cx, |input, cx| input.focus(window, cx));
        offer(chat, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        assert!(chat.input_state.read(cx).value().is_empty());
        assert!(chat.input_state.read(cx).inline_completion_text().is_none());
        assert_eq!(chat.prompt_suggestion.suffix("", 0..0), None);
    });
}

#[gpui::test]
fn prompt_suggestion_prefix_typing_and_selection_do_not_replace_user_text(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.input_state
            .update(cx, |input, cx| input.focus(window, cx));
        offer(chat, cx);
    });
    cx.simulate_input("Run");
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        assert_eq!(
            chat.input_state.read(cx).inline_completion_text(),
            Some(" the tests")
        );
        chat.input_state
            .update(cx, |input, cx| input.set_selected_range(0..3, cx));
    });
    chat.update_in(cx, |chat, window, cx| {
        assert!(!chat.accept_prompt_suggestion(window, cx))
    });
    chat.update(cx, |chat, cx| {
        assert_eq!(chat.input_state.read(cx).value(), "Run")
    });
}

#[gpui::test]
fn prompt_suggestion_stays_out_of_ime_and_attachment_drafts(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.input_state
            .update(cx, |input, cx| input.focus(window, cx));
        offer(chat, cx);
        chat.input_state.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(None, "R", Some(1..1), window, cx);
        });
    });
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        assert!(
            !chat.accept_prompt_suggestion(window, cx),
            "IME owns the matching prefix"
        );
        chat.input_state
            .update(cx, |input, cx| input.set_value("", window, cx));
        chat.text_attachments
            .push(bruhs_models::logic::composer_text::TextAttachment {
                name: "note.txt".into(),
                text: "context".into(),
            });
        cx.notify();
    });
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        assert!(chat.input_state.read(cx).inline_completion_text().is_none());
        assert!(
            !chat.accept_prompt_suggestion(window, cx),
            "attachments take precedence"
        );
    });
}
