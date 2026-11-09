use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlTextAreaElement;

/// The empty hero and populated timeline currently own separate composers.
/// Restore an interrupted editor only when loading actually removed it; user
/// navigation, another focus target and newer drafts always take precedence.
pub(crate) fn retain_composer_focus(
    active: ReadSignal<Option<String>>,
    thread_id: String,
) -> impl FnOnce() {
    let editor = document()
        .active_element()
        .filter(|element| element.id() == "chat-input")
        .and_then(|element| element.dyn_into::<HtmlTextAreaElement>().ok());
    let selection = editor.as_ref().map(|editor| {
        (
            editor.value(),
            editor.selection_start().ok().flatten(),
            editor.selection_end().ok().flatten(),
            editor.selection_direction().ok().flatten(),
        )
    });
    move || {
        let (Some(editor), Some((value, start, end, direction))) = (editor, selection) else {
            return;
        };
        // Nested reactive render effects can outlive the first microtask.
        // Restore after that DOM work, before the browser paints the new layout.
        request_animation_frame(move || {
            if editor.is_connected()
                || active.with_untracked(|id| id.as_deref() != Some(thread_id.as_str()))
                || document()
                    .active_element()
                    .is_some_and(|element| element.tag_name() != "BODY")
            {
                return;
            }
            let Some(replacement) = document()
                .get_element_by_id("chat-input")
                .and_then(|element| element.dyn_into::<HtmlTextAreaElement>().ok())
                .filter(|editor| editor.value() == value)
            else {
                return;
            };
            let _ = replacement.focus();
            let _ = replacement.set_selection_start(start);
            let _ = replacement.set_selection_end(end);
            let _ = replacement.set_selection_direction(direction.as_deref());
        });
    }
}
