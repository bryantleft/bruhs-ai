//! Move in's By prompt tab (D13): a prompt to paste into another assistant,
//! and its answer proposed as a chosen bruh's memory. The answer joins the
//! bruh's memory only as a proposal the owner approves on its Memory tab.

use bruhs_models::imports::{
    memory_proposed_line, moved_in_from, moved_in_memory, prefill_for_answer, propose_memory,
    ImportPrefill, COPIED, COPY_PROMPT, FOR_BRUH, MAKE_A_BRUH, MOVE_IN_HINT, MOVE_IN_PROMPT,
    NO_BRUHS, PROPOSE_MEMORY, THEIR_ANSWER,
};
use leptos::prelude::*;

use super::{BUTTON, ERROR, FIELD, HELP, LABEL};
use crate::components::crew::data::CrewData;
use crate::state::CrewEdits;
use crate::ui::ResetTimer;

/// How long **Copy prompt** says it copied.
const COPIED_MS: u32 = 2_000;
const STATUS: &str = "text-label-sm text-pandan-300 empty:sr-only";

/// The heading a pasted answer goes under: today's date in the browser's zone.
fn pasted_from_today() -> String {
    let date = js_sys::Date::new_0();
    moved_in_from(&format!(
        "{:04}-{:02}-{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date()
    ))
}

/// The By prompt tab's text, choice and outcome. Move in owns it, so
/// switching tabs mid-proposal keeps the answer, the confirmation and the
/// refresh of the views behind (M2).
#[derive(Clone, Copy)]
pub(crate) struct ByPromptState {
    pub answer: RwSignal<String>,
    /// The heading the answer goes under, when it came from an export.
    pub from: RwSignal<Option<String>>,
    pub chosen: RwSignal<String>,
    pub status: RwSignal<Option<String>>,
    pub error: RwSignal<Option<&'static str>>,
    /// The bruh whose proposal is on its way.
    pub proposing: RwSignal<Option<String>>,
    edits: Option<RwSignal<u64>>,
}

impl ByPromptState {
    pub(crate) fn new(text: Option<String>, from: Option<String>) -> Self {
        Self {
            answer: RwSignal::new(text.unwrap_or_default()),
            from: RwSignal::new(from),
            chosen: RwSignal::new(String::new()),
            status: RwSignal::new(None),
            error: RwSignal::new(None),
            proposing: RwSignal::new(None),
            edits: use_context::<CrewEdits>().map(|edits| edits.0),
        }
    }

    /// Settles a proposal once `data`'s queue answers.
    pub(crate) fn watch(self, data: CrewData) {
        Effect::new(move |_| {
            if self.proposing.with(Option::is_none) || data.busy.get() {
                return;
            }
            self.answered(data.error.with_untracked(Option::is_none));
        });
    }

    /// The proposal answered. Accepted: every open crew view reads again
    /// (the bruh page behind shows the card), the line says where to
    /// approve it, and the area clears. Refused: the text stays for another
    /// try, with the service's reason in the error line.
    pub(crate) fn answered(self, accepted: bool) {
        let Some(name) = self.proposing.try_update(Option::take).flatten() else {
            return;
        };
        if !accepted {
            return;
        }
        if let Some(edits) = self.edits {
            edits.try_update(|n| *n = n.wrapping_add(1));
        }
        self.status.try_set(Some(memory_proposed_line(&name)));
        self.answer.try_set(String::new());
        self.from.try_set(None);
    }
}

#[component]
pub(crate) fn ByPromptTab(
    data: CrewData,
    state: ByPromptState,
    /// Opens Create bruh, with the area's text as its memory when it has any.
    on_make_bruh: Callback<Option<ImportPrefill>>,
) -> impl IntoView {
    let ByPromptState {
        answer,
        from,
        chosen,
        status,
        error,
        proposing,
        ..
    } = state;
    // `None` until the crew loads, so "Make a bruh first." never flashes.
    let bruhs = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            snapshot.as_ref().map(|snapshot| {
                snapshot
                    .bruhs
                    .iter()
                    .map(|bruh| (bruh.agent_id.clone(), bruh.name.clone()))
                    .collect::<Vec<_>>()
            })
        })
    });
    // The chosen bruh while it is still in the crew, else the first.
    let target = Memo::new(move |_| {
        bruhs.with(|bruhs| {
            let bruhs = bruhs.as_deref().unwrap_or_default();
            chosen
                .with(|id| bruhs.iter().find(|(each, _)| each == id))
                .or_else(|| bruhs.first())
                .cloned()
        })
    });
    let copied = RwSignal::new(false);
    let reset = ResetTimer::new();
    let waiting = move || data.busy.get() || proposing.with(Option::is_some);
    let empty = move || answer.with(|answer| answer.trim().is_empty());
    let make_bruh = move || {
        let heading = from.get_untracked().unwrap_or_else(pasted_from_today);
        match answer.with_untracked(|answer| prefill_for_answer(answer, &heading)) {
            Err(copy) => error.set(Some(copy)),
            Ok(prefill) => {
                error.set(None);
                on_make_bruh.run(prefill);
            }
        }
    };

    let copy = move |_| {
        crate::ui::copy_to_clipboard(MOVE_IN_PROMPT);
        copied.set(true);
        reset.after(COPIED_MS, move || {
            copied.try_set(false);
        });
    };
    let propose = move || {
        if untrack(waiting) || untrack(empty) {
            return;
        }
        let Some((agent_id, name)) = target.get_untracked() else {
            return;
        };
        let current = data.snapshot.with_untracked(|snapshot| {
            snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.agents.iter().find(|agent| agent.id == agent_id))
                .map(|agent| agent.memory.clone())
                .unwrap_or_default()
        });
        let heading = from.get_untracked().unwrap_or_else(pasted_from_today);
        status.set(None);
        match answer.with_untracked(|answer| moved_in_memory(&current, answer, &heading)) {
            Err(copy) => error.set(Some(copy)),
            Ok(memory) => {
                error.set(None);
                proposing.set(Some(name));
                data.send.run(propose_memory(agent_id, memory));
            }
        }
    };
    let picker = move || {
        let bruhs = bruhs.get()?;
        if bruhs.is_empty() {
            return Some(
                view! {
                    <div class="flex flex-wrap items-center gap-2">
                        <p data-move-in-no-bruhs="" class="text-label-sm text-rambutan-200">{NO_BRUHS}</p>
                        <button type="button" data-move-in-make-bruh="" class=BUTTON on:click=move |_| make_bruh()>{MAKE_A_BRUH}</button>
                    </div>
                }
                .into_any(),
            );
        }
        Some(
            view! {
                <label class=LABEL>
                    <span>{FOR_BRUH}</span>
                    <select
                        data-move-in-bruh=""
                        class=FIELD
                        prop:value=move || target.get().map(|(id, _)| id).unwrap_or_default()
                        on:change=move |event| chosen.set(event_target_value(&event))
                    >
                        {bruhs
                            .into_iter()
                            .map(|(id, name)| {
                                let selected = {
                                    let id = id.clone();
                                    move || target.with(|target| target.as_ref().is_some_and(|(each, _)| *each == id))
                                };
                                view! { <option data-move-in-option="" value=id selected=selected>{name}</option> }
                            })
                            .collect_view()}
                    </select>
                </label>
            }
            .into_any(),
        )
    };
    let has_bruhs =
        move || bruhs.with(|bruhs| bruhs.as_ref().is_some_and(|bruhs| !bruhs.is_empty()));

    view! {
        <div data-move-in-by-prompt="" class="flex flex-col gap-2 px-4 py-3">
            <div data-move-in-prompt="" class="max-h-40 overflow-y-auto whitespace-pre-wrap break-words select-text rounded-grape bg-longan-950/60 p-3 text-label-sm text-rambutan-200 inset-ring inset-ring-longan-700/50">{MOVE_IN_PROMPT}</div>
            <div>
                <button type="button" data-move-in-copy="" class=BUTTON on:click=copy>{move || if copied.get() { COPIED } else { COPY_PROMPT }}</button>
            </div>
            <p data-move-in-hint="" class=HELP>{MOVE_IN_HINT}</p>
            {picker}
            <label class=LABEL>
                <span>{THEIR_ANSWER}</span>
                <textarea
                    data-move-in-answer=""
                    data-autofocus=""
                    rows="6"
                    class=format!("{FIELD} resize-y py-2")
                    prop:value=move || answer.get()
                    on:input=move |event| answer.set(event_target_value(&event))
                >{answer.get_untracked()}</textarea>
            </label>
            <Show when=has_bruhs>
                <div>
                    <button type="button" data-move-in-propose="" class=BUTTON aria-disabled=move || (empty() || waiting()).then_some("true") on:click=move |_| propose()>{PROPOSE_MEMORY}</button>
                </div>
            </Show>
            <p role="status" data-move-in-status="" class=STATUS>{move || status.get()}</p>
            <p role="alert" data-move-in-error="" class=ERROR>{move || error.get().map(str::to_string).or_else(|| data.error.get())}</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::crew::{BruhAccess, BruhLook, BruhState, BruhStatus, CREW_PROTOCOL};
    use bruhs_models::imports::{
        COPY_PROMPT, FOR_BRUH, MAKE_A_BRUH, MOVE_IN_HINT, MOVE_IN_PROMPT, NO_BRUHS, PROPOSE_MEMORY,
        THEIR_ANSWER,
    };
    use bruhs_models::reactive::ReactiveSnapshot;
    use leptos::tachys;

    fn bruh(id: &str, name: &str) -> BruhState {
        BruhState {
            agent_id: id.into(),
            name: name.into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status: BruhStatus::Asleep,
            next_wake_ms: None,
            unread: 0,
            computer: false,
            vault: Vec::new(),
            apps: Vec::new(),
            mail: Vec::new(),
        }
    }

    fn render(data: CrewData, answer: &str) -> String {
        let _ = any_spawner::Executor::init_futures_executor();
        let state = ByPromptState::new(Some(answer.to_string()), None);
        untrack(|| {
            tachys::view::RenderHtml::to_html(view! {
                <ByPromptTab data=data state=state on_make_bruh=Callback::new(|_| {}) />
            })
        })
    }

    #[test]
    fn an_accepted_proposal_refreshes_the_crew_views_and_clears() {
        Owner::new().with(|| {
            let edits = RwSignal::new(0u64);
            provide_context(CrewEdits(edits));
            let state =
                ByPromptState::new(Some("Likes tea.".into()), Some("Claude.ai export".into()));
            // Refused: the text stays, nothing else moves.
            state.proposing.set(Some("Pinch".into()));
            state.answered(false);
            assert_eq!(state.answer.get_untracked(), "Likes tea.");
            assert_eq!(edits.get_untracked(), 0);
            assert_eq!(state.status.get_untracked(), None);
            assert_eq!(state.proposing.get_untracked(), None);
            // Accepted: every crew view reads again, and the line says where.
            state.proposing.set(Some("Pinch".into()));
            state.answered(true);
            assert_eq!(edits.get_untracked(), 1);
            assert_eq!(
                state.status.get_untracked(),
                Some(memory_proposed_line("Pinch"))
            );
            assert_eq!(state.answer.get_untracked(), "");
            assert_eq!(state.from.get_untracked(), None);
            // Only a proposal on its way settles.
            state.answered(true);
            assert_eq!(edits.get_untracked(), 1);
        });
    }

    fn escaped(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// The text of every element carrying `marker`, in page order.
    fn texts<'a>(html: &'a str, marker: &str) -> Vec<&'a str> {
        html.match_indices(marker)
            .map(|(at, _)| {
                let start = at + html[at..].find('>').unwrap() + 1;
                let end = start + html[start..].find('<').unwrap();
                &html[start..end]
            })
            .collect()
    }

    /// The opening tag of the element carrying `marker`.
    fn tag<'a>(html: &'a str, marker: &str) -> &'a str {
        let at = html
            .find(marker)
            .unwrap_or_else(|| panic!("{marker} in {html}"));
        let start = html[..at].rfind('<').unwrap();
        let end = at + html[at..].find('>').unwrap();
        &html[start..=end]
    }

    fn with_bruhs(bruhs: Vec<BruhState>) -> CrewData {
        let data = CrewData::for_tests(CREW_PROTOCOL);
        data.snapshot.set(Some(ReactiveSnapshot {
            protocol: CREW_PROTOCOL,
            bruhs,
            ..ReactiveSnapshot::default()
        }));
        data
    }

    #[test]
    fn the_by_prompt_tab_proposes_for_the_chosen_bruh() {
        Owner::new().with(|| {
            let data = with_bruhs(vec![bruh("a", "Pinch"), bruh("b", "Scout")]);
            let html = render(data, "");
            assert_eq!(
                texts(&html, "data-move-in-prompt"),
                [escaped(MOVE_IN_PROMPT)]
            );
            assert_eq!(texts(&html, "data-move-in-copy"), [COPY_PROMPT]);
            assert_eq!(texts(&html, "data-move-in-hint"), [MOVE_IN_HINT]);
            assert!(html.contains(&format!(">{FOR_BRUH}<")), "{html}");
            assert_eq!(texts(&html, "data-move-in-option"), ["Pinch", "Scout"]);
            assert!(html.contains(&format!(">{THEIR_ANSWER}<")), "{html}");
            assert!(tag(&html, "data-move-in-answer").contains("data-autofocus"));
            // Nothing to propose yet: the button says so, and keeps focus.
            let propose = tag(&html, "data-move-in-propose");
            assert!(propose.contains("aria-disabled=\"true\""), "{propose}");
            assert!(!propose.contains(" disabled=\""), "{propose}");
            assert_eq!(texts(&html, "data-move-in-propose"), [PROPOSE_MEMORY]);
            assert!(!html.contains(NO_BRUHS), "{html}");
            for marker in ["data-move-in-status", "data-move-in-error"] {
                assert!(tag(&html, marker).contains("empty:sr-only"), "{html}");
            }

            // A prefilled start shows its text in the area.
            let html = render(data, "Likes tea & short answers.");
            // (The test renderer writes a textarea's text as is; the app
            // sets its value through the DOM.)
            assert_eq!(
                texts(&html, "data-move-in-answer"),
                ["Likes tea & short answers."],
                "{html}"
            );
            assert!(!tag(&html, "data-move-in-propose").contains("aria-disabled=\""));

            // Busy: the button waits.
            data.busy.set(true);
            let html = render(data, "Likes tea.");
            assert!(tag(&html, "data-move-in-propose").contains("aria-disabled=\"true\""));
        });
    }

    #[test]
    fn with_no_bruhs_the_tab_offers_to_make_one() {
        Owner::new().with(|| {
            let html = render(with_bruhs(Vec::new()), "");
            assert_eq!(texts(&html, "data-move-in-no-bruhs"), [NO_BRUHS]);
            assert_eq!(texts(&html, "data-move-in-make-bruh"), [MAKE_A_BRUH]);
            assert!(texts(&html, "data-move-in-option").is_empty(), "{html}");
            assert!(texts(&html, "data-move-in-propose").is_empty(), "{html}");

            // Before the crew loads, neither shows.
            let loading = CrewData::for_tests(CREW_PROTOCOL);
            loading.snapshot.set(None);
            let html = render(loading, "");
            assert!(texts(&html, "data-move-in-no-bruhs").is_empty(), "{html}");
            assert!(texts(&html, "data-move-in-option").is_empty(), "{html}");
        });
    }
}
