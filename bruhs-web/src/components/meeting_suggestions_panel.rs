//! The suggestions section of the live meeting dock. Private to the user:
//! nothing is shared with other participants and nothing acts on its own.

use bruhs_models::meeting_suggestions::{Proactivity, RoundStatus, Suggestion, SuggestionState};
use leptos::prelude::*;

use crate::meeting_suggestions::MeetingSuggestions;

const ACTION: &str = "min-h-11 sm:min-h-[28px] rounded-seed px-2 text-label-sm text-rambutan-200 hover:bg-longan-800 disabled:opacity-40 disabled:cursor-not-allowed focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400";

/// "2 new suggestions", for the collapsed pill. None when there is nothing new.
pub fn unseen_label(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 new suggestion".into()),
        count => Some(format!("{count} new suggestions")),
    }
}

#[component]
pub fn MeetingSuggestionsPanel() -> impl IntoView {
    let suggestions = expect_context::<MeetingSuggestions>();
    let live = suggestions.live;
    let prefs = suggestions.prefs;
    let proactivity = Memo::new(move |_| prefs.with(|prefs| prefs.proactivity));
    let thinking = Memo::new(move |_| live.with(|live| live.is_thinking()));
    let paused = Memo::new(move |_| live.with(|live| live.paused));
    // Opening the dock shows every suggestion, so none stays "new".
    Effect::new(move |_| {
        if live.with(|live| live.unseen()) > 0 {
            suggestions.mark_seen();
        }
    });

    view! {
        <Show when=move || proactivity.get() != Proactivity::Off>
            <section aria-label="Meeting suggestions" data-testid="meeting-suggestions" class="space-y-2 rounded-grape bg-longan-950 inset-ring inset-ring-longan-800 p-2">
                <div class="flex items-center justify-between gap-2">
                    <p class="text-label-sm text-rambutan-400">"Suggestions"</p>
                    <div class="flex items-center gap-1">
                        <Show when=move || proactivity.get().is_automatic()>
                            <button type="button" class=ACTION data-testid="meeting-suggestions-pause"
                                aria-pressed=move || paused.get().to_string()
                                on:click=move |_| suggestions.set_paused(!paused.get_untracked())>
                                {move || if paused.get() { "Resume" } else { "Pause" }}
                            </button>
                        </Show>
                        <button type="button" class=ACTION data-testid="meeting-suggestions-now"
                            disabled=move || thinking.get()
                            on:click=move |_| suggestions.suggest_now()>
                            "Suggest now"
                        </button>
                    </div>
                </div>
                <p role="status" class="text-label-sm text-rambutan-400 empty:hidden">
                    {move || live.with(|live| match &live.status {
                        RoundStatus::Thinking => "Reading the last two minutes…".to_string(),
                        RoundStatus::Failed(message) => message.clone(),
                        RoundStatus::Idle if live.paused => "Paused for this meeting.".to_string(),
                        RoundStatus::Idle => suggestions.note.get().unwrap_or_default(),
                    })}
                </p>
                <Show when=move || live.with(|live| live.visible().next().is_none())>
                    <p class="text-body-sm text-rambutan-500">
                        {move || if proactivity.get().is_automatic() {
                            "Suggestions appear here after new speech or a pause."
                        } else {
                            "Select Suggest now for notes on the last two minutes."
                        }}
                    </p>
                </Show>
                // Rows are keyed by suggestion, so a decision or a new round
                // never rebuilds the buttons under the pointer or focus.
                <ul class="space-y-2 empty:hidden" aria-label="Suggestions">
                    <For
                        each=move || live.with(|live| live.visible().cloned().collect::<Vec<_>>())
                        key=|item| item.id.clone()
                        let:item
                    >
                        <SuggestionRow item=item />
                    </For>
                </ul>
            </section>
        </Show>
    }
}

/// One suggestion. Its state is read live, so Add task turns into Task
/// added in place.
#[component]
fn SuggestionRow(item: Suggestion) -> impl IntoView {
    let suggestions = expect_context::<MeetingSuggestions>();
    let id = StoredValue::new(item.id.clone());
    let added = Memo::new(move |_| {
        suggestions.live.with(|live| {
            id.with_value(|id| {
                live.items
                    .iter()
                    .any(|other| other.id == *id && other.state == SuggestionState::Added)
            })
        })
    });
    let actionable = item.kind.is_actionable();
    let decide = move |state: SuggestionState| id.with_value(|id| suggestions.decide(id, state));
    view! {
        <li data-testid="meeting-suggestion" class="space-y-1 rounded-seed bg-longan-900 px-2 py-1.5">
            <p class="text-label-sm text-blueberry-300">{item.kind.label()}</p>
            <p class="text-body-sm text-rambutan-100 break-words">{item.text.clone()}</p>
            {(!item.reason.is_empty()).then(|| view! {
                <p class="text-label-sm text-rambutan-400 break-words">{item.reason.clone()}</p>
            })}
            <div class="flex flex-wrap items-center gap-1">
                <Show when=move || actionable && !added.get()>
                    <button type="button" class=ACTION data-testid="meeting-suggestion-add"
                        on:click=move |_| decide(SuggestionState::Added)>"Add task"</button>
                </Show>
                <Show when=move || added.get()>
                    <span class="px-2 text-label-sm text-pandan-300">"Task added"</span>
                </Show>
                <button type="button" class=ACTION data-testid="meeting-suggestion-copy"
                    on:click=move |_| decide(SuggestionState::Copied)>"Copy"</button>
                <Show when=move || !added.get()>
                    <button type="button" class=ACTION data-testid="meeting-suggestion-dismiss"
                        on:click=move |_| decide(SuggestionState::Dismissed)>"Dismiss"</button>
                </Show>
            </div>
        </li>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_counts_only_new_suggestions() {
        assert_eq!(unseen_label(0), None);
        assert_eq!(unseen_label(1).as_deref(), Some("1 new suggestion"));
        assert_eq!(unseen_label(3).as_deref(), Some("3 new suggestions"));
    }
}
