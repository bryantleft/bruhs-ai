use leptos::prelude::*;

use crate::components::task_pagination::page_start;
use crate::workflow_executor::WorkflowExecutor;

const PAGE_SIZE: usize = 50;

#[component]
pub fn MeetingNotes() -> impl IntoView {
    let executor = expect_context::<WorkflowExecutor>();
    let crate::state::SetActiveView(set_active_view) = expect_context();
    let notes = executor.meeting_notes;
    let page = RwSignal::new(0_usize);
    let selected = RwSignal::new(None::<String>);
    let total = Memo::new(move |_| notes.with(Vec::len));
    let start = Memo::new(move |_| page_start(total.get(), page.get(), PAGE_SIZE));
    // Only title/identity data for the visible page; never clone all note bodies.
    let rows = Memo::new(move |_| {
        notes.with(|notes| {
            notes
                .iter()
                .skip(start.get())
                .take(PAGE_SIZE)
                .map(|note| (note.id.clone(), note.title.clone()))
                .collect::<Vec<_>>()
        })
    });
    let body = Memo::new(move |_| {
        selected.with(|id| {
            notes.with(|notes| {
                id.as_deref()
                    .and_then(|id| bruhs_models::MeetingNote::find_unique(notes, id).ok())
                    .map(|note| note.body.clone())
            })
        })
    });
    view! {
        <section aria-label="Meeting notes" class="min-w-0 border-b border-longan-800 p-4 sm:p-6">
            <h2 class="text-body text-rambutan-100">"Meeting notes"</h2>
            <button type="button" disabled=move || !executor.ready.get()
                class="mt-2 min-h-11 rounded-grape px-3 py-2 text-label-sm text-persimmon-300 hover:bg-longan-800 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                on:click=move |_| executor.note_editor.begin(None)>"New meeting note"</button>
            <super::meeting_note_editor::MeetingNoteEditor/>
            <p class="mt-1 text-body-sm text-rambutan-400">"Local text notes, separate from recordings and generated summaries."</p>
            <Show when=move || executor.ready.get() fallback=move || view! {
                <p role="status" class="mt-3 text-body-sm text-rambutan-400">"Loading local meeting notes…"</p>
                {move || executor.load_error.get().map(|message| view! {
                    <p role="alert" class="mt-2 text-body-sm text-rambutan-200">{message}</p>
                    <button type="button" class="min-h-11 rounded-grape px-3 py-2 text-label-sm text-persimmon-300"
                        on:click=move |_| executor.load()>"Retry loading notes"</button>
                })}
            }>
            <Show when=move || { total.get() > 0 } fallback=|| view! {
                <p class="mt-3 text-body-sm text-rambutan-400">"Create a note here or through an approved workflow."</p>
            }>
                <div class="mt-3 max-h-64 overflow-y-auto">
                    <For each=move || rows.get() key=|(id, title)| (id.clone(), title.clone()) children=move |(id, title)| {
                        let selected_id = id.clone();
                        view! {
                            <button type="button" class="block min-h-11 w-full rounded-grape px-3 py-2 text-left text-body-sm text-rambutan-200 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 break-words [overflow-wrap:anywhere]"
                                aria-pressed=move || selected.with(|current| if current.as_ref() == Some(&selected_id) { "true" } else { "false" })
                                on:click=move |_| selected.set(Some(id.clone()))>{title}</button>
                        }
                    } />
                </div>
                <Show when=move || { total.get() > PAGE_SIZE }>
                    <nav aria-label="Meeting notes pagination" class="mt-2 flex flex-wrap items-center gap-2 text-label-sm text-rambutan-300">
                        <button type="button" class="min-h-11 rounded-grape px-3 py-2 hover:bg-longan-800 disabled:opacity-40"
                            disabled=move || start.get() == 0
                            on:click=move |_| page.set((start.get_untracked() / PAGE_SIZE).saturating_sub(1))>"Previous notes"</button>
                        <span role="status">{move || format!("{}–{} of {}", start.get() + 1, (start.get() + PAGE_SIZE).min(total.get()), total.get())}</span>
                        <button type="button" class="min-h-11 rounded-grape px-3 py-2 hover:bg-longan-800 disabled:opacity-40"
                            disabled=move || { start.get() + PAGE_SIZE >= total.get() }
                            on:click=move |_| page.set(start.get_untracked() / PAGE_SIZE + 1)>"Next notes"</button>
                    </nav>
                </Show>
            </Show>
            {move || body.get().map(|text| view! {
                <section aria-label="Meeting note content" tabindex="0" class="mt-4 max-h-96 overflow-y-auto whitespace-pre-wrap rounded-grape border border-longan-800 p-3 text-body-sm text-rambutan-200 [overflow-wrap:anywhere]">
                    {text}
                </section>
                <button type="button" class="mt-2 min-h-11 rounded-grape px-3 py-2 text-label-sm text-persimmon-300 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400"
                    on:click=move |_| { if let Some(note_id) = selected.get_untracked() {
                        set_active_view.set(crate::state::AppView::MeetingNoteDetail { note_id });
                    } }>"Open note"</button>
            })}
            </Show>
        </section>
    }
}
