//! The server owns event agents; this entity only edits and observes them.
use super::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::{
    app_state::spawn_tokio,
    server_client::{ReactiveHookToken, ServerClient, ServerError},
    theme,
};
use bruhs_models::reactive::*;
mod editor_fields;
mod memory;
mod notifications;
mod runs;
mod setup;
mod updates;
mod weekly_reports;
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};
pub(crate) use updates::{unread_badge, unread_label};

// Match the web panel and existing connection status polling.
const REFRESH: Duration = Duration::from_secs(2);
// While the panel is off screen, keep the unread badge and arrival notices current.
const BACKGROUND_REFRESH: Duration = Duration::from_secs(15);

struct AgentEditor {
    agent: ReactiveAgent,
    name: Entity<InputState>,
    instructions: Entity<InputState>,
    model: Entity<InputState>,
    path: Entity<InputState>,
    memory: Entity<InputState>,
}
struct SourceEditor {
    source: EventSource,
    name: Entity<InputState>,
    kind: &'static str,
    value: Entity<InputState>,
    cwd: Entity<InputState>,
    pattern: Entity<InputState>,
    interval: Entity<InputState>,
    settle: Entity<InputState>,
    timeout: Entity<InputState>,
    recursive: bool,
    times: Entity<InputState>,
    weekdays: Vec<u8>,
    time_zone: Entity<InputState>,
}
struct SubscriptionEditor {
    subscription: bruhs_models::reactive::Subscription,
    instructions: Entity<InputState>,
    debounce: Entity<InputState>,
    throttle: Entity<InputState>,
}
enum Editor {
    Automation(Box<setup::AutomationEditor>),
    Agent(AgentEditor),
    Source(SourceEditor),
    Subscription(SubscriptionEditor),
    Chat {
        agent_id: String,
        message: Entity<InputState>,
    },
    Publish {
        source_id: String,
        key: Entity<InputState>,
        payload: Entity<InputState>,
    },
}

#[derive(Clone)]
pub(crate) struct ReactivePreview {
    pub title: String,
    pub operations: Vec<bruhs_models::WorkflowOperation>,
    /// The run whose draft this is, so the owner's decision can flow back to it.
    pub run_id: String,
}

/// A new unread agent update arrived while the app was open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactiveArrival {
    pub message: String,
}

/// Where a failed command's message is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    General,
    Updates,
    Agent(String),
    Notifications,
}

#[derive(Clone)]
pub(crate) struct ReactiveProjectOpened {
    pub path: String,
}

pub(crate) struct ReactiveAutomations {
    client: ServerClient,
    snapshot: Option<ReactiveSnapshot>,
    error: Option<String>,
    load_error: Option<String>,
    loaded: bool,
    reading: bool,
    busy: bool,
    revision: u64,
    editor: Option<Editor>,
    advanced_open: bool,
    hook: Option<ReactiveHookToken>,
    expanded: HashSet<String>,
    request: Option<Task<()>>,
    poll: Option<Task<()>>,
    last_render: Instant,
    last_refresh: Option<Instant>,
    weekly: weekly_reports::WeeklyReportsState,
    /// Agent names from the unfiltered snapshot, so managed agents still have names.
    agent_names: HashMap<String, String>,
    /// Unread outbox ids already seen; `None` until the first snapshot.
    outbox_seen: Option<HashSet<String>>,
    reply: Option<(String, Entity<InputState>)>,
    /// Redraws while the reply is typed, so Send tracks whether it is empty.
    reply_watch: Option<gpui::Subscription>,
    memory_edit: Option<memory::MemoryEdit>,
    notify_form: Option<notifications::NotificationForm>,
    service: notifications::ServiceState,
    scoped_error: Option<(Scope, String)>,
    /// A draft-decision report is in flight (owned by the workspace reconciler).
    draft_reporting: bool,
    awareness: Option<gpui::Subscription>,
}

trait ReactiveButton {
    fn on_activate(
        self,
        cx: &Context<'_, ReactiveAutomations>,
        action: impl Fn(&mut ReactiveAutomations, &mut Window, &mut Context<'_, ReactiveAutomations>)
            + 'static,
    ) -> Self;
}
impl ReactiveButton for Stateful<Div> {
    fn on_activate(
        self,
        cx: &Context<'_, ReactiveAutomations>,
        action: impl Fn(&mut ReactiveAutomations, &mut Window, &mut Context<'_, ReactiveAutomations>)
            + 'static,
    ) -> Self {
        let action = Rc::new(action);
        let click = Rc::clone(&action);
        self.on_click(cx.listener(move |this, _, window, cx| {
            if !this.busy {
                click(this, window, cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if !this.busy
                && super::web_button::is_web_button_activation_key(event.keystroke.key.as_str())
            {
                cx.stop_propagation();
                action(this, window, cx);
            }
        }))
    }
}

fn edit_input(
    text: &str,
    multiline: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).multi_line(multiline);
        input.set_value(text, window, cx);
        input
    })
}
fn value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}
fn apply(change: ReactiveChange) -> ReactiveCommand {
    ReactiveCommand::Apply { change }
}
fn control(id: impl Into<ElementId>, label: &str, disabled: bool) -> Stateful<Div> {
    control_as(
        id,
        label,
        disabled,
        matches!(
            label,
            "New automation" | "Review automation" | "Enable automation"
        ),
    )
}
fn control_as(
    id: impl Into<ElementId>,
    label: &str,
    disabled: bool,
    primary: bool,
) -> Stateful<Div> {
    styled_control(
        id,
        label,
        disabled,
        if primary {
            WebButtonVariant::Primary
        } else {
            WebButtonVariant::Outline
        },
    )
}
/// A quiet text button, like web's ghost actions (Mark read, Dismiss).
fn ghost_control(id: impl Into<ElementId>, label: &str, disabled: bool) -> Stateful<Div> {
    styled_control(id, label, disabled, WebButtonVariant::Ghost)
}
fn styled_control(
    id: impl Into<ElementId>,
    label: &str,
    disabled: bool,
    variant: WebButtonVariant,
) -> Stateful<Div> {
    web_button(
        id,
        variant,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .aria_label(label.to_string())
    .debug_selector({
        let label = label.to_string();
        move || format!("reactive-{label}")
    })
    .child(label.to_string())
}
fn card() -> Div {
    div()
        .min_w_0()
        .w_full()
        .p_3()
        .rounded(px(8.0))
        .bg(theme::longan_950())
        .border_1()
        .border_color(theme::longan_700().opacity(0.5))
        .flex()
        .flex_col()
        .gap_2()
}
fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(12.0))
        .line_height(relative(1.5))
        .text_color(theme::rambutan_400())
        .child(text.into())
}
fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::rambutan_100())
        .child(text.into())
}
fn field(label: &'static str, input: &Entity<InputState>, multiline: bool, disabled: bool) -> Div {
    div()
        .min_w_0()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(muted(label))
        .child(
            Styled::h(
                Input::new(input).aria_label(label).disabled(disabled),
                px(if multiline { 96.0 } else { 32.0 }),
            )
            .text_size(px(14.0)),
        )
}
fn reactive_error(error: ServerError) -> String {
    let detail = match &error {
        ServerError::Status { body, .. } => serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .filter(|detail| !detail.is_empty())
                    .map(str::to_string)
            }),
        _ => None,
    };
    detail.unwrap_or_else(|| error.to_string())
}

/// What a successful command closes or resets.
#[derive(Debug, PartialEq, Eq)]
enum Effect {
    CloseAutomation,
    CloseReply(String),
    CloseMemory { agent_id: String, key: String },
    ResetNotifications,
    None,
}
impl Effect {
    fn of(command: &ReactiveCommand) -> Self {
        match command {
            ReactiveCommand::Apply {
                change: ReactiveChange::SaveAutomation { .. },
            } => Self::CloseAutomation,
            ReactiveCommand::AnswerOutbox { id, .. } => Self::CloseReply(id.clone()),
            ReactiveCommand::EditMemory { agent_id, key, .. }
            | ReactiveCommand::ForgetMemory { agent_id, key } => Self::CloseMemory {
                agent_id: agent_id.clone(),
                key: key.clone(),
            },
            ReactiveCommand::SetNotificationSettings { .. } => Self::ResetNotifications,
            _ => Self::None,
        }
    }
}

fn run_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::AwaitingReview => "Awaiting review",
        RunStatus::Queued => "Queued",
        RunStatus::Running => "Running",
        RunStatus::Completed => "Draft ready",
        RunStatus::Failed => "Failed",
        RunStatus::Interrupted => "Interrupted",
        RunStatus::Cancelled => "Cancelled",
        RunStatus::Merged => "Handled with another run",
    }
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
fn seconds(millis: u64) -> String {
    if millis.is_multiple_of(1000) {
        (millis / 1000).to_string()
    } else {
        format!("{}.{:03}", millis / 1000, millis % 1000)
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
        .checked_mul(1000)
        .and_then(|n| n.checked_add(fraction))
        .ok_or_else(|| "That interval is too long.".to_string())?;
    if millis == 0 && !zero {
        Err("The interval must be greater than zero.".into())
    } else {
        Ok(millis)
    }
}

impl ReactiveAutomations {
    pub(crate) fn new(cx: &mut Context<'_, Self>) -> Self {
        let client =
            ServerClient::from_env().unwrap_or_else(|error| ServerClient::unavailable(&error));
        let mut panel = Self {
            client,
            snapshot: None,
            error: None,
            load_error: None,
            loaded: false,
            reading: false,
            busy: false,
            revision: 0,
            editor: None,
            advanced_open: false,
            hook: None,
            expanded: HashSet::new(),
            request: None,
            poll: None,
            last_render: Instant::now(),
            last_refresh: None,
            weekly: weekly_reports::WeeklyReportsState::default(),
            agent_names: HashMap::new(),
            outbox_seen: None,
            reply: None,
            reply_watch: None,
            memory_edit: None,
            notify_form: None,
            service: notifications::ServiceState::default(),
            scoped_error: None,
            draft_reporting: false,
            awareness: None,
        };
        if cfg!(not(test)) {
            panel.poll = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(REFRESH).await;
                if this
                    .update(cx, |this, cx| {
                        if this.last_render.elapsed() < REFRESH.saturating_mul(2) {
                            this.refresh(cx);
                        } else if this
                            .last_refresh
                            .is_none_or(|at| at.elapsed() >= BACKGROUND_REFRESH)
                        {
                            this.refresh_snapshot(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }));
        }
        panel
    }
    pub(crate) fn client(&self) -> ServerClient {
        self.client.clone()
    }
    /// Unread agent updates, for the navigation badge.
    pub(crate) fn unread_count(&self) -> usize {
        self.snapshot
            .as_ref()
            .map_or(0, ReactiveSnapshot::unread_count)
    }
    /// Show `snapshot` as if the server had sent it, for tests outside this
    /// module.
    #[cfg(test)]
    pub(crate) fn show_snapshot_for_test(
        &mut self,
        snapshot: ReactiveSnapshot,
        cx: &mut Context<'_, Self>,
    ) {
        self.loaded = true;
        self.set_snapshot(snapshot, cx);
        cx.notify();
    }
    /// The workspace listens for arrivals once; the subscription lives with the panel.
    pub(crate) fn has_awareness(&self) -> bool {
        self.awareness.is_some()
    }
    pub(crate) fn set_awareness(&mut self, subscription: gpui::Subscription) {
        self.awareness = Some(subscription);
    }
    /// Claim the single in-flight draft-decision report; false when one is running.
    pub(crate) fn begin_draft_reporting(&mut self) -> bool {
        !std::mem::replace(&mut self.draft_reporting, true)
    }
    pub(crate) fn finish_draft_reporting(
        &mut self,
        snapshot: Option<ReactiveSnapshot>,
        cx: &mut Context<'_, Self>,
    ) {
        self.draft_reporting = false;
        if let Some(snapshot) = snapshot {
            if !self.busy {
                self.set_snapshot(snapshot, cx);
                cx.notify();
            }
        }
    }
    /// Every snapshot passes through here so arrivals are detected exactly once.
    fn set_snapshot(&mut self, snapshot: ReactiveSnapshot, cx: &mut Context<'_, Self>) {
        self.agent_names = snapshot
            .agents
            .iter()
            .map(|agent| (agent.id.clone(), agent.name.clone()))
            .collect();
        if let Some(item) = updates::new_arrival(&mut self.outbox_seen, &snapshot.outbox) {
            cx.emit(ReactiveArrival {
                message: format!("{}: {}", self.agent_name(&item.agent_id), item.title),
            });
        }
        self.snapshot = Some(bruhs_models::weekly_reports::editable_automation_snapshot(
            snapshot,
        ));
        let settings = self.snapshot.as_ref().map(|s| &s.notification_settings);
        if self
            .notify_form
            .as_ref()
            .is_some_and(|form| !form.dirty && Some(&form.base) != settings)
        {
            self.notify_form = None;
        }
    }
    fn agent_name(&self, id: &str) -> &str {
        self.agent_names.get(id).map_or("Agent", String::as_str)
    }
    fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        self.refresh_weekly(cx);
        self.refresh_snapshot(cx);
    }
    fn refresh_snapshot(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy || self.reading {
            return;
        }
        self.loaded = true;
        self.last_refresh = Some(Instant::now());
        self.reading = true;
        self.revision = self.revision.wrapping_add(1);
        let revision = self.revision;
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.reactive_snapshot().await });
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                this.reading = false;
                match result {
                    Ok(snapshot) => {
                        this.set_snapshot(snapshot, cx);
                        this.load_error = None;
                    }
                    Err(error) => this.load_error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn send(&mut self, command: ReactiveCommand, cx: &mut Context<'_, Self>) {
        self.send_in(command, Scope::General, cx);
    }
    fn send_in(&mut self, command: ReactiveCommand, scope: Scope, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.reading = false;
        self.error = None;
        self.scoped_error = None;
        self.hook = None;
        self.revision = self.revision.wrapping_add(1);
        let revision = self.revision;
        let effect = Effect::of(&command);
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.reactive_command(&command).await });
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(snapshot) => {
                        this.apply_effect(effect);
                        this.set_snapshot(snapshot, cx);
                    }
                    Err(error) => {
                        let message = reactive_error(error);
                        if scope == Scope::General {
                            this.error = Some(message);
                        } else {
                            this.scoped_error = Some((scope, message));
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn apply_effect(&mut self, effect: Effect) {
        match effect {
            Effect::CloseAutomation => self.editor = None,
            Effect::CloseReply(id) => {
                if self.reply.as_ref().is_some_and(|(open, _)| *open == id) {
                    self.reply = None;
                }
            }
            Effect::CloseMemory { agent_id, key } => {
                if self
                    .memory_edit
                    .as_ref()
                    .is_some_and(|edit| edit.agent_id == agent_id && edit.key == key)
                {
                    self.memory_edit = None;
                }
            }
            Effect::ResetNotifications => self.notify_form = None,
            Effect::None => {}
        }
    }
    fn command(
        &self,
        key: String,
        label: &str,
        command: ReactiveCommand,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        control(SharedString::from(key), label, self.busy)
            .on_activate(cx, move |this, _, cx| this.send(command.clone(), cx))
    }
    fn command_in(
        &self,
        key: String,
        label: &str,
        command: ReactiveCommand,
        scope: Scope,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        control(SharedString::from(key), label, self.busy).on_activate(cx, move |this, _, cx| {
            this.send_in(command.clone(), scope.clone(), cx)
        })
    }
    /// `command_in` drawn as a quiet text button.
    fn ghost_command_in(
        &self,
        key: String,
        label: &str,
        command: ReactiveCommand,
        scope: Scope,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        ghost_control(SharedString::from(key), label, self.busy)
            .on_activate(cx, move |this, _, cx| {
                this.send_in(command.clone(), scope.clone(), cx)
            })
    }
    fn scoped_error(&self, scope: &Scope) -> Option<Stateful<Div>> {
        let (_, message) = self.scoped_error.as_ref().filter(|(at, _)| at == scope)?;
        Some(
            div()
                .id(SharedString::from(format!(
                    "reactive-scoped-error-{scope:?}"
                )))
                .role(Role::Alert)
                .text_size(px(13.0))
                .text_color(theme::lychee_300())
                .child(message.clone()),
        )
    }
    fn open_agent(
        &mut self,
        agent: Option<ReactiveAgent>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.snapshot.is_none() {
            return;
        }
        self.advanced_open = true;
        let agent = agent.unwrap_or_else(|| ReactiveAgent {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            instructions: String::new(),
            memory: String::new(),
            model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
            project_path: String::new(),
            enabled: false,
        });
        self.editor = Some(Editor::Agent(AgentEditor {
            name: edit_input(&agent.name, false, window, cx),
            instructions: edit_input(&agent.instructions, true, window, cx),
            model: edit_input(&agent.model, false, window, cx),
            path: edit_input(&agent.project_path, false, window, cx),
            memory: edit_input(&agent.memory, true, window, cx),
            agent,
        }));
        self.error = None;
        cx.notify();
    }
    fn open_source(
        &mut self,
        source: Option<EventSource>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.snapshot.is_none() {
            return;
        }
        self.advanced_open = true;
        let source = source.unwrap_or_else(|| EventSource {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Scans".into(),
            enabled: false,
            kind: SourceKind::Folder {
                path: String::new(),
                pattern: "*.pdf".into(),
                recursive: true,
                settle_ms: 2000,
                poll_ms: 5000,
            },
        });
        let mut editor = SourceEditor {
            source: source.clone(),
            name: edit_input(&source.name, false, window, cx),
            kind: "folder",
            value: edit_input("", false, window, cx),
            cwd: edit_input("", false, window, cx),
            pattern: edit_input("*.pdf", false, window, cx),
            interval: edit_input("5", false, window, cx),
            settle: edit_input("2", false, window, cx),
            timeout: edit_input("30", false, window, cx),
            recursive: true,
            times: edit_input("08:00", false, window, cx),
            weekdays: Vec::new(),
            time_zone: edit_input(&editor_fields::local_zone(), false, window, cx),
        };
        let (kind, text) = match &source.kind {
            SourceKind::Folder {
                path,
                pattern,
                recursive,
                settle_ms,
                poll_ms,
            } => {
                editor
                    .pattern
                    .update(cx, |input, cx| input.set_value(pattern, window, cx));
                editor.recursive = *recursive;
                editor.settle.update(cx, |input, cx| {
                    input.set_value(seconds(*settle_ms), window, cx)
                });
                editor.interval.update(cx, |input, cx| {
                    input.set_value(seconds(*poll_ms), window, cx)
                });
                ("folder", path.as_str())
            }
            SourceKind::Http { url, poll_ms } => {
                editor.interval.update(cx, |input, cx| {
                    input.set_value(seconds(*poll_ms), window, cx)
                });
                ("http", url.as_str())
            }
            SourceKind::Command {
                command,
                cwd,
                poll_ms,
                timeout_ms,
            } => {
                editor
                    .cwd
                    .update(cx, |input, cx| input.set_value(cwd, window, cx));
                editor.interval.update(cx, |input, cx| {
                    input.set_value(seconds(*poll_ms), window, cx)
                });
                editor.timeout.update(cx, |input, cx| {
                    input.set_value(seconds(*timeout_ms), window, cx)
                });
                ("command", command.as_str())
            }
            SourceKind::Schedule { every_ms } => {
                editor.interval.update(cx, |input, cx| {
                    input.set_value(seconds(*every_ms), window, cx)
                });
                ("schedule", "")
            }
            SourceKind::Calendar {
                times,
                weekdays,
                time_zone,
            } => {
                editor.times.update(cx, |input, cx| {
                    input.set_value(times.join(", "), window, cx)
                });
                editor
                    .time_zone
                    .update(cx, |input, cx| input.set_value(time_zone, window, cx));
                editor.weekdays.clone_from(weekdays);
                ("calendar", "")
            }
            SourceKind::Webhook => ("webhook", ""),
            SourceKind::AppEvent { topic } => ("app_event", topic.as_str()),
            SourceKind::Custom => ("custom", ""),
        };
        editor.kind = kind;
        editor
            .value
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.editor = Some(Editor::Source(editor));
        self.error = None;
        cx.notify();
    }
    fn open_subscription(
        &mut self,
        subscription: Option<bruhs_models::reactive::Subscription>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.advanced_open = true;
        let subscription=subscription.unwrap_or_else(||bruhs_models::reactive::Subscription{id:uuid::Uuid::new_v4().to_string(),agent_id:self.snapshot.as_ref().and_then(|s|s.agents.first()).map(|a|a.id.clone()).unwrap_or_default(),source_id:self.snapshot.as_ref().and_then(|s|s.sources.first()).map(|a|a.id.clone()).unwrap_or_default(),instructions:"Read each new scan and prepare a draft with the payee, amount, due date, and payment reference. Flag missing or uncertain details. Do not make payments.".into(),enabled:false,auto_run:false,debounce_ms:0,throttle_ms:0,on_busy:BusyPolicy::Queue});
        self.editor = Some(Editor::Subscription(SubscriptionEditor {
            instructions: edit_input(&subscription.instructions, true, window, cx),
            debounce: edit_input(&seconds(subscription.debounce_ms), false, window, cx),
            throttle: edit_input(&seconds(subscription.throttle_ms), false, window, cx),
            subscription,
        }));
        self.error = None;
        cx.notify();
    }
    fn save_editor(&mut self, cx: &mut Context<'_, Self>) {
        let command = (|| -> Result<ReactiveCommand, String> {
            Ok(match self.editor.as_ref() {
                Some(Editor::Automation(_)) => {
                    return Err("Review the complete automation before enabling it.".into())
                }
                Some(Editor::Agent(editor)) => {
                    let mut agent = editor.agent.clone();
                    agent.name = value(&editor.name, cx);
                    agent.instructions = value(&editor.instructions, cx);
                    agent.model = value(&editor.model, cx);
                    agent.project_path = value(&editor.path, cx);
                    agent.memory = value(&editor.memory, cx);
                    apply(ReactiveChange::SaveAgent { agent })
                }
                Some(Editor::Source(editor)) => {
                    let mut source = editor.source.clone();
                    source.name = value(&editor.name, cx);
                    let text = value(&editor.value, cx);
                    source.kind = match editor.kind {
                        "folder" => SourceKind::Folder {
                            path: text,
                            pattern: value(&editor.pattern, cx),
                            recursive: editor.recursive,
                            settle_ms: milliseconds(&value(&editor.settle, cx), true)?,
                            poll_ms: milliseconds(&value(&editor.interval, cx), false)?,
                        },
                        "http" => SourceKind::Http {
                            url: text,
                            poll_ms: milliseconds(&value(&editor.interval, cx), false)?,
                        },
                        "command" => SourceKind::Command {
                            command: text,
                            cwd: value(&editor.cwd, cx),
                            poll_ms: milliseconds(&value(&editor.interval, cx), false)?,
                            timeout_ms: milliseconds(&value(&editor.timeout, cx), false)?,
                        },
                        "schedule" => SourceKind::Schedule {
                            every_ms: milliseconds(&value(&editor.interval, cx), false)?,
                        },
                        "calendar" => editor_fields::calendar_kind(
                            &value(&editor.times, cx),
                            &editor.weekdays,
                            &value(&editor.time_zone, cx),
                        )?,
                        "webhook" => SourceKind::Webhook,
                        "app_event" => SourceKind::AppEvent { topic: text },
                        "custom" => SourceKind::Custom,
                        _ => return Err("Choose a supported source type.".into()),
                    };
                    apply(ReactiveChange::SaveSource { source })
                }
                Some(Editor::Subscription(editor)) => {
                    let mut subscription = editor.subscription.clone();
                    subscription.instructions = value(&editor.instructions, cx);
                    subscription.debounce_ms = milliseconds(&value(&editor.debounce, cx), true)?;
                    subscription.throttle_ms = milliseconds(&value(&editor.throttle, cx), true)?;
                    apply(ReactiveChange::SaveSubscription { subscription })
                }
                Some(Editor::Chat { agent_id, message }) => ReactiveCommand::Chat {
                    agent_id: agent_id.clone(),
                    message: value(message, cx),
                },
                Some(Editor::Publish {
                    source_id,
                    key,
                    payload,
                }) => ReactiveCommand::Publish {
                    source_id: source_id.clone(),
                    key: value(key, cx),
                    payload: serde_json::from_str(&value(payload, cx))
                        .map_err(|error| format!("Check the event JSON: {error}"))?,
                },
                None => return Err("Open an editor first.".into()),
            })
        })();
        match command {
            Ok(command) => self.send(command, cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
    fn reveal_hook(&mut self, source: String, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.reading = false;
        self.revision = self.revision.wrapping_add(1);
        let revision = self.revision;
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.reactive_hook_token(&source).await });
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(token) => this.hook = Some(token),
                    Err(error) => this.error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    /// Web's Meeting event list: the topic is picked, not typed.
    fn render_topic_picker(&self, editor: &SourceEditor, cx: &Context<'_, Self>) -> Stateful<Div> {
        let current = editor.value.read(cx).value().to_string();
        div()
            .id("reactive-source-topics")
            .flex()
            .flex_wrap()
            .gap_2()
            .role(Role::Group)
            .aria_label("Meeting event")
            .children(
                bruhs_models::reactive::APP_EVENT_TOPICS
                    .into_iter()
                    .map(|(topic, label)| {
                        control(
                            SharedString::from(format!("reactive-source-topic-{topic}")),
                            label,
                            self.busy,
                        )
                        .when(current == topic, |button| {
                            button.bg(theme::blueberry_500().opacity(0.15))
                        })
                        .on_activate(cx, move |this, window, cx| {
                            if let Some(Editor::Source(editor)) = &mut this.editor {
                                editor
                                    .value
                                    .update(cx, |input, cx| input.set_value(topic, window, cx));
                            }
                            cx.notify();
                        })
                    }),
            )
    }

    fn render_editor(&self, cx: &Context<'_, Self>) -> Option<Div> {
        let editor = self.editor.as_ref()?;
        let mut body = card();
        match editor {
            Editor::Automation(editor) => return Some(self.render_automation_editor(editor, cx)),
            Editor::Agent(editor) => {
                body=body.child(heading("Agent settings")).child(field("Agent name", &editor.name, false, self.busy)).child(field("Model", &editor.model, false, self.busy)).child(field("Working folder", &editor.path, false, self.busy)).child(field("Purpose and instructions", &editor.instructions, true, self.busy)).child(field("Persistent memory", &editor.memory, true, self.busy)).child(muted("Memory is included in future turns. Save only what the agent should remember."))
                    .child(control("reactive-editor-agent-enabled",if editor.agent.enabled{"Agent enabled"}else{"Agent paused"},self.busy).on_activate(cx, |this,_,cx|{if let Some(Editor::Agent(editor))=&mut this.editor{editor.agent.enabled= !editor.agent.enabled;}cx.notify();}));
            }
            Editor::Source(editor) => {
                body = body
                    .child(heading("Event source settings"))
                    .child(field("Source name", &editor.name, false, self.busy))
                    .child(muted("Source type"))
                    .child(
                        div().flex().flex_wrap().gap_2().children(
                            [
                                ("folder", "Folder"),
                                ("http", "Website"),
                                ("command", "Command"),
                                ("schedule", "Schedule"),
                                ("calendar", "Calendar"),
                                ("webhook", "Webhook"),
                                ("app_event", "App event"),
                                ("custom", "Custom event"),
                            ]
                            .into_iter()
                            .map(|(kind, label)| {
                                control(
                                    SharedString::from(format!("reactive-source-kind-{kind}")),
                                    label,
                                    self.busy,
                                )
                                .when(editor.kind == kind, |button| {
                                    button.bg(theme::blueberry_500().opacity(0.15))
                                })
                                .on_activate(
                                    cx,
                                    move |this, _, cx| {
                                        if let Some(Editor::Source(editor)) = &mut this.editor {
                                            editor.kind = kind;
                                        }
                                        cx.notify();
                                    },
                                )
                            }),
                        ),
                    );
                body=match editor.kind {
                    "folder"=>body.child(field("Folder to watch", &editor.value, false, self.busy)).child(field("File pattern", &editor.pattern, false, self.busy)).child(field("File settle time in seconds", &editor.settle, false, self.busy)).child(control("reactive-source-recursive",if editor.recursive{"Include subfolders: On"}else{"Include subfolders: Off"},self.busy).on_activate(cx, |this,_,cx|{if let Some(Editor::Source(editor))=&mut this.editor{editor.recursive= !editor.recursive;}cx.notify();})),
                    "http"=>body.child(field("Website URL", &editor.value, false, self.busy)),
                    "command"=>body.child(field("Command to run", &editor.value, true, self.busy)).child(field("Command working folder", &editor.cwd, false, self.busy)).child(field("Command timeout in seconds", &editor.timeout, false, self.busy)).child(muted("This command runs on your machine when the source is enabled. Use a command you trust.")),
                    "app_event"=>body.child(muted("Meeting event")).child(self.render_topic_picker(editor, cx)),
                    "calendar"=>body.child(self.render_calendar_fields(editor, cx)),
                    "webhook"=>body.child(muted("Save the source, then reveal its private token from the source row.")),
                    "custom"=>body.child(muted("MQTT and Zigbee bridges can publish through the local API. Use a folder source for an FTP upload folder.")),
                    _=>body.child(muted("Run on a repeating interval while the local server is available.")),
                };
                if matches!(editor.kind, "folder" | "http" | "command" | "schedule") {
                    body = body.child(field(
                        "Interval in seconds",
                        &editor.interval,
                        false,
                        self.busy,
                    ));
                }
                body = body.child(
                    control(
                        "reactive-source-enabled",
                        if editor.source.enabled {
                            "Source enabled"
                        } else {
                            "Source paused"
                        },
                        self.busy,
                    )
                    .on_activate(cx, |this, _, cx| {
                        if let Some(Editor::Source(editor)) = &mut this.editor {
                            editor.source.enabled = !editor.source.enabled;
                        }
                        cx.notify();
                    }),
                );
            }
            Editor::Subscription(editor) => {
                body = body
                    .child(heading("Connect source to agent"))
                    .child(muted("Agent"));
                if let Some(snapshot) = &self.snapshot {
                    body = body
                        .child(div().flex().flex_wrap().gap_2().children(
                            snapshot.agents.iter().map(|agent| {
                                let id = agent.id.clone();
                                control(
                                    SharedString::from(format!("reactive-select-agent-{id}")),
                                    &agent.name,
                                    self.busy,
                                )
                                .when(editor.subscription.agent_id == id, |b| {
                                    b.bg(theme::blueberry_500().opacity(0.15))
                                })
                                .on_activate(
                                    cx,
                                    move |this, _, cx| {
                                        if let Some(Editor::Subscription(editor)) = &mut this.editor
                                        {
                                            editor.subscription.agent_id = id.clone();
                                        }
                                        cx.notify();
                                    },
                                )
                            }),
                        ))
                        .child(muted("Source"))
                        .child(div().flex().flex_wrap().gap_2().children(
                            snapshot.sources.iter().map(|source| {
                                let id = source.id.clone();
                                control(
                                    SharedString::from(format!("reactive-select-source-{id}")),
                                    &source.name,
                                    self.busy,
                                )
                                .when(editor.subscription.source_id == id, |b| {
                                    b.bg(theme::blueberry_500().opacity(0.15))
                                })
                                .on_activate(
                                    cx,
                                    move |this, _, cx| {
                                        if let Some(Editor::Subscription(editor)) = &mut this.editor
                                        {
                                            editor.subscription.source_id = id.clone();
                                        }
                                        cx.notify();
                                    },
                                )
                            }),
                        ));
                }
                body=body.child(field("What to do when this source reports an event", &editor.instructions, true, self.busy)).child(field("Debounce in seconds", &editor.debounce, false, self.busy)).child(field("Minimum gap between runs in seconds", &editor.throttle, false, self.busy))
                    .child(self.render_busy_choice(editor.subscription.on_busy, cx))
                    .child(control("reactive-sub-enabled",if editor.subscription.enabled{"Subscription enabled"}else{"Subscription paused"},self.busy).on_activate(cx, |this,_,cx|{if let Some(Editor::Subscription(editor))=&mut this.editor{editor.subscription.enabled= !editor.subscription.enabled;}cx.notify();}))
                    .child(control("reactive-sub-auto",if editor.subscription.auto_run{"Automatic read-only drafts: On"}else{"Review each event before running"},self.busy).on_activate(cx, |this,_,cx|{if let Some(Editor::Subscription(editor))=&mut this.editor{editor.subscription.auto_run= !editor.subscription.auto_run;}cx.notify();}))
                    .child(muted("Automatic drafts never grant write access. Leave them off to review each event first."));
            }
            Editor::Chat { agent_id, message } => {
                let name = self
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.agents.iter().find(|a| a.id == *agent_id))
                    .map_or("agent", |a| a.name.as_str());
                body=body.child(heading(format!("Chat with {name}"))).child(field("Message to agent", message, true, self.busy)).child(muted("Replies appear in activity below. The agent keeps its conversation across events and restarts."));
            }
            Editor::Publish {
                source_id,
                key,
                payload,
            } => {
                let name = self
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.sources.iter().find(|a| a.id == *source_id))
                    .map_or("custom source", |s| s.name.as_str());
                body = body
                    .child(heading(format!("Publish event to {name}")))
                    .child(field("Unique event key", key, false, self.busy))
                    .child(field("Event payload as JSON", payload, true, self.busy))
                    .child(muted(
                        "Reusing a key keeps the same event from running twice.",
                    ));
            }
        }
        let label = match editor {
            Editor::Automation(_) => "Review automation",
            Editor::Agent(_) => "Save agent",
            Editor::Source(_) => "Save source",
            Editor::Subscription(_) => "Save subscription",
            Editor::Chat { .. } => "Send message",
            Editor::Publish { .. } => "Publish event",
        };
        Some(
            body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        control("reactive-save-editor", label, self.busy)
                            .on_activate(cx, |this, _, cx| this.save_editor(cx)),
                    )
                    .child(
                        control("reactive-close-editor", "Close", self.busy).on_activate(
                            cx,
                            |this, _, cx| {
                                this.editor = None;
                                cx.notify();
                            },
                        ),
                    ),
            ),
        )
    }
}

impl EventEmitter<ReactivePreview> for ReactiveAutomations {}
impl EventEmitter<ReactiveProjectOpened> for ReactiveAutomations {}
impl EventEmitter<ReactiveArrival> for ReactiveAutomations {}

impl Render for ReactiveAutomations {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.last_render = Instant::now();
        if !self.loaded && cfg!(not(test)) {
            self.refresh(cx);
        }
        if !self.service.loaded && cfg!(not(test)) {
            self.load_service(cx);
        }
        let disabled = self.busy || self.snapshot.is_none();
        self.ensure_weekly_inputs(_window, cx);
        self.ensure_notification_form(_window, cx);
        let mut panel=div().id("reactive-automations").w_full().min_w_0().mb_4().p_4().rounded(px(8.0)).bg(theme::longan_900()).border_1().border_color(theme::longan_700().opacity(0.5)).flex().flex_col().gap_3()
            .child(div().flex().flex_wrap().items_center().justify_between().gap_2().child(div().flex().flex_wrap().items_center().gap_2().child(heading("Automations")).child(updates::badge_info("Drafts only"))).child(control("reactive-refresh","Refresh",self.busy||self.reading).on_activate(cx, |this,_,cx|this.refresh(cx))))
            .child(muted("Describe a job and choose what starts it. Review once to enable a complete automation, then pause or resume it with one control."))
            .child(muted("Watches and schedules keep working while the local server runs, even with this page closed."))
            // Web order: Updates sit inside the Automations card, under its introduction.
            .when(self.snapshot.is_some(), |panel| panel.child(self.render_updates(cx)))
            .when(self.snapshot.is_none()&&self.reading,|panel|panel.child(muted("Connecting to the local automation server…")))
            .when_some(self.load_error.clone(),|panel,error|panel.child(div().id("reactive-load-error").role(Role::Alert).text_size(px(13.0)).text_color(theme::lychee_300()).child(error)))
            .when_some(self.error.clone(),|panel,error|panel.child(div().id("reactive-error").role(Role::Alert).text_size(px(13.0)).text_color(theme::lychee_300()).child(error)))
            .child(div().flex().child(control("automation-new","New automation",disabled).on_activate(cx,|this,window,cx|this.open_automation(None,window,cx))));
        if matches!(
            &self.editor,
            Some(Editor::Automation(_) | Editor::Chat { .. })
        ) {
            if let Some(editor) = self.render_editor(cx) {
                panel = panel.child(editor);
            }
        }
        panel = panel.child(self.render_weekly_reports(cx));
        panel = panel.child(self.render_automation_cards(cx));
        // Both wait for the server: no settings or service state exists before it answers.
        if self.notify_form.is_some() {
            panel = panel.child(self.render_notifications(cx));
        }
        if let Some(service) = self.render_background_service(cx) {
            panel = panel.child(service);
        }
        panel = panel.child(
            control(
                "automation-advanced-settings",
                "Advanced settings",
                self.busy,
            )
            .aria_expanded(self.advanced_open)
            .on_activate(cx, |this, _, cx| {
                this.advanced_open = !this.advanced_open;
                cx.notify();
            }),
        );
        if let Some(snapshot) = &self.snapshot {
            if self.advanced_open {
                panel=panel.child(muted("Manage separate agents, sources, and subscriptions, or inspect private integration credentials."))
                .child(div().flex().flex_wrap().gap_2().child(control("reactive-new-agent","New agent",disabled).on_activate(cx, |this,window,cx|this.open_agent(None,window,cx))).child(control("reactive-scans","Watch scans",disabled).on_activate(cx, |this,window,cx|this.open_source(None,window,cx))).child(control("reactive-new-subscription","Connect source to agent",disabled).on_activate(cx, |this,window,cx|this.open_subscription(None,window,cx))));
                if !matches!(
                    &self.editor,
                    Some(Editor::Automation(_) | Editor::Chat { .. })
                ) {
                    if let Some(editor) = self.render_editor(cx) {
                        panel = panel.child(editor);
                    }
                }
                if snapshot.agents.is_empty() {
                    panel=panel.child(muted("Start with a bill assistant: create an agent, choose Watch scans, then connect the two. New agents and sources stay paused until you enable them."));
                }
                let on_cards = setup::automation_agent_ids(snapshot);
                for agent in &snapshot.agents {
                    let edit = agent.clone();
                    let mut toggle = agent.clone();
                    toggle.enabled = !toggle.enabled;
                    let chat_id = agent.id.clone();
                    panel =
                        panel.child(
                            card()
                                .child(heading(agent.name.clone()))
                                .child(muted(format!(
                                    "{}: {}",
                                    if agent.enabled { "Enabled" } else { "Paused" },
                                    agent.model
                                )))
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_2()
                                        .child(
                                            control(
                                                SharedString::from(format!(
                                                    "reactive-edit-agent-{}",
                                                    agent.id
                                                )),
                                                "Edit and memory",
                                                self.busy,
                                            )
                                            .on_activate(cx, move |this, window, cx| {
                                                this.open_agent(Some(edit.clone()), window, cx)
                                            }),
                                        )
                                        .child(
                                            control(
                                                SharedString::from(format!(
                                                    "reactive-chat-agent-{}",
                                                    agent.id
                                                )),
                                                "Chat",
                                                self.busy,
                                            )
                                            .on_activate(cx, move |this, window, cx| {
                                                this.editor = Some(Editor::Chat {
                                                    agent_id: chat_id.clone(),
                                                    message: edit_input("", true, window, cx),
                                                });
                                                cx.notify();
                                            }),
                                        )
                                        .child(self.command(
                                            format!("reactive-toggle-agent-{}", agent.id),
                                            if agent.enabled { "Pause" } else { "Enable" },
                                            apply(ReactiveChange::SaveAgent { agent: toggle }),
                                            cx,
                                        ))
                                        .child(self.command(
                                            format!("reactive-delete-agent-{}", agent.id),
                                            "Delete agent",
                                            apply(ReactiveChange::DeleteAgent {
                                                id: agent.id.clone(),
                                            }),
                                            cx,
                                        )),
                                )
                                .when(!on_cards.contains(&agent.id), |card| {
                                    card.child(self.render_agent_records(&agent.id, cx))
                                }),
                        );
                }
                panel = panel.child(heading("Event sources")).child(
                    control("reactive-new-source", "New event source", disabled)
                        .on_activate(cx, |this, window, cx| this.open_source(None, window, cx)),
                );
                for source in &snapshot.sources {
                    let edit = source.clone();
                    let mut toggle = source.clone();
                    toggle.enabled = !toggle.enabled;
                    let mut row = card()
                        .child(heading(source.name.clone()))
                        .child(muted(format!(
                            "{}: {}",
                            source_label(&source.kind),
                            if source.enabled { "Enabled" } else { "Paused" }
                        )))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    control(
                                        SharedString::from(format!(
                                            "reactive-edit-source-{}",
                                            source.id
                                        )),
                                        "Edit source",
                                        self.busy,
                                    )
                                    .on_activate(
                                        cx,
                                        move |this, window, cx| {
                                            this.open_source(Some(edit.clone()), window, cx)
                                        },
                                    ),
                                )
                                .child(self.command(
                                    format!("reactive-toggle-source-{}", source.id),
                                    if source.enabled { "Pause" } else { "Enable" },
                                    apply(ReactiveChange::SaveSource { source: toggle }),
                                    cx,
                                ))
                                .child(self.command(
                                    format!("reactive-delete-source-{}", source.id),
                                    "Delete source",
                                    apply(ReactiveChange::DeleteSource {
                                        id: source.id.clone(),
                                    }),
                                    cx,
                                )),
                        );
                    if matches!(source.kind, SourceKind::Webhook) {
                        let id = source.id.clone();
                        row = row.child(
                            control(
                                SharedString::from(format!("reactive-hook-{id}")),
                                "Show webhook token",
                                self.busy,
                            )
                            .on_activate(cx, move |this, _, cx| this.reveal_hook(id.clone(), cx)),
                        );
                    }
                    if matches!(source.kind, SourceKind::Custom) {
                        let id = source.id.clone();
                        row = row.child(
                            control(
                                SharedString::from(format!("reactive-publish-{id}")),
                                "Publish event",
                                self.busy,
                            )
                            .on_activate(
                                cx,
                                move |this, window, cx| {
                                    this.editor = Some(Editor::Publish {
                                        source_id: id.clone(),
                                        key: edit_input("", false, window, cx),
                                        payload: edit_input("{}", true, window, cx),
                                    });
                                    cx.notify();
                                },
                            ),
                        );
                    }
                    if let Some(error) = snapshot
                        .source_statuses
                        .iter()
                        .find(|status| status.source_id == source.id)
                        .and_then(|status| status.error.as_ref())
                    {
                        row = row.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::lychee_300())
                                .child(error.clone()),
                        );
                    }
                    panel = panel.child(row);
                }
                if let Some(hook) = &self.hook {
                    let credentials =
                        format!("{}\nAuthorization: Bearer {}", hook.path, hook.token);
                    let copy = credentials.clone();
                    panel = panel.child(
                        card()
                            .child(heading("Private webhook credentials"))
                            .child(muted(credentials))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        control("reactive-copy-hook", "Copy credentials", false)
                                            .on_activate(cx, move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    copy.clone(),
                                                ))
                                            }),
                                    )
                                    .child(
                                        control("reactive-hide-hook", "Hide token", false)
                                            .on_activate(cx, |this, _, cx| {
                                                this.hook = None;
                                                cx.notify();
                                            }),
                                    ),
                            ),
                    );
                }
                panel = panel.child(heading("Subscriptions"));
                for subscription in &snapshot.subscriptions {
                    let edit = subscription.clone();
                    let mut toggle = subscription.clone();
                    toggle.enabled = !toggle.enabled;
                    let agent = snapshot
                        .agents
                        .iter()
                        .find(|a| a.id == subscription.agent_id)
                        .map_or("Agent", |a| a.name.as_str());
                    let source = snapshot
                        .sources
                        .iter()
                        .find(|s| s.id == subscription.source_id)
                        .map_or("source", |s| s.name.as_str());
                    panel =
                        panel.child(
                            card()
                                .child(heading(format!("{agent} receives {source}")))
                                .child(muted(format!(
                                    "{}: {}",
                                    if subscription.enabled {
                                        "Enabled"
                                    } else {
                                        "Paused"
                                    },
                                    if subscription.auto_run {
                                        "Automatic read-only drafts"
                                    } else {
                                        "Review each event before running"
                                    }
                                )))
                                .when(subscription.on_busy == BusyPolicy::Restart, |card| {
                                    card.child(muted(format!(
                                        "While the agent is busy: {}",
                                        editor_fields::BUSY_RESTART_LABEL
                                    )))
                                })
                                .child(muted(subscription.instructions.clone()))
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_2()
                                        .child(
                                            control(
                                                SharedString::from(format!(
                                                    "reactive-edit-sub-{}",
                                                    subscription.id
                                                )),
                                                "Edit subscription",
                                                self.busy,
                                            )
                                            .on_activate(cx, move |this, window, cx| {
                                                this.open_subscription(
                                                    Some(edit.clone()),
                                                    window,
                                                    cx,
                                                )
                                            }),
                                        )
                                        .child(self.command(
                                            format!("reactive-toggle-sub-{}", subscription.id),
                                            if subscription.enabled {
                                                "Pause"
                                            } else {
                                                "Enable"
                                            },
                                            apply(ReactiveChange::SaveSubscription {
                                                subscription: toggle,
                                            }),
                                            cx,
                                        ))
                                        .child(self.command(
                                            format!("reactive-delete-sub-{}", subscription.id),
                                            "Delete subscription",
                                            apply(ReactiveChange::DeleteSubscription {
                                                id: subscription.id.clone(),
                                            }),
                                            cx,
                                        )),
                                ),
                        );
                }
            }
            for proposal in snapshot
                .proposals
                .iter()
                .filter(|p| p.status == ProposalStatus::Pending)
            {
                let approve_label = match &proposal.change {
                    ReactiveChange::SaveAutomation { automation } if automation.enabled() => {
                        "Enable automation"
                    }
                    ReactiveChange::SaveAutomation { .. } => "Save automation",
                    _ => "Approve setup change",
                };
                let label=match &proposal.change {ReactiveChange::SaveAutomation{automation}=>format!("{} {}. Review its complete purpose, source, model, folder, memory, and timing below. {} Writes still use workflow review.",if automation.enabled(){"Enable automation"}else{"Save paused automation"},automation.agent.name,if automation.subscription.auto_run{"Configured for automatic read-only drafts."}else{"Each event waits for review before a read-only draft runs."}),ReactiveChange::SaveAgent{agent}=>format!("Save agent {}. Review its instructions, model, folder, memory, and enabled state below.",agent.name),ReactiveChange::SaveSource{source}=>format!("Save {} source {}. Review all connection settings below.",source_label(&source.kind),source.name),ReactiveChange::SaveSubscription{..}=>"Connect an event source to an agent. Review its instructions and automatic-draft setting below.".into(),ReactiveChange::DeleteAgent{id}=>format!("Delete agent {id} and its subscriptions."),ReactiveChange::DeleteSource{id}=>format!("Delete source {id} and its subscriptions."),ReactiveChange::DeleteSubscription{id}=>format!("Delete subscription {id}."),ReactiveChange::SetMemory{agent_id,..}=>format!("Replace persistent memory for agent {agent_id}. Review the complete replacement below.")};
                let payload = serde_json::to_string_pretty(&proposal.change)
                    .unwrap_or_else(|error| format!("Cannot display proposal: {error}"));
                let mut proposal_card = card()
                    .child(heading("Setup proposal awaiting review"))
                    .child(muted(label));
                if let ReactiveChange::SaveAutomation { automation } = &proposal.change {
                    for (label, text) in setup::review_fields(automation) {
                        proposal_card = proposal_card.child(
                            div().flex().flex_col().gap_1().child(muted(label)).child(
                                div()
                                    .text_size(px(13.0))
                                    .line_height(relative(1.5))
                                    .text_color(theme::rambutan_200())
                                    .child(text),
                            ),
                        );
                    }
                    let key = format!("proposal:{}", proposal.id);
                    let expanded = self.expanded.contains(&key);
                    proposal_card = proposal_card.child(
                        control(
                            SharedString::from(format!(
                                "reactive-proposal-advanced-{}",
                                proposal.id
                            )),
                            "Advanced setup definition",
                            false,
                        )
                        .aria_expanded(expanded)
                        .on_activate(cx, move |this, _, cx| {
                            if !this.expanded.remove(&key) {
                                this.expanded.insert(key.clone());
                            }
                            cx.notify();
                        }),
                    );
                    if expanded {
                        proposal_card = proposal_card.child(muted(payload));
                    }
                } else {
                    proposal_card = proposal_card.child(muted(payload));
                }
                panel = panel.child(
                    proposal_card.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(self.command(
                                format!("reactive-approve-proposal-{}", proposal.id),
                                approve_label,
                                ReactiveCommand::DecideProposal {
                                    id: proposal.id.clone(),
                                    approve: true,
                                },
                                cx,
                            ))
                            .child(self.command(
                                format!("reactive-reject-proposal-{}", proposal.id),
                                "Decline",
                                ReactiveCommand::DecideProposal {
                                    id: proposal.id.clone(),
                                    approve: false,
                                },
                                cx,
                            )),
                    ),
                );
            }
            panel = panel.child(heading("Activity and conversations"));
            if snapshot.runs.is_empty() {
                panel = panel.child(muted(
                    "No runs yet. Chat with an agent or enable an automation.",
                ));
            }
            for run in &snapshot.runs {
                let name = snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.id == run.agent_id)
                    .map_or("Agent", |agent| agent.name.as_str());
                let expanded = self.expanded.contains(&run.id);
                let id = run.id.clone();
                let mut row = card()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            // Web: name, trigger and outcome on the left; status on the right.
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(heading(name.to_string()))
                                    .child(updates::badge_plain(runs::trigger_label(run)))
                                    .children(runs::outcome_label(run).map(updates::badge_info)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(theme::blueberry_200())
                                    .child(run_label(run.status)),
                            ),
                    )
                    .children(runs::run_notes(run).into_iter().map(runs::note))
                    .child(
                        div().flex().child(
                            ghost_control(
                                SharedString::from(format!("reactive-context-{}", run.id)),
                                if expanded {
                                    "Hide request and event context"
                                } else {
                                    "Request and event context"
                                },
                                false,
                            )
                            .aria_expanded(expanded)
                            .on_activate(cx, move |this, _, cx| {
                                if !this.expanded.remove(&id) {
                                    this.expanded.insert(id.clone());
                                }
                                cx.notify();
                            }),
                        ),
                    );
                if expanded {
                    row = row.child(muted(run.prompt.clone()));
                }
                if let Some(result) = &run.result {
                    let copy = result.clone();
                    row = row
                        .child(
                            div()
                                .text_size(px(13.0))
                                .line_height(relative(1.5))
                                .text_color(theme::rambutan_200())
                                .child(
                                    workflow_draft(result)
                                        .map_or_else(|| result.clone(), |draft| draft.summary),
                                ),
                        )
                        .child(
                            div().flex().child(
                                ghost_control(
                                    SharedString::from(format!("reactive-copy-run-{}", run.id)),
                                    "Copy draft",
                                    false,
                                )
                                .on_activate(
                                    cx,
                                    move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy.clone(),
                                        ))
                                    },
                                ),
                            ),
                        );
                }
                if let Some(draft) = run
                    .result
                    .as_deref()
                    .filter(|_| run.status == RunStatus::Completed)
                    .and_then(workflow_draft)
                {
                    let event = ReactivePreview {
                        title: draft.summary,
                        operations: draft.operations,
                        run_id: run.id.clone(),
                    };
                    row=row.child(control(SharedString::from(format!("reactive-preview-run-{}",run.id)),"Preview changes",self.busy).on_activate(cx,move |_,_,cx|cx.emit(event.clone()))).child(muted("Creates a separate reviewed workflow. This draft is not marked as applied."));
                }
                if let Some(error) = runs::run_error(run) {
                    row = row.child(
                        div()
                            .text_size(px(13.0))
                            .text_color(theme::lychee_300())
                            .child(error.to_owned()),
                    );
                }
                let mut actions = div().flex().flex_wrap().gap_2();
                if run.status == RunStatus::AwaitingReview {
                    actions = actions.child(self.command(
                        format!("reactive-approve-run-{}", run.id),
                        "Run read-only draft",
                        ReactiveCommand::ApproveRun { id: run.id.clone() },
                        cx,
                    ));
                }
                if matches!(
                    run.status,
                    RunStatus::AwaitingReview | RunStatus::Queued | RunStatus::Running
                ) {
                    actions = actions.child(self.command(
                        format!("reactive-cancel-run-{}", run.id),
                        "Cancel run",
                        ReactiveCommand::CancelRun { id: run.id.clone() },
                        cx,
                    ));
                }
                if matches!(
                    run.status,
                    RunStatus::Failed | RunStatus::Interrupted | RunStatus::Cancelled
                ) {
                    actions = actions.child(self.command(
                        format!("reactive-retry-run-{}", run.id),
                        "Retry with review",
                        ReactiveCommand::RetryRun { id: run.id.clone() },
                        cx,
                    ));
                }
                panel = panel.child(row.child(actions));
            }
        }
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::{
        milliseconds, reactive_error, seconds, value, Editor, ReactiveAutomations,
        ReactiveSnapshot, ServerError,
    };
    use crate::theme;
    use gpui::AppContext as _;
    use std::rc::Rc;
    #[test]
    fn native_interval_editor_round_trips_milliseconds_without_float_casts() {
        assert_eq!(milliseconds(&seconds(1250), false), Ok(1250));
        assert!(milliseconds("-1", false).is_err());
        assert!(milliseconds("0", false).is_err());
        assert_eq!(milliseconds("0", true), Ok(0));
    }
    #[test]
    fn native_error_keeps_server_validation_instead_of_an_opaque_status() {
        let error = ServerError::Status {
            request: "POST /api/reactive/commands".into(),
            code: reqwest::StatusCode::BAD_REQUEST,
            body: r#"{"error":"Choose an existing working folder."}"#.into(),
        };
        assert_eq!(reactive_error(error), "Choose an existing working folder.");
    }

    #[gpui::test]
    fn new_agent_editor_starts_paused_and_keeps_memory_independent_of_snapshots(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            gpui_component::Root::new(panel, window, cx)
        });
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, window, cx| {
            panel.snapshot = Some(ReactiveSnapshot::default());
            panel.loaded = true;
            panel.open_agent(None, window, cx);
            let Some(Editor::Agent(editor)) = &mut panel.editor else {
                panic!("agent editor");
            };
            assert!(!editor.agent.enabled);
            editor.memory.update(cx, |input, cx| {
                input.set_value("Bills need review before payment.", window, cx)
            });
            panel.snapshot = Some(ReactiveSnapshot::default());
            let Some(Editor::Agent(editor)) = &panel.editor else {
                panic!("agent editor");
            };
            assert_eq!(
                value(&editor.memory, cx),
                "Bills need review before payment."
            );
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("reactive-Save agent").is_some());
    }

    #[gpui::test]
    fn an_app_event_source_picks_its_topic_from_the_web_list(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            gpui_component::Root::new(panel, window, cx)
        });
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, window, cx| {
            panel.snapshot = Some(ReactiveSnapshot::default());
            panel.loaded = true;
            panel.open_source(None, window, cx);
            let Some(Editor::Source(editor)) = &mut panel.editor else {
                panic!("source editor");
            };
            editor.kind = "app_event";
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        for (_, label) in bruhs_models::reactive::APP_EVENT_TOPICS {
            assert!(
                cx.debug_bounds(Box::leak(format!("reactive-{label}").into_boxed_str()))
                    .is_some(),
                "{label} is offered"
            );
        }
        let summary = cx.debug_bounds("reactive-Summary ready").expect("summary");
        cx.simulate_click(summary.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        panel.read_with(cx, |panel, cx| {
            let Some(Editor::Source(editor)) = &panel.editor else {
                panic!("source editor");
            };
            assert_eq!(value(&editor.value, cx), "meeting.summary");
        });
    }
}
