//! A reviewed automation saves its agent, source, and subscription as one command.
use super::app_poll::{AppPollFields, NewerSourceConfirm};
use super::calendar::{calendar_source, local_time_zone, WeekdayToggles};
use super::{
    apply, command_button, id, milliseconds, seconds, source_kind, Check, Field, CARD, SELECT,
};
use crate::components::crew::face::BruhFace;
use crate::ui::{Button, ButtonSize, ButtonVariant};
use bruhs_models::apps::{
    check_poll_account, describe_app_poll, new_poll_form, open_apps_from_own_address,
    poll_account_label, poll_form_for, poll_form_kind, AppPollForm, AppsListing, FROM_NEWER_SOURCE,
    SOURCE_KIND_OTHER, STARTS_WHEN_APP,
};
use bruhs_models::crew::{
    access_summary, agent_run_level, apps_ready, apps_reason, event_policy, event_policy_line,
    AccessLevel, BruhLook, BruhStatus, ServiceNotice, AUTO_RUN_LABEL, COMMAND_SOURCE_ACCESS,
    RAIL_FACE_PX,
};
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

/// A new automation; for one agent's routines, it starts from that agent.
fn new_setup_for(
    snapshot: Option<&ReactiveSnapshot>,
    agent_filter: Option<&str>,
) -> AutomationSetup {
    let mut setup = new_setup();
    if let Some(agent) = agent_filter.and_then(|id| {
        snapshot.and_then(|snapshot| snapshot.agents.iter().find(|agent| agent.id == id))
    }) {
        setup.agent = agent.clone();
        setup.subscription.agent_id = agent.id.clone();
    }
    setup
}

/// What the setup form says about the agent.
struct AgentFields {
    name: String,
    purpose: String,
    folder: String,
    model: String,
    memory: String,
}

/// The setup with the form's agent fields applied. A routine made on a bruh's
/// page runs as that bruh: its agent stays exactly as saved, so a routine
/// never renames it, rewrites its job, moves its folder, switches its model
/// past the access check or turns it on.
fn with_agent_fields(
    mut setup: AutomationSetup,
    fields: AgentFields,
    agent_locked: bool,
) -> Result<AutomationSetup, String> {
    if agent_locked {
        return Ok(setup);
    }
    if fields.name.trim().is_empty()
        || fields.purpose.trim().is_empty()
        || fields.folder.trim().is_empty()
    {
        return Err(
            "Enter an automation name, purpose, and working folder before reviewing.".into(),
        );
    }
    bruhs_models::agent::ModelId::try_from(fields.model.clone())
        .map_err(|error| error.to_string())?;
    setup.agent.name = fields.name;
    setup.agent.instructions = fields.purpose;
    setup.agent.project_path = fields.folder;
    setup.agent.model = fields.model;
    setup.agent.memory = fields.memory;
    setup.agent.enabled = true;
    Ok(setup)
}

/// The automations to list: every one, or only one agent's.
fn shown_setups(snapshot: &ReactiveSnapshot, agent_filter: Option<&str>) -> Vec<AutomationSetup> {
    snapshot
        .subscriptions
        .iter()
        .filter(|sub| {
            agent_filter.is_none_or(|id| sub.agent_id == id)
                && bruhs_models::weekly_reports::managed_preset_id(&sub.id).is_none()
        })
        .filter_map(|sub| AutomationSetup::from_subscription(snapshot, sub))
        .collect()
}

/// The "Starts when" choices: the App source only on a service that keeps
/// one, or for a routine that already has one (shown off with the reason),
/// and a newer Bruhs' source only for a routine that has one.
fn starts_when_options(apps: bool, current: &str) -> Vec<(&'static str, &'static str)> {
    let mut options = vec![
        ("folder", "A file arrives"),
        ("http", "A website changes"),
        ("command", "A command reports a change"),
        ("schedule", "On a repeating schedule"),
        ("calendar", "At set times of day"),
        ("webhook", "A webhook arrives"),
        ("app_event", "A meeting event occurs"),
        ("custom", "A custom event arrives"),
    ];
    if apps || current == "app_poll" {
        options.push(("app_poll", STARTS_WHEN_APP));
    }
    if current == "other" {
        options.push(("other", SOURCE_KIND_OTHER));
    }
    options
}

/// The source an App or newer-Bruhs type reviews as; `None` for every
/// other type, which the editor builds itself. An App source is refused on
/// an older service (`blocked`), with a malformed form, or with an account
/// the loaded listing no longer offers. A newer Bruhs' source is never
/// saved (ruling F3).
fn fixed_kind_source(
    kind: &str,
    form: &AppPollForm,
    apps: Option<&AppsListing>,
    blocked: Option<&'static str>,
) -> Option<Result<SourceKind, String>> {
    match kind {
        "app_poll" => Some((|| {
            if let Some(reason) = blocked {
                return Err(reason.to_owned());
            }
            let source = poll_form_kind(form)?;
            check_poll_account(form, apps)?;
            Ok(source)
        })()),
        "other" => Some(Err(FROM_NEWER_SOURCE.into())),
        _ => None,
    }
}

/// The account line an App routine names: the mailbox a new mail poll
/// reads, when the listing has it.
fn poll_label(kind: &SourceKind, apps: Option<&AppsListing>) -> Option<String> {
    apps.and_then(|apps| poll_account_label(kind, apps))
}

#[component]
pub(crate) fn AutomationSetupPanel(
    snapshot: RwSignal<Option<ReactiveSnapshot>>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    send: Callback<ReactiveCommand>,
    on_chat: Callback<String>,
    /// Show only this agent's automations, and start new ones from it.
    #[prop(optional)]
    agent_filter: Option<String>,
) -> impl IntoView {
    let agent_filter = StoredValue::new(agent_filter);
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
    let poll_form = RwSignal::new(new_poll_form());
    // The owner's app accounts and mailboxes, for App sources. Never stored.
    let apps = RwSignal::new(None::<AppsListing>);
    let apps_notice = RwSignal::new(None::<String>);
    // The type picked for a newer Bruhs' source, waiting on Change type.
    let change_kind = RwSignal::new(None::<String>);
    // Bumped by each load, so an older answer never replaces a newer one.
    let apps_generation = StoredValue::new(0u64);
    let load_apps = move || {
        apps_generation.update_value(|generation| *generation = generation.wrapping_add(1));
        let requested = apps_generation.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let (listing, notice) = match crate::api::apps_listing().await {
                Ok(listing) => (Some(listing), None),
                Err(error) if error.is_status(404) => {
                    (None, Some(ServiceNotice::ServingOld.message().to_owned()))
                }
                Err(error) if error.is_status(403) => {
                    (None, Some(open_apps_from_own_address(crate::api::BASE_URL)))
                }
                Err(error) => (None, Some(crate::api::vault_message(&error))),
            };
            if apps_generation.try_get_value() != Some(requested) {
                return;
            }
            apps.try_set(listing);
            apps_notice.try_set(notice);
        });
    };
    load_apps();
    let protocol = move || snapshot.with(|s| s.as_ref().map(|s| s.protocol));
    let apps_on = move || protocol().is_some_and(apps_ready);
    // Why an App source can't be saved here: an older service.
    let app_block = Signal::derive(move || {
        (!apps_on()).then(|| apps_reason(protocol()).unwrap_or(ServiceNotice::ServingOld.message()))
    });
    let open = Callback::new(move |setup: AutomationSetup| {
        poll_form.set(poll_form_for(&setup.source.kind).unwrap_or_else(new_poll_form));
        change_kind.set(None);
        load_apps();
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
            SourceKind::AppPoll { .. } => kind.set("app_poll".into()),
            SourceKind::Other => kind.set("other".into()),
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
    let set_kind = move |value: String| {
        if value == "app_event" {
            source_value.set("meeting.completed".into());
        }
        if value == "schedule" {
            interval.set("3600".into());
        }
        kind.set(value);
    };
    let prepare = move |_| {
        let Some(mut setup) = base.get_untracked() else {
            return;
        };
        let build = (|| -> Result<AutomationSetup, String> {
            let fields = AgentFields {
                name: name.get_untracked(),
                purpose: purpose.get_untracked(),
                folder: folder.get_untracked(),
                model: model.get_untracked(),
                memory: memory.get_untracked(),
            };
            setup = with_agent_fields(setup, fields, agent_filter.with_value(Option::is_some))?;
            if setup.source.name.trim().is_empty() {
                setup.source.name = format!("{} source", setup.agent.name);
            }
            let cwd = if command_folder.get_untracked().trim().is_empty() {
                setup.agent.project_path.clone()
            } else {
                command_folder.get_untracked()
            };
            setup.source.kind = if kind.get_untracked() == "calendar" {
                calendar_source(
                    &cal_times.get_untracked(),
                    &cal_days.get_untracked(),
                    &cal_zone.get_untracked(),
                )?
            } else if let Some(source) = apps.with_untracked(|apps| {
                fixed_kind_source(
                    &kind.get_untracked(),
                    &poll_form.get_untracked(),
                    apps.as_ref(),
                    app_block.get_untracked(),
                )
            }) {
                source?
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
            <Button size=ButtonSize::Small on_click=Callback::new(move |_|open.run(snapshot.with_untracked(|s|agent_filter.with_value(|filter|new_setup_for(s.as_ref(),filter.as_deref())))))>"New automation"</Button>
            <Show when=move || base.get().is_some() && review.get().is_none()>
                <div class=CARD>
                    <h3 class="text-label text-rambutan-100">"Set up an automation"</h3>
                    <p class="text-body-sm text-rambutan-400">"Describe the outcome and choose what starts it. Nothing is saved or enabled until you review the complete automation."</p>
                    <Show when=move ||agent_filter.with_value(Option::is_some)><p class="mt-2 text-label-sm text-rambutan-400">"This routine runs as this bruh. Change its job and model in Settings."</p></Show>
                    <div class="mt-3 grid gap-3 sm:grid-cols-2"><Show when=move ||agent_filter.with_value(Option::is_none)><Field label="Automation name" value=name /><Field label="Model" value=model /><div class="sm:col-span-2"><Field label="What should this automation do?" value=purpose multiline=true /></div><div class="sm:col-span-2"><Field label="Automation working folder" value=folder /></div></Show>
                        <label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Starts when"</span><select aria-label="Automation source type" class=SELECT prop:value=move ||kind.get() on:change=move |event| {let value=event_target_value(&event);
                        // Changing a newer Bruhs' source replaces it: ask first.
                        if kind.get_untracked()=="other" && value!="other"{change_kind.set(Some(value));kind.set("other".into());return;}
                        set_kind(value);}>
                            {move ||starts_when_options(apps_on(),&kind.get()).into_iter().map(|(value,label)|view!{<option value=value selected=move ||kind.get()==value>{label}</option>}).collect_view()}
                        </select></label>
                        <NewerSourceConfirm pending=change_kind on_confirm=Callback::new(set_kind) />
                        {move ||match kind.get().as_str(){
                            "folder"=>view!{<div class="space-y-3 sm:col-span-2"><Field label="Folder to watch" value=source_value /><Field label="File pattern" value=pattern /></div>}.into_any(),
                            "http"=>view!{<div class="sm:col-span-2"><Field label="Website URL" value=source_value /></div>}.into_any(),
                            "command"=>view!{<div class="sm:col-span-2"><Field label="Command to run" value=source_value multiline=true /><p class="mt-1 text-label-sm text-rambutan-400">"This command runs on your machine once you enable the automation."</p></div>}.into_any(),
                            "schedule"=>view!{<Field label="Run every, in seconds" value=interval />}.into_any(),
                            "calendar"=>view!{<div class="space-y-3 sm:col-span-2"><Field label="Times (HH:MM, comma-separated)" value=cal_times /><WeekdayToggles value=cal_days /><Field label="Time zone" value=cal_zone /></div>}.into_any(),
                            "app_event"=>view!{<label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Meeting event"</span><select aria-label="Meeting event" class=SELECT prop:value=move ||source_value.get() on:change=move |event|source_value.set(event_target_value(&event))>{bruhs_models::reactive::APP_EVENT_TOPICS.into_iter().map(|(topic,label)|view!{<option value=topic>{label}</option>}).collect_view()}</select></label>}.into_any(),
                            "webhook"=>view!{<p class="text-body-sm text-rambutan-400 sm:col-span-2">"A private endpoint is created when you enable this automation. Reveal its token in Advanced settings."</p>}.into_any(),
                            "app_poll"=>view!{<AppPollFields form=poll_form listing=Signal::derive(move ||apps.get()) notice=Signal::derive(move ||apps_notice.get()) disabled=app_block />}.into_any(),
                            "other"=>view!{<p class="text-body-sm text-rambutan-300 sm:col-span-2">{FROM_NEWER_SOURCE}</p>}.into_any(),
                            _=>view!{<p class="text-body-sm text-rambutan-400 sm:col-span-2">"Device bridges can publish events through the local API. Use a watched folder for FTP uploads."</p>}.into_any(),
                        }}
                    </div>
                    <details class="mt-3 rounded-grape bg-longan-900 p-3"><summary class="cursor-pointer text-label text-rambutan-300">"Advanced options"</summary><div class="mt-3 space-y-3"><Show when=move ||agent_filter.with_value(Option::is_none)><Field label="Automation memory" value=memory multiline=true /></Show><Field label="Event-specific instructions" value=note multiline=true /><Check label=AUTO_RUN_LABEL value=auto_run />
                        <Show when=move ||kind.get()!="schedule" && matches!(kind.get().as_str(),"folder"|"http"|"command")><Field label="Check interval in seconds" value=interval /></Show>
                        <Show when=move ||kind.get()=="folder"><Field label="File settle time in seconds" value=settle /><Check label="Include subfolders" value=recursive /></Show>
                        <Show when=move ||kind.get()=="command"><Field label="Command working folder, if different" value=command_folder /><Field label="Command timeout in seconds" value=timeout /></Show>
                        <Field label="Debounce in seconds" value=debounce /><Field label="Minimum gap between runs in seconds" value=throttle />
                    </div></details>
                    <div class="mt-3 flex flex-wrap gap-2"><Button size=ButtonSize::Small on_click=Callback::new(prepare)>"Review automation"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|{base.set(None);review.set(None);})>"Cancel setup"</Button></div>
                </div>
            </Show>
            {move ||review.get().map(|setup| {
                let level=snapshot.with_untracked(|s|agent_run_level(s.as_ref(),&setup.agent.id,&setup.agent.model));
                let account=apps.with_untracked(|apps|poll_label(&setup.source.kind,apps.as_ref()));
                let review_rows=review_fields_with(&setup,level,account.as_deref());
                view!{<article class=CARD><h3 class="text-label text-rambutan-100">"Review automation"</h3><dl class="mt-3 space-y-3">{review_rows.into_iter().map(|(label,value)|view!{<div><dt class="text-label-sm text-rambutan-400">{label}</dt><dd class="mt-1 whitespace-pre-wrap break-words text-body-sm text-rambutan-200">{value}</dd></div>}).collect_view()}</dl><p class="mt-3 text-body-sm text-blueberry-200">{event_policy_line(setup.subscription.auto_run,level)}</p><p class="mt-1 text-label-sm text-rambutan-500">"The local server must be running. This enables the complete automation in one saved change."</p><div class="mt-3 flex flex-wrap gap-2"><Button size=ButtonSize::Small on_click=Callback::new(move |_|{saving.set(true);send.run(apply(ReactiveChange::SaveAutomation{automation:Box::new(setup.clone())}));})>"Enable automation"</Button><Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_|review.set(None))>"Back to edit"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|{base.set(None);review.set(None);})>"Cancel setup"</Button></div></article>}
            })}
            <Show when=move ||snapshot.with(|s|s.as_ref().is_some_and(|s|agent_filter.with_value(|filter|shown_setups(s,filter.as_deref()).is_empty())))><p class="text-body-sm text-rambutan-400">"No automations yet. Start with a scan folder, a meeting event, or a repeating check."</p></Show>
            {move ||snapshot.with(|s|s.as_ref().map(|s|agent_filter.with_value(|filter|shown_setups(s,filter.as_deref()))).unwrap_or_default()).into_iter().map(|setup|{
                // One agent's routines sit beside its own memory tab.
                let with_memory=agent_filter.with_value(Option::is_none);
                let edit=setup.clone();let agent_id=setup.agent.id.clone();let memory_agent=setup.agent.id.clone();let subscription_id=setup.subscription.id.clone();let active=setup.enabled();let status=automation_status(&setup,snapshot.with(|s|agent_run_level(s.as_ref(),&setup.agent.id,&setup.agent.model)));
                let latest=snapshot.with(|s|s.as_ref().and_then(|s|s.runs.iter().find(|run|run.subscription_id.as_deref()==Some(subscription_id.as_str()))).cloned());
                let source_error=snapshot.with(|s|s.as_ref().and_then(|s|s.source_statuses.iter().find(|status|status.source_id==setup.source.id)).and_then(|status|status.error.clone()));
                // The agent shows its face, as its bruh, in its live state.
                let (look,face_status)=snapshot.with(|s|s.as_ref().and_then(|s|s.bruhs.iter().find(|bruh|bruh.agent_id==setup.agent.id)).map_or_else(||(BruhLook::default(),BruhStatus::Asleep),|bruh|(bruh.look.clone(),bruh.status)));
                view!{<article class=CARD><div class="flex flex-wrap items-start justify-between gap-2"><div class="min-w-0"><div class="flex min-w-0 items-center gap-2"><BruhFace size=f64::from(RAIL_FACE_PX) look=look status=face_status /><h3 class="min-w-0 break-words text-label text-rambutan-100">{setup.agent.name}</h3></div><p class="mt-1 whitespace-pre-wrap break-words text-body-sm text-rambutan-300">{setup.agent.instructions}</p><p class="mt-2 break-all text-label-sm text-rambutan-400">{source_summary(&setup.source.kind,apps.with(|apps|poll_label(&setup.source.kind,apps.as_ref())).as_deref())}</p><p class="mt-1 text-label-sm text-blueberry-200">{status}</p>{latest.map(|run|view!{<p class="mt-1 text-label-sm text-rambutan-400">{format!("Latest activity: {}",super::run_label(run.status))}</p>})}{source_error.map(|message|view!{<p role="status" class="mt-1 break-words text-body-sm text-lychee-200">{message}</p>})}</div><div class="flex flex-wrap gap-2">{command_button(if active{"Pause automation"}else{"Resume automation"},ReactiveCommand::SetAutomationEnabled{id:subscription_id,enabled:!active},send)}<Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|open.run(edit.clone()))>"Edit automation"</Button><Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_|on_chat.run(agent_id.clone()))>"Chat"</Button></div></div>{with_memory.then(||view!{<super::memory::AgentMemoryPanel agent_id=memory_agent snapshot=snapshot send=send />})}</article>}
            }).collect_view()}
        </div>
    }
}

/// `account` names a new mail poll's mailbox.
pub(super) fn source_summary(kind: &SourceKind, account: Option<&str>) -> String {
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
        SourceKind::AppPoll { query, .. } => describe_app_poll(query, account),
        SourceKind::Other => FROM_NEWER_SOURCE.into(),
    }
}
/// `level` is the level the automation's agent runs at.
fn automation_status(setup: &AutomationSetup, level: AccessLevel) -> String {
    if !setup.subscription.enabled {
        "Paused".into()
    } else if !setup.agent.enabled {
        "Paused: agent disabled".into()
    } else if !setup.source.enabled {
        "Paused: source disabled".into()
    } else if setup.subscription.auto_run {
        format!("Enabled: {}", event_policy(true, level))
    } else {
        "Enabled: review each event".into()
    }
}
fn source_details(kind: &SourceKind, account: Option<&str>) -> String {
    match kind {
        SourceKind::Folder{path,pattern,recursive,settle_ms,poll_ms}=>format!("Folder: {path}\nFiles: {pattern}\nSubfolders: {}\nFile settle time: {} seconds\nCheck interval: {} seconds",if *recursive{"Included"}else{"Excluded"},seconds(*settle_ms),seconds(*poll_ms)),
        SourceKind::Http{url,poll_ms}=>format!("URL: {url}\nCheck interval: {} seconds",seconds(*poll_ms)),
        SourceKind::Command{command,cwd,poll_ms,timeout_ms}=>format!("Command: {command}\nWorking folder: {cwd}\nCheck interval: {} seconds\nTimeout: {} seconds\n{COMMAND_SOURCE_ACCESS}",seconds(*poll_ms),seconds(*timeout_ms)),
        SourceKind::Schedule{every_ms}=>format!("Repeat every {} seconds",seconds(*every_ms)),
        SourceKind::Calendar{times,weekdays,time_zone}=>describe_calendar(times,weekdays,time_zone),
        SourceKind::Webhook=>"Authenticated incoming webhook. The private token is available after enabling.".into(),
        SourceKind::AppEvent{topic}=>format!("App event: {topic}"),
        SourceKind::Custom=>"Events published by your integrations through the local API.".into(),
        SourceKind::AppPoll{query,poll_ms,..}=>format!("{}\nCheck interval: {} seconds",describe_app_poll(query,account),seconds(*poll_ms)),
        SourceKind::Other=>FROM_NEWER_SOURCE.into(),
    }
}

/// `level` is the level the automation's agent runs at.
pub(super) fn review_fields(
    setup: &AutomationSetup,
    level: AccessLevel,
) -> Vec<(&'static str, String)> {
    review_fields_with(setup, level, None)
}

/// [`review_fields`], with `account` naming a new mail poll's mailbox.
fn review_fields_with(
    setup: &AutomationSetup,
    level: AccessLevel,
    account: Option<&str>,
) -> Vec<(&'static str, String)> {
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
            event_policy_line(setup.subscription.auto_run, level),
        ),
        ("Starts when", source_summary(&setup.source.kind, account)),
        (
            "Complete source settings",
            source_details(&setup.source.kind, account),
        ),
        ("Working folder", setup.agent.project_path.clone()),
        ("Model", setup.agent.model.clone()),
        (
            if matches!(setup.source.kind, SourceKind::Command { .. }) {
                "Agent access"
            } else {
                "Access"
            },
            access_summary(level).into(),
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
        assert!(review_fields(&setup, AccessLevel::ReadOnly)
            .iter()
            .any(|(label, value)| *label == "Access" && value.starts_with("Read only.")));
        // A Full agent's automation says so instead of promising drafts.
        let full = review_fields(&setup, AccessLevel::Full);
        assert!(full
            .iter()
            .any(|(label, value)| *label == "Access" && value.starts_with("Full access.")));
        assert!(full.iter().any(|(label, value)| *label == "Event policy"
            && value == "Runs each event automatically at Full access."));
        assert!(!full.iter().any(|(_, value)| value.contains("ead-only")));
        assert_eq!(
            automation_status(&setup, AccessLevel::Full),
            "Enabled: Automatic runs at Full access"
        );
    }
    #[test]
    fn automation_cards_preserve_manual_policy_and_explain_disabled_dependencies() {
        let mut setup = new_setup();
        setup.subscription.auto_run = false;
        assert_eq!(
            automation_status(&setup, AccessLevel::ReadOnly),
            "Enabled: review each event"
        );
        setup.agent.enabled = false;
        assert_eq!(
            automation_status(&setup, AccessLevel::ReadOnly),
            "Paused: agent disabled"
        );
        setup.agent.enabled = true;
        setup.source.enabled = false;
        assert_eq!(
            automation_status(&setup, AccessLevel::ReadOnly),
            "Paused: source disabled"
        );
        setup.source.enabled = true;
        setup.subscription.enabled = false;
        assert_eq!(automation_status(&setup, AccessLevel::ReadOnly), "Paused");
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
    fn one_agents_routines_list_only_its_automations_and_start_from_it() {
        let mine = new_setup();
        let other = new_setup();
        let snapshot = ReactiveSnapshot {
            agents: vec![mine.agent.clone(), other.agent.clone()],
            sources: vec![mine.source.clone(), other.source.clone()],
            subscriptions: vec![mine.subscription.clone(), other.subscription],
            ..ReactiveSnapshot::default()
        };
        assert_eq!(shown_setups(&snapshot, None).len(), 2);
        let shown = shown_setups(&snapshot, Some(&mine.agent.id));
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].agent.id, mine.agent.id);

        let fresh = new_setup_for(Some(&snapshot), Some(&mine.agent.id));
        assert_eq!(fresh.agent, mine.agent);
        assert_eq!(fresh.subscription.agent_id, mine.agent.id);
        assert_ne!(fresh.subscription.id, mine.subscription.id);
        assert_ne!(new_setup_for(Some(&snapshot), None).agent.id, mine.agent.id);
    }

    #[test]
    fn a_routine_saved_from_a_bruh_page_sends_the_bruh_unchanged() {
        let mut bruh = new_setup().agent;
        bruh.enabled = false;
        bruh.model = "gpt-5.1-codex".into();
        bruh.project_path = "/home/bruhs/b1/work".into();
        let snapshot = ReactiveSnapshot {
            agents: vec![bruh.clone()],
            ..ReactiveSnapshot::default()
        };
        let edited = || AgentFields {
            name: "Renamed".into(),
            purpose: "A new job".into(),
            folder: "/elsewhere".into(),
            model: "claude-sonnet-4-6".into(),
            memory: "new memory".into(),
        };
        let setup = new_setup_for(Some(&snapshot), Some(&bruh.id));
        let saved = with_agent_fields(setup, edited(), true).unwrap();
        let command = apply(ReactiveChange::SaveAutomation {
            automation: Box::new(saved),
        });
        let ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation { automation },
        } = command
        else {
            panic!("expected a saved automation");
        };
        assert_eq!(automation.agent, bruh);
        assert_eq!(automation.subscription.agent_id, bruh.id);

        // The Automations page still edits the agent and enables it.
        let free = with_agent_fields(new_setup(), edited(), false).unwrap();
        assert_eq!(
            (
                free.agent.name.as_str(),
                free.agent.project_path.as_str(),
                free.agent.enabled
            ),
            ("Renamed", "/elsewhere", true)
        );
        let blank = AgentFields {
            name: String::new(),
            ..edited()
        };
        assert!(with_agent_fields(new_setup(), blank, false).is_err());
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
    /// Review refuses a newer Bruhs' source, an App source on an older
    /// service, a malformed one and one whose account is gone; it builds a
    /// good one and leaves every other type to the editor.
    #[test]
    fn review_refuses_newer_sources_and_unusable_app_sources() {
        use bruhs_models::apps::{
            AppAccount, AppQuery, PollWatch, APP_POLL_ACCOUNT_GONE, REPO_SHAPE,
        };
        use bruhs_models::secrets::SavedCredentialState;
        use bruhs_models::vault::CredentialScope;
        let form = AppPollForm {
            watch: PollWatch::GithubIssues,
            account: "v1-g".into(),
            repo: "o/r".into(),
            minutes: "5".into(),
        };
        let listing = AppsListing {
            accounts: vec![AppAccount {
                id: "v1-g".into(),
                app: "github".into(),
                name: "GitHub octo".into(),
                account: "octo".into(),
                scope: CredentialScope::Global,
                state: SavedCredentialState::Saved,
                attention: None,
            }],
            ..AppsListing::default()
        };
        assert_eq!(
            fixed_kind_source("other", &form, Some(&listing), None),
            Some(Err(FROM_NEWER_SOURCE.to_owned()))
        );
        assert_eq!(
            fixed_kind_source("app_poll", &form, Some(&listing), Some("older")),
            Some(Err("older".to_owned()))
        );
        let bad = AppPollForm {
            repo: "o".into(),
            ..form.clone()
        };
        assert_eq!(
            fixed_kind_source("app_poll", &bad, Some(&listing), None),
            Some(Err(REPO_SHAPE.to_owned()))
        );
        assert_eq!(
            fixed_kind_source("app_poll", &form, Some(&AppsListing::default()), None),
            Some(Err(APP_POLL_ACCOUNT_GONE.to_owned()))
        );
        assert_eq!(
            fixed_kind_source("app_poll", &form, Some(&listing), None),
            Some(Ok(SourceKind::AppPoll {
                app: "github".into(),
                account: "v1-g".into(),
                query: AppQuery::GithubIssues { repo: "o/r".into() },
                poll_ms: 300_000,
            }))
        );
        assert_eq!(fixed_kind_source("folder", &form, None, None), None);
    }

    /// D10: the App source is offered only where the service keeps one; a
    /// routine that has one still shows it, as does a newer Bruhs' source.
    #[test]
    fn starts_when_offers_the_app_source_only_when_the_service_keeps_one() {
        let values = |apps, current| {
            starts_when_options(apps, current)
                .into_iter()
                .map(|(value, _)| value)
                .collect::<Vec<_>>()
        };
        assert!(!values(false, "folder").contains(&"app_poll"));
        assert!(values(true, "folder").contains(&"app_poll"));
        assert!(values(false, "app_poll").contains(&"app_poll"));
        assert!(!values(true, "folder").contains(&"other"));
        assert_eq!(values(true, "other").last(), Some(&"other"));
        assert!(starts_when_options(true, "folder").contains(&("app_poll", STARTS_WHEN_APP)));
    }
    #[test]
    fn app_and_newer_sources_describe_themselves() {
        let app = SourceKind::AppPoll {
            app: "mail".into(),
            account: "m".into(),
            query: bruhs_models::apps::AppQuery::NewMail,
            poll_ms: 300_000,
        };
        assert_eq!(source_summary(&app, None), "Email: new mail in your mail");
        assert_eq!(
            source_summary(&app, Some("me@x.test")),
            "Email: new mail in me@x.test"
        );
        assert_eq!(
            source_details(&app, None),
            "Email: new mail in your mail\nCheck interval: 300 seconds"
        );
        assert_eq!(source_summary(&SourceKind::Other, None), FROM_NEWER_SOURCE);
        assert_eq!(source_details(&SourceKind::Other, None), FROM_NEWER_SOURCE);
    }
}
