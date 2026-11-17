//! A reviewed automation saves its agent, source, and subscription as one command.
use super::calendar::{calendar_source, local_time_zone, WeekdayToggles};
use super::{
    apply, command_button, id, milliseconds, seconds, source_kind, Check, Field, CARD, SELECT,
};
use crate::ui::{Button, ButtonSize, ButtonVariant};
use bruhs_models::reactive::*;
use leptos::prelude::*;

fn new_setup() -> AutomationSetup {
    let agent_id = id();
    let source_id = id();
    AutomationSetup {
        agent:ReactiveAgent{id:agent_id.clone(),name:String::new(),instructions:String::new(),memory:String::new(),model:bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),project_path:String::new(),enabled:true},
        source:EventSource{id:source_id.clone(),name:String::new(),enabled:true,kind:SourceKind::Folder{path:String::new(),pattern:"*.pdf".into(),recursive:true,settle_ms:2_000,poll_ms:5_000}},
        subscription:Subscription{id:id(),agent_id,source_id,instructions:"Follow the automation's purpose for this event. Prepare a draft and flag uncertainty. Never perform writes or payments.".into(),enabled:true,auto_run:true,debounce_ms:0,throttle_ms:0,on_busy:BusyPolicy::Queue},
    }
}

#[component]
pub(super) fn AutomationSetupPanel(
    snapshot: RwSignal<Option<ReactiveSnapshot>>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    send: Callback<ReactiveCommand>,
    on_chat: Callback<String>,
) -> impl IntoView {
    let base = RwSignal::new(None::<AutomationSetup>);
    let review = RwSignal::new(None::<AutomationSetup>);
    let saving = RwSignal::new(false);
    let name = RwSignal::new(String::new());
    let purpose = RwSignal::new(String::new());
    let folder = RwSignal::new(String::new());
    let model = RwSignal::new(bruhs_models::agent::DEFAULT_AGENT_MODEL.into());
    let memory = RwSignal::new(String::new());
    let kind = RwSignal::new("folder".to_string());
    let source_value = RwSignal::new(String::new());
    let command_folder = RwSignal::new(String::new());
    let pattern = RwSignal::new("*.pdf".into());
    let recursive = RwSignal::new(true);
    let interval = RwSignal::new("5".into());
    let settle = RwSignal::new("2".into());
    let timeout = RwSignal::new("30".into());
    let note = RwSignal::new(String::new());
    let auto_run = RwSignal::new(true);
    let debounce = RwSignal::new("0".into());
    let throttle = RwSignal::new("0".into());
    let cal_times = RwSignal::new("08:00".to_string());
    let cal_days = RwSignal::new(Vec::<u8>::new());
    let cal_zone = RwSignal::new(local_time_zone());
    let open = Callback::new(move |setup: AutomationSetup| {
        cal_times.set("08:00".into());
        cal_days.set(Vec::new());
        cal_zone.set(local_time_zone());
        command_folder.set(String::new());
        timeout.set("30".into());
        source_value.set(String::new());
        pattern.set("*.pdf".into());
        recursive.set(true);
        interval.set("5".into());
        settle.set("2".into());
        name.set(setup.agent.name.clone());
        purpose.set(setup.agent.instructions.clone());
        folder.set(setup.agent.project_path.clone());
        model.set(setup.agent.model.clone());
        memory.set(setup.agent.memory.clone());
        note.set(setup.subscription.instructions.clone());
        auto_run.set(setup.subscription.auto_run);
        debounce.set(seconds(setup.subscription.debounce_ms));
        throttle.set(seconds(setup.subscription.throttle_ms));
        match &setup.source.kind {
            SourceKind::Folder {
                path,
                pattern: mask,
                recursive: recurse,
                settle_ms,
                poll_ms,
            } => {
                kind.set("folder".into());
                source_value.set(path.clone());
                pattern.set(mask.clone());
                recursive.set(*recurse);
                settle.set(seconds(*settle_ms));
                interval.set(seconds(*poll_ms));
            }
            SourceKind::Http { url, poll_ms } => {
                kind.set("http".into());
                source_value.set(url.clone());
                interval.set(seconds(*poll_ms));
            }
            SourceKind::Command {
                command,
                cwd,
                poll_ms,
                timeout_ms,
            } => {
                kind.set("command".into());
                source_value.set(command.clone());
                command_folder.set(cwd.clone());
                interval.set(seconds(*poll_ms));
                timeout.set(seconds(*timeout_ms));
            }
            SourceKind::Schedule { every_ms } => {
                kind.set("schedule".into());
                interval.set(seconds(*every_ms));
            }
            SourceKind::Calendar {
                times,
                weekdays,
                time_zone,
            } => {
                kind.set("calendar".into());
                cal_times.set(times.join(", "));
                cal_days.set(weekdays.clone());
                cal_zone.set(time_zone.clone());
            }
            SourceKind::Webhook => kind.set("webhook".into()),
            SourceKind::AppEvent { topic } => {
                kind.set("app_event".into());
                source_value.set(topic.clone());
            }
            SourceKind::Custom => kind.set("custom".into()),
        }
        review.set(None);
        base.set(Some(setup));
        error.set(None);
    });
    Effect::new(move |_| {
        if saving.get() && !busy.get() {
            if error.get().is_none()
                && review.with(|review| {
                    review.as_ref().is_some_and(|setup| {
                        snapshot.with(|s| {
                            s.as_ref().is_some_and(|s| {
                                s.subscriptions
                                    .iter()
                                    .any(|sub| sub.id == setup.subscription.id)
                            })
                        })
                    })
                })
            {
                base.set(None);
                review.set(None);
            }
            saving.set(false);
        }
    });
    let prepare = move |_| {
        let Some(mut setup) = base.get_untracked() else {
            return;
        };
        let build = (|| -> Result<AutomationSetup, String> {
            if name.get_untracked().trim().is_empty()
                || purpose.get_untracked().trim().is_empty()
                || folder.get_untracked().trim().is_empty()
            {
                return Err(
                    "Enter an automation name, purpose, and working folder before reviewing."
                        .into(),
                );
            }
            bruhs_models::agent::ModelId::try_from(model.get_untracked())
                .map_err(|error| error.to_string())?;
            setup.agent.name = name.get_untracked();
            setup.agent.instructions = purpose.get_untracked();
            setup.agent.project_path = folder.get_untracked();
            setup.agent.model = model.get_untracked();
            setup.agent.memory = memory.get_untracked();
            setup.agent.enabled = true;
            if setup.source.name.trim().is_empty() {
                setup.source.name = format!("{} source", setup.agent.name);
            }
            let cwd = if command_folder.get_untracked().trim().is_empty() {
                folder.get_untracked()
            } else {
                command_folder.get_untracked()
            };
            setup.source.kind = if kind.get_untracked() == "calendar" {
                calendar_source(
                    &cal_times.get_untracked(),
                    &cal_days.get_untracked(),
                    &cal_zone.get_untracked(),
                )?
            } else {
                source_kind(
                    &kind.get_untracked(),
                    source_value.get_untracked(),
                    cwd,
                    pattern.get_untracked(),
                    recursive.get_untracked(),
                    &interval.get_untracked(),
                    &settle.get_untracked(),
                    &timeout.get_untracked(),
                )?
            };
            setup.source.enabled = true;
            setup.subscription.instructions = note.get_untracked();
            setup.subscription.enabled = true;
            setup.subscription.auto_run = auto_run.get_untracked();
            setup.subscription.debounce_ms = milliseconds(&debounce.get_untracked(), true)?;
            setup.subscription.throttle_ms = milliseconds(&throttle.get_untracked(), true)?;
            Ok(setup)
        })();
        match build {
            Ok(setup) => {
                error.set(None);
                review.set(Some(setup));
            }
            Err(message) => error.set(Some(message)),
        }
    };
    view! {
        <div class="space-y-3">
            <Button size=ButtonSize::Small on_click=Callback::new(move |_|open.run(new_setup()))>"New automation"</Button>
            <Show when=move || base.get().is_some() && review.get().is_none()>
                <div class=CARD>
                    <h3 class="text-label text-rambutan-100">"Set up an automation"</h3>
                    <p class="text-body-sm text-rambutan-400">"Describe the outcome and choose what starts it. Nothing is saved or enabled until you review the complete automation."</p>
                    <div class="mt-3 grid gap-3 sm:grid-cols-2"><Field label="Automation name" value=name /><Field label="Model" value=model /><div class="sm:col-span-2"><Field label="What should this automation do?" value=purpose multiline=true /></div><div class="sm:col-span-2"><Field label="Automation working folder" value=folder /></div>
                        <label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Starts when"</span><select aria-label="Automation source type" class=SELECT prop:value=move ||kind.get() on:change=move |event| {let value=event_target_value(&event);if value=="app_event"{source_value.set("meeting.completed".into());}
                        if value=="schedule"{interval.set("3600".into());}kind.set(value);}>
                            {[("folder","A file arrives"),("http","A website changes"),("command","A command reports a change"),("schedule","On a repeating schedule"),("calendar","At set times of day"),("webhook","A webhook arrives"),("app_event","A meeting event occurs"),("custom","A custom event arrives")].into_iter().map(|(value,label)|view!{<option value=value>{label}</option>}).collect_view()}
                        </select></label>
                        {move ||match kind.get().as_str(){
                            "folder"=>view!{<div class="space-y-3 sm:col-span-2"><Field label="Folder to watch" value=source_value /><Field label="File pattern" value=pattern /></div>}.into_any(),
                            "http"=>view!{<div class="sm:col-span-2"><Field label="Website URL" value=source_value /></div>}.into_any(),
                            "command"=>view!{<div class="sm:col-span-2"><Field label="Command to run" value=source_value multiline=true /><p class="mt-1 text-label-sm text-rambutan-400">"This command runs on your machine once you enable the automation."</p></div>}.into_any(),
                            "schedule"=>view!{<Field label="Run every, in seconds" value=interval />}.into_any(),
                            "calendar"=>view!{<div class="space-y-3 sm:col-span-2"><Field label="Times (HH:MM, comma-separated)" value=cal_times /><WeekdayToggles value=cal_days /><Field label="Time zone" value=cal_zone /></div>}.into_any(),
                            "app_event"=>view!{<label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Meeting event"</span><select aria-label="Meeting event" class=SELECT prop:value=move ||source_value.get() on:change=move |event|source_value.set(event_target_value(&event))>{bruhs_models::reactive::APP_EVENT_TOPICS.into_iter().map(|(topic,label)|view!{<option value=topic>{label}</option>}).collect_view()}</select></label>}.into_any(),
                            "webhook"=>view!{<p class="text-body-sm text-rambutan-400 sm:col-span-2">"A private endpoint is created when you enable this automation. Reveal its token in Advanced settings."</p>}.into_any(),
                            _=>view!{<p class="text-body-sm text-rambutan-400 sm:col-span-2">"Device bridges can publish events through the local API. Use a watched folder for FTP uploads."</p>}.into_any(),
                        }}
                    </div>
                    <details class="mt-3 rounded-grape bg-longan-900 p-3"><summary class="cursor-pointer text-label text-rambutan-300">"Advanced options"</summary><div class="mt-3 space-y-3"><Field label="Automation memory" value=memory multiline=true /><Field label="Event-specific instructions" value=note multiline=true /><Check label="Run read-only drafts automatically" value=auto_run />
                        <Show when=move ||kind.get()!="schedule" && matches!(kind.get().as_str(),"folder"|"http"|"command")><Field label="Check interval in seconds" value=interval /></Show>
                        <Show when=move ||kind.get()=="folder"><Field label="File settle time in seconds" value=settle /><Check label="Include subfolders" value=recursive /></Show>
                        <Show when=move ||kind.get()=="command"><Field label="Command working folder, if different" value=command_folder /><Field label="Command timeout in seconds" value=timeout /></Show>
                        <Field label="Debounce in seconds" value=debounce /><Field label="Minimum gap between runs in seconds" value=throttle />
                    </div></details>
                    <div class="mt-3 flex flex-wrap gap-2"><Button size=ButtonSize::Small on_click=Callback::new(prepare)>"Review automation"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|{base.set(None);review.set(None);})>"Cancel setup"</Button></div>
                </div>
            </Show>
            {move ||review.get().map(|setup| {
                let review_rows=review_fields(&setup);
                view!{<article class=CARD><h3 class="text-label text-rambutan-100">"Review automation"</h3><dl class="mt-3 space-y-3">{review_rows.into_iter().map(|(label,value)|view!{<div><dt class="text-label-sm text-rambutan-400">{label}</dt><dd class="mt-1 whitespace-pre-wrap break-words text-body-sm text-rambutan-200">{value}</dd></div>}).collect_view()}</dl><p class="mt-3 text-body-sm text-blueberry-200">{if setup.subscription.auto_run{"Runs read-only drafts automatically. Writes still use workflow review."}else{"Each event waits for review before a read-only draft runs. Writes still use workflow review."}}</p><p class="mt-1 text-label-sm text-rambutan-500">"The local server must be running. This enables the complete automation in one saved change."</p><div class="mt-3 flex flex-wrap gap-2"><Button size=ButtonSize::Small on_click=Callback::new(move |_|{saving.set(true);send.run(apply(ReactiveChange::SaveAutomation{automation:Box::new(setup.clone())}));})>"Enable automation"</Button><Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_|review.set(None))>"Back to edit"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|{base.set(None);review.set(None);})>"Cancel setup"</Button></div></article>}
            })}
            <Show when=move ||snapshot.with(|s|s.as_ref().is_some_and(|s|s.subscriptions.is_empty()))><p class="text-body-sm text-rambutan-400">"No automations yet. Start with a scan folder, a meeting event, or a repeating check."</p></Show>
            {move ||snapshot.with(|s|s.as_ref().map(|s|s.subscriptions.iter().filter_map(|sub|AutomationSetup::from_subscription(s,sub)).collect::<Vec<_>>()).unwrap_or_default()).into_iter().map(|setup|{
                let edit=setup.clone();let agent_id=setup.agent.id.clone();let memory_agent=setup.agent.id.clone();let subscription_id=setup.subscription.id.clone();let active=setup.enabled();let status=automation_status(&setup);
                let latest=snapshot.with(|s|s.as_ref().and_then(|s|s.runs.iter().find(|run|run.subscription_id.as_deref()==Some(subscription_id.as_str()))).cloned());
                let source_error=snapshot.with(|s|s.as_ref().and_then(|s|s.source_statuses.iter().find(|status|status.source_id==setup.source.id)).and_then(|status|status.error.clone()));
                view!{<article class=CARD><div class="flex flex-wrap items-start justify-between gap-2"><div class="min-w-0"><h3 class="break-words text-label text-rambutan-100">{setup.agent.name}</h3><p class="mt-1 whitespace-pre-wrap break-words text-body-sm text-rambutan-300">{setup.agent.instructions}</p><p class="mt-2 break-all text-label-sm text-rambutan-400">{source_summary(&setup.source.kind)}</p><p class="mt-1 text-label-sm text-blueberry-200">{status}</p>{latest.map(|run|view!{<p class="mt-1 text-label-sm text-rambutan-400">{format!("Latest activity: {}",super::run_label(run.status))}</p>})}{source_error.map(|message|view!{<p role="status" class="mt-1 break-words text-body-sm text-lychee-200">{message}</p>})}</div><div class="flex flex-wrap gap-2">{command_button(if active{"Pause automation"}else{"Resume automation"},ReactiveCommand::SetAutomationEnabled{id:subscription_id,enabled:!active},send)}<Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|open.run(edit.clone()))>"Edit automation"</Button><Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_|on_chat.run(agent_id.clone()))>"Chat"</Button></div></div><super::memory::AgentMemoryPanel agent_id=memory_agent snapshot=snapshot send=send /></article>}
            }).collect_view()}
        </div>
    }
}

pub(super) fn source_summary(kind: &SourceKind) -> String {
    match kind {
        SourceKind::Folder { path, pattern, .. } => format!("Folder: {path} ({pattern})"),
        SourceKind::Http { url, .. } => format!("Website: {url}"),
        SourceKind::Command { command, .. } => format!("Command: {command}"),
        SourceKind::Schedule { every_ms } => format!("Every {} seconds", seconds(*every_ms)),
        SourceKind::Calendar {
            times,
            weekdays,
            time_zone,
        } => describe_calendar(times, weekdays, time_zone),
        SourceKind::Webhook => "Authenticated webhook".into(),
        SourceKind::AppEvent { topic } => format!("App event: {topic}"),
        SourceKind::Custom => "Custom events from your integrations".into(),
    }
}
fn automation_status(setup: &AutomationSetup) -> &'static str {
    if !setup.subscription.enabled {
        "Paused"
    } else if !setup.agent.enabled {
        "Paused: agent disabled"
    } else if !setup.source.enabled {
        "Paused: source disabled"
    } else if setup.subscription.auto_run {
        "Enabled: automatic read-only drafts"
    } else {
        "Enabled: review each event"
    }
}
fn source_details(kind: &SourceKind) -> String {
    match kind {
        SourceKind::Folder{path,pattern,recursive,settle_ms,poll_ms}=>format!("Folder: {path}\nFiles: {pattern}\nSubfolders: {}\nFile settle time: {} seconds\nCheck interval: {} seconds",if *recursive{"Included"}else{"Excluded"},seconds(*settle_ms),seconds(*poll_ms)),
        SourceKind::Http{url,poll_ms}=>format!("URL: {url}\nCheck interval: {} seconds",seconds(*poll_ms)),
        SourceKind::Command{command,cwd,poll_ms,timeout_ms}=>format!("Command: {command}\nWorking folder: {cwd}\nCheck interval: {} seconds\nTimeout: {} seconds\nSource command access: Runs with the local server account's permissions. The agent's read-only tool policy does not restrict this command.",seconds(*poll_ms),seconds(*timeout_ms)),
        SourceKind::Schedule{every_ms}=>format!("Repeat every {} seconds",seconds(*every_ms)),
        SourceKind::Calendar{times,weekdays,time_zone}=>describe_calendar(times,weekdays,time_zone),
        SourceKind::Webhook=>"Authenticated incoming webhook. The private token is available after enabling.".into(),
        SourceKind::AppEvent{topic}=>format!("App event: {topic}"),
        SourceKind::Custom=>"Events published by your integrations through the local API.".into(),
    }
}

pub(super) fn review_fields(setup: &AutomationSetup) -> Vec<(&'static str, String)> {
    vec![
        ("Name", setup.agent.name.clone()),
        ("Purpose", setup.agent.instructions.clone()),
        ("Event source name", setup.source.name.clone()),
        (
            "Agent state",
            if setup.agent.enabled {
                "Enabled"
            } else {
                "Paused"
            }
            .into(),
        ),
        (
            "Source state",
            if setup.source.enabled {
                "Enabled"
            } else {
                "Paused"
            }
            .into(),
        ),
        (
            "Automation state",
            if setup.subscription.enabled {
                "Enabled"
            } else {
                "Paused"
            }
            .into(),
        ),
        (
            "Event policy",
            if setup.subscription.auto_run {
                "Automatic read-only drafts"
            } else {
                "Review each event before running a read-only draft"
            }
            .into(),
        ),
        ("Starts when", source_summary(&setup.source.kind)),
        (
            "Complete source settings",
            source_details(&setup.source.kind),
        ),
        ("Working folder", setup.agent.project_path.clone()),
        ("Model", setup.agent.model.clone()),
        (
            if matches!(setup.source.kind, SourceKind::Command { .. }) {
                "Agent access"
            } else {
                "Access"
            },
            "Read-only drafts. No direct writes or payments.".into(),
        ),
        (
            "Persistent memory",
            if setup.agent.memory.is_empty() {
                "None".into()
            } else {
                setup.agent.memory.clone()
            },
        ),
        (
            "Event instructions",
            setup.subscription.instructions.clone(),
        ),
        (
            "Debounce",
            format!("{} seconds", seconds(setup.subscription.debounce_ms)),
        ),
        (
            "Minimum gap",
            format!("{} seconds", seconds(setup.subscription.throttle_ms)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unified_setup_has_one_stable_identity_and_an_explicit_automatic_draft_policy() {
        let setup = new_setup();
        assert_eq!(setup.subscription.agent_id, setup.agent.id);
        assert_eq!(setup.subscription.source_id, setup.source.id);
        assert!(setup.enabled());
        assert!(setup.subscription.auto_run);
        assert!(review_fields(&setup)
            .iter()
            .any(|(label, value)| *label == "Access" && value.contains("Read-only")));
    }
    #[test]
    fn automation_cards_preserve_manual_policy_and_explain_disabled_dependencies() {
        let mut setup = new_setup();
        setup.subscription.auto_run = false;
        assert_eq!(automation_status(&setup), "Enabled: review each event");
        setup.agent.enabled = false;
        assert_eq!(automation_status(&setup), "Paused: agent disabled");
        setup.agent.enabled = true;
        setup.source.enabled = false;
        assert_eq!(automation_status(&setup), "Paused: source disabled");
        setup.source.enabled = true;
        setup.subscription.enabled = false;
        assert_eq!(automation_status(&setup), "Paused");
        assert!(!setup.subscription.auto_run);
        let command = ReactiveCommand::SetAutomationEnabled {
            id: setup.subscription.id.clone(),
            enabled: true,
        };
        let wire = serde_json::to_value(command).unwrap();
        assert_eq!(wire["action"], "set_automation_enabled");
        assert_eq!(wire["id"], setup.subscription.id);
        assert!(wire.get("auto_run").is_none());
    }
    #[test]
    fn enabling_a_reviewed_setup_serializes_one_complete_change() {
        let setup = new_setup();
        let command = apply(ReactiveChange::SaveAutomation {
            automation: Box::new(setup.clone()),
        });
        let wire = serde_json::to_value(command).unwrap();
        assert_eq!(wire["action"], "apply");
        assert_eq!(wire["change"]["type"], "save_automation");
        let bundled = &wire["change"]["automation"];
        assert_eq!(bundled["subscription"]["agent_id"], setup.agent.id);
        assert_eq!(bundled["subscription"]["source_id"], setup.source.id);
        assert_eq!(bundled["subscription"]["auto_run"], true);
        assert_eq!(bundled["agent"]["enabled"], true);
        assert_eq!(bundled["source"]["enabled"], true);
        assert_eq!(bundled["subscription"]["enabled"], true);
    }
}
