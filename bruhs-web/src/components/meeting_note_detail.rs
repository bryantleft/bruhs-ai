use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::{AppView, SetActiveView};
use crate::workflow_executor::WorkflowExecutor;

#[component]
pub fn MeetingNoteDetail(note_id: String) -> impl IntoView {
    let executor = expect_context::<WorkflowExecutor>();
    let crate::state::MeetingsSignal(meetings) = expect_context();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let content = Memo::new(move |_| {
        executor
            .meeting_notes
            .with(|notes| bruhs_models::MeetingNote::find_unique(notes, &note_id).cloned())
    });
    let root = NodeRef::<leptos::html::Section>::new();
    Effect::new(move |_| {
        if let Some(element) = root.get() {
            let _ = element.unchecked_ref::<web_sys::HtmlElement>().focus();
        }
    });
    view! {
        <section node_ref=root tabindex="-1" aria-label="Local meeting note" class="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto p-4 sm:p-6 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400">
            <div class="mb-4 flex shrink-0 items-center gap-3">
                <button type="button" class="min-h-11 rounded-grape px-3 py-2 text-label-sm text-rambutan-300 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                    on:click=move |_| set_active_view.set(AppView::Meetings)>"Back to meeting notes"</button>
                <span class="text-label-sm text-rambutan-400">"Local text note"</span>
            </div>
            <super::meeting_note_editor::MeetingNoteEditor/>
            <Show when=move || executor.ready.get() fallback=move || view! {
                <p role="status" class="text-body-sm text-rambutan-400">"Loading local meeting note…"</p>
                {move || executor.load_error.get().map(|message| view! {
                    <p role="alert" class="mt-2 text-body-sm text-rambutan-200">{message}</p>
                    <button type="button" class="mt-2 min-h-11 rounded-grape px-3 py-2 text-label-sm text-persimmon-300 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                        on:click=move |_| executor.load()>"Retry loading notes"</button>
                })}
            }>
                {move || match content.get() {
                    Ok(note) => {
                        let editable = note.clone();
                        view! {
                        <button type="button" class="mb-3 min-h-11 self-start rounded-grape px-3 py-2 text-label-sm text-persimmon-300 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                            on:click=move |_| executor.note_editor.begin(Some(&editable))>"Edit note"</button>
                        <h1 class="text-body text-rambutan-100 [overflow-wrap:anywhere]">{note.title}</h1>
                        <p class="mt-1 text-body-sm text-rambutan-400">"Separate from recordings and generated summaries."</p>
                        {note.recording_id.map(|recording_id| {
                            let available_id = recording_id.clone();
                            let available = Signal::derive(move || meetings.with(|items| items.iter().filter(|meeting| meeting.id == available_id).count() == 1));
                            view! {
                                <button type="button" disabled=move || !available.get()
                                    class="mt-3 min-h-11 rounded-grape px-3 py-2 text-label-sm text-persimmon-300 hover:bg-longan-800 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                                    on:click=move |_| { if available.get_untracked() { set_active_view.set(AppView::MeetingDetail { meeting_id: recording_id.clone() }); } }>"Open linked recording"</button>
                                <Show when=move || !available.get()><p role="status" class="text-body-sm text-rambutan-400">"Linked recording is not available locally. The note and link are retained."</p></Show>
                            }
                        })}
                        <div aria-label="Meeting note content" tabindex="0" class="mt-4 whitespace-pre-wrap text-body-sm text-rambutan-200 [overflow-wrap:anywhere] focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400">{note.body}</div>
                    }.into_any() },
                    Err(message) => view! {
                        <p role="status" class="text-body-sm text-rambutan-300">{message.message()}</p>
                    }.into_any(),
                }}
            </Show>
        </section>
    }
}
