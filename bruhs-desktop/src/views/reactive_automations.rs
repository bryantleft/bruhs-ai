//! The server owns event agents; this entity only edits and observes them.
use super::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::{
    app_state::spawn_tokio,
    server_client::{ReactiveHookToken, Screencast, ServerClient, ServerError},
    theme,
};
use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
use bruhs_models::crew::{
    agent_run_level, auto_run_note, crew_commands_ready, event_policy, event_policy_line,
    rooms_ready, run_reviewed_label, saved_agent_level, service_notice, AccessLevel, BruhState,
    ServiceNotice,
};
use bruhs_models::local_tools::{ComputerInput, ComputerRequest, ComputerState};
use bruhs_models::reactive::*;
use bruhs_ui::zoom::px;
mod bruh_page;
mod computer_view;
mod create_bruh;
mod editor_fields;
mod face_editor;
mod memory;
mod notifications;
mod room_dialog;
mod room_page;
mod runs;
mod setup;
mod transcript_list;
mod updates;
mod vault_grants;
mod weekly_reports;
pub(crate) use bruh_page::ReactiveOpenAutomations;
use bruhs_ui::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use bruhs_ui::text_field::{TextField, TextInput};
pub(crate) use create_bruh::ReactiveBruhCreated;
use gpui::{prelude::*, *};
pub(crate) use room_dialog::ReactiveRoomDone;
pub(crate) use room_page::ReactiveOpenRoomSettings;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    time::{Duration, Instant},
};
pub(crate) use updates::{unread_badge, unread_label};

/// The service's live crew frames: bruh states and its protocol.
const CREW_STREAM_PATH: &str = "/api/reactive/stream";
/// An older service is updated automatically at most once per launch.
static AUTO_UPDATE_TRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A local date and time for an instant, as the automation cards write it.
pub(crate) fn local_time(at: bruhs_models::EpochMs) -> String {
    updates::local_time(at)
}

/// Why ⌘. stopped nothing before the service ever answered.
const SERVICE_UNREACHABLE: &str = "the background service hasn't answered yet";

/// The auto run switch's label: the policy, and that it is on.
fn auto_run_control(auto_run: bool, level: AccessLevel) -> String {
    if auto_run {
        format!("{}: On", event_policy(true, level))
    } else {
        event_policy(false, level)
    }
}

/// The installed service's crew protocol. An installed service whose release
/// can't say counts as the oldest.
fn installed_protocol(service: &BackgroundService) -> Option<u32> {
    service
        .installed
        .then(|| service.reactive_protocol.unwrap_or(0))
}

/// How the app can bring an older service up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServiceUpdate {
    /// The current server installs itself as the login service.
    InstallFromServer,
    /// The bundle's own `bruhs-server service install`: the answering server
    /// is the old one, and installing through it would copy itself again.
    Bundled,
}

/// Whether `update` may run without a click: only a release build installs
/// itself. A debug server (a worktree's `cargo run`) or a debug app waits
/// for Update service.
fn auto_update_allowed(update: ServiceUpdate, serving_debug: bool, app_debug: bool) -> bool {
    match update {
        ServiceUpdate::InstallFromServer => !serving_debug,
        ServiceUpdate::Bundled => !app_debug,
    }
}

/// The update for `notice`, when this launch has one.
fn service_update(notice: ServiceNotice, bundled: bool) -> Option<ServiceUpdate> {
    match notice {
        ServiceNotice::InstalledOld => Some(ServiceUpdate::InstallFromServer),
        ServiceNotice::ServingOld => bundled.then_some(ServiceUpdate::Bundled),
    }
}

/// The crew as the rail shows it, kept current by the live stream.
#[derive(Default)]
struct CrewState {
    bruhs: Vec<BruhState>,
    /// Every room, as the service last reported them.
    rooms: Vec<bruhs_models::crew::room::RoomState>,
    /// The serving service's crew protocol, once a frame or snapshot says.
    protocol: Option<u32>,
    /// When a run last changed, as the latest frame says. A run that starts
    /// and ends between two frames leaves `bruhs` as it was.
    runs_at: Option<bruhs_models::EpochMs>,
    /// The outbox as the latest frame sums it up. A bruh's second approval
    /// in one run leaves `bruhs` and `runs_at` as they were.
    outbox: bruhs_models::reactive::OutboxMark,
    /// The installed login service's protocol; `None` when none is installed.
    installed_protocol: Option<u32>,
    /// The serving protocol the installed service was last read for.
    installed_read_for: Option<u32>,
    /// The answering server is a debug build.
    serving_debug: bool,
    /// The last Computer state read said this system runs no bruh browser.
    computer_unsupported: bool,
    stream: Option<Task<()>>,
    service_request: Option<Task<()>>,
    updating: bool,
    update_error: Option<String>,
}

// Match the web panel and existing connection status polling.
const REFRESH: Duration = Duration::from_secs(2);
// While the panel is off screen, keep the unread badge and arrival notices current.
const BACKGROUND_REFRESH: Duration = Duration::from_secs(15);
/// Web's `lg` breakpoint: Notifications and Background service side by side.
const SETTINGS_TWO_COLUMNS_PX: f32 = 1024.0;
/// Web `bg-longan-950/60 inset-ring-longan-700/50` around Delete agent's
/// and Delete room's questions.
const CONFIRM_FILL_OPACITY: f32 = 0.6;
const CONFIRM_RING_OPACITY: f32 = 0.5;

/// Delete agent asks first, inline in the agent's card, as Delete room
/// does.
struct AgentDelete {
    /// The agent whose Delete agent asked last. Its button holds
    /// `delete_focus`, so Keep can give focus back to it.
    agent_id: String,
    /// The question is showing.
    confirming: bool,
    delete_focus: FocusHandle,
    keep_focus: FocusHandle,
    new_agent_focus: FocusHandle,
    /// The agent went while its question showed: New agent takes focus
    /// on the next frame.
    focus_new_agent: bool,
}

struct AgentEditor {
    agent: ReactiveAgent,
    name: Entity<InputState>,
    instructions: Entity<TextareaState>,
    model: Entity<InputState>,
    path: Entity<InputState>,
    memory: Entity<TextareaState>,
}
struct SourceEditor {
    source: EventSource,
    name: Entity<InputState>,
    kind: &'static str,
    value: TextField,
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
    instructions: Entity<TextareaState>,
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
        message: Entity<TextareaState>,
    },
    Publish {
        source_id: String,
        key: Entity<InputState>,
        payload: Entity<TextareaState>,
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
    /// The crew rail.
    Crew,
    /// The create bruh dialog.
    CreateBruh,
    /// New room and Room settings.
    Room,
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
    reply: Option<(String, Entity<TextareaState>)>,
    /// Redraws while the reply is typed, so Send tracks whether it is empty.
    reply_watch: Option<gpui::Subscription>,
    memory_edit: Option<memory::MemoryEdit>,
    notify_form: Option<notifications::NotificationForm>,
    service: notifications::ServiceState,
    scoped_error: Option<(Scope, String)>,
    /// A draft-decision report is in flight (owned by the workspace reconciler).
    draft_reporting: bool,
    awareness: Option<gpui::Subscription>,
    crew: CrewState,
    create: Option<create_bruh::CreateBruh>,
    /// New room or Room settings, while open.
    room_dialog: Option<room_dialog::RoomDialog>,
    /// The bruh page on screen, while the workspace shows one.
    bruh: Option<bruh_page::BruhPage>,
    /// The room page on screen, while the workspace shows one.
    room: Option<room_page::RoomPage>,
    /// A room just made: its composer takes focus once its page shows.
    focus_room: Option<String>,
    /// Commands waiting for the one in flight, sent in order; a failure
    /// drops them, since they assumed it would apply.
    queued: VecDeque<(ReactiveCommand, Scope)>,
    /// Bumped whenever the snapshot changes.
    snapshot_rev: u64,
    agent_delete: AgentDelete,
    /// Every command sent, in order; tests never reach a server.
    #[cfg(test)]
    sent: Vec<ReactiveCommand>,
    /// Every Computer request, by agent id, in order.
    #[cfg(test)]
    computer_requests: Vec<(String, ComputerRequest)>,
    /// Every live view input, in order.
    #[cfg(test)]
    screencast_inputs: Vec<ComputerInput>,
    /// What each stop command of ⌘. fails with in a test.
    #[cfg(test)]
    stop_failure: Option<String>,
    /// What `send_now` fails with in a test.
    #[cfg(test)]
    send_failure: Option<String>,
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
        bruhs_ui::press::on_press_view(self, cx, move |this, window, cx| {
            if !this.busy {
                action(this, window, cx);
            }
        })
    }
}

fn edit_input(text: &str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx);
        input.set_value(text, window, cx);
        input
    })
}
/// An [`edit_input`] or [`edit_area`] chosen at run time.
fn edit_field(text: &str, multiline: bool, window: &mut Window, cx: &mut App) -> TextField {
    if multiline {
        edit_area(text, window, cx).into()
    } else {
        edit_input(text, window, cx).into()
    }
}
/// A multi-line [`edit_input`].
fn edit_area(text: &str, window: &mut Window, cx: &mut App) -> Entity<TextareaState> {
    cx.new(|cx| {
        let mut input = TextareaState::new(window, cx);
        input.set_value(text, window, cx);
        input
    })
}
fn value(input: &impl TextInput, cx: &App) -> String {
    input.text_value(cx).to_string()
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
fn field(label: &'static str, input: &impl TextInput, multiline: bool, disabled: bool) -> Div {
    div()
        .min_w_0()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(muted(label))
        .child(
            input
                .text_element(label.into(), disabled)
                .h(px(if multiline { 96.0 } else { 32.0 }))
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
    CloseMemory {
        agent_id: String,
        key: String,
    },
    ResetNotifications,
    /// Open the bruh this dialog made.
    OpenBruh(String),
    /// A room was made, saved or deleted: close the dialog that asked.
    RoomDone(String),
    /// A post reached the room: its draft won't come back.
    RoomPosted,
    None,
}
impl Effect {
    fn of(command: &ReactiveCommand) -> Self {
        match command {
            ReactiveCommand::CreateBruh { agent, .. } => Self::OpenBruh(agent.id.clone()),
            ReactiveCommand::CreateRoom { room } | ReactiveCommand::UpdateRoom { room } => {
                Self::RoomDone(room.id.clone())
            }
            ReactiveCommand::DeleteRoom { id } => Self::RoomDone(id.clone()),
            ReactiveCommand::PostRoom { .. } => Self::RoomPosted,
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
            crew: CrewState::default(),
            create: None,
            room_dialog: None,
            bruh: None,
            room: None,
            focus_room: None,
            queued: VecDeque::new(),
            snapshot_rev: 0,
            agent_delete: AgentDelete {
                agent_id: String::new(),
                confirming: false,
                delete_focus: cx.focus_handle().tab_index(0).tab_stop(true),
                keep_focus: cx.focus_handle().tab_index(0).tab_stop(true),
                new_agent_focus: cx.focus_handle().tab_index(0).tab_stop(true),
                focus_new_agent: false,
            },
            #[cfg(test)]
            sent: Vec::new(),
            #[cfg(test)]
            computer_requests: Vec::new(),
            #[cfg(test)]
            screencast_inputs: Vec::new(),
            #[cfg(test)]
            stop_failure: None,
            #[cfg(test)]
            send_failure: None,
        };
        if cfg!(not(test)) {
            panel.start_crew_stream(cx);
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
        self.crew.bruhs.clone_from(&snapshot.bruhs);
        self.crew.rooms.clone_from(&snapshot.rooms);
        self.note_crew_protocol(snapshot.protocol, cx);
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
        let snapshot = bruhs_models::weekly_reports::editable_automation_snapshot(snapshot);
        // Once the agent has gone, so have its card and buttons: the
        // question closes and focus moves to New agent, never to another
        // agent's Delete.
        let asking = &mut self.agent_delete;
        if asking.confirming
            && !snapshot
                .agents
                .iter()
                .any(|agent| agent.id == asking.agent_id)
        {
            asking.confirming = false;
            asking.focus_new_agent = true;
        }
        let changed = self.snapshot.as_ref() != Some(&snapshot);
        self.snapshot = Some(snapshot);
        if changed {
            self.snapshot_rev = self.snapshot_rev.wrapping_add(1);
        }
        // A bruh page reads its newest runs again whenever anything changed.
        if changed {
            self.reload_bruh_runs(cx);
        }
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
        // Tests never reach a server.
        if cfg!(test) {
            return;
        }
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
        #[cfg(test)]
        self.sent.push(command.clone());
        // Tests record the command, and any queued after it, and never reach
        // a real server.
        if cfg!(test) {
            if let Some((next, scope)) = self.queued.pop_front() {
                self.send_in(next, scope, cx);
            }
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
                        this.set_snapshot(snapshot, cx);
                        this.apply_effect(effect, cx);
                        if let Some((next, scope)) = this.queued.pop_front() {
                            this.send_in(next, scope, cx);
                        }
                    }
                    Err(error) => this.command_failed(scope, reactive_error(error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    /// The service refused a command: drop what was queued behind it, let
    /// the dialog that sent it try again, and show why where it was sent.
    fn command_failed(&mut self, scope: Scope, message: String) {
        self.queued.clear();
        // A post that failed, or was dropped from the queue, comes back.
        self.room_post_failed();
        if scope == Scope::CreateBruh {
            self.create_failed();
        }
        if scope == Scope::Room {
            self.room_failed();
        }
        if scope == Scope::General {
            self.error = Some(message);
        } else {
            self.scoped_error = Some((scope, message));
        }
    }
    /// Send `commands` in order, after any command already on its way.
    fn send_all(
        &mut self,
        commands: Vec<ReactiveCommand>,
        scope: Scope,
        cx: &mut Context<'_, Self>,
    ) {
        self.queued
            .extend(commands.into_iter().map(|command| (command, scope.clone())));
        if !self.busy {
            if let Some((next, scope)) = self.queued.pop_front() {
                self.send_in(next, scope, cx);
            }
        }
    }
    fn apply_effect(&mut self, effect: Effect, cx: &mut Context<'_, Self>) {
        match effect {
            Effect::OpenBruh(agent_id) => self.created_bruh(agent_id, cx),
            Effect::RoomDone(room_id) => self.room_done(&room_id, cx),
            Effect::RoomPosted => self.room_posted(),
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
    /// Delete agent: ask first, with focus on Keep, the safe choice.
    fn ask_delete_agent(
        &mut self,
        agent_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let asking = &mut self.agent_delete;
        agent_id.clone_into(&mut asking.agent_id);
        asking.confirming = true;
        asking.keep_focus.focus(window, cx);
        cx.notify();
    }
    /// Keep: drop the question, with focus back on Delete agent.
    fn keep_agent(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.agent_delete.confirming = false;
        self.agent_delete.delete_focus.focus(window, cx);
        cx.notify();
    }
    /// Delete agent's question in the agent's card: Delete goes ahead and
    /// Keep, which has focus, closes it.
    fn render_agent_delete(&self, agent_id: &str, question: String, cx: &Context<'_, Self>) -> Div {
        let delete = apply(ReactiveChange::DeleteAgent {
            id: agent_id.to_string(),
        });
        // Delete stays on while the delete is on its way, so focus stays
        // on it; `on_activate` sends only while nothing is in flight.
        let yes = web_button(
            "desktop-delete-agent-yes",
            WebButtonVariant::Destructive,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(|| "desktop-delete-agent-yes".to_string())
        .aria_label(bruhs_models::crew::DELETE_BRUH_YES)
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .child(bruhs_models::crew::DELETE_BRUH_YES)
        .on_activate(cx, move |this, _, cx| this.send(delete.clone(), cx));
        let keep = bruhs_ui::press::on_press_view(
            web_button(
                "desktop-delete-agent-keep",
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector(|| "desktop-delete-agent-keep".to_string())
            .aria_label(bruhs_models::crew::DELETE_BRUH_NO)
            .track_focus(&self.agent_delete.keep_focus)
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .child(bruhs_models::crew::DELETE_BRUH_NO),
            cx,
            |this, window, cx| this.keep_agent(window, cx),
        );
        div()
            .debug_selector(|| "desktop-delete-agent-confirm".to_string())
            .p_3()
            .rounded(px(8.0))
            .bg(theme::longan_950().opacity(CONFIRM_FILL_OPACITY))
            .border_1()
            .border_color(theme::longan_700().opacity(CONFIRM_RING_OPACITY))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_200())
                    .child(question),
            )
            .child(div().flex().items_center().gap_2().child(yes).child(keep))
    }
    /// What Delete agent asks, while it asks.
    fn agent_delete_question(&self) -> Option<String> {
        let asking = &self.agent_delete;
        if !asking.confirming {
            return None;
        }
        let agent = self
            .snapshot
            .as_ref()?
            .agents
            .iter()
            .find(|agent| agent.id == asking.agent_id)?;
        Some(bruhs_models::crew::delete_bruh_confirm(&agent.name))
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
            name: edit_input(&agent.name, window, cx),
            instructions: edit_area(&agent.instructions, window, cx),
            model: edit_input(&agent.model, window, cx),
            path: edit_input(&agent.project_path, window, cx),
            memory: edit_area(&agent.memory, window, cx),
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
            name: edit_input(&source.name, window, cx),
            kind: "folder",
            value: edit_field("", false, window, cx),
            cwd: edit_input("", window, cx),
            pattern: edit_input("*.pdf", window, cx),
            interval: edit_input("5", window, cx),
            settle: edit_input("2", window, cx),
            timeout: edit_input("30", window, cx),
            recursive: true,
            times: edit_input("08:00", window, cx),
            weekdays: Vec::new(),
            time_zone: edit_input(&editor_fields::local_zone(), window, cx),
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
        editor.value.set_value(text, window, cx);
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
            instructions: edit_area(&subscription.instructions, window, cx),
            debounce: edit_input(&seconds(subscription.debounce_ms), window, cx),
            throttle: edit_input(&seconds(subscription.throttle_ms), window, cx),
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
                    run_id: None,
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
        let current = editor.value.value(cx).to_string();
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
                                editor.value.set_value(topic, window, cx);
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
                body = body
                    .child(field(
                        "What to do when this source reports an event",
                        &editor.instructions,
                        true,
                        self.busy,
                    ))
                    .child(field(
                        "Debounce in seconds",
                        &editor.debounce,
                        false,
                        self.busy,
                    ))
                    .child(field(
                        "Minimum gap between runs in seconds",
                        &editor.throttle,
                        false,
                        self.busy,
                    ))
                    .child(self.render_busy_choice(editor.subscription.on_busy, cx))
                    .child(
                        control(
                            "reactive-sub-enabled",
                            if editor.subscription.enabled {
                                "Subscription enabled"
                            } else {
                                "Subscription paused"
                            },
                            self.busy,
                        )
                        .on_activate(cx, |this, _, cx| {
                            if let Some(Editor::Subscription(editor)) = &mut this.editor {
                                editor.subscription.enabled = !editor.subscription.enabled;
                            }
                            cx.notify();
                        }),
                    )
                    .child(
                        control(
                            "reactive-sub-auto",
                            &auto_run_control(
                                editor.subscription.auto_run,
                                saved_agent_level(
                                    self.snapshot.as_ref(),
                                    &editor.subscription.agent_id,
                                ),
                            ),
                            self.busy,
                        )
                        .on_activate(cx, |this, _, cx| {
                            if let Some(Editor::Subscription(editor)) = &mut this.editor {
                                editor.subscription.auto_run = !editor.subscription.auto_run;
                            }
                            cx.notify();
                        }),
                    )
                    .child(muted(auto_run_note(saved_agent_level(
                        self.snapshot.as_ref(),
                        &editor.subscription.agent_id,
                    ))));
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

/// The crew: live bruh states, the service's protocol, and updating an
/// older service.
impl ReactiveAutomations {
    /// Every bruh, as the service last reported them.
    pub(crate) fn crew(&self) -> &[BruhState] {
        &self.crew.bruhs
    }
    /// The serving service's crew protocol, once known.
    pub(crate) fn crew_protocol(&self) -> Option<u32> {
        self.crew.protocol
    }
    /// Whether the service understands the crew commands. Unknown counts as
    /// not yet.
    pub(crate) fn crew_ready(&self) -> bool {
        self.crew.protocol.is_some_and(crew_commands_ready)
    }
    /// Whether the service has rooms. Unknown counts as not yet.
    pub(crate) fn rooms_ready(&self) -> bool {
        self.crew.protocol.is_some_and(rooms_ready)
    }
    /// Why rooms are off, once the service is known to be older.
    pub(crate) fn rooms_reason(&self) -> Option<&'static str> {
        bruhs_models::crew::room::rooms_reason(self.crew.protocol)
    }
    /// Whether the service keeps vault grants. Unknown counts as not yet.
    pub(crate) fn vault_ready(&self) -> bool {
        self.crew
            .protocol
            .is_some_and(bruhs_models::crew::vault_ready)
    }
    /// Why vault grants are off, once the service is known to be older.
    /// Empty while they work.
    pub(crate) fn vault_reason(&self) -> Option<&'static str> {
        if self.vault_ready() {
            return None;
        }
        bruhs_models::crew::vault_reason(self.crew.protocol)
    }
    /// Whether the service runs bruh Computers. Unknown counts as not yet.
    pub(crate) fn computer_ready(&self) -> bool {
        self.crew
            .protocol
            .is_some_and(bruhs_models::crew::computer_ready)
    }
    /// Bruh `agent_id`'s Computer as the snapshot and the service have it.
    pub(crate) fn computer_gate(&self, agent_id: &str) -> bruhs_models::crew::ComputerGate {
        bruhs_models::crew::computer_gate(self.snapshot.as_ref(), agent_id, self.crew.protocol)
    }
    /// Whether the last Computer state read said this system runs no bruh
    /// browser.
    pub(crate) fn computer_unsupported(&self) -> bool {
        self.crew.computer_unsupported
    }
    /// A Computer state was read: Settings learns whether this system runs
    /// a bruh's browser.
    pub(crate) fn set_computer_unsupported(
        &mut self,
        unsupported: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if self.crew.computer_unsupported != unsupported {
            self.crew.computer_unsupported = unsupported;
            cx.notify();
        }
    }
    /// Ask for something of `agent_id`'s Computer: open or close its
    /// browser, take or hand back control, or open it in a window. `None`
    /// when the service doesn't run Computers, so nothing is sent; the task
    /// ends with the new state or why it failed.
    pub(crate) fn computer_request(
        &mut self,
        agent_id: &str,
        request: ComputerRequest,
        cx: &mut Context<'_, Self>,
    ) -> Option<Task<Result<ComputerState, String>>> {
        if !self.computer_ready() {
            return None;
        }
        #[cfg(test)]
        {
            let _ = cx;
            self.computer_requests.push((agent_id.to_string(), request));
            Some(Task::ready(Ok(ComputerState {
                agent_id: agent_id.to_string(),
                ..ComputerState::default()
            })))
        }
        #[cfg(not(test))]
        {
            let client = self.client.clone();
            let agent_id = agent_id.to_string();
            Some(spawn_tokio(cx, async move {
                client
                    .computer_request(&agent_id, &request)
                    .await
                    .map_err(reactive_error)
            }))
        }
    }
    /// Pass the owner's input to the live view, when one is open.
    pub(crate) fn screencast_input(
        &mut self,
        screencast: Option<&Screencast>,
        input: ComputerInput,
    ) {
        #[cfg(test)]
        self.screencast_inputs.push(input.clone());
        if let Some(screencast) = screencast {
            screencast.send(input);
        }
    }
    #[cfg(test)]
    pub(crate) fn computer_requests_for_test(&self) -> &[(String, ComputerRequest)] {
        &self.computer_requests
    }
    #[cfg(test)]
    pub(crate) fn screencast_inputs_for_test(&self) -> &[ComputerInput] {
        &self.screencast_inputs
    }
    /// Which older service to update, once the serving protocol is known.
    pub(crate) fn crew_notice(&self) -> Option<ServiceNotice> {
        service_notice(self.crew.protocol?, self.crew.installed_protocol)
    }
    /// The snapshot could not be read and no crew is known yet.
    pub(crate) fn crew_load_error(&self) -> Option<&str> {
        self.crew
            .protocol
            .is_none()
            .then_some(self.load_error.as_deref())
            .flatten()
    }
    /// Why the last crew command failed.
    pub(crate) fn crew_error(&self) -> Option<&str> {
        self.scoped_error
            .as_ref()
            .filter(|(scope, _)| *scope == Scope::Crew)
            .map(|(_, message)| message.as_str())
    }
    pub(crate) fn crew_busy(&self) -> bool {
        self.busy
    }
    /// The service update is running.
    pub(crate) fn service_updating(&self) -> bool {
        self.crew.updating
    }
    pub(crate) fn service_update_error(&self) -> Option<&str> {
        self.crew.update_error.as_deref()
    }
    /// Whether this launch can update the older service itself.
    pub(crate) fn can_update_service(&self) -> bool {
        self.crew_notice()
            .and_then(|notice| service_update(notice, crate::helpers::bundled()))
            .is_some()
    }
    /// Send a command from the crew rail; a failure shows in the rail.
    pub(crate) fn send_crew(&mut self, command: ReactiveCommand, cx: &mut Context<'_, Self>) {
        self.send_in(command, Scope::Crew, cx);
    }
    /// Send `command` at once, beside any command in flight, for a caller
    /// that must hear how it went, such as `message_bruh`. The task ends
    /// with the server's error, if any.
    pub(crate) fn send_now(
        &mut self,
        command: ReactiveCommand,
        cx: &mut Context<'_, Self>,
    ) -> Task<Result<(), String>> {
        #[cfg(test)]
        {
            let _ = cx;
            self.sent.push(command);
            Task::ready(self.send_failure.clone().map_or(Ok(()), Err))
        }
        #[cfg(not(test))]
        {
            let client = self.client.clone();
            let sending = spawn_tokio(cx, async move {
                client
                    .reactive_command(&command)
                    .await
                    .map(|_| ())
                    .map_err(reactive_error)
            });
            cx.spawn(async move |this, cx| {
                let result = sending.await;
                let _ = this.update(cx, |this, cx| this.refresh_snapshot(cx));
                result
            })
        }
    }
    /// Stop every bruh: one command to a current service, a cancel per live
    /// run to an older one, going on past a cancel that fails. Sent at once,
    /// beside any command in flight: Stop never waits in the queue, where a
    /// failure ahead of it would drop it. The task ends with what ⌘. says.
    pub(crate) fn stop_all_crew(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) -> Task<Result<&'static str, String>> {
        let Some(protocol) = self
            .crew
            .protocol
            .or_else(|| self.snapshot.as_ref().map(|snapshot| snapshot.protocol))
        else {
            // Nothing has answered yet, so nothing can be stopped: say so
            // rather than claim a stop that never went out.
            return Task::ready(bruhs_models::crew::stop_all_result(&[
                SERVICE_UNREACHABLE.to_string()
            ]));
        };
        let runs = self
            .snapshot
            .as_ref()
            .map_or(&[][..], |snapshot| snapshot.runs.as_slice());
        let commands = bruhs_models::crew::stop_commands(protocol, runs);
        #[cfg(test)]
        {
            let _ = cx;
            self.sent.extend(commands.iter().cloned());
            let failed: Vec<String> = self
                .stop_failure
                .iter()
                .cloned()
                .cycle()
                .take(commands.len())
                .collect();
            Task::ready(bruhs_models::crew::stop_all_result(&failed))
        }
        #[cfg(not(test))]
        {
            let client = self.client.clone();
            let sending = spawn_tokio(cx, async move {
                bruhs_models::crew::send_stop_commands(commands, |command| {
                    let client = client.clone();
                    async move {
                        client
                            .reactive_command(&command)
                            .await
                            .map(|_| ())
                            .map_err(reactive_error)
                    }
                })
                .await
            });
            cx.spawn(async move |this, cx| {
                let failed = sending.await;
                let _ = this.update(cx, |this, cx| this.refresh_snapshot(cx));
                bruhs_models::crew::stop_all_result(&failed)
            })
        }
    }
    /// Hold the panel as if a command were in flight.
    #[cfg(test)]
    pub(crate) fn set_busy_for_test(&mut self, busy: bool) {
        self.busy = busy;
    }
    /// Make every `send_now` fail with `error`.
    #[cfg(test)]
    pub(crate) fn fail_sends_for_test(&mut self, error: &str) {
        self.send_failure = Some(error.to_string());
    }
    /// Make every stop command of ⌘. fail with `error`.
    #[cfg(test)]
    pub(crate) fn fail_stops_for_test(&mut self, error: &str) {
        self.stop_failure = Some(error.to_string());
    }
    /// Read the crew again, such as after a failed load.
    pub(crate) fn reload_crew(&mut self, cx: &mut Context<'_, Self>) {
        self.refresh_snapshot(cx);
    }
    /// Show `states` as if the service had sent them.
    #[cfg(test)]
    pub(crate) fn show_crew_for_test(
        &mut self,
        states: Vec<BruhState>,
        protocol: u32,
        cx: &mut Context<'_, Self>,
    ) {
        self.crew.bruhs = states;
        self.crew.protocol = Some(protocol);
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn sent_for_test(&self) -> &[ReactiveCommand] {
        &self.sent
    }
    /// Refuse the last command sent, from the room dialog, as the service
    /// would.
    #[cfg(test)]
    pub(crate) fn refuse_room_for_test(&mut self, error: &str, cx: &mut Context<'_, Self>) {
        self.command_failed(Scope::Room, error.to_string());
        cx.notify();
    }
    /// Answer the last command sent as the service would on success:
    /// `snapshot`, then what the command closes or opens.
    #[cfg(test)]
    pub(crate) fn answer_last_for_test(
        &mut self,
        snapshot: ReactiveSnapshot,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_snapshot(snapshot, cx);
        if let Some(command) = self.sent.last() {
            let effect = Effect::of(command);
            self.apply_effect(effect, cx);
        }
        cx.notify();
    }

    /// Follow the service's live crew frames for as long as the panel lives.
    fn start_crew_stream(&mut self, cx: &mut Context<'_, Self>) {
        let Some(runtime) = cx
            .try_global::<crate::app_state::DesktopState>()
            .map(|state| std::sync::Arc::clone(&state.runtime))
        else {
            return;
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<
            crate::server_client::SseMessage<OutboxSignal>,
        >();
        runtime.spawn(
            std::sync::Arc::new(self.client.clone())
                .run_sse::<OutboxSignal>(CREW_STREAM_PATH.to_string(), tx),
        );
        self.crew.stream = Some(cx.spawn(async move |this, cx| {
            while let Some(message) = rx.recv().await {
                let crate::server_client::SseMessage::Event(signal) = message else {
                    continue;
                };
                if this
                    .update(cx, |this, cx| this.apply_crew_signal(signal, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
    fn apply_crew_signal(&mut self, signal: OutboxSignal, cx: &mut Context<'_, Self>) {
        let mut changed = false;
        let outbox = signal.outbox_mark();
        if self.crew.bruhs != signal.bruhs {
            self.crew.bruhs = signal.bruhs;
            cx.emit(ReactiveCrewChanged);
            cx.notify();
            changed = true;
        }
        if self.crew.rooms != signal.rooms {
            self.crew.rooms = signal.rooms;
            cx.notify();
            changed = true;
        }
        if self.crew.runs_at != signal.runs_at {
            self.crew.runs_at = signal.runs_at;
            changed = true;
        }
        if self.crew.outbox != outbox {
            self.crew.outbox = outbox;
            changed = true;
        }
        // A bruh or room page reads the crew again when its bruhs, rooms,
        // runs or outbox change, like the web's.
        if changed && (self.bruh.is_some() || self.room.is_some()) {
            self.refresh_snapshot(cx);
        }
        self.note_crew_protocol(signal.protocol, cx);
    }
    /// Record the serving protocol; a new one reads the installed service
    /// again.
    fn note_crew_protocol(&mut self, protocol: u32, cx: &mut Context<'_, Self>) {
        if self.crew.protocol != Some(protocol) {
            self.crew.protocol = Some(protocol);
            cx.notify();
        }
        if self.crew.installed_read_for != Some(protocol) && cfg!(not(test)) {
            self.crew.installed_read_for = Some(protocol);
            self.read_installed_service(cx);
        }
    }
    fn read_installed_service(&mut self, cx: &mut Context<'_, Self>) {
        if self.crew.updating {
            return;
        }
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.background_service().await });
        self.crew.service_request = Some(cx.spawn(async move |this, cx| {
            let status = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.crew.updating {
                    return;
                }
                if let Ok(status) = status {
                    this.crew.installed_protocol = installed_protocol(&status);
                    this.crew.serving_debug = status.debug_build;
                    this.auto_update_service(cx);
                }
                cx.notify();
            });
        }));
    }
    /// An older service is updated once per launch; the notice and its
    /// button stay for when that fails.
    fn auto_update_service(&mut self, cx: &mut Context<'_, Self>) {
        let allowed = self
            .crew_notice()
            .and_then(|notice| service_update(notice, crate::helpers::bundled()))
            .is_some_and(|update| {
                auto_update_allowed(update, self.crew.serving_debug, cfg!(debug_assertions))
            });
        if allowed && !AUTO_UPDATE_TRIED.swap(true, std::sync::atomic::Ordering::AcqRel) {
            self.update_service(cx);
        }
    }
    /// Bring the older service up to date: the bundle's own server reinstalls
    /// the service when the answering one is old; otherwise the current
    /// server installs itself.
    pub(crate) fn update_service(&mut self, cx: &mut Context<'_, Self>) {
        let Some(notice) = self.crew_notice() else {
            return;
        };
        if self.crew.updating {
            return;
        }
        let Some(update) = service_update(notice, crate::helpers::bundled()) else {
            self.crew.update_error = Some(crate::helpers::SERVICE_INSTALL_HINT.to_string());
            cx.notify();
            return;
        };
        self.crew.updating = true;
        self.crew.update_error = None;
        let client = self.client.clone();
        let operation: Task<Result<Option<BackgroundService>, String>> = match update {
            ServiceUpdate::InstallFromServer => spawn_tokio(cx, async move {
                client
                    .set_background_service(BackgroundServiceAction::Install)
                    .await
                    .map(Some)
                    .map_err(reactive_error)
            }),
            ServiceUpdate::Bundled => cx.background_spawn(async move {
                crate::helpers::update_bundled_service().map(|()| None)
            }),
        };
        self.crew.service_request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.crew.updating = false;
                match result {
                    Ok(Some(status)) => {
                        this.crew.installed_protocol = installed_protocol(&status);
                    }
                    // The service restarted on the new release: read it again.
                    Ok(None) => {
                        this.crew.installed_read_for = None;
                        this.refresh_snapshot(cx);
                    }
                    Err(error) => this.crew.update_error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl EventEmitter<ReactivePreview> for ReactiveAutomations {}
impl EventEmitter<ReactiveProjectOpened> for ReactiveAutomations {}
impl EventEmitter<ReactiveArrival> for ReactiveAutomations {}
impl EventEmitter<ReactiveBruhCreated> for ReactiveAutomations {}
impl EventEmitter<ReactiveRoomDone> for ReactiveAutomations {}
impl EventEmitter<ReactiveOpenAutomations> for ReactiveAutomations {}
impl EventEmitter<ReactiveOpenRoomSettings> for ReactiveAutomations {}
impl EventEmitter<ReactiveCrewChanged> for ReactiveAutomations {}

/// A live crew frame changed some bruh's state, so a run may have ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactiveCrewChanged;

impl Render for ReactiveAutomations {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.last_render = Instant::now();
        // Taken on every frame: only the panel's own view draws New agent.
        let focus_new_agent = std::mem::take(&mut self.agent_delete.focus_new_agent);
        if !self.loaded && cfg!(not(test)) {
            self.refresh(cx);
        }
        if !self.service.loaded && cfg!(not(test)) {
            self.load_service(cx);
        }
        if let Some(agent_id) = self.bruh_page_agent().map(str::to_string) {
            self.ensure_bruh_page(_window, cx);
            self.sync_bruh_stored(cx);
            return self
                .render_bruh_page(&agent_id, _window, cx)
                .into_any_element();
        }
        if let Some(room_id) = self.room_page_id().map(str::to_string) {
            self.ensure_room_page(_window, cx);
            return self
                .render_room_page(&room_id, _window, cx)
                .into_any_element();
        }
        if focus_new_agent && self.advanced_open {
            self.agent_delete.new_agent_focus.focus(_window, cx);
        }
        let disabled = self.busy || self.snapshot.is_none();
        self.ensure_weekly_inputs(_window, cx);
        self.ensure_notification_form(_window, cx);
        // Web `lg:grid-cols-2`: Notifications beside Background service.
        let settings_columns =
            if bruhs_ui::zoom::css_px(_window.viewport_size().width) >= SETTINGS_TWO_COLUMNS_PX {
                2
            } else {
                1
            };
        // Web order: the introduction with Refresh under it, Updates, Weekly
        // engineering reports, then New automation and its automations, then
        // Notifications and Background service.
        let mut panel=div().id("reactive-automations").w_full().min_w_0().mb_4().p_4().rounded(px(8.0)).bg(theme::longan_900()).border_1().border_color(theme::longan_700().opacity(0.5)).flex().flex_col().gap_3()
            .child(div().flex().flex_wrap().items_center().gap_2().child(heading("Automations")).child(updates::badge_info("Drafts only")))
            .child(
                div()
                    .debug_selector(|| "reactive-introduction".to_string())
                    .min_w_0()
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(13.0))
                    .line_height(relative(1.5))
                    .text_color(theme::rambutan_400())
                    .child("Describe a job and choose what starts it. Review once to enable a complete automation, then pause or resume it with one control."),
            )
            .child(muted("Watches and schedules keep working while the local server runs, even with this page closed.").text_color(theme::rambutan_500()))
            .child(div().flex().child(ghost_control("reactive-refresh","Refresh",self.busy||self.reading).on_activate(cx, |this,_,cx|this.refresh(cx))))
            .when(self.snapshot.is_none()&&self.reading,|panel|panel.child(muted("Connecting to the local automation server…")))
            .when_some(self.load_error.clone(),|panel,error|panel.child(div().id("reactive-load-error").role(Role::Alert).text_size(px(13.0)).text_color(theme::lychee_300()).child(error)))
            .when_some(self.error.clone(),|panel,error|panel.child(div().id("reactive-error").role(Role::Alert).text_size(px(13.0)).text_color(theme::lychee_300()).child(error)))
            .when(self.snapshot.is_some(), |panel| panel.child(self.render_updates(cx)))
            .child(self.render_weekly_reports(cx))
            .child(div().flex().child(control("automation-new","New automation",disabled).on_activate(cx,|this,window,cx|this.open_automation(None,window,cx))));
        if matches!(&self.editor, Some(Editor::Automation(_))) {
            if let Some(editor) = self.render_editor(cx) {
                panel = panel.child(editor);
            }
        }
        panel = panel.child(self.render_automation_cards_for(None, cx));
        // Both wait for the server: no settings or service state exists before it answers.
        let notifications = self
            .notify_form
            .is_some()
            .then(|| self.render_notifications(cx));
        let service = self.render_background_service(cx);
        if notifications.is_some() || service.is_some() {
            panel = panel.child(
                div()
                    .debug_selector(|| "reactive-settings-cards".to_string())
                    .grid()
                    .grid_cols(settings_columns)
                    .gap_4()
                    .children(notifications)
                    .children(service),
            );
        }
        // Web: an agent's chat opens under the settings cards.
        if matches!(&self.editor, Some(Editor::Chat { .. })) {
            if let Some(editor) = self.render_editor(cx) {
                panel = panel.child(editor);
            }
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
                .child(div().flex().flex_wrap().gap_2().child(control("reactive-new-agent","New agent",disabled).track_focus(&self.agent_delete.new_agent_focus).on_activate(cx, |this,window,cx|this.open_agent(None,window,cx))).child(control("reactive-scans","Watch scans",disabled).on_activate(cx, |this,window,cx|this.open_source(None,window,cx))).child(control("reactive-new-subscription","Connect source to agent",disabled).on_activate(cx, |this,window,cx|this.open_subscription(None,window,cx))));
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
                let question = self.agent_delete_question();
                for agent in &snapshot.agents {
                    let edit = agent.clone();
                    let mut toggle = agent.clone();
                    toggle.enabled = !toggle.enabled;
                    let chat_id = agent.id.clone();
                    let asking = self.agent_delete.agent_id == agent.id;
                    let confirming = asking && self.agent_delete.confirming;
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
                                                    message: edit_area("", window, cx),
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
                                        .when(!confirming, |row| {
                                            let id = agent.id.clone();
                                            row.child(
                                                control(
                                                    SharedString::from(format!(
                                                        "reactive-delete-agent-{}",
                                                        agent.id
                                                    )),
                                                    "Delete agent",
                                                    self.busy,
                                                )
                                                .when(asking, |el| {
                                                    el.track_focus(&self.agent_delete.delete_focus)
                                                })
                                                .on_activate(cx, move |this, window, cx| {
                                                    this.ask_delete_agent(&id, window, cx)
                                                }),
                                            )
                                        }),
                                )
                                .when(confirming, |card| {
                                    card.child(self.render_agent_delete(
                                        &agent.id,
                                        question.clone().unwrap_or_default(),
                                        cx,
                                    ))
                                })
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
                                        key: edit_input("", window, cx),
                                        payload: edit_area("{}", window, cx),
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
                                    event_policy(
                                        subscription.auto_run,
                                        saved_agent_level(
                                            self.snapshot.as_ref(),
                                            &subscription.agent_id
                                        )
                                    )
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
                let level = match &proposal.change {
                    ReactiveChange::SaveAutomation { automation } => agent_run_level(
                        self.snapshot.as_ref(),
                        &automation.agent.id,
                        &automation.agent.model,
                    ),
                    _ => AccessLevel::ReadOnly,
                };
                let label=match &proposal.change {ReactiveChange::SaveAutomation{automation}=>format!("{} {}. Review its complete purpose, source, model, folder, memory, and timing below. {}",if automation.enabled(){"Enable automation"}else{"Save paused automation"},automation.agent.name,event_policy_line(automation.subscription.auto_run,level)),ReactiveChange::SaveAgent{agent}=>format!("Save agent {}. Review its instructions, model, folder, memory, and enabled state below.",agent.name),ReactiveChange::SaveSource{source}=>format!("Save {} source {}. Review all connection settings below.",source_label(&source.kind),source.name),ReactiveChange::SaveSubscription{..}=>"Connect an event source to an agent. Review its instructions and automatic run setting below.".into(),ReactiveChange::DeleteAgent{id}=>bruhs_models::crew::delete_agent_line_for(self.snapshot.as_ref(),id),ReactiveChange::DeleteSource{id}=>format!("Delete source {id} and its subscriptions."),ReactiveChange::DeleteSubscription{id}=>format!("Delete subscription {id}."),ReactiveChange::SetMemory{agent_id,..}=>format!("Replace persistent memory for agent {agent_id}. Review the complete replacement below.")};
                let payload = serde_json::to_string_pretty(&proposal.change)
                    .unwrap_or_else(|error| format!("Cannot display proposal: {error}"));
                let mut proposal_card = card()
                    .child(heading("Setup proposal awaiting review"))
                    .child(muted(label));
                if let ReactiveChange::SaveAutomation { automation } = &proposal.change {
                    for (label, text) in setup::review_fields(automation, level) {
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
                    let label = run_reviewed_label(saved_agent_level(
                        self.snapshot.as_ref(),
                        &run.agent_id,
                    ));
                    actions = actions.child(self.command(
                        format!("reactive-approve-run-{}", run.id),
                        &label,
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
        panel.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        milliseconds, reactive_error, seconds, value, Editor, ReactiveAutomations,
        ReactiveSnapshot, ServerError,
    };
    use crate::theme;
    use gpui::{AppContext as _, Entity, VisualTestContext};
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
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
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

    /// One Space or Enter on a focused toggle flips it once, as a click
    /// does: the keyboard's own click never runs the action a second time.
    #[gpui::test]
    fn a_key_press_on_a_toggle_flips_it_once(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, window, cx| {
            panel.snapshot = Some(ReactiveSnapshot::default());
            panel.loaded = true;
            panel.open_agent(None, window, cx);
        });
        cx.run_until_parked();
        let enabled = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
            panel.read_with(cx, |panel, _| match &panel.editor {
                Some(Editor::Agent(editor)) => editor.agent.enabled,
                _ => panic!("agent editor"),
            })
        };
        let toggle = cx
            .debug_bounds("reactive-Agent paused")
            .expect("the toggle");
        cx.simulate_click(toggle.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(enabled(&panel, cx), "a click flips it once");
        for key in ["space", "enter"] {
            let before = enabled(&panel, cx);
            let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.simulate_event(gpui::KeyUpEvent { keystroke });
            cx.run_until_parked();
            assert_ne!(enabled(&panel, cx), before, "{key} flips it once");
        }
    }

    #[gpui::test]
    fn an_app_event_source_picks_its_topic_from_the_web_list(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
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

#[cfg(test)]
mod computer_tests {
    use super::ReactiveAutomations;
    use crate::theme;
    use bruhs_models::crew::{delete_agent_line, delete_bruh_confirm, ServiceNotice};
    use bruhs_models::local_tools::{ComputerInput, ComputerRequest};
    use bruhs_models::reactive::{
        ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveSnapshot,
    };
    use gpui::{AppContext as _, Entity, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn agent(id: &str, name: &str) -> ReactiveAgent {
        ReactiveAgent {
            id: id.into(),
            name: name.into(),
            instructions: String::new(),
            memory: String::new(),
            model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
            project_path: String::new(),
            enabled: true,
        }
    }

    fn snapshot(agents: Vec<ReactiveAgent>, protocol: u32) -> ReactiveSnapshot {
        ReactiveSnapshot {
            agents,
            protocol,
            ..ReactiveSnapshot::default()
        }
    }

    /// A standalone panel window (F13) with Advanced settings open.
    fn open(
        cx: &mut gpui::TestAppContext,
        snapshot: ReactiveSnapshot,
    ) -> (Entity<ReactiveAutomations>, &mut VisualTestContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update(cx, |panel, cx| {
            panel.loaded = true;
            panel.advanced_open = true;
            panel.set_snapshot(snapshot, cx);
        });
        cx.run_until_parked();
        (panel, cx)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).expect(selector);
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn delete_agent_asks_first_and_names_what_goes(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, snapshot(vec![agent("a", "Ada")], 3));
        assert!(cx.debug_bounds("desktop-delete-agent-confirm").is_none());
        click(cx, "reactive-Delete agent");
        assert!(cx.debug_bounds("desktop-delete-agent-confirm").is_some());
        assert!(
            cx.debug_bounds("reactive-Delete agent").is_none(),
            "the question takes the button's place"
        );
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.agent_delete_question(),
                Some(delete_bruh_confirm("Ada"))
            );
            assert!(panel.sent_for_test().is_empty(), "nothing yet");
        });
        let focus = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
            panel.update_in(cx, |panel, window, _| {
                let asking = &panel.agent_delete;
                (
                    asking.delete_focus.is_focused(window),
                    asking.keep_focus.is_focused(window),
                    asking.new_agent_focus.is_focused(window),
                )
            })
        };
        assert_eq!(focus(&panel, cx), (false, true, false), "focus is on Keep");
        click(cx, "desktop-delete-agent-keep");
        assert!(cx.debug_bounds("desktop-delete-agent-confirm").is_none());
        assert_eq!(
            focus(&panel, cx),
            (true, false, false),
            "back on Delete agent"
        );
        panel.read_with(cx, |panel, _| assert!(panel.sent_for_test().is_empty()));
        click(cx, "reactive-Delete agent");
        click(cx, "desktop-delete-agent-yes");
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.sent_for_test(),
                [ReactiveCommand::Apply {
                    change: ReactiveChange::DeleteAgent { id: "a".into() }
                }]
            );
        });
        panel.update(cx, |panel, cx| {
            panel.answer_last_for_test(snapshot(Vec::new(), 3), cx);
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-delete-agent-confirm").is_none());
        assert_eq!(
            focus(&panel, cx),
            (false, false, true),
            "focus moves to New agent"
        );
    }

    #[test]
    fn a_delete_agent_proposal_names_the_agent_and_what_goes_with_it() {
        use bruhs_models::crew::delete_agent_line_for;
        let snapshot = snapshot(vec![agent("a1", "Ada")], 3);
        assert_eq!(
            delete_agent_line_for(Some(&snapshot), "a1"),
            delete_agent_line("Ada")
        );
        // An agent the app hasn't loaded is named by its id.
        assert_eq!(delete_agent_line_for(None, "zz"), delete_agent_line("zz"));
    }

    /// Computer requests and live view input reach only a service that
    /// runs Computers; an older one says why.
    #[gpui::test]
    fn computer_requests_wait_for_a_service_with_computers(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, snapshot(vec![agent("a", "Ada")], 2));
        panel.update(cx, |panel, cx| {
            assert!(!panel.computer_ready());
            assert_eq!(
                panel.computer_gate("a").reason,
                Some(ServiceNotice::ServingOld.message())
            );
            assert!(panel
                .computer_request("a", ComputerRequest::Open, cx)
                .is_none());
            assert!(panel.computer_requests_for_test().is_empty());
            panel.set_snapshot(snapshot(vec![agent("a", "Ada")], 3), cx);
            assert!(panel.computer_ready());
            assert_eq!(panel.computer_gate("a").reason, None);
            assert!(panel
                .computer_request("a", ComputerRequest::Open, cx)
                .is_some());
            assert_eq!(
                panel.computer_requests_for_test(),
                [("a".to_string(), ComputerRequest::Open)]
            );
            panel.screencast_input(None, ComputerInput::Reload);
            assert_eq!(panel.screencast_inputs_for_test(), [ComputerInput::Reload]);
        });
    }
}

#[cfg(test)]
mod crew_tests {
    use super::{
        auto_update_allowed, installed_protocol, service_update, ReactiveAutomations, ServiceUpdate,
    };
    use crate::theme;
    use bruhs_models::api::BackgroundService;
    use bruhs_models::crew::{BruhAccess, BruhLook, BruhState, BruhStatus, ServiceNotice};
    use bruhs_models::reactive::{OutboxSignal, ReactiveSnapshot};

    fn service(installed: bool, protocol: Option<u32>) -> BackgroundService {
        BackgroundService {
            supported: true,
            installed,
            loaded: installed,
            binary: None,
            detail: String::new(),
            serving: false,
            service_pid: None,
            responder_pid: None,
            log_path: None,
            reactive_protocol: protocol,
            debug_build: false,
        }
    }

    #[test]
    fn an_installed_service_that_cannot_say_counts_as_old() {
        assert_eq!(installed_protocol(&service(false, None)), None);
        assert_eq!(installed_protocol(&service(true, None)), Some(0));
        assert_eq!(installed_protocol(&service(true, Some(1))), Some(1));
    }

    /// An old installed service is reinstalled by the current server; an old
    /// answering server only by this bundle's own binary.
    #[test]
    fn each_older_service_has_its_own_update() {
        assert_eq!(
            service_update(ServiceNotice::InstalledOld, false),
            Some(ServiceUpdate::InstallFromServer)
        );
        assert_eq!(
            service_update(ServiceNotice::ServingOld, true),
            Some(ServiceUpdate::Bundled)
        );
        assert_eq!(service_update(ServiceNotice::ServingOld, false), None);
    }

    /// A debug server or app never installs itself as the service unasked.
    #[test]
    fn only_a_release_build_updates_the_service_without_a_click() {
        let from_server = ServiceUpdate::InstallFromServer;
        assert!(auto_update_allowed(from_server, false, true));
        assert!(!auto_update_allowed(from_server, true, false));
        assert!(auto_update_allowed(ServiceUpdate::Bundled, true, false));
        assert!(!auto_update_allowed(ServiceUpdate::Bundled, false, true));
    }

    #[gpui::test]
    fn live_frames_and_snapshots_keep_the_crew_current(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let (panel, cx) = cx.add_window_view(|_, cx| ReactiveAutomations::new(cx));
        let ann = BruhState {
            agent_id: "ann".into(),
            name: "Ann".into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status: BruhStatus::Working,
            next_wake_ms: None,
            unread: 2,
            computer: false,
            vault: Vec::new(),
            apps: Vec::new(),
            mail: Vec::new(),
        };
        panel.update(cx, |panel, cx| {
            assert_eq!(panel.crew_protocol(), None);
            assert!(!panel.crew_ready(), "unknown is not ready");
            assert_eq!(panel.crew_notice(), None, "no notice before the protocol");
            panel.apply_crew_signal(
                OutboxSignal {
                    bruhs: vec![ann.clone()],
                    protocol: 1,
                    ..OutboxSignal::default()
                },
                cx,
            );
            assert_eq!(panel.crew(), std::slice::from_ref(&ann));
            assert!(panel.crew_ready());
            // Protocol 1 keeps the crew commands but has no rooms.
            assert!(!panel.rooms_ready());
            assert_eq!(
                panel.rooms_reason(),
                Some(ServiceNotice::ServingOld.message())
            );
            // An older service's snapshot has no crew and protocol 0.
            panel.set_snapshot(ReactiveSnapshot::default(), cx);
            assert!(panel.crew().is_empty());
            assert_eq!(panel.crew_protocol(), Some(0));
            assert_eq!(panel.crew_notice(), Some(ServiceNotice::ServingOld));
            assert!(!panel.crew_ready());
        });
    }
}
