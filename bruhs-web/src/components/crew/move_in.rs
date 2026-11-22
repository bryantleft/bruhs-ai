//! The Move in dialog (D1b): a bruh from an OpenClaw workspace or a Hermes
//! Agent home, or memory another assistant has of the owner, By prompt.
//!
//! A persona never becomes a bruh here: **Make this bruh** opens the Create
//! bruh dialog prefilled, and the owner makes it there. The bruh starts
//! Read only, with empty memory and its own folder; the persona's memory
//! waits as a proposal on its Memory tab (D11, D12).

pub mod by_prompt;

use bruhs_models::imports::{
    looked_in_line, persona_lines, prefill_from_persona, AgentScan, ImportPrefill, MoveInStart,
    MoveInTab, PersonaCandidate, AGENTS_PRIVACY, BY_PROMPT_TAB, FOLDER, LOOK_HERE, MAKE_BRUH,
    MOVE_IN_TITLE, NONE_FOUND_HERMES, NONE_FOUND_OPENCLAW,
};
use bruhs_models::logic::portable::ImportSource;
use leptos::prelude::*;

use self::by_prompt::{ByPromptState, ByPromptTab};
use super::data::use_crew_snapshot;
use crate::api::{imports as api, ApiError};
use crate::state::{CreateBruhOpen, CreateBruhPrefill, MoveInOpen};
use crate::ui::Dialog;

pub(crate) const BUTTON: &str = "bruhs-control min-h-control shrink-0 cursor-pointer rounded-seed px-3 text-label-sm text-rambutan-200 inset-ring inset-ring-longan-700 transition-colors duration-150 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 aria-disabled:cursor-not-allowed aria-disabled:opacity-50";
pub(crate) const FIELD: &str = "bruhs-control mt-1 block min-h-control w-full min-w-0 rounded-seed bg-longan-950 px-3 text-body-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-longan-700 focus:outline focus:outline-2 focus:outline-persimmon-400";
pub(crate) const LABEL: &str = "block min-w-0 text-label-sm text-rambutan-300";
pub(crate) const HELP: &str = "text-label-sm text-rambutan-300";
// Live regions stay in the accessibility tree while empty, so the first
// line put in them is announced.
pub(crate) const ERROR: &str = "text-label-sm text-lychee-300 empty:sr-only";
const ROW: &str = "bruhs-control flex min-h-control w-full min-w-0 flex-col items-start gap-0.5 rounded-grape px-3 py-2 text-left cursor-pointer focus-visible:outline-2 focus-visible:outline-persimmon-400";
const TAB_ON: &str = "bruhs-control shrink-0 whitespace-nowrap rounded-grape bg-blueberry-600/20 px-2.5 py-1 text-label-sm text-blueberry-200 inset-ring inset-ring-blueberry-500/30 cursor-pointer focus-visible:outline-2 focus-visible:outline-persimmon-400";
const TAB_OFF: &str = "bruhs-control shrink-0 whitespace-nowrap rounded-grape px-2.5 py-1 text-label-sm text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-200 transition-colors cursor-pointer focus-visible:outline-2 focus-visible:outline-persimmon-400";

const TABS: [MoveInTab; 3] = [MoveInTab::OpenClaw, MoveInTab::Hermes, MoveInTab::ByPrompt];

fn tab_label(tab: MoveInTab) -> &'static str {
    match tab {
        MoveInTab::OpenClaw => ImportSource::OpenClaw.label(),
        MoveInTab::Hermes => ImportSource::Hermes.label(),
        MoveInTab::ByPrompt => BY_PROMPT_TAB,
    }
}

/// The agent a tab scans, or `None` for By prompt.
fn tab_source(tab: MoveInTab) -> Option<ImportSource> {
    match tab {
        MoveInTab::OpenClaw => Some(ImportSource::OpenClaw),
        MoveInTab::Hermes => Some(ImportSource::Hermes),
        MoveInTab::ByPrompt => None,
    }
}

/// One agent tab's scan: what it found, the folder typed, the persona
/// chosen, and the call in flight.
#[derive(Clone, Copy)]
pub(crate) struct AgentState {
    pub scan: RwSignal<Option<AgentScan>>,
    pub folder: RwSignal<String>,
    pub picked: RwSignal<usize>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    /// Aborted when another scan starts or the dialog closes.
    abort: StoredValue<Option<web_sys::AbortController>, LocalStorage>,
}

impl AgentState {
    pub(crate) fn new() -> Self {
        Self {
            scan: RwSignal::new(None),
            folder: RwSignal::new(String::new()),
            picked: RwSignal::new(0),
            loading: RwSignal::new(false),
            error: RwSignal::new(None),
            abort: StoredValue::new_local(None),
        }
    }

    /// Drops the scan in flight, if any.
    fn stop(self) {
        if let Some(Some(controller)) = self.abort.try_update_value(Option::take) {
            controller.abort();
        }
    }

    /// Scans `source`'s default places, or `path` when one was typed.
    fn run(self, source: ImportSource, path: Option<String>) {
        self.stop();
        let Ok(controller) = web_sys::AbortController::new() else {
            return;
        };
        let signal = controller.signal();
        self.abort.set_value(Some(controller));
        self.loading.set(true);
        self.error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = api::scan_agents(source, path.as_deref(), &signal).await;
            match result {
                // Another scan started or the dialog closed: this one's state
                // isn't its to touch.
                Err(_) if signal.aborted() => {}
                Err(ApiError::Aborted) => {}
                Ok(scan) => {
                    self.loading.try_set(false);
                    self.picked.try_set(0);
                    self.scan.try_set(Some(scan));
                }
                Err(error) => {
                    self.loading.try_set(false);
                    self.error.try_set(Some(error.to_string()));
                }
            }
        });
    }
}

#[component]
pub(crate) fn AgentTab(
    source: ImportSource,
    state: AgentState,
    on_make: Callback<PersonaCandidate>,
) -> impl IntoView {
    let none_found = if source == ImportSource::Hermes {
        NONE_FOUND_HERMES
    } else {
        NONE_FOUND_OPENCLAW
    };
    let look = move || {
        if state.loading.get_untracked() {
            return;
        }
        let path = state.folder.get_untracked();
        let path = (!path.trim().is_empty()).then_some(path);
        state.run(source, path);
    };
    let chosen = Memo::new(move |_| {
        state.scan.with(|scan| {
            scan.as_ref().and_then(|scan| {
                let at = state
                    .picked
                    .get()
                    .min(scan.personas.len().saturating_sub(1));
                scan.personas.get(at).cloned()
            })
        })
    });
    let looked_in = move || {
        state.scan.with(|scan| {
            scan.as_ref()
                .filter(|scan| !scan.looked_in.is_empty())
                .map(|scan| looked_in_line(&scan.looked_in))
                .map(|line| view! { <p data-move-in-places="" class=HELP>{line}</p> })
        })
    };
    // Redrawn only when a scan lands: choosing a row restyles it in place,
    // so focus stays on the row pressed.
    let personas = Memo::new(move |_| {
        state
            .scan
            .with(|scan| scan.as_ref().map(|scan| scan.personas.clone()))
    });
    let rows = move || {
        let personas = personas.get()?;
        if personas.is_empty() {
            return Some(
                view! { <p data-move-in-none="" class="text-label-sm text-rambutan-200">{none_found}</p> }
                    .into_any(),
            );
        }
        let last = personas.len().saturating_sub(1);
        Some(
            view! {
                <div class="-mx-1 flex max-h-48 flex-col gap-1 overflow-y-auto px-1" role="group" aria-label=source.label()>
                    {personas
                        .into_iter()
                        .enumerate()
                        .map(|(at, persona)| {
                            let on = move || state.picked.get().min(last) == at;
                            view! {
                                <button
                                    type="button"
                                    aria-pressed=move || on().to_string()
                                    // Blueberry marks the selection (design rule).
                                    class=move || if on() {
                                        format!("{ROW} bg-blueberry-600/20 inset-ring inset-ring-blueberry-500/30")
                                    } else {
                                        format!("{ROW} hover:bg-longan-700/40")
                                    }
                                    on:click=move |_| state.picked.set(at)
                                >
                                    <span data-move-in-name="" class="w-full truncate text-label-sm text-rambutan-100">{persona.name}</span>
                                    <span class="w-full truncate text-label-sm text-rambutan-400">{persona.folder}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            }
            .into_any(),
        )
    };
    let preview = move || {
        chosen.get().map(|persona| {
            let lines = persona_lines(&persona)
                .into_iter()
                .map(|line| view! { <p data-move-in-line="" class=HELP>{line}</p> })
                .collect_view();
            view! {
                <div data-move-in-preview="" class="space-y-1 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50">
                    {lines}
                    <div class="pt-1">
                        <button type="button" data-move-in-make="" class=BUTTON on:click=move |_| on_make.run(persona.clone())>{MAKE_BRUH}</button>
                    </div>
                </div>
            }
        })
    };

    view! {
        <div data-move-in-agents="" class="flex flex-col gap-2 px-4 py-3">
            <p data-move-in-privacy="" class=HELP>{AGENTS_PRIVACY}</p>
            <div class="flex min-w-0 items-end gap-2">
                <label class=format!("{LABEL} flex-1")>
                    <span>{FOLDER}</span>
                    <input
                        data-move-in-folder=""
                        data-autofocus=""
                        type="text"
                        class=FIELD
                        autocomplete="off"
                        spellcheck="false"
                        prop:value=move || state.folder.get()
                        on:input=move |event| state.folder.set(event_target_value(&event))
                        on:keydown=move |event: web_sys::KeyboardEvent| {
                            if event.key() == "Enter" {
                                event.prevent_default();
                                look();
                            }
                        }
                    />
                </label>
                <button type="button" data-move-in-look="" class=BUTTON aria-disabled=move || state.loading.get().then_some("true") on:click=move |_| look()>{LOOK_HERE}</button>
            </div>
            {looked_in}
            {rows}
            {preview}
            <p role="alert" data-move-in-error="" class=ERROR>{move || state.error.get()}</p>
        </div>
    }
}

/// The Move in dialog, mounted once in the shell, hung from the top so its
/// tabs stay put while a scan's rows land. It opens on the tab its
/// [`MoveInOpen`] start names: the palette opens OpenClaw, the Export file
/// tab opens By prompt with an export's memory.
#[component]
pub fn MoveInDialog() -> impl IntoView {
    let MoveInOpen(start) = expect_context::<MoveInOpen>();
    let close = Callback::new(move |()| {
        start.set(None);
        super::create::keep_focus_after_close();
    });
    view! {
        <Dialog open=Signal::derive(move || start.with(Option::is_some)) on_close=close max_width="max-w-lg".to_string() label=MOVE_IN_TITLE anchor_top=true>
            {move || start.get_untracked().map(|start| view! { <MoveInBody start=start close=close /> })}
        </Dialog>
    }
}

/// One opening of the dialog, with its own crew data and scans.
#[component]
fn MoveInBody(start: MoveInStart, close: Callback<()>) -> impl IntoView {
    let data = use_crew_snapshot();
    let create_open = use_context::<CreateBruhOpen>().map(|open| open.0);
    let prefill = use_context::<CreateBruhPrefill>().map(|prefill| prefill.0);
    let tab = RwSignal::new(start.tab);
    let openclaw = AgentState::new();
    let hermes = AgentState::new();
    let by_prompt = ByPromptState::new(start.text, start.from);
    by_prompt.watch(data);
    let state_for = move |tab: MoveInTab| match tab {
        MoveInTab::OpenClaw => Some(openclaw),
        MoveInTab::Hermes => Some(hermes),
        MoveInTab::ByPrompt => None,
    };

    // Each agent tab scans its default places the first time it shows.
    Effect::new(move |_| {
        let tab = tab.get();
        if let (Some(source), Some(state)) = (tab_source(tab), state_for(tab)) {
            if state.scan.with_untracked(Option::is_none) && !state.loading.get_untracked() {
                state.run(source, None);
            }
        }
    });
    on_cleanup(move || {
        openclaw.stop();
        hermes.stop();
    });

    // Hands over to the Create bruh dialog, with or without a prefill.
    let open_create = move |next: Option<ImportPrefill>| {
        if let Some(prefill) = prefill {
            prefill.set(next);
        }
        close.run(());
        if let Some(create_open) = create_open {
            create_open.set(true);
        }
    };
    let on_make = Callback::new(move |persona: PersonaCandidate| {
        open_create(Some(prefill_from_persona(&persona)));
    });
    let on_make_bruh = Callback::new(open_create);

    let body = move || match tab.get() {
        MoveInTab::OpenClaw => {
            view! { <AgentTab source=ImportSource::OpenClaw state=openclaw on_make=on_make /> }
                .into_any()
        }
        MoveInTab::Hermes => {
            view! { <AgentTab source=ImportSource::Hermes state=hermes on_make=on_make /> }
                .into_any()
        }
        MoveInTab::ByPrompt => {
            view! { <ByPromptTab data=data state=by_prompt on_make_bruh=on_make_bruh /> }.into_any()
        }
    };

    view! {
        <div class="border-b border-longan-800 px-4 py-3">
            <h2 class="text-label-sm text-rambutan-200">{MOVE_IN_TITLE}</h2>
            <div class="mt-2 flex gap-1 overflow-x-auto" role="tablist" aria-label=MOVE_IN_TITLE>
                {TABS
                    .into_iter()
                    .map(|each| view! {
                        <button
                            type="button"
                            role="tab"
                            aria-selected=move || (tab.get() == each).to_string()
                            on:click=move |_| tab.set(each)
                            class=move || if tab.get() == each { TAB_ON } else { TAB_OFF }
                        >{tab_label(each)}</button>
                    })
                    .collect_view()}
            </div>
        </div>
        <div role="tabpanel" aria-label=move || tab_label(tab.get()) class="max-h-[70vh] overflow-y-auto">
            {body}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::imports::{
        looked_in_line, persona_lines, FileUse, PersonaFile, SkipReason, AGENTS_PRIVACY, FOLDER,
        LOOK_HERE, MAKE_BRUH, NONE_FOUND_HERMES, NONE_FOUND_OPENCLAW,
    };
    use leptos::tachys;

    fn persona(name: &str, memory: &str) -> PersonaCandidate {
        PersonaCandidate {
            id: format!("id-{name}"),
            source: ImportSource::OpenClaw,
            folder: format!("/home/example/{name}"),
            name: name.into(),
            instructions: format!("## SOUL.md\n\nBe {name}."),
            memory: memory.into(),
            files: vec![
                PersonaFile {
                    name: "SOUL.md".into(),
                    use_: FileUse::Instructions,
                },
                PersonaFile {
                    name: "USER.md".into(),
                    use_: FileUse::Memory,
                },
                PersonaFile {
                    name: "HEARTBEAT.md".into(),
                    use_: FileUse::Skipped {
                        reason: SkipReason::NotImported,
                    },
                },
            ],
            clipped: false,
            dropped_identity: vec!["Emoji".into()],
            left_out_logs: 0,
        }
    }

    fn render(source: ImportSource, state: AgentState) -> String {
        let _ = any_spawner::Executor::init_futures_executor();
        untrack(|| {
            tachys::view::RenderHtml::to_html(
                view! { <AgentTab source=source state=state on_make=Callback::new(|_| {}) /> },
            )
        })
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

    #[test]
    fn the_move_in_tabs_read_from_the_shared_copy() {
        Owner::new().with(|| {
            let state = AgentState::new();
            let scan = AgentScan {
                personas: vec![persona("Pinch", "Likes tea."), persona("Scout", "")],
                looked_in: vec!["~/clawd".into(), "~/.openclaw/workspace".into()],
            };
            state.scan.set(Some(scan.clone()));
            let html = render(ImportSource::OpenClaw, state);
            assert_eq!(texts(&html, "data-move-in-privacy"), [AGENTS_PRIVACY]);
            assert_eq!(
                texts(&html, "data-move-in-places"),
                [looked_in_line(&scan.looked_in)]
            );
            assert_eq!(texts(&html, "data-move-in-name"), ["Pinch", "Scout"]);
            assert!(html.contains(&format!(">{FOLDER}<")), "{html}");
            assert!(tag(&html, "data-move-in-folder").contains("data-autofocus"));
            assert_eq!(texts(&html, "data-move-in-look"), [LOOK_HERE]);
            assert!(texts(&html, "data-move-in-none").is_empty(), "{html}");

            // The chosen persona shows its preview and Make this bruh.
            let lines: Vec<String> = persona_lines(&scan.personas[0])
                .iter()
                .map(|line| escaped(line))
                .collect();
            assert!(lines.len() > 2, "{lines:?}");
            assert_eq!(texts(&html, "data-move-in-line"), lines, "{html}");
            assert_eq!(texts(&html, "data-move-in-make"), [MAKE_BRUH]);
            // The chosen row wears the selection tone (blueberry).
            assert!(
                tag(&html, "aria-pressed=\"true\"").contains("bg-blueberry"),
                "{html}"
            );
            state.picked.set(1);
            let html = render(ImportSource::OpenClaw, state);
            let lines: Vec<String> = persona_lines(&scan.personas[1])
                .iter()
                .map(|line| escaped(line))
                .collect();
            assert_eq!(texts(&html, "data-move-in-line"), lines, "{html}");

            // Nothing found: the source's own line, and no preview.
            state.scan.set(Some(AgentScan {
                personas: Vec::new(),
                looked_in: vec!["~/clawd".into()],
            }));
            let html = render(ImportSource::OpenClaw, state);
            assert_eq!(texts(&html, "data-move-in-none"), [NONE_FOUND_OPENCLAW]);
            assert!(texts(&html, "data-move-in-make").is_empty(), "{html}");
            let html = render(ImportSource::Hermes, state);
            assert_eq!(texts(&html, "data-move-in-none"), [NONE_FOUND_HERMES]);
        });
    }

    #[test]
    fn a_scan_in_flight_keeps_its_buttons_focusable() {
        Owner::new().with(|| {
            let state = AgentState::new();
            state.loading.set(true);
            let html = render(ImportSource::OpenClaw, state);
            let look = tag(&html, "data-move-in-look");
            assert!(look.contains("aria-disabled=\"true\""), "{look}");
            assert!(!look.contains(" disabled=\""), "{look}");
            // Nothing found shows only once the scan answers.
            assert!(texts(&html, "data-move-in-none").is_empty(), "{html}");
            let error = tag(&html, "data-move-in-error");
            assert!(error.contains("empty:sr-only"), "{error}");
        });
    }
}
