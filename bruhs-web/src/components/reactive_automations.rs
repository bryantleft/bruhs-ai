//! Event agents live on the server, independently of the open browser and local workflows.
use bruhs_models::crew::{
    auto_run_note, event_policy, event_policy_line, run_reviewed_label, saved_agent_level,
    AccessLevel, AUTO_RUN_LABEL,
};
use bruhs_models::reactive::*;
use leptos::prelude::*;

use crate::ui::{button_class, Button, ButtonSize, ButtonVariant, Input, Textarea};
mod calendar;
mod memory;
mod notifications;
mod runs;
mod setup;
mod updates;
mod weekly_reports;
pub(crate) use memory::AgentMemoryPanel;
use runs::run_label;
pub(crate) use setup::AutomationSetupPanel;

const SELECT: &str = "bruhs-control min-h-[32px] w-full rounded-grape bg-longan-950 px-3 py-1.5 text-body text-rambutan-100 inset-ring inset-ring-longan-700 focus-visible:outline-2 focus-visible:outline-persimmon-400";
const CARD: &str = "min-w-0 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50";
// Status refresh follows the existing account connection status cadence.
#[cfg(target_arch = "wasm32")]
const REFRESH_MS: u32 = 2_000;

/// A local date and time for an instant, as the automation cards write it.
pub(crate) fn local_time(time: bruhs_models::EpochMs) -> String {
    calendar::local_time(time)
}

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn apply(change: ReactiveChange) -> ReactiveCommand {
    ReactiveCommand::Apply { change }
}

fn source_label(kind: &SourceKind) -> &'static str {
    match kind {
        SourceKind::Folder { .. } => "Folder",
        SourceKind::Http { .. } => "Website",
        SourceKind::Command { .. } => "Command",
        SourceKind::Schedule { .. } => "Schedule",
        SourceKind::Calendar { .. } => "Calendar",
        SourceKind::Webhook => "Webhook",
        SourceKind::AppEvent { .. } => "App event",
        SourceKind::Custom => "Custom event",
    }
}

#[component]
fn Field(
    label: &'static str,
    value: RwSignal<String>,
    #[prop(optional)] multiline: bool,
) -> impl IntoView {
    view! { <label class="block min-w-0 space-y-1 text-label-sm text-rambutan-300">
        <span>{label}</span>
        {if multiline {
            view! { <Textarea aria_label=label value=Signal::derive(move || value.get()) on_input=Callback::new(move |text| value.set(text)) rows=4 /> }.into_any()
        } else {
            view! { <Input aria_label=label value=Signal::derive(move || value.get()) on_input=Callback::new(move |text| value.set(text)) /> }.into_any()
        }}
    </label> }
}

#[component]
fn Check(label: &'static str, value: RwSignal<bool>) -> impl IntoView {
    view! { <label class="bruhs-control flex min-h-control cursor-pointer items-center gap-2 text-label-sm text-rambutan-300">
        <input type="checkbox" class="size-4 accent-persimmon-500" prop:checked=move || value.get() on:change=move |event| value.set(event_target_checked(&event)) />
        <span>{label}</span>
    </label> }
}

/// Focus `target` once the view has drawn it.
fn focus_after_render(target: NodeRef<leptos::html::Button>) {
    request_animation_frame(move || {
        if let Some(Some(button)) = target.try_get_untracked() {
            let _ = button.focus();
        }
    });
}

fn command_button(
    label: impl Into<String>,
    command: ReactiveCommand,
    send: Callback<ReactiveCommand>,
) -> impl IntoView {
    let label = label.into();
    view! { <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| send.run(command.clone()))>{label}</Button> }
}

/// A run's draft to preview as a reviewed workflow: title, operations, run id.
pub type DraftPreview = (String, Vec<bruhs_models::WorkflowOperation>, String);

/// The options of the subscription editor's busy policy.
const BUSY_CHOICES: [(&str, &str); 2] = [
    ("queue", "Wait, then handle new events together"),
    ("restart", "Restart with the new event"),
];

fn busy_policy(value: &str) -> BusyPolicy {
    if value == "restart" {
        BusyPolicy::Restart
    } else {
        BusyPolicy::Queue
    }
}

#[component]
pub fn ReactiveAutomations(on_preview: Callback<DraftPreview>) -> impl IntoView {
    let snapshot = RwSignal::new(None::<ReactiveSnapshot>);
    provide_context(memory::MemoryEditor::new());
    // The unread badge follows every snapshot this view loads, not only the stream.
    if let Some(updates) = use_context::<crate::agent_updates::AgentUpdates>() {
        Effect::new(move |_| {
            if let Some(unread) = snapshot.with(|s| s.as_ref().map(ReactiveSnapshot::unread_count))
            {
                updates.unread.set(unread);
            }
        });
    }
    let error = RwSignal::new(None::<String>);
    let load_error = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let alive = RwSignal::new(true);
    let revision = RwSignal::new(0u64);
    let hook = RwSignal::new(None::<crate::api::ReactiveHookToken>);
    let refresh = Callback::new(move |_: ()| {
        if loading.get_untracked() || busy.get_untracked() {
            return;
        }
        loading.set(true);
        revision.update(|value| *value = value.wrapping_add(1));
        let requested = revision.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::reactive_snapshot().await;
            if alive.try_get_untracked() != Some(true)
                || revision.try_get_untracked() != Some(requested)
            {
                return;
            }
            loading.set(false);
            match result {
                Ok(next) => {
                    let next = bruhs_models::weekly_reports::editable_automation_snapshot(next);
                    // An unchanged poll leaves the cards, and any edit in them, alone.
                    if snapshot.with_untracked(|current| current.as_ref() != Some(&next)) {
                        snapshot.set(Some(next));
                    }
                    load_error.set(None);
                }
                Err(err) => load_error.set(Some(err.to_string())),
            }
        });
    });
    let send = Callback::new(move |command: ReactiveCommand| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        loading.set(false);
        error.set(None);
        hook.set(None);
        revision.update(|value| *value = value.wrapping_add(1));
        let requested = revision.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::reactive_command(&command).await;
            if alive.try_get_untracked() != Some(true)
                || revision.try_get_untracked() != Some(requested)
            {
                return;
            }
            busy.set(false);
            match result {
                Ok(next) => snapshot.set(Some(
                    bruhs_models::weekly_reports::editable_automation_snapshot(next),
                )),
                Err(err) => error.set(Some(err.to_string())),
            }
        });
    });
    Effect::new(move |_| refresh.run(()));
    #[cfg(target_arch = "wasm32")]
    let timer = StoredValue::new_local(Some(gloo_timers::callback::Interval::new(
        REFRESH_MS,
        move || refresh.run(()),
    )));
    on_cleanup(move || {
        alive.set(false);
        #[cfg(target_arch = "wasm32")]
        timer.update_value(|timer| {
            timer.take();
        });
    });

    let agent_id = RwSignal::new(None::<String>);
    let agent_open = RwSignal::new(false);
    let name = RwSignal::new(String::new());
    let instructions = RwSignal::new(String::new());
    let model = RwSignal::new(bruhs_models::agent::DEFAULT_AGENT_MODEL.to_string());
    let path = RwSignal::new(String::new());
    let memory = RwSignal::new(String::new());
    let enabled = RwSignal::new(false);
    let chat = RwSignal::new(String::new());
    let chat_agent = RwSignal::new(String::new());
    // The agent whose Delete is asking first. It lives here, not in the
    // card, so a snapshot that redraws the cards keeps the question open.
    let deleting = RwSignal::new(None::<String>);
    // Once the agent has gone, its card and buttons have too: the question
    // closes and focus moves to New agent, never to another Delete.
    let new_agent_ref = NodeRef::<leptos::html::Button>::new();
    Effect::new(move |_| {
        let Some(id) = deleting.get() else {
            return;
        };
        let gone = snapshot.with(|s| {
            s.as_ref()
                .is_some_and(|s| !s.agents.iter().any(|agent| agent.id == id))
        });
        if gone {
            deleting.set(None);
            focus_after_render(new_agent_ref);
        }
    });
    let edit_agent = Callback::new(move |agent: ReactiveAgent| {
        agent_id.set(Some(agent.id.clone()));
        name.set(agent.name);
        instructions.set(agent.instructions);
        model.set(agent.model);
        path.set(agent.project_path);
        memory.set(agent.memory);
        enabled.set(agent.enabled);
        chat_agent.set(agent.id);
        agent_open.set(true);
    });
    let source_id = RwSignal::new(None::<String>);
    let source_open = RwSignal::new(false);
    let source_name = RwSignal::new(String::new());
    let source_type = RwSignal::new("folder".to_string());
    let source_value = RwSignal::new(String::new());
    let source_cwd = RwSignal::new(String::new());
    let pattern = RwSignal::new("*.pdf".to_string());
    let recursive = RwSignal::new(true);
    let interval = RwSignal::new("5".to_string());
    let settle = RwSignal::new("2".to_string());
    let timeout = RwSignal::new("30".to_string());
    let source_enabled = RwSignal::new(false);
    let cal_times = RwSignal::new("08:00".to_string());
    let cal_days = RwSignal::new(Vec::<u8>::new());
    let cal_zone = RwSignal::new(calendar::local_time_zone());
    let edit_source = Callback::new(move |source: EventSource| {
        source_id.set(Some(source.id));
        source_name.set(source.name);
        source_enabled.set(source.enabled);
        match source.kind {
            SourceKind::Folder {
                path,
                pattern: mask,
                recursive: recurse,
                settle_ms,
                poll_ms,
            } => {
                source_type.set("folder".into());
                source_value.set(path);
                pattern.set(mask);
                recursive.set(recurse);
                interval.set(seconds(poll_ms));
                settle.set(seconds(settle_ms));
            }
            SourceKind::Http { url, poll_ms } => {
                source_type.set("http".into());
                source_value.set(url);
                interval.set(seconds(poll_ms));
            }
            SourceKind::Command {
                command,
                cwd,
                poll_ms,
                timeout_ms,
            } => {
                source_type.set("command".into());
                source_value.set(command);
                source_cwd.set(cwd);
                interval.set(seconds(poll_ms));
                timeout.set(seconds(timeout_ms));
            }
            SourceKind::Schedule { every_ms } => {
                source_type.set("schedule".into());
                interval.set(seconds(every_ms));
            }
            SourceKind::Calendar {
                times,
                weekdays,
                time_zone,
            } => {
                source_type.set("calendar".into());
                cal_times.set(times.join(", "));
                cal_days.set(weekdays);
                cal_zone.set(time_zone);
            }
            SourceKind::Webhook => source_type.set("webhook".into()),
            SourceKind::AppEvent { topic } => {
                source_type.set("app_event".into());
                source_value.set(topic);
            }
            SourceKind::Custom => source_type.set("custom".into()),
        }
        source_open.set(true);
    });
    let subscription_id = RwSignal::new(None::<String>);
    let subscription_open = RwSignal::new(false);
    let sub_agent = RwSignal::new(String::new());
    let sub_source = RwSignal::new(String::new());
    let sub_note = RwSignal::new(String::new());
    let sub_enabled = RwSignal::new(false);
    let sub_auto = RwSignal::new(false);
    let debounce = RwSignal::new("0".into());
    let throttle = RwSignal::new("0".into());
    let sub_busy = RwSignal::new("queue".to_string());
    let edit_subscription = Callback::new(move |subscription: Subscription| {
        subscription_id.set(Some(subscription.id));
        sub_agent.set(subscription.agent_id);
        sub_source.set(subscription.source_id);
        sub_note.set(subscription.instructions);
        sub_enabled.set(subscription.enabled);
        sub_auto.set(subscription.auto_run);
        debounce.set(seconds(subscription.debounce_ms));
        throttle.set(seconds(subscription.throttle_ms));
        sub_busy.set(
            match subscription.on_busy {
                BusyPolicy::Queue => "queue",
                BusyPolicy::Restart => "restart",
            }
            .into(),
        );
        subscription_open.set(true);
    });
    let publish_source = RwSignal::new(String::new());
    let publish_key = RwSignal::new(String::new());
    let publish_payload = RwSignal::new("{}".to_string());

    view! {
        <section class="mx-auto mb-4 max-w-6xl rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50" aria-labelledby="reactive-title">
            <div class="flex flex-wrap items-start justify-between gap-3">
                <div class="min-w-0">
                    <div class="flex flex-wrap items-center gap-2"><h2 id="reactive-title" class="text-h4 text-rambutan-100">"Automations"</h2><span class="rounded-seed bg-blueberry-500/15 px-2 py-1 text-label-sm text-blueberry-200">"Drafts only"</span></div>
                    <p class="mt-1 text-body-sm text-rambutan-400">"Describe a job and choose what starts it. Review once to enable a complete automation, then pause or resume it with one control."</p>
                    <p class="mt-1 text-label-sm text-rambutan-500">"Watches and schedules keep working while the local server runs, even with this page closed."</p>
                </div>
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move || busy.get() || loading.get()) on_click=Callback::new(move |_| refresh.run(()))>"Refresh"</Button>
            </div>
            <Show when=move || snapshot.get().is_none() && loading.get()><p role="status" class="mt-3 text-body-sm text-rambutan-400">"Connecting to the local automation server…"</p></Show>
            {move || load_error.get().map(|message| view! { <p role="alert" class="mt-3 rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200">{message}</p> })}
            {move || error.get().map(|message| view! { <p role="alert" class="mt-3 rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200">{message}</p> })}
            <fieldset disabled=move || busy.get() || snapshot.get().is_none() class="m-0 mt-4 min-w-0 space-y-4 border-0 p-0">
                <updates::UpdatesPanel snapshot=snapshot send=send />
                <weekly_reports::WeeklyReportsPanel on_chat=Callback::new(move |(id,body)|{chat_agent.set(id);chat.set(body);}) />
                <setup::AutomationSetupPanel snapshot=snapshot busy=busy error=error send=send on_chat=Callback::new(move |id|chat_agent.set(id)) />
                <div class="grid min-w-0 gap-4 lg:grid-cols-2">
                    <notifications::NotificationsPanel snapshot=snapshot busy=busy error=error send=send />
                    <notifications::BackgroundServicePanel />
                </div>
                <Show when=move || !chat_agent.get().is_empty()>
                    <div class=CARD><h3 class="mb-2 text-label text-rambutan-100">{move || snapshot.with(|s| s.as_ref().and_then(|s| s.agents.iter().find(|a| a.id == chat_agent.get())).map_or_else(|| "Agent chat".into(), |a| format!("Chat with {}", a.name)))}</h3><Field label="Message to agent" value=chat multiline=true /><div class="mt-2 flex gap-2"><Button size=ButtonSize::Small on_click=Callback::new(move |_| { send.run(ReactiveCommand::Chat { agent_id: chat_agent.get_untracked(), message: chat.get_untracked(), run_id: None }); })>"Send message"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| chat_agent.set(String::new()))>"Close"</Button></div><p class="mt-2 text-label-sm text-rambutan-500">"Replies appear in activity below. The agent keeps its conversation across events and restarts."</p></div>
                </Show>
                <details class=CARD><summary class="cursor-pointer text-label text-rambutan-300">"Advanced settings"</summary><p class="mt-2 text-label-sm text-rambutan-500">"Manage separate agents, sources, and subscriptions, or inspect private integration credentials."</p><div class="mt-3 space-y-4">
                <div class="flex flex-wrap gap-2">
                    <button type="button" node_ref=new_agent_ref data-new-agent="" class=button_class(ButtonVariant::Outline, ButtonSize::Small, "") on:click=move |_| {
                        agent_id.set(None); name.set(String::new()); instructions.set(String::new()); memory.set(String::new()); enabled.set(false); agent_open.set(true);
                    }>"New agent"</button>
                    <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| {
                        source_id.set(None); source_name.set("Scans".into()); source_type.set("folder".into()); source_value.set(String::new()); pattern.set("*.pdf".into()); source_enabled.set(false); source_open.set(true);
                    })>"Watch scans"</Button>
                    <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| {
                        subscription_id.set(None); sub_agent.set(snapshot.with(|s| s.as_ref().and_then(|s| s.agents.first()).map(|a| a.id.clone()).unwrap_or_default()));
                        sub_source.set(snapshot.with(|s| s.as_ref().and_then(|s| s.sources.first()).map(|a| a.id.clone()).unwrap_or_default()));
                        sub_note.set("Read each new scan and prepare a draft with the payee, amount, due date, and payment reference. Flag missing or uncertain details. Do not make payments.".into()); sub_enabled.set(false); sub_auto.set(false); sub_busy.set("queue".into()); subscription_open.set(true);
                    })>"Connect source to agent"</Button>
                </div>
                <Show when=move || snapshot.with(|s| s.as_ref().is_some_and(|s| s.agents.is_empty()))><p class="text-body-sm text-rambutan-400">"Start with a bill assistant: create an agent, choose Watch scans, then connect the two. New agents and sources stay paused until you enable them."</p></Show>
                <Show when=move || agent_open.get()>
                    <div class=CARD>
                        <h3 class="mb-3 text-label text-rambutan-100">{move || if agent_id.get().is_some() { "Edit agent" } else { "New agent" }}</h3>
                        <div class="grid gap-3 sm:grid-cols-2"><Field label="Agent name" value=name /><Field label="Model" value=model /><div class="sm:col-span-2"><Field label="Working folder" value=path /></div><div class="sm:col-span-2"><Field label="Purpose and instructions" value=instructions multiline=true /></div><div class="sm:col-span-2"><Field label="Persistent memory" value=memory multiline=true /></div></div>
                        <p class="mt-1 text-label-sm text-rambutan-500">"Memory is included in future turns. Save only what the agent should remember."</p>
                        <Check label="Agent enabled" value=enabled />
                        <div class="mt-2 flex flex-wrap gap-2"><Button size=ButtonSize::Small on_click=Callback::new(move |_| {
                            let next_id=agent_id.get_untracked().unwrap_or_else(id); agent_id.set(Some(next_id.clone())); send.run(apply(ReactiveChange::SaveAgent { agent: ReactiveAgent { id: next_id, name: name.get_untracked(), instructions: instructions.get_untracked(), memory: memory.get_untracked(), model: model.get_untracked(), project_path: path.get_untracked(), enabled: enabled.get_untracked() } }));
                        })>"Save agent"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| agent_open.set(false))>"Close"</Button></div>
                    </div>
                </Show>
                {move || snapshot.with(|s| s.as_ref().map(|s| s.agents.clone()).unwrap_or_default()).into_iter().map(|agent| {
                    let edit = agent.clone(); let mut toggle = agent.clone(); toggle.enabled = !toggle.enabled;
                    let chat_id = agent.id.clone();
                    // Delete agent asks first, inline as Delete room does: the
                    // question takes the button's place with focus on Keep, the
                    // safe choice, and Keep gives focus back to Delete agent.
                    let delete_ref = NodeRef::<leptos::html::Button>::new();
                    let keep_ref = NodeRef::<leptos::html::Button>::new();
                    let asking = { let id = agent.id.clone(); Signal::derive(move || deleting.with(|deleting| deleting.as_deref() == Some(id.as_str()))) };
                    let ask_id = agent.id.clone();
                    let confirm = bruhs_models::crew::delete_bruh_confirm(&agent.name);
                    let delete = apply(ReactiveChange::DeleteAgent { id: agent.id.clone() });
                    // Automation cards already show the memory of their agents.
                    let in_automation = snapshot.with(|s| s.as_ref().is_some_and(|s| s.subscriptions.iter().any(|sub| sub.agent_id == agent.id && AutomationSetup::from_subscription(s, sub).is_some())));
                    view! { <article class=CARD>
                        <div class="flex flex-wrap items-center justify-between gap-2"><div class="min-w-0"><h3 class="break-words text-label text-rambutan-100">{agent.name.clone()}</h3><p class="break-all text-label-sm text-rambutan-400">{format!("{}: {}", if agent.enabled { "Enabled" } else { "Paused" }, agent.model)}</p></div><div class="flex flex-wrap gap-2">
                            <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| edit_agent.run(edit.clone()))>"Edit and memory"</Button>
                            <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| chat_agent.set(chat_id.clone()))>"Chat"</Button>
                            {command_button(if agent.enabled { "Pause" } else { "Enable" }, apply(ReactiveChange::SaveAgent { agent: toggle }), send)}
                            {move || (!asking.get()).then(|| { let id = ask_id.clone(); view! {
                                <button type="button" node_ref=delete_ref data-agent-delete="" class=button_class(ButtonVariant::Outline, ButtonSize::Small, "") on:click=move |_| { deleting.set(Some(id.clone())); focus_after_render(keep_ref); }>"Delete agent"</button>
                            } })}
                        </div></div>
                        {move || asking.get().then(|| { let delete = delete.clone(); view! {
                            <div data-agent-delete-confirm="" class="mt-2 space-y-2 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50">
                                <p id="agent-delete-question" class="text-body-sm text-rambutan-200">{confirm.clone()}</p>
                                <div class="flex flex-wrap items-center gap-2">
                                    <button type="button" data-agent-delete-yes="" class=button_class(ButtonVariant::Destructive, ButtonSize::Small, "") on:click=move |_| send.run(delete.clone())>{bruhs_models::crew::DELETE_BRUH_YES}</button>
                                    <button type="button" node_ref=keep_ref data-agent-delete-keep="" aria-describedby="agent-delete-question" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") on:click=move |_| { deleting.set(None); focus_after_render(delete_ref); }>{bruhs_models::crew::DELETE_BRUH_NO}</button>
                                </div>
                            </div>
                        } })}
                        {(!in_automation).then(|| view! { <memory::AgentMemoryPanel agent_id=agent.id snapshot=snapshot send=send /> })}
                    </article> }
                }).collect_view()}

                <h3 class="text-label text-rambutan-100">"Event sources"</h3>
                {move || snapshot.with(|s| s.as_ref().map(|s| s.sources.clone()).unwrap_or_default()).into_iter().map(|source| {
                    let edit = source.clone(); let mut toggle = source.clone(); toggle.enabled = !toggle.enabled; let hook_id = source.id.clone();
                    let status = snapshot.with(|s| s.as_ref().and_then(|s| s.source_statuses.iter().find(|status| status.source_id == source.id)).cloned());
                    view! { <article class=CARD><div class="flex flex-wrap items-center justify-between gap-2"><div class="min-w-0"><h4 class="break-words text-label text-rambutan-100">{source.name.clone()}</h4><p class="text-label-sm text-rambutan-400">{format!("{}: {}", source_label(&source.kind), if source.enabled { "Enabled" } else { "Paused" })}</p></div><div class="flex flex-wrap gap-2"><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| edit_source.run(edit.clone()))>"Edit source"</Button>{command_button(if source.enabled { "Pause" } else { "Enable" }, apply(ReactiveChange::SaveSource { source: toggle }), send)}{command_button("Delete source", apply(ReactiveChange::DeleteSource { id: source.id.clone() }), send)}
                        {matches!(source.kind, SourceKind::Webhook).then(|| view! { <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| { if busy.get_untracked() { return; } let source_id = hook_id.clone(); busy.set(true); loading.set(false); revision.update(|value| *value=value.wrapping_add(1)); let requested=revision.get_untracked(); wasm_bindgen_futures::spawn_local(async move { let result=crate::api::reactive_hook_token(&source_id).await; if alive.try_get_untracked()!=Some(true) || revision.try_get_untracked()!=Some(requested) { return; } busy.set(false); match result { Ok(token) => hook.set(Some(token)), Err(err) => error.set(Some(err.to_string())) } }); })>"Show webhook token"</Button> })}
                    </div></div>{status.and_then(|s| s.error).map(|error| view! { <p role="status" class="mt-2 break-words text-body-sm text-lychee-200">{error}</p> })}</article> }
                }).collect_view()}
                {move || hook.get().map(|credentials| view! { <div class=CARD><p class="text-label-sm text-rambutan-300">"Private webhook credentials"</p><p class="mt-1 break-all select-all text-body-sm text-rambutan-200">{credentials.path}</p><p class="mt-1 break-all select-all text-body-sm text-rambutan-200">{format!("Authorization: Bearer {}", credentials.token)}</p><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| hook.set(None))>"Hide token"</Button></div> })}
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| { source_id.set(None); source_name.set(String::new()); source_type.set("folder".into()); source_value.set(String::new()); source_cwd.set(String::new()); cal_times.set("08:00".into()); cal_days.set(Vec::new()); cal_zone.set(calendar::local_time_zone()); source_enabled.set(false); source_open.set(true); })>"New event source"</Button>
                <Show when=move || source_open.get()>
                    <div class=CARD><div class="grid gap-3 sm:grid-cols-2"><Field label="Source name" value=source_name /><label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Source type"</span><select aria-label="Source type" class=SELECT prop:value=move || source_type.get() on:change=move |event| source_type.set(event_target_value(&event))>{[("folder","Folder"),("http","Website"),("command","Command"),("schedule","Schedule"),("calendar","Calendar"),("webhook","Webhook"),("app_event","App event"),("custom","Custom event")].into_iter().map(|(value,label)| view! { <option value=value>{label}</option> }).collect_view()}</select></label>
                        {move || match source_type.get().as_str() {
                            "folder" => view! { <div class="space-y-3 sm:col-span-2"><Field label="Folder to watch" value=source_value /><Field label="File pattern" value=pattern /><Check label="Include subfolders" value=recursive /><Field label="File settle time in seconds" value=settle /></div> }.into_any(),
                            "http" => view! { <div class="sm:col-span-2"><Field label="Website URL" value=source_value /></div> }.into_any(),
                            "command" => view! { <div class="space-y-3 sm:col-span-2"><Field label="Command to run" value=source_value multiline=true /><Field label="Command working folder" value=source_cwd /><Field label="Command timeout in seconds" value=timeout /><p class="text-label-sm text-rambutan-400">"This command runs on your machine when the source is enabled. Use a command you trust."</p></div> }.into_any(),
                            "app_event" => view! { <div class="sm:col-span-2"><Field label="App event topic" value=source_value /><p class="mt-1 text-label-sm text-rambutan-500">"Choose meeting.completed, meeting.transcribed, or meeting.summary."</p></div> }.into_any(),
                            "webhook" => view! { <p class="text-body-sm text-rambutan-400 sm:col-span-2">"Save the source, then reveal its private token from the source row."</p> }.into_any(),
                            "calendar" => view! { <div class="space-y-3 sm:col-span-2"><Field label="Times (HH:MM, comma-separated)" value=cal_times /><calendar::WeekdayToggles value=cal_days /><Field label="Time zone" value=cal_zone /></div> }.into_any(),
                            "custom" => view! { <p class="text-body-sm text-rambutan-400 sm:col-span-2">"Publish events through the local API or the event form below. MQTT and Zigbee bridges can publish here. An FTP upload folder can use a folder source."</p> }.into_any(),
                            _ => view! { <p class="text-body-sm text-rambutan-400 sm:col-span-2">"Run on a repeating interval while the local server is available."</p> }.into_any(),
                        }}
                        <Show when=move || matches!(source_type.get().as_str(), "folder" | "http" | "command" | "schedule")><Field label="Interval in seconds" value=interval /></Show>
                    </div><Check label="Source enabled" value=source_enabled /><div class="mt-2 flex gap-2"><Button size=ButtonSize::Small on_click=Callback::new(move |_| {
                        let kind = if source_type.get_untracked() == "calendar" { calendar::calendar_source(&cal_times.get_untracked(), &cal_days.get_untracked(), &cal_zone.get_untracked()) } else { source_kind(&source_type.get_untracked(), source_value.get_untracked(), source_cwd.get_untracked(), pattern.get_untracked(), recursive.get_untracked(), &interval.get_untracked(), &settle.get_untracked(), &timeout.get_untracked()) };
                        match kind { Ok(kind) => { let next_id=source_id.get_untracked().unwrap_or_else(id); source_id.set(Some(next_id.clone())); send.run(apply(ReactiveChange::SaveSource { source: EventSource { id: next_id, name: source_name.get_untracked(), enabled: source_enabled.get_untracked(), kind } })); }, Err(err) => error.set(Some(err)), }
                    })>"Save source"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| source_open.set(false))>"Close"</Button></div></div>
                </Show>
                <h3 class="text-label text-rambutan-100">"Subscriptions"</h3>
                {move || snapshot.with(|s| s.as_ref().map(|s| s.subscriptions.clone()).unwrap_or_default()).into_iter().map(|subscription| {
                    let edit = subscription.clone(); let mut toggle = subscription.clone(); toggle.enabled = !toggle.enabled;
                    let label = snapshot.with(|s| { let s=s.as_ref(); format!("{} receives {}", s.and_then(|s| s.agents.iter().find(|a| a.id==subscription.agent_id)).map_or("Agent", |a| a.name.as_str()), s.and_then(|s| s.sources.iter().find(|a| a.id==subscription.source_id)).map_or("source", |a| a.name.as_str())) });
                    view! { <article class=CARD><h4 class="break-words text-label text-rambutan-100">{label}</h4><p class="mt-1 text-label-sm text-rambutan-400">{format!("{}: {}", if subscription.enabled { "Enabled" } else { "Paused" }, event_policy(subscription.auto_run, snapshot.with(|s| saved_agent_level(s.as_ref(), &subscription.agent_id))))}</p>{(subscription.on_busy == BusyPolicy::Restart).then(|| view! { <p class="mt-1 text-label-sm text-rambutan-400">{format!("While the agent is busy: {}", BUSY_CHOICES[1].1)}</p> })}<p class="mt-2 whitespace-pre-wrap break-words text-body-sm text-rambutan-300">{subscription.instructions.clone()}</p><div class="mt-2 flex flex-wrap gap-2"><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| edit_subscription.run(edit.clone()))>"Edit subscription"</Button>{command_button(if subscription.enabled { "Pause" } else { "Enable" }, apply(ReactiveChange::SaveSubscription { subscription: toggle }), send)}{command_button("Delete subscription", apply(ReactiveChange::DeleteSubscription { id: subscription.id }), send)}</div></article> }
                }).collect_view()}
                <Show when=move || subscription_open.get()>
                    <div class=CARD><div class="grid gap-3 sm:grid-cols-2"><label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Agent"</span><select aria-label="Subscription agent" class=SELECT prop:value=move || sub_agent.get() on:change=move |event| sub_agent.set(event_target_value(&event))><option value="">"Choose agent"</option>{move || snapshot.with(|s| s.as_ref().map(|s| s.agents.clone()).unwrap_or_default()).into_iter().map(|agent| view! { <option value=agent.id>{agent.name}</option> }).collect_view()}</select></label><label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Source"</span><select aria-label="Subscription source" class=SELECT prop:value=move || sub_source.get() on:change=move |event| sub_source.set(event_target_value(&event))><option value="">"Choose source"</option>{move || snapshot.with(|s| s.as_ref().map(|s| s.sources.clone()).unwrap_or_default()).into_iter().map(|source| view! { <option value=source.id>{source.name}</option> }).collect_view()}</select></label><div class="sm:col-span-2"><Field label="What to do when this source reports an event" value=sub_note multiline=true /></div><Field label="Debounce in seconds" value=debounce /><Field label="Minimum gap between runs in seconds" value=throttle /><label class="block space-y-1 text-label-sm text-rambutan-300 sm:col-span-2"><span>"While the agent is busy"</span><select aria-label="While the agent is busy" class=SELECT prop:value=move || sub_busy.get() on:change=move |event| sub_busy.set(event_target_value(&event))>{BUSY_CHOICES.into_iter().map(|(value,label)| view! { <option value=value>{label}</option> }).collect_view()}</select></label></div><Check label="Subscription enabled" value=sub_enabled /><Check label=AUTO_RUN_LABEL value=sub_auto /><p class="text-label-sm text-rambutan-500">{move || snapshot.with(|s| auto_run_note(saved_agent_level(s.as_ref(), &sub_agent.get())))}</p><div class="mt-2 flex gap-2"><Button size=ButtonSize::Small on_click=Callback::new(move |_| {
                        match (milliseconds(&debounce.get_untracked(), true), milliseconds(&throttle.get_untracked(), true)) { (Ok(debounce_ms),Ok(throttle_ms)) => { let next_id=subscription_id.get_untracked().unwrap_or_else(id); subscription_id.set(Some(next_id.clone())); send.run(apply(ReactiveChange::SaveSubscription { subscription: Subscription { id: next_id, source_id: sub_source.get_untracked(), agent_id: sub_agent.get_untracked(), instructions: sub_note.get_untracked(), enabled: sub_enabled.get_untracked(), auto_run: sub_auto.get_untracked(), debounce_ms, throttle_ms, on_busy: busy_policy(&sub_busy.get_untracked()) } })); }, _ => error.set(Some("Enter a nonnegative number of seconds for debounce and minimum gap.".into())) }
                    })>"Save subscription"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| subscription_open.set(false))>"Close"</Button></div></div>
                </Show>
                <details class=CARD><summary class="cursor-pointer text-label text-rambutan-300">"Publish a custom event"</summary><div class="mt-3 space-y-3"><label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Custom event source"</span><select aria-label="Custom event source" class=SELECT prop:value=move || publish_source.get() on:change=move |event| publish_source.set(event_target_value(&event))><option value="">"Choose source"</option>{move || snapshot.with(|s| s.as_ref().map(|s| s.sources.clone()).unwrap_or_default()).into_iter().filter(|s| matches!(s.kind, SourceKind::Custom)).map(|s| view! { <option value=s.id>{s.name}</option> }).collect_view()}</select></label><Field label="Unique event key" value=publish_key /><Field label="Event payload as JSON" value=publish_payload multiline=true /><p class="text-label-sm text-rambutan-500">"Reusing a key keeps the same event from running twice."</p><Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| match serde_json::from_str(&publish_payload.get_untracked()) { Ok(payload) => send.run(ReactiveCommand::Publish { source_id: publish_source.get_untracked(), key: publish_key.get_untracked(), payload }), Err(err) => error.set(Some(format!("Check the event JSON: {err}"))) })>"Publish event"</Button></div></details>
                </div></details>
                {move || snapshot.with(|s| s.as_ref().map(|s| s.proposals.clone()).unwrap_or_default()).into_iter().filter(|p| p.status==ProposalStatus::Pending).map(|proposal| {
                    let level = match &proposal.change { ReactiveChange::SaveAutomation { automation } => snapshot.with(|s| bruhs_models::crew::agent_run_level(s.as_ref(), &automation.agent.id, &automation.agent.model)), _ => AccessLevel::ReadOnly }; let label = snapshot.with(|s| proposal_label(&proposal.change, level, s.as_ref())); let approve_label=match &proposal.change {ReactiveChange::SaveAutomation{automation} if automation.enabled()=>"Enable automation",ReactiveChange::SaveAutomation{..}=>"Save automation",_=>"Approve setup change"}; let fields=match &proposal.change{ReactiveChange::SaveAutomation{automation}=>Some(setup::review_fields(automation, level)),_=>None}; let payload=serde_json::to_string_pretty(&proposal.change).unwrap_or_else(|err| format!("Cannot display proposal: {err}"));
                    view! { <article class=CARD><h3 class="text-label text-rambutan-100">"Setup proposal awaiting review"</h3><p class="mt-1 text-body-sm text-rambutan-300">{label}</p>{if let Some(fields)=fields { view! { <dl class="mt-3 space-y-3">{fields.into_iter().map(|(label,value)|view!{<div><dt class="text-label-sm text-rambutan-400">{label}</dt><dd class="mt-1 whitespace-pre-wrap break-words text-body-sm text-rambutan-200">{value}</dd></div>}).collect_view()}</dl><details class="mt-3"><summary class="cursor-pointer text-label-sm text-rambutan-400">"Advanced setup definition"</summary><pre class="mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words text-label-sm text-rambutan-300">{payload}</pre></details> }.into_any() }else{view!{<pre class="mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words text-label-sm text-rambutan-300">{payload}</pre>}.into_any()}}<div class="mt-3 flex gap-2">{command_button(approve_label, ReactiveCommand::DecideProposal { id: proposal.id.clone(), approve: true }, send)}{command_button("Decline", ReactiveCommand::DecideProposal { id: proposal.id, approve: false }, send)}</div></article> }
                }).collect_view()}
                <h3 class="text-label text-rambutan-100">"Activity and conversations"</h3>
                <Show when=move || snapshot.with(|s| s.as_ref().is_some_and(|s| s.runs.is_empty()))><p class="text-body-sm text-rambutan-500">"No runs yet. Chat with an agent or enable an automation."</p></Show>
                {move || snapshot.with(|s| s.as_ref().map(|s| s.runs.clone()).unwrap_or_default()).into_iter().map(|run| {
                    let agent = snapshot.with(|s| s.as_ref().and_then(|s| s.agents.iter().find(|a| a.id==run.agent_id)).map_or_else(|| "Agent".into(), |a| a.name.clone()));
                    let draft=run.result.as_deref().filter(|_|run.status==RunStatus::Completed).and_then(workflow_draft);
                    let draft_run=run.id.clone();
                    let notes=runs::run_notes(&run); let run_error=runs::run_error(&run); let warning=runs::report_warning(&run); let outcome=runs::outcome_badge(&run); let trigger=runs::trigger_label(&run);
                    view! { <article class=CARD><div class="flex flex-wrap justify-between gap-2"><div class="flex min-w-0 flex-wrap items-center gap-2"><h4 class="break-words text-label text-rambutan-100">{agent}</h4><span class="rounded-seed bg-longan-800 px-2 py-0.5 text-label-sm text-rambutan-300">{trigger}</span>{outcome.map(|label| view! { <span class="rounded-seed bg-blueberry-500/15 px-2 py-0.5 text-label-sm text-blueberry-200">{label}</span> })}</div><span class="text-label-sm text-blueberry-200">{run_label(run.status)}</span></div>{notes.into_iter().map(|note| view! { <p class="mt-1 text-label-sm text-rambutan-400">{note}</p> }).collect_view()}{warning.map(|warning| view! { <p class="mt-1 break-words text-label-sm text-durian-300">{warning}</p> })}<details class="mt-2"><summary class="cursor-pointer text-label-sm text-rambutan-400">"Request and event context"</summary><pre class="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words text-body-sm text-rambutan-300">{run.prompt}</pre></details>{run.result.map(|result| view! { <div class="mt-3 whitespace-pre-wrap break-words select-text text-body-sm text-rambutan-200">{workflow_draft(&result).map_or_else(|| result.clone(), |draft| draft.summary)}</div><crate::ui::CopyButton text=result label="Copy draft" /> })}{run_error.map(|error| view! { <p class="mt-2 whitespace-pre-wrap break-words text-body-sm text-lychee-200">{error}</p> })}<div class="mt-2 flex flex-wrap gap-2">
                        {draft.map(|draft| view! { <Button size=ButtonSize::Small on_click=Callback::new(move |_| on_preview.run((draft.summary.clone(),draft.operations.clone(),draft_run.clone())))>"Preview changes"</Button><p class="text-label-sm text-rambutan-500">"Creates a separate reviewed workflow. This draft is not marked as applied."</p> })}
                        {(run.status==RunStatus::AwaitingReview).then(|| command_button(run_reviewed_label(snapshot.with(|s| saved_agent_level(s.as_ref(), &run.agent_id))), ReactiveCommand::ApproveRun { id: run.id.clone() }, send))}
                        {matches!(run.status,RunStatus::AwaitingReview|RunStatus::Queued|RunStatus::Running).then(|| command_button("Cancel run", ReactiveCommand::CancelRun { id: run.id.clone() }, send))}
                        {matches!(run.status,RunStatus::Failed|RunStatus::Interrupted|RunStatus::Cancelled).then(|| command_button("Retry with review", ReactiveCommand::RetryRun { id: run.id }, send))}
                    </div></article> }
                }).collect_view()}
            </fieldset>
        </section>
    }
}

fn seconds(millis: u64) -> String {
    if millis.is_multiple_of(1_000) {
        (millis / 1_000).to_string()
    } else {
        format!("{}.{:03}", millis / 1_000, millis % 1_000)
    }
}

fn milliseconds(text: &str, zero: bool) -> Result<u64, String> {
    let mut parts = text.trim().split('.');
    let whole = parts
        .next()
        .unwrap_or("")
        .parse::<u64>()
        .map_err(|_| "Enter a number of seconds.".to_string())?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some() || fraction.len() > 3 || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return Err("Use seconds with at most three decimal places.".into());
    }
    let fraction = format!("{fraction:0<3}")
        .parse::<u64>()
        .map_err(|_| "Enter a number of seconds.".to_string())?;
    let millis = whole
        .checked_mul(1_000)
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| "That interval is too long.".to_string())?;
    if millis == 0 && !zero {
        Err("The interval must be greater than zero.".into())
    } else {
        Ok(millis)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the source editor's distinct fields are validated together"
)]
fn source_kind(
    kind: &str,
    value: String,
    cwd: String,
    pattern: String,
    recursive: bool,
    interval: &str,
    settle: &str,
    timeout: &str,
) -> Result<SourceKind, String> {
    Ok(match kind {
        "folder" => SourceKind::Folder {
            path: value,
            pattern,
            recursive,
            settle_ms: milliseconds(settle, true)?,
            poll_ms: milliseconds(interval, false)?,
        },
        "http" => SourceKind::Http {
            url: value,
            poll_ms: milliseconds(interval, false)?,
        },
        "command" => SourceKind::Command {
            command: value,
            cwd,
            poll_ms: milliseconds(interval, false)?,
            timeout_ms: milliseconds(timeout, false)?,
        },
        "schedule" => SourceKind::Schedule {
            every_ms: milliseconds(interval, false)?,
        },
        "webhook" => SourceKind::Webhook,
        "app_event" => SourceKind::AppEvent { topic: value },
        "custom" => SourceKind::Custom,
        _ => return Err("Choose a supported source type.".into()),
    })
}

/// `level` is the level the automation's agent runs at; `snapshot` names
/// the agent a delete would remove.
fn proposal_label(
    change: &ReactiveChange,
    level: AccessLevel,
    snapshot: Option<&ReactiveSnapshot>,
) -> String {
    match change {
        ReactiveChange::SaveAutomation { automation } => format!("{} {}. Review its complete purpose, source, model, folder, memory, and timing below. {}",if automation.enabled(){"Enable automation"}else{"Save paused automation"}, automation.agent.name,event_policy_line(automation.subscription.auto_run, level)),
        ReactiveChange::SaveAgent { agent } => format!("Save agent {} with model {}, working folder {}, enabled {}. Review its instructions and memory below.", agent.name,agent.model,agent.project_path,agent.enabled),
        ReactiveChange::SaveSource { source } => format!("Save {} source {}, enabled {}. Review the complete connection settings below.", source_label(&source.kind),source.name,source.enabled),
        ReactiveChange::SaveSubscription { subscription } => format!("Connect source {} to agent {}. Enabled: {}. Runs automatically: {}.", subscription.source_id,subscription.agent_id,subscription.enabled,subscription.auto_run),
        ReactiveChange::DeleteAgent { id } => bruhs_models::crew::delete_agent_line_for(snapshot, id),
        ReactiveChange::DeleteSource { id } => format!("Delete source {id} and its subscriptions."),
        ReactiveChange::DeleteSubscription { id } => format!("Delete subscription {id}."),
        ReactiveChange::SetMemory { agent_id, .. } => format!("Replace the persistent memory of agent {agent_id}. Review the complete replacement below."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_delete_agent_proposal_names_the_agent_and_what_goes_with_it() {
        let snapshot = ReactiveSnapshot {
            agents: vec![ReactiveAgent {
                id: "a1".into(),
                name: "Ada".into(),
                instructions: String::new(),
                memory: String::new(),
                model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
                project_path: String::new(),
                enabled: true,
            }],
            ..ReactiveSnapshot::default()
        };
        let delete = |id: &str| ReactiveChange::DeleteAgent { id: id.into() };
        assert_eq!(
            proposal_label(&delete("a1"), AccessLevel::ReadOnly, Some(&snapshot)),
            bruhs_models::crew::delete_agent_line("Ada")
        );
        // An agent the app hasn't loaded is named by its id.
        assert_eq!(
            proposal_label(&delete("zz"), AccessLevel::ReadOnly, None),
            bruhs_models::crew::delete_agent_line("zz")
        );
    }
    #[test]
    fn editor_preserves_subsecond_intervals_and_rejects_overflow_or_negative_values() {
        assert_eq!(milliseconds(&seconds(1_250), false), Ok(1_250));
        for value in ["-1", "NaN", "1.0001", "18446744073709551615", "1.2.3"] {
            assert!(milliseconds(value, false).is_err());
        }
        assert!(milliseconds("0", false).is_err());
        assert_eq!(milliseconds("0", true), Ok(0));
    }
    #[test]
    fn scanner_source_never_enables_execution_implicitly() {
        assert!(matches!(
            source_kind(
                "folder",
                "/tmp/scans".into(),
                String::new(),
                "*.pdf".into(),
                true,
                "5",
                "2",
                "30"
            )
            .unwrap(),
            SourceKind::Folder {
                settle_ms: 2_000,
                poll_ms: 5_000,
                ..
            }
        ));
    }
    #[test]
    fn nonpolling_sources_ignore_unused_interval_fields_and_commands_keep_their_folder() {
        assert_eq!(
            source_kind(
                "webhook",
                String::new(),
                String::new(),
                String::new(),
                false,
                "invalid",
                "invalid",
                "invalid"
            )
            .unwrap(),
            SourceKind::Webhook
        );
        assert_eq!(
            source_kind(
                "command",
                "printf ready".into(),
                "/tmp/project".into(),
                String::new(),
                false,
                "1.250",
                "invalid",
                "30"
            )
            .unwrap(),
            SourceKind::Command {
                command: "printf ready".into(),
                cwd: "/tmp/project".into(),
                poll_ms: 1250,
                timeout_ms: 30000
            }
        );
    }
    #[test]
    fn existing_zero_settle_source_round_trips_through_the_editor() {
        assert!(matches!(
            source_kind(
                "folder",
                "/tmp/scans".into(),
                String::new(),
                "*.pdf".into(),
                true,
                "5",
                "0",
                "30"
            )
            .unwrap(),
            SourceKind::Folder { settle_ms: 0, .. }
        ));
    }
}
