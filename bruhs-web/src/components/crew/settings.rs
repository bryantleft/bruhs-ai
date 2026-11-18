//! A bruh's settings: who it is, how it runs, what it may do and its face.
//! Save sends `settings_commands`, so a face edit never touches the agent.

use bruhs_models::agent::{AgentProvider, ModelId};
use bruhs_models::crew::{
    access_options, clamp_level, computer_reason, computer_unavailable, current_settings,
    settings_commands, AccessLevel, BruhAccess, BruhLook, BruhSettings, COMPUTER_HINT,
    COMPUTER_SWITCH, SEND_AS_ME_HINT,
};
use bruhs_models::logic::run_place::CLAUDE_EFFORTS;
use bruhs_models::reactive::ReactiveCommand;
use leptos::prelude::*;

use super::data::use_crew_snapshot;
use super::face_editor::FaceEditor;
use crate::ui::{Button, ButtonSize, Dropdown, DropdownOption, Input, Switch, Textarea};

const LABEL: &str = "block min-w-0 space-y-1 text-label-sm text-rambutan-300";
const SECTION: &str =
    "min-w-0 space-y-3 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50";
const RADIO: &str = "bruhs-control inline-flex min-h-control flex-1 items-center justify-center rounded-seed px-3 text-label-sm transition-colors focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50";
const DELIVER_NEEDS_FULL: &str = "Needs Full access.";

/// Why the Computer switch is off: the bruh's model or level first, since
/// the owner can change those here, then an older service.
fn computer_switch_block(
    level: AccessLevel,
    model: &str,
    protocol: Option<u32>,
) -> Option<&'static str> {
    computer_unavailable(level, AgentProvider::for_model(model))
        .or_else(|| computer_reason(protocol))
}

fn effort_options() -> Vec<DropdownOption> {
    std::iter::once(DropdownOption::new("", "Default"))
        .chain(
            CLAUDE_EFFORTS
                .iter()
                .map(|level| DropdownOption::new(*level, *level)),
        )
        .collect()
}

#[component]
pub fn BruhSettingsTab(agent_id: String) -> impl IntoView {
    let data = use_crew_snapshot();
    let id = StoredValue::new(agent_id);
    let saved = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            snapshot
                .as_ref()
                .and_then(|snapshot| id.with_value(|id| current_settings(snapshot, id)))
        })
    });

    let name = RwSignal::new(String::new());
    let instructions = RwSignal::new(String::new());
    let model = RwSignal::new(String::new());
    let effort = RwSignal::new(None::<String>);
    let level = RwSignal::new(AccessLevel::ReadOnly);
    let send_as_me = RwSignal::new(false);
    let can_deliver = RwSignal::new(false);
    let computer = RwSignal::new(false);
    let look = RwSignal::new(BruhLook::default());
    let invalid = RwSignal::new(None::<String>);
    let edited = move || BruhSettings {
        name: name.get(),
        instructions: instructions.get(),
        model: model.get(),
        effort: effort.get(),
        look: look.get(),
        access: BruhAccess {
            level: level.get(),
            send_as_me: send_as_me.get(),
            can_deliver: can_deliver.get(),
        },
        computer: computer.get(),
    };

    // Fill the form from the service, unless the owner has unsaved edits.
    let shown = StoredValue::new(None::<BruhSettings>);
    Effect::new(move |_| {
        let Some(next) = saved.get() else {
            return;
        };
        let current = untrack(edited);
        let unsaved =
            shown.with_value(|shown| shown.as_ref().is_some_and(|shown| *shown != current));
        if unsaved && next != current {
            return;
        }
        name.set(next.name.clone());
        instructions.set(next.instructions.clone());
        model.set(next.model.clone());
        effort.set(next.effort.clone());
        level.set(next.access.level);
        send_as_me.set(next.access.send_as_me);
        can_deliver.set(next.access.can_deliver);
        computer.set(next.computer);
        look.set(next.look.clone());
        shown.set_value(Some(next));
    });
    // A model that can't run the chosen level drops it to Read only, which
    // the service would otherwise refuse.
    Effect::new(move |_| {
        let model = model.get();
        let current = level.get_untracked();
        let clamped = clamp_level(current, &model);
        if clamped != current {
            level.set(clamped);
        }
    });
    // Delivery needs Full.
    Effect::new(move |_| {
        if level.get() != AccessLevel::Full && can_deliver.get_untracked() {
            can_deliver.set(false);
        }
    });

    // The Computer switch is off while the bruh can't have one, or the
    // service can't run one. It then shows the stored value, which stays
    // stored for when the bruh can again.
    let computer_block =
        Memo::new(move |_| computer_switch_block(level.get(), &model.get(), data.protocol()));
    let computer_off =
        Signal::derive(move || computer_block.get().is_some() || !data.computer_ready());
    Effect::new(move |_| {
        if !computer_off.get() {
            return;
        }
        let stored =
            saved.with_untracked(|saved| saved.as_ref().is_some_and(|saved| saved.computer));
        if computer.get_untracked() != stored {
            computer.set(stored);
        }
    });

    let dirty = Memo::new(move |_| saved.get().is_some_and(|saved| saved != edited()));
    let save_disabled =
        Signal::derive(move || !dirty.get() || data.busy.get() || !data.crew_ready());
    let save = move |_| {
        let Some(snapshot) = data.snapshot.get_untracked() else {
            return;
        };
        let edited = untrack(edited);
        if edited.name.trim().is_empty() {
            invalid.set(Some("Give the bruh a name.".into()));
            return;
        }
        if let Err(error) = ModelId::try_from(edited.model.clone()) {
            invalid.set(Some(error.to_string()));
            return;
        }
        invalid.set(None);
        let commands = id.with_value(|id| {
            let agent = snapshot.agents.iter().find(|agent| agent.id == *id)?;
            let state = snapshot.bruhs.iter().find(|bruh| bruh.agent_id == *id)?;
            Some(settings_commands(agent, state, &edited))
        });
        // An older service doesn't know SetComputer.
        let computer_ready = data.computer_ready();
        for command in commands.unwrap_or_default() {
            if computer_ready || !matches!(command, ReactiveCommand::SetComputer { .. }) {
                data.send.run(command);
            }
        }
    };

    view! {
        <form class="mx-auto w-full max-w-3xl min-w-0 space-y-4 p-4 sm:px-6" aria-label="Bruh settings" on:submit=move |event| event.prevent_default()>
            <section class=SECTION aria-labelledby="bruh-settings-who">
                <h2 id="bruh-settings-who" class="text-label text-rambutan-100">"Who it is"</h2>
                <label class=LABEL>
                    <span>"Name"</span>
                    <Input aria_label="Name" value=Signal::derive(move || name.get()) on_input=Callback::new(move |text| name.set(text)) />
                </label>
                <label class=LABEL>
                    <span>"Job"</span>
                    <Textarea aria_label="Job" value=Signal::derive(move || instructions.get()) on_input=Callback::new(move |text| instructions.set(text)) rows=5 />
                </label>
            </section>
            <section class=SECTION aria-labelledby="bruh-settings-run">
                <h2 id="bruh-settings-run" class="text-label text-rambutan-100">"How it runs"</h2>
                <div class="grid min-w-0 gap-3 sm:grid-cols-2">
                    <label class=LABEL>
                        <span>"Model"</span>
                        <Input aria_label="Model" value=Signal::derive(move || model.get()) on_input=Callback::new(move |text| model.set(text)) />
                    </label>
                    <div class=LABEL>
                        <span>"Effort"</span>
                        <Dropdown
                            options=Signal::derive(effort_options)
                            value=Signal::derive(move || effort.get().unwrap_or_default())
                            on_change=Callback::new(move |value: String| effort.set(Some(value).filter(|value| !value.is_empty())))
                            label="Effort"
                            class="w-full"
                        />
                    </div>
                </div>
            </section>
            <section class=SECTION aria-labelledby="bruh-settings-access">
                <h2 id="bruh-settings-access" class="text-label text-rambutan-100">"What it may do"</h2>
                <div role="radiogroup" aria-label="Access" class="flex min-w-0 flex-wrap gap-1 rounded-grape bg-longan-900 p-0.5 inset-ring inset-ring-longan-800">
                    {move || access_options(&model.get()).into_iter().map(|(option, reason)| view! {
                        <button
                            type="button"
                            role="radio"
                            aria-checked=move || (level.get() == option).to_string()
                            disabled=reason.is_some()
                            title=reason.unwrap_or_default()
                            class=move || format!("{RADIO} {}", if level.get() == option { "bg-longan-700 text-rambutan-100" } else { "text-rambutan-400 hover:text-rambutan-200" })
                            on:click=move |_| level.set(option)
                        >{option.label()}</button>
                    }).collect_view()}
                </div>
                {move || {
                    let mut reasons: Vec<&'static str> = access_options(&model.get()).into_iter().filter_map(|(_, reason)| reason).collect();
                    reasons.dedup();
                    reasons.into_iter().map(|reason| view! { <p class="text-label-sm text-rambutan-500">{reason}</p> }).collect_view()
                }}
                <div class="flex min-h-control items-center justify-between gap-3">
                    <div class="min-w-0">
                        <span class="text-label-sm text-rambutan-300">"Send as me"</span>
                        <p data-send-as-me-hint="" class="text-label-sm text-rambutan-500">{SEND_AS_ME_HINT}</p>
                    </div>
                    <Switch label="Send as me" checked=Signal::derive(move || send_as_me.get()) on_change=Callback::new(move |on| send_as_me.set(on)) />
                </div>
                <div class="flex min-h-control items-center justify-between gap-3">
                    <div class="min-w-0">
                        <span class="text-label-sm text-rambutan-300">"Can deliver"</span>
                        <Show when=move || level.get() != AccessLevel::Full>
                            <p class="text-label-sm text-rambutan-500">{DELIVER_NEEDS_FULL}</p>
                        </Show>
                    </div>
                    <Switch label="Can deliver" checked=Signal::derive(move || can_deliver.get()) disabled=Signal::derive(move || level.get() != AccessLevel::Full) on_change=Callback::new(move |on| can_deliver.set(on)) />
                </div>
                <div class="flex min-h-control items-center justify-between gap-3">
                    <div class="min-w-0">
                        <span class="text-label-sm text-rambutan-300">{COMPUTER_SWITCH}</span>
                        <p data-bruh-computer-hint="" class="text-label-sm text-rambutan-500">{COMPUTER_HINT}</p>
                        {move || computer_block.get().map(|reason| view! { <p data-bruh-computer-reason="" class="text-label-sm text-rambutan-500">{reason}</p> })}
                    </div>
                    // The label makes the whole 44px touch target press the switch.
                    <label class="bruhs-control inline-flex shrink-0 items-center justify-center">
                        <Switch
                            label=COMPUTER_SWITCH
                            checked=Signal::derive(move || computer.get())
                            disabled=computer_off
                            on_change=Callback::new(move |on| computer.set(on))
                            attr:data-bruh-computer=""
                            attr:title=move || computer_block.get()
                        />
                    </label>
                </div>
            </section>
            <section class=SECTION aria-labelledby="bruh-settings-face">
                <h2 id="bruh-settings-face" class="text-label text-rambutan-100">"Face"</h2>
                <FaceEditor look=look />
            </section>
            {move || invalid.get().map(|message| view! { <p role="alert" class="rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200">{message}</p> })}
            <div class="flex flex-wrap items-center gap-3">
                <Button size=ButtonSize::Small disabled=save_disabled on_click=Callback::new(save)>"Save"</Button>
                {move || data.service_reason().map(|reason| view! { <p class="min-w-0 text-label-sm text-rambutan-400">{reason}</p> })}
            </div>
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::crew::{ServiceNotice, CREW_PROTOCOL};

    #[test]
    fn the_computer_switch_says_why_it_is_off() {
        let claude = bruhs_models::agent::DEFAULT_AGENT_MODEL;
        assert_eq!(
            computer_switch_block(AccessLevel::ReadOnly, claude, Some(CREW_PROTOCOL)),
            Some("Computer needs Ask first or Full access.")
        );
        assert_eq!(
            computer_switch_block(AccessLevel::Full, "gpt-5.1-codex", Some(CREW_PROTOCOL)),
            Some("Computer needs a Claude model.")
        );
        assert_eq!(
            computer_switch_block(AccessLevel::Full, claude, Some(2)),
            Some(ServiceNotice::ServingOld.message())
        );
        assert_eq!(
            computer_switch_block(AccessLevel::Full, claude, Some(CREW_PROTOCOL)),
            None
        );
        assert_eq!(
            computer_switch_block(AccessLevel::Ask, claude, Some(CREW_PROTOCOL)),
            None
        );
    }
}
