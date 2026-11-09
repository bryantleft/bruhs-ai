use crate::ui::{input::Input, textarea::Textarea};
use crate::workflow_executor::WorkflowExecutor;
use leptos::prelude::*;

#[component]
pub fn MeetingNoteEditor() -> impl IntoView {
    let executor = expect_context::<WorkflowExecutor>();
    let editor = executor.note_editor;
    let title = Signal::derive(move || {
        editor.draft.with(|draft| {
            draft
                .as_ref()
                .map(|draft| draft.title.clone())
                .unwrap_or_default()
        })
    });
    let body = Signal::derive(move || {
        editor.draft.with(|draft| {
            draft
                .as_ref()
                .map(|draft| draft.body.clone())
                .unwrap_or_default()
        })
    });
    let changed = move || {
        editor.message.set(None);
        editor.failed.set(false);
    };
    view! {
        <Show when=move || editor.draft.with(Option::is_some)>
            <form aria-label="Edit local meeting note" class="my-4 flex min-w-0 flex-col gap-3 border-y border-longan-800 py-4"
                on:submit=move |event| { event.prevent_default(); executor.save_note(); }
                on:keydown=move |event| {
                    if (event.meta_key() || event.ctrl_key()) && event.key() == "Enter" && !event.is_composing() {
                        event.prevent_default(); event.stop_propagation(); executor.save_note();
                    }
                }>
                <h2 class="text-body text-rambutan-100">"Edit local meeting note"</h2>
                <p class="text-body-sm text-rambutan-400">"Draft stays available during navigation. Save before closing this tab. ⌘/Ctrl+Enter to save."</p>
                <label class="flex min-w-0 flex-col gap-1 text-label-sm text-rambutan-300">
                    "Title"
                    <Input value=title aria_label="Meeting note title" autofocus=true disabled=editor.saving
                        on_input=Callback::new(move |value| { if !editor.saving.get_untracked() {
                            editor.draft.update(|draft| { if let Some(draft) = draft { draft.title = value; } }); changed();
                        } })/>
                </label>
                <label class="flex min-w-0 flex-col gap-1 text-label-sm text-rambutan-300">
                    "Notes"
                    <Textarea value=body rows=10 disabled=editor.saving
                        on_input=Callback::new(move |value| { if !editor.saving.get_untracked() {
                            editor.draft.update(|draft| { if let Some(draft) = draft { draft.body = value; } }); changed();
                        } })/>
                </label>
                <div class="flex flex-wrap items-center gap-2">
                    <Show when=move || editor.draft.with(|draft| draft.as_ref().is_some_and(|draft| draft.recording_id.is_some()))>
                        <p class="text-body-sm text-rambutan-300 [overflow-wrap:anywhere]">{move || editor.draft.with(|draft| draft.as_ref().and_then(|draft| draft.recording_id.as_ref()).map(|id| format!("Linked recording: {id}")).unwrap_or_default())}</p>
                        <button type="button" disabled=move || editor.saving.get()
                            class="min-h-11 rounded-grape px-3 py-2 text-label-sm text-rambutan-300 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                            on:click=move |_| { if !editor.saving.get_untracked() {
                                editor.draft.update(|draft| { if let Some(draft) = draft { draft.recording_id = None; } }); changed();
                            } }>"Remove recording link"</button>
                    </Show>
                    <button type="submit" disabled=move || editor.saving.get()
                        class="min-h-11 rounded-grape bg-persimmon-500 px-3 py-2 text-label-sm text-longan-950 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400">"Save note"</button>
                    <button type="button" disabled=move || editor.saving.get() on:click=move |_| editor.discard()
                        class="min-h-11 rounded-grape px-3 py-2 text-label-sm text-rambutan-300 hover:bg-longan-800 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400">"Close editor / discard unsaved changes"</button>
                </div>
                {move || editor.message.get().map(|message| view! {
                    <p role=move || if editor.failed.get() { "alert" } else { "status" }
                        class="text-body-sm text-rambutan-200 [overflow-wrap:anywhere]">{message}</p>
                })}
            </form>
        </Show>
    }
}
