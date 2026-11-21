//! Shared domain UI, with a narrowly scoped bridge into the current draft.
use bruhs_models::logic::workbench_draft::{append_dictation_draft, DICTATION_DRAFT_MESSAGE};
use leptos::prelude::*;
use wasm_bindgen::{closure::Closure, JsCast};

#[derive(serde::Deserialize)]
struct DraftMessage {
    r#type: String,
    text: String,
}

#[component]
pub fn Workbench() -> impl IntoView {
    let handoff = expect_context::<crate::state::WorkbenchTaskHandoff>().0;
    let selected_handoff = handoff.get_untracked();
    handoff.set(None);
    let source = format!(
        "{}/api/workbench/ui{}",
        crate::api::BASE_URL,
        selected_handoff.map_or_else(String::new, |id| format!("#handoff={id}"))
    );
    let frame = NodeRef::<leptos::html::Iframe>::new();
    let draft = expect_context::<crate::state::ComposerDraft>().0;
    let active_thread = expect_context::<crate::state::ActiveThreadId>().0;
    let original_thread = active_thread.get_untracked();
    let view = expect_context::<crate::state::SetActiveView>().0;
    let expected_origin = web_sys::Url::new(crate::api::BASE_URL)
        .ok()
        .map(|url| url.origin());
    let callback =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if expected_origin.as_deref() != Some(&event.origin())
                || active_thread.get_untracked() != original_thread
            {
                return;
            }
            let child = frame
                .get_untracked()
                .and_then(|frame| frame.content_window());
            if !event
                .source()
                .zip(child)
                .is_some_and(|(source, child)| js_sys::Object::is(source.as_ref(), child.as_ref()))
            {
                return;
            }
            let Ok(message) = serde_wasm_bindgen::from_value::<DraftMessage>(event.data()) else {
                return;
            };
            if message.r#type != DICTATION_DRAFT_MESSAGE || message.text.trim().is_empty() {
                return;
            }
            draft.update(|text| *text = append_dictation_draft(text, &message.text));
            view.set(crate::state::AppView::Chat);
        });
    if let Some(window) = web_sys::window() {
        let _ =
            window.add_event_listener_with_callback("message", callback.as_ref().unchecked_ref());
    }
    let callback = StoredValue::new_local(callback);
    on_cleanup(move || {
        callback.with_value(|callback| {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "message",
                    callback.as_ref().unchecked_ref(),
                );
            }
        })
    });
    view! {
        <iframe node_ref=frame class="h-full min-h-0 w-full flex-1 border-0"
            src=source
            on:load=move |_| {
                if let Some(child)=frame.get_untracked().and_then(|frame|frame.content_window()) {
                    let message=js_sys::Object::new();
                    let _=js_sys::Reflect::set(&message,&"type".into(),&"bruhs:composer-bridge-ready".into());
                    if let Ok(url)=web_sys::Url::new(crate::api::BASE_URL) {let _=child.post_message(&message,&url.origin());}
                }
            }
            title="Local agent workbench" allow="microphone; clipboard-write" />
    }
}
