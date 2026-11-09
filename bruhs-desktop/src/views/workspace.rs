use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::resizable::{
    h_resizable, resizable_panel, v_resizable, ResizablePanelEvent, ResizableState,
};
use gpui_component::scroll::{ScrollableElement, Scrollbar};
use gpui_component::tooltip::Tooltip;
use gpui_component::{ActiveTheme, Icon, IconName, WindowExt};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{hash_map::DefaultHasher, BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::{atomic::AtomicU64, Arc, Mutex};

use bruhs_models::logic::attachments::{FileKind, UploadedFile};

use crate::server_client::ServerClient;
use crate::views::meetings_hub::{MeetingCache, MeetingPrefs, MeetingsHub, PeopleSnapshot};
use crate::views::native_blur::BlurScrimHandle;
use crate::views::sidebar_order::{relative_age, sidebar_order, ThreadMeta};
use crate::views::{
    chat::{
        ChatView, ChatViewEvent, ComposerPrefs, CyclePermissionMode, DesktopToolCall, Message,
        MessageRole, PermissionMode, TurnState,
    },
    diff_panel::{turn_files_from_messages, DiffPanelEvent, DiffPanelView},
    dropdown::{dropdown, DropdownGroup, DropdownOption, Listbox, SelectHandler, ToggleHandler},
    preview_panel::{PreviewPanelEvent, PreviewPanelView},
    tasks_panel::{TasksPanelEvent, TasksPanelView},
    terminal_panel::{
        clamp_terminal_drawer_height, TerminalPanelEvent, TerminalPanelMode, TerminalPanelView,
        TERMINAL_DRAWER_HEIGHT_PX, TERMINAL_DRAWER_MAX_HEIGHT_PX, TERMINAL_DRAWER_MIN_HEIGHT_PX,
    },
    web_button::{
        is_web_button_activation_key, web_button, ButtonState, WebButtonSize, WebButtonVariant,
    },
};
use crate::{app_state::DesktopState, assets, theme};
use bruhs_core::preview::LocalServer;
use bruhs_models::agent::{
    model_info, AgentProvider, ModelInfo, DEFAULT_AGENT_MODEL, MODEL_CATALOG,
};
use bruhs_models::logic::compact::{autocompact_flag, autocompact_tokens, IdleCompact};
use bruhs_models::logic::pages::{page_from_key, page_key, PageHistory};
use bruhs_models::logic::run_place::{RememberedWorktree, WorkspaceChoice};
use bruhs_models::logic::sidebar::SidebarSection;
use bruhs_models::EpochMs;
use bruhs_models::{
    backlinks_for, AutomationState, CaptureKind, CaptureRecord, ConflictResolution,
    InboxConversation, InboxMessage, InboxParticipant, InboxSource, KnowledgeCitation,
    KnowledgeDocument, KnowledgeLink, KnowledgeLinkKind, LifeContext, PersonProfile,
    RelationshipItemKind, RelationshipProfile, SchedulingLink, SchedulingPreferences, SourceClip,
    TodaySnapshot, WorkItem, WorkItemPlanning,
};

mod automations_ui;
mod calendar_ui;
mod capture_ui;
#[cfg(test)]
mod chat_events_tests;
mod credential_settings;
mod date_picker_ui;
mod event_dialog_ui;
#[cfg(test)]
mod import_tests;
mod import_ui;
pub(crate) use import_ui::portable_messages;
mod inbox_ui;
mod keybindings_ui;
mod knowledge_storage;
mod knowledge_ui;
mod life_ui;
mod mcp_bridge;
mod meetings_ui;
mod modal_ui;
mod modes_ui;
#[cfg(test)]
mod notification_tests;
#[cfg(test)]
mod overlay_tests;
#[cfg(test)]
mod palette_tests;
mod palette_ui;
mod panels_ui;
mod persisted_enum;
mod persistence;
mod person_merge;
#[cfg(test)]
mod project_picker_tests;
mod project_picker_ui;
mod save_queue;
mod save_status;
mod scheduling_ui;
mod search_record_detail;
mod settings_page_ui;
mod settings_ui;
mod sidebar_ui;
#[cfg(debug_assertions)]
mod smoke;
mod state_file;
#[cfg(test)]
mod state_fixture_tests;
mod task_forms_ui;
mod tasks_ui;
mod thread_snapshot;
mod threads_ui;
mod today_ui;
mod updates_ui;
use calendar_ui::*;
use event_dialog_ui::*;
use modal_ui::*;
#[cfg(test)]
use palette_ui::*;
use panels_ui::*;
use persistence::*;
use project_picker_ui::*;
use save_status::SaveStatus;
use settings_page_ui::*;
use sidebar_ui::{SIDEBAR_HEADER_LEFT_INSET_PX, SIDEBAR_WIDTH_PX};
use state_file::{DiskVersion, SaveError};
use task_forms_ui::*;
use tasks_ui::*;

// --- Actions ---
actions!(
    workspace,
    [
        NewThread,
        ToggleSidebar,
        ToggleTasks,
        FocusCommandBar,
        FocusChatInput,
        OpenTaskCreateShortcut,
        OpenCapture,
        CloseOverlays,
        OpenChatView,
        OpenTodayView,
        OpenInboxView,
        OpenKnowledgeView,
        SaveKnowledgeDocument,
        OpenTasksView,
        OpenCalendarView,
        OpenSettings,
        OpenProjectPicker,
        PinThread,
        SettleThread,
        CopyThreadLink,
        CopyPullLink,
        CopyPullNumber,
        ToggleDiff,
        TogglePanel,
        ClosePanel,
        ToggleModelPicker,
        ToggleWorkspaceMenu,
        ToggleBranchMenu,
        ToggleEffortMenu,
        UsePreviousWorktree,
        PrevThread,
        NextThread,
        NavigateBack,
        NavigateForward,
        MentionFile,
        OpenTerminal,
        ToggleTerminal,
        OpenTerminalView,
        TogglePreview,
        OpenLocalServers,
        SearchThreads,
        UndoSidebar,
        OpenProjectInEditor,
        ImportThread,
        CopyResumeCommand,
        CopyThreadMarkdown,
        ContinueInT3
    ]
);

/// ⌘1–⌘9: open the nth thread of the sidebar order (0-based payload).
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct JumpThread(pub usize);

/// Glyph table the shortcut-coverage test checks. Settings edits the rule list.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the shortcut-coverage test reads this table; settings edits the rule list"
    )
)]
pub(crate) const DESKTOP_KEYMAP: &[(&str, &[&str])] = &[
    ("Command palette", &["⌘K"]),
    ("Capture anything", &["⌘⇧SPACE"]),
    ("New thread in this project", &["⌘N", "⌘⇧O", "⌘⇧N"]),
    ("Toggle sidebar", &["⌘B"]),
    ("Toggle right panel", &["⌘⌥B"]),
    ("Close right panel", &["⌘W"]),
    ("Toggle diff", &["⌘D"]),
    ("Toggle tasks panel", &["⌘⇧T"]),
    ("Change model", &["⌘⇧M"]),
    ("Where this thread runs", &["⌘⇧X"]),
    ("Switch or choose a base branch", &["⌘⇧G"]),
    ("Claude effort", &["⌘⇧E"]),
    ("Use the previous worktree", &["⌘⇧L"]),
    ("Mention a file", &["⌘P"]),
    ("Previous / next thread", &["⌘⇧[", "⌘⇧]"]),
    ("Back / forward", &["⌘[", "⌘]"]),
    ("Jump to thread 1–9", &["⌘1", "⌘9"]),
    ("Pin / unpin thread", &["⌘⇧P"]),
    ("Settle / unsettle thread", &["⌘⇧S"]),
    ("Copy thread link", &["⌘⇧C"]),
    ("Settings", &["⌘/"]),
    ("Toggle terminal", &["⌘J"]),
    ("Toggle preview", &["⌘⇧J"]),
    ("Stash the draft", &["⌘S"]),
    ("Search everything", &["⌘⇧F"]),
    ("Find in the thread or the focused diff", &["⌘F"]),
    ("Open Today", &["⌘⇧Y"]),
    ("Open Knowledge", &["⌘⇧K"]),
    ("Open Inbox", &["⌘⇧I"]),
    ("Focus the composer", &["/"]),
    ("New task (Tasks view)", &["C"]),
    ("Close dialogs and menus", &["Esc"]),
];

const DESKTOP_TEXT_INPUT_GUARDED_SHORTCUTS: &[&str] = &["/", "c"];

fn focused_input_should_receive_plain_shortcut(shortcut: &str, has_focused_input: bool) -> bool {
    debug_assert!(DESKTOP_TEXT_INPUT_GUARDED_SHORTCUTS.contains(&shortcut));
    has_focused_input
}

// --- Data types ---
//
// UI surface enums live in `bruhs-models` so every surface (web/desktop)
// shares one definition. We re-export under the legacy `Desktop*` names
// for in-file readability and to keep call sites unchanged.
pub use bruhs_models::AppView;
pub use bruhs_models::CalendarMode as DesktopCalendarMode;
pub use bruhs_models::Priority as DesktopPriority;
pub use bruhs_models::SidebarTab as DesktopSidebarTab;
pub use bruhs_models::TaskViewMode as DesktopTaskViewMode;
pub use bruhs_models::WorkItemStatus as DesktopWorkItemStatus;

use bruhs_models::LifeSection as DesktopLifeTab;

/// Glyphs for the shared status / priority enums; identical to the web's
/// `task_style` so both surfaces draw the same marks.
trait TaskGlyph {
    fn glyph(&self) -> &'static str;
}

impl TaskGlyph for DesktopWorkItemStatus {
    fn glyph(&self) -> &'static str {
        match self {
            Self::Backlog => "⬚",
            Self::Todo => "⃝",
            Self::InProgress => "◐",
            Self::Done => "●",
            Self::Canceled => "⊘",
        }
    }
}

impl TaskGlyph for DesktopPriority {
    fn glyph(&self) -> &'static str {
        match self {
            Self::Urgent => "‼",
            Self::High => "▲",
            Self::Medium => "◆",
            Self::Low => "▼",
            Self::None => "—",
        }
    }
}

#[derive(Clone)]
struct DesktopProject {
    id: String,
    name: String,
}

#[derive(Clone)]
struct DesktopProjectLocation {
    path: String,
    name: String,
}

#[derive(Clone, Serialize)]
struct DesktopWorkItem {
    id: String,
    project_id: Option<String>,
    title: String,
    description: Option<String>,
    status: DesktopWorkItemStatus,
    priority: DesktopPriority,
    due_date: Option<String>,
    planning: WorkItemPlanning,
    updated_at: EpochMs,
}

#[derive(Clone)]
struct DesktopTaskDrag {
    item_id: String,
    title: String,
    priority: DesktopPriority,
}

#[derive(Clone, Serialize)]
struct DesktopCalendarEvent {
    id: String,
    title: String,
    start_at: NaiveDateTime,
    end_at: NaiveDateTime,
    all_day: bool,
    color: Option<String>,
    notes: Option<String>,
}

#[derive(Clone)]
struct DesktopAppliedPlan {
    previous_planning: Vec<(String, WorkItemPlanning)>,
    event_ids: Vec<String>,
}

struct DesktopEventFormValues<'a> {
    title: &'a str,
    start_at: NaiveDateTime,
    end_at: NaiveDateTime,
    all_day: bool,
    color: &'a str,
    notes: Option<&'a str>,
}

#[derive(Clone)]
struct DesktopLaidOutEvent {
    event: DesktopCalendarEvent,
    top_px: f32,
    height_px: f32,
    lane: usize,
    lanes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DesktopTimegridDragKind {
    Move,
    ResizeEnd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskCreateMenu {
    Project,
    Status,
    Priority,
}

/// The form dialog over the workspace, if any: one at a time, so opening
/// one replaces another. Popovers that belong to a dialog live in its
/// variant and close with it.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Overlay {
    Capture,
    Settings {
        /// Id of the settings dropdown whose listbox is open, if any.
        open_dropdown: Option<&'static str>,
    },
    TaskCreate {
        menu: Option<TaskCreateMenu>,
    },
    EventCreate {
        color_menu_open: bool,
    },
    ProjectDeleteConfirm {
        project_id: String,
    },
    /// The sidebar Update pill's "Update N providers?".
    UpdateConfirm,
    /// "Restart to update" for a downloaded release.
    RestartConfirm,
}

/// The command bar or the project picker. Like the web, where both are
/// mounted over everything, they open above a form dialog without closing
/// it, and Escape closes both layers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TopLayer {
    CommandBar,
    ProjectPicker {
        /// Opened from the terminal's "Choose folder…": the pick opens a
        /// shell there instead of a project thread.
        for_terminal: bool,
    },
    /// Import a thread from Claude Code, Codex, T3 Code or Cursor.
    ImportThread,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskDetailMenu {
    Project,
    Status,
    Priority,
}

#[derive(Clone)]
struct DesktopTimegridDrag {
    event_id: String,
    kind: DesktopTimegridDragKind,
    title: String,
    color: Option<String>,
}

struct DesktopTimegridDragState {
    event_id: String,
    kind: DesktopTimegridDragKind,
    grab_offset_y_px: f32,
}

#[derive(Debug)]
struct CommandAction {
    id: &'static str,
    group: &'static str,
    label: &'static str,
    hint: &'static str,
}

/// Commands mix local records and server results once a query is typed;
/// ⌘⇧J restricts the palette to local servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaletteMode {
    Commands,
    Servers,
}

#[derive(Clone, Debug)]
enum PaletteItem {
    Action(&'static CommandAction),
    Thread {
        id: String,
        title: String,
        hint: String,
    },
    Server {
        port: u16,
        command: String,
    },
    Record(bruhs_models::SearchHit),
}

/// Command that opens the system terminal at `path`.
fn system_terminal_command(path: &str) -> (String, Vec<String>) {
    if cfg!(target_os = "macos") {
        (
            "open".to_string(),
            vec!["-a".to_string(), "Terminal".to_string(), path.to_string()],
        )
    } else {
        (
            "x-terminal-emulator".to_string(),
            vec!["--working-directory".to_string(), path.to_string()],
        )
    }
}

/// Command that opens `url` in the default browser.
fn browser_open_command(url: &str) -> (String, Vec<String>) {
    if cfg!(target_os = "macos") {
        ("open".to_string(), vec![url.to_string()])
    } else {
        ("xdg-open".to_string(), vec![url.to_string()])
    }
}

/// The bruhs-server client for meetings; without one (the HTTP client could
/// not be built) meetings report the server as unavailable.
fn meetings_client() -> ServerClient {
    ServerClient::from_env().unwrap_or_else(|error| {
        tracing::error!("meetings are unavailable: {error}");
        ServerClient::unavailable(&error)
    })
}

/// Launch `command` without waiting for it; a small thread reaps it when it
/// exits so it never lingers as a zombie.
fn spawn_detached(command: (String, Vec<String>)) {
    let (program, args) = command;
    let child = std::process::Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) => {
            tracing::warn!("could not run {program}: {err}");
            return;
        }
    };
    let reaper = std::thread::Builder::new()
        .name("bruhs-launch-reaper".to_string())
        .spawn(move || {
            if let Err(err) = child.wait() {
                tracing::debug!("waiting for {program} failed: {err}");
            }
        });
    if let Err(err) = reaper {
        tracing::warn!("could not watch a launched program; it may linger: {err}");
    }
}

/// A provider and its catalog models with their catalog indices.
type ProviderModels = (AgentProvider, Vec<(usize, &'static ModelInfo)>);

/// The catalog grouped by provider, providers in wire-name order (Claude,
/// then Codex), like the web settings dropdown.
fn desktop_model_provider_groups_like_web() -> Vec<ProviderModels> {
    let mut groups: BTreeMap<&'static str, ProviderModels> = BTreeMap::new();
    for (index, model) in MODEL_CATALOG.iter().enumerate() {
        groups
            .entry(model.provider.as_str())
            .or_insert_with(|| (model.provider, Vec::new()))
            .1
            .push((index, model));
    }
    groups.into_values().collect()
}

pub(crate) fn provider_label(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "Anthropic",
        AgentProvider::Codex => "OpenAI Codex",
    }
}

pub(crate) fn provider_mark(provider: AgentProvider, _cx: &App) -> AnyElement {
    let logo_size = px(assets::PROVIDER_LOGO_SIZE_PX);
    div()
        .size(logo_size)
        .min_w(logo_size)
        .max_w(logo_size)
        .min_h(logo_size)
        .max_h(logo_size)
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .overflow_hidden()
        .debug_selector(|| format!("desktop-provider-logo-{provider}"))
        .opacity(assets::PROVIDER_LOGO_OPACITY)
        .text_color(theme::rambutan_100())
        .child(
            svg()
                .path(assets::provider_logo_path(provider))
                .size(logo_size)
                .text_color(theme::rambutan_100())
                .min_w(logo_size)
                .max_w(logo_size)
                .min_h(logo_size)
                .max_h(logo_size)
                .flex_none()
                .text_color(theme::rambutan_100()),
        )
        .into_any_element()
}

/// Redraw the workspace whenever `input` is edited.
fn redraw_on_edit(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
) -> Subscription {
    cx.subscribe_in(
        input,
        window,
        |_this: &mut Workspace, _input, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        },
    )
}

pub(crate) fn element_key(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn today_date() -> NaiveDate {
    Local::now().date_naive()
}

/// Left-nav sections of the settings page (web `settings_page::SECTIONS`,
/// in page order). Sections render stacked on one scrolling page; the nav
/// jumps to a section and a scroll spy highlights the one under the fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsSection {
    General,
    Updates,
    Beta,
    Credentials,
    Calendar,
    Recording,
    Transcription,
    Speakers,
    People,
    Keyboard,
    About,
}

/// Which surface the right panel shows. One at a time, like the web's
/// `RightPanel { open, surface }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RightPanelSurface {
    Tasks,
    Diff,
    Preview,
    /// The project tree.
    Files,
    /// The GitHub pull request for this thread's branch.
    Pull,
    /// The open attachment. Present only while a file is open.
    File,
}

/// A GitHub pull request the right panel is showing.
#[derive(Clone, Debug)]
struct DesktopBranchPull {
    number: u64,
    title: String,
    url: String,
    state: String,
}

/// A file the right panel is reading. `source` records a line link.
#[derive(Clone, Debug)]
struct DesktopOpenFile {
    path: String,
    label: String,
    project: Option<String>,
    source: bool,
}

#[derive(Clone, Debug)]
struct DesktopReadyFile {
    label: String,
    path: String,
    kind: FileKind,
    text: Option<String>,
    truncated: bool,
}

#[derive(Clone, Debug)]
enum DesktopFilePreview {
    Empty,
    Loading,
    Failed(String),
    Ready(DesktopReadyFile),
}

fn is_web_checkbox_activation_key(key: &str) -> bool {
    key == "space"
}

struct DesktopThread {
    id: String,
    project_name: String,
    project_path: String,
    /// Last activity; drives card ages and the Settled fold.
    updated_at_ms: EpochMs,
    pinned: bool,
    settled: bool,
    /// Drag order within Pinned. `None` sits above arranged pins.
    pin_rank: Option<i64>,
    /// Drag order within Active. Activity does not clear this.
    active_rank: Option<i64>,
    /// Wake time. `None` means the thread is not snoozed.
    snoozed_until_ms: Option<i64>,
    /// Branch and model at the start of the last turn (sidebar card line 3).
    branch: Option<String>,
    model_id: String,
    chat: Entity<ChatView>,
    _chat_observer: Subscription,
    _chat_event_subscription: Subscription,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct PersistedWorkspaceState {
    current_model: Option<String>,
    #[serde(with = "persisted_enum::optional")]
    permission_mode: Option<PermissionMode>,
    sidebar_visible: Option<bool>,
    #[serde(with = "persisted_enum::optional")]
    sidebar_tab: Option<DesktopSidebarTab>,
    #[serde(with = "persisted_enum::optional")]
    active_view: Option<AppView>,
    active_thread_id: Option<String>,
    #[serde(with = "persisted_enum::optional")]
    task_view_mode: Option<DesktopTaskViewMode>,
    selected_project_id: Option<String>,
    selected_work_item_id: Option<String>,
    #[serde(with = "persisted_enum::optional")]
    calendar_mode: Option<DesktopCalendarMode>,
    calendar_anchor: Option<String>,
    selected_calendar_event_id: Option<String>,
    threads: Vec<PersistedThread>,
    projects: Vec<PersistedProject>,
    work_items: Vec<PersistedWorkItem>,
    calendar_events: Vec<PersistedCalendarEvent>,
    captures: Vec<CaptureRecord>,
    knowledge_documents: Vec<KnowledgeDocument>,
    source_clips: Vec<SourceClip>,
    inbox_conversations: Vec<InboxConversation>,
    meeting_notes: Vec<bruhs_models::MeetingNote>,
    relationships: Vec<RelationshipProfile>,
    people_cache: PeopleSnapshot,
    meeting_cache: MeetingCache,
    meeting_actions: crate::views::meetings_hub::ActionCheckpoints,
    scheduling_preferences: SchedulingPreferences,
    life_context: LifeContext,
    automation_state: AutomationState,
    active_project: Option<PersistedProjectLocation>,
    recent_projects: Vec<PersistedProjectLocation>,
    sidebar_settled_open: Option<bool>,
    /// Missing means closed, matching the web default.
    sidebar_working_open: Option<bool>,
    sidebar_snoozed_open: Option<bool>,
    /// Missing means the Working section is on.
    sidebar_working_section: Option<bool>,
    /// Missing means update checks are on (web `bruhs.updateChecks`).
    update_checks: Option<bool>,
    title_instructions: Option<String>,
    title_instructions_by_project: BTreeMap<String, String>,
    editor_id: Option<String>,
    /// Height of the ⌘J terminal drawer (web: `bruhs.terminalHeight`).
    terminal_drawer_height: Option<f32>,
    /// Web `bruhs.rightPanelWidth`.
    right_panel_width: Option<f32>,
    /// Web `bruhs.whisper_model` / `bruhs.diarize` / `bruhs.auto_record` /
    /// `bruhs.personality` preferences.
    meeting_prefs: Option<MeetingPrefs>,
    /// `checkout`, `new_worktree`, or `previous_worktree`. Empty means checkout.
    #[serde(default)]
    run_choice: String,
    /// Claude `--effort` for the next send. Codex has no effort flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_effort: Option<String>,
    /// `steer`, or absent for queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    follow_up: Option<String>,
    /// Claude `--autocompact` token count. Absent omits the flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_autocompact: Option<String>,
    /// `before` or `never`. Absent means compact when you return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compact_idle: Option<String>,
    /// Last worktree this client created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_worktree: Option<RememberedWorktree>,
    /// Shortcut rules. Empty means the shipped defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    keybindings: Vec<bruhs_models::logic::keybindings::Keybinding>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedThread {
    id: String,
    title: String,
    project_name: String,
    project_path: Option<String>,
    model_id: String,
    #[serde(default, with = "persisted_enum::required")]
    permission_mode: PermissionMode,
    messages: Vec<PersistedMessage>,
    /// Last activity. Builds before 2026-10 also wrote a placeholder
    /// `updated_at: "now"` string, which is ignored on read.
    #[serde(default, deserialize_with = "bruhs_models::time::deserialize_legacy")]
    updated_at_ms: EpochMs,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    settled: bool,
    #[serde(default)]
    pin_rank: Option<i64>,
    #[serde(default)]
    active_rank: Option<i64>,
    #[serde(default)]
    snoozed_until_ms: Option<i64>,
    #[serde(default)]
    branch: Option<String>,
    /// Prompts waiting for this thread. Missing on states saved before the queue.
    #[serde(default)]
    prompt_queue: Vec<bruhs_models::logic::queue::QueuedPrompt>,
    /// The next turn forgets the provider session. Set by Edit from here
    /// until a new session id arrives.
    #[serde(default, skip_serializing_if = "skip_false")]
    fresh_session: bool,
    /// Provider session the next turn resumes, so a restart keeps the
    /// conversation. Missing on states saved before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// `claude` or `codex`: the provider that owns `session_id`. A turn of
    /// the other provider starts its own session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_provider: Option<String>,
    /// Transcript of a thread imported without a session, sent with turns
    /// until a session holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    import_context: Option<String>,
    /// Local `/goal` text. Missing on states saved before goals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    goal: Option<bruhs_models::logic::commands::ThreadGoal>,
    /// Worktree this thread runs in, once the first send created or reused one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
    /// Effort applied on the last successful send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    /// The thread keeps its checkout or worktree. Missing means it has not sent.
    #[serde(default, skip_serializing_if = "skip_false")]
    run_locked: bool,
}

fn skip_false(value: &bool) -> bool {
    !*value
}

/// Shelf fields a five-second Undo can put back. The transcript stays.
#[derive(Clone, Debug)]
struct ShelfSnapshot {
    id: String,
    pinned: bool,
    settled: bool,
    pin_rank: Option<i64>,
    active_rank: Option<i64>,
    snoozed_until_ms: Option<i64>,
}

#[derive(Clone, Debug)]
enum SidebarUndoRestore {
    Shelf(ShelfSnapshot),
    Draft { thread_id: String, text: String },
}

#[derive(Clone, Debug)]
struct SidebarUndo {
    message: String,
    restore: SidebarUndoRestore,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedProjectLocation {
    path: String,
    name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedMessage {
    #[serde(with = "persisted_enum::required")]
    role: MessageRole,
    content: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    thinking: String,
    #[serde(default)]
    usage: Option<bruhs_core::UsageInfo>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    started_at_ms: Option<i64>,
    #[serde(default)]
    completed_at_ms: Option<i64>,
    #[serde(default)]
    tool_calls: Vec<DesktopToolCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<UploadedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoint: Option<String>,
    #[serde(default)]
    stopped: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedProject {
    id: String,
    name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedWorkItem {
    id: String,
    project_id: Option<String>,
    title: String,
    description: Option<String>,
    #[serde(with = "persisted_enum::required")]
    status: DesktopWorkItemStatus,
    #[serde(with = "persisted_enum::required")]
    priority: DesktopPriority,
    due_date: Option<String>,
    #[serde(default)]
    planning: WorkItemPlanning,
    /// Builds before 2026-10 wrote the placeholder `"now"`, which reads as
    /// the epoch: the time was never recorded.
    #[serde(deserialize_with = "bruhs_models::time::deserialize_legacy")]
    updated_at: EpochMs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedCalendarEvent {
    id: String,
    title: String,
    start_at: String,
    end_at: String,
    all_day: bool,
    color: Option<String>,
    notes: Option<String>,
}

const DESKTOP_DATE_FORMAT: &str = "%Y-%m-%d";
const DESKTOP_DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";
const DESKTOP_DIALOG_MD_WIDTH_PX: f32 = 448.0;
const DESKTOP_DIALOG_LG_WIDTH_PX: f32 = 576.0;
const DESKTOP_DIALOG_MAX_WIDTH_RATIO: f32 = 0.9;
const DESKTOP_DIALOG_RADIUS_PX: f32 = 14.0;
/// Web dialog headers: `h2.text-h4` (18px Xenon 500, line-height 1.5).
const DESKTOP_MODAL_TITLE_TEXT_SIZE_PX: f32 = 18.0;
const DESKTOP_MODAL_OVERLAY_OPACITY: f32 = 0.70;
const DESKTOP_MODAL_HEADER_X_PADDING_PX: f32 = 16.0;
const DESKTOP_MODAL_HEADER_Y_PADDING_PX: f32 = 12.0;
const DESKTOP_DIALOG_TOP_PADDING_PX: f32 = 128.0;
const DESKTOP_DIALOG_BOTTOM_GAP_PX: f32 = 16.0;
const DESKTOP_DIALOG_MIN_AVAILABLE_HEIGHT_PX: f32 = 240.0;
const FORM_DIALOG_PREFERRED_HEIGHT_PX: f32 = 650.0;
/// Opaque: Settings behaves like a page, so the thread sidebar
/// and chat never show behind it.
/// Web settings page: the card column is `max-w-[44rem]`, shrinking with the
/// window (nav + paddings taken off).
/// Web `ui::Input`: 40px tall, 16px Neon, 12px side padding.
fn form_text_input(input: Input) -> Input {
    // `Input::h` (inherent) only applies to multi-line inputs; the `Styled`
    // height is refined last, so it sizes single-line fields too.
    let input = Styled::h(input, px(FORM_INPUT_HEIGHT_PX));
    input
        .px(px(FORM_INPUT_X_PADDING_PX))
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(FORM_INPUT_TEXT_SIZE_PX))
}

/// Web `ui::Textarea`: the input's type on `rows` 24px lines.
fn form_textarea(input: Input, height_px: f32) -> Input {
    input
        .h(px(height_px))
        .px(px(FORM_INPUT_X_PADDING_PX))
        .py(px(FORM_INPUT_Y_PADDING_PX))
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(FORM_INPUT_TEXT_SIZE_PX))
}
/// bruhs type scale: every role is `line-height: 1.5`.
const WORKSPACE_LINE_HEIGHT: f32 = 1.5;
const WORKSPACE_COMPACT_BREAKPOINT_PX: f32 = 960.0;
const CHAT_NO_ACTIVE_THREAD_LABEL: &str = "Pick a conversation, or start a new one.";
const CHAT_NO_ACTIVE_THREAD_TEXT_SIZE_PX: f32 = 13.0;
/// Web sidebar `w-[272px]`.
const WORKSPACE_SIDE_PANEL_WIDTH_PX: f32 = SIDEBAR_WIDTH_PX;
const WORKSPACE_SIDE_PANEL_MIN_WIDTH_PX: f32 = 208.0;
/// Diff rows need more room than the tasks list.
/// Web `RIGHT_PANEL_DEFAULT_WIDTH`.
const WORKSPACE_RIGHT_PANEL_WIDTH_PX: f32 = 540.0;
/// Web `RIGHT_PANEL_MIN_WIDTH` / `CHAT_COLUMN_MIN_WIDTH`.
const RIGHT_PANEL_MIN_WIDTH_PX: f32 = 360.0;
const CHAT_COLUMN_MIN_WIDTH_PX: f32 = 360.0;
const WORKSPACE_SIDEBAR_BORDER_OPACITY: f32 = 0.50;
const WORKSPACE_SIDEBAR_TAB_RADIUS_PX: f32 = 4.0;
const WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY: f32 = 0.30;
const FORM_FIELD_GAP_PX: f32 = 6.0;
const FORM_CONTROL_RADIUS_PX: f32 = 8.0;
const FORM_SELECT_BODY_FONT_FAMILY: &str = "Monaspace Neon";
const FORM_SELECT_BODY_TEXT_SIZE_PX: f32 = 14.0;
const FORM_SELECT_HEIGHT_PX: f32 = 32.0;
const FORM_SELECT_MENU_MAX_HEIGHT_PX: f32 = 184.0;
/// Web `ui::Input` / `ui::Textarea`: `px-3 py-2 text-body` — 16px Neon on a
/// 24px line, so 40px single-line and `rows × 24 + 16` for textareas.
const FORM_INPUT_HEIGHT_PX: f32 = 40.0;
const FORM_INPUT_TEXT_SIZE_PX: f32 = 16.0;
const FORM_INPUT_X_PADDING_PX: f32 = 12.0;
const FORM_INPUT_Y_PADDING_PX: f32 = 8.0;
const FORM_THREE_ROW_TEXTAREA_HEIGHT_PX: f32 = 88.0;
const FORM_FIVE_ROW_TEXTAREA_HEIGHT_PX: f32 = 136.0;
const FORM_DIALOG_SECTION_X_PADDING_PX: f32 = 16.0;
const FORM_GRID_GAP_PX: f32 = 12.0;
const FORM_SELECT_MENU_Y_PADDING_PX: f32 = 4.0;
const FORM_SELECT_OPTION_GAP_PX: f32 = 8.0;
const FORM_SELECT_OPTION_X_PADDING_PX: f32 = 12.0;
const FORM_SELECT_OPTION_Y_PADDING_PX: f32 = 6.0;

#[derive(Default)]
struct PersistenceCoordinator {
    latest_generation: AtomicU64,
    write_lock: Mutex<DiskVersion>,
}

fn current_dir_string() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .unwrap_or_else(|| ".".to_string())
}

fn permission_mode_after_cycle(current_permission_mode: PermissionMode) -> PermissionMode {
    current_permission_mode.next()
}

struct DesktopDirectoryListing {
    current: String,
    name: String,
    parent: Option<String>,
    dirs: Vec<DesktopProjectLocation>,
}

pub struct Workspace {
    pub sidebar_visible: bool,
    compact_sidebar_open: bool,
    pub tasks_panel_visible: bool,
    pub diff_panel_visible: bool,
    pub preview_panel_visible: bool,
    /// The file tab is showing. The open file stays when another surface is up.
    file_surface: bool,
    open_file: Option<DesktopOpenFile>,
    file_preview: DesktopFilePreview,
    file_generation: u64,
    files_panel_visible: bool,
    files_dir: String,
    files_root: Option<String>,
    files_entries: Vec<bruhs_core::files::FileEntry>,
    files_error: Option<String>,
    files_loading: bool,
    files_generation: u64,
    pull_panel_visible: bool,
    branch_pull: Option<DesktopBranchPull>,
    opened_pull_key: Option<String>,
    pull_generation: u64,
    last_panel_surface: RightPanelSurface,
    /// Editor id for ⌘O / diff "Open" (`bruhs_models::logic::editor::EDITORS`).
    editor_id: String,
    overlay: Option<Overlay>,
    /// The command bar or project picker, above `overlay`.
    top_layer: Option<TopLayer>,
    selected_meeting_note_id: Option<String>,
    meeting_notes_page: usize,
    note_editor: Option<meetings_ui::note_editor::NoteEditor>,
    capture_kind: CaptureKind,
    capture_edit_id: Option<String>,
    capture_error: Option<String>,
    captures: Vec<CaptureRecord>,
    knowledge_documents: Vec<KnowledgeDocument>,
    source_clips: Vec<SourceClip>,
    inbox_conversations: Vec<InboxConversation>,
    meeting_notes: Vec<bruhs_models::MeetingNote>,
    relationships: Vec<RelationshipProfile>,
    scheduling_preferences: SchedulingPreferences,
    life_context: LifeContext,
    automation_state: AutomationState,
    palette_mode: PaletteMode,
    selected_search_record: Option<search_record_detail::SelectedSearchRecord>,
    search_record_focus: FocusHandle,
    automation_error: Option<String>,
    reviewed_workflow_id: Option<String>,
    mcp_bridge: mcp_bridge::WorkspaceBridge,
    local_servers: Vec<LocalServer>,
    local_servers_loading: bool,
    #[cfg_attr(
        test,
        expect(dead_code, reason = "only the cfg(not(test)) server scan reads it")
    )]
    local_servers_generation: u64,
    settings_section: SettingsSection,
    credential_settings: Entity<credential_settings::CredentialSettings>,
    /// Provider CLI and app update checks (Settings → Updates, sidebar button).
    updates: Entity<updates_ui::DesktopUpdates>,
    /// Scroll position of the stacked settings sections.
    settings_scroll: ScrollHandle,
    /// Section to scroll to once the sections have laid out (web `jump`).
    settings_scroll_target: Option<SettingsSection>,
    /// Last-frame bounds of each settings slider track, for pointer hit-testing.
    settings_slider_bounds: Vec<Rc<std::cell::Cell<Option<Bounds<Pixels>>>>>,
    settings_slider_drag: Option<settings_ui::SettingsSlider>,
    /// Text fields of the Recording steppers (web `Stepper` input).
    settings_stepper_inputs: Vec<Entity<InputState>>,
    /// Open date / date-time picker popover (web `DatePicker` / `DateTimePicker`).
    date_picker: Option<date_picker_ui::DatePickerOpen>,
    date_picker_hour_input: Entity<InputState>,
    date_picker_minute_input: Entity<InputState>,
    pub active_view: AppView,
    command_selected_index: usize,
    command_scroll: ScrollHandle,
    persistence_error: Option<String>,
    save_status: SaveStatus,
    #[cfg(test)]
    persistence_test_path: Option<PathBuf>,
    /// Calls to `persist` (tests assert what does and does not save).
    #[cfg(test)]
    persist_requests: usize,
    day_plan_proposal: Option<bruhs_models::DayPlanProposal>,
    day_plan_undo: Option<DesktopAppliedPlan>,
    persistence: Arc<PersistenceCoordinator>,
    save_queue: save_queue::SaveQueue,

    sidebar_tab: DesktopSidebarTab,
    sidebar_filter: sidebar_ui::SidebarFilter,
    sidebar_filter_open: bool,
    /// Title search box in the sidebar (web: the `Search` input).
    sidebar_search: Entity<InputState>,
    /// Threads whose turn finished while another thread was active.
    unread_thread_ids: HashSet<String>,
    sidebar_settled_open: bool,
    sidebar_working_open: bool,
    sidebar_snoozed_open: bool,
    /// On unless this device saved it off.
    sidebar_working_section: bool,
    title_instructions: String,
    title_instructions_by_project: BTreeMap<String, String>,
    title_instructions_input: Entity<InputState>,
    title_project_input: Entity<InputState>,
    title_project_instructions_input: Entity<InputState>,
    keybindings: Vec<bruhs_models::logic::keybindings::Keybinding>,
    keybinding_index: usize,
    keybinding_key_input: Entity<InputState>,
    keybinding_when_input: Entity<InputState>,
    /// Shortcuts currently installed, so the next edit can unbind them.
    keybinding_installed: Vec<keybindings_ui::InstalledBinding>,
    /// Thread whose action menu is open.
    thread_menu_id: Option<String>,
    sidebar_undo: Option<SidebarUndo>,
    sidebar_undo_generation: u32,
    threads: Vec<DesktopThread>,
    workflow_window: gpui::AnyWindowHandle,
    automation_pending: bool,
    active_thread_id: Option<String>,
    current_model: String,
    permission_mode: PermissionMode,
    /// Where a thread that has not sent yet will run. Copied onto every chat.
    run_choice: WorkspaceChoice,
    /// Claude effort chip. `None` is Default, which omits `--effort`.
    claude_effort: Option<String>,
    /// Stored follow-up. Claude can steer a running turn. Codex still queues.
    follow_up_steer: bool,
    /// Claude `--autocompact`. `None` omits the flag.
    claude_autocompact: Option<String>,
    autocompact_input: Entity<InputState>,
    /// What to do with an idle Claude thread over the token floor.
    compact_idle: IdleCompact,
    /// Visited pages. `Cmd+[` and `Cmd+]` walk this stack.
    page_history: PageHistory,
    /// Offered as Previous worktree. Shared by every thread.
    previous_worktree: Option<RememberedWorktree>,
    /// Base branch for a new worktree. This session only; not saved.
    base_branch: String,
    task_view_mode: DesktopTaskViewMode,
    projects: Vec<DesktopProject>,
    selected_project_id: Option<String>,
    work_items: Vec<DesktopWorkItem>,
    selected_work_item_id: Option<String>,
    task_create_project_id: Option<String>,
    task_create_status: DesktopWorkItemStatus,
    task_create_priority: DesktopPriority,
    task_create_error: Option<String>,
    task_detail_menu_open: Option<TaskDetailMenu>,
    calendar_mode: DesktopCalendarMode,
    calendar_anchor: NaiveDate,
    calendar_events: Vec<DesktopCalendarEvent>,
    selected_calendar_event_id: Option<String>,
    event_create_all_day: bool,
    event_create_color: String,
    event_create_error: Option<String>,
    timegrid_drag: Option<DesktopTimegridDragState>,
    active_project: Option<DesktopProjectLocation>,
    recent_projects: Vec<DesktopProjectLocation>,
    project_picker_current_path: String,
    project_picker_current_name: String,
    project_picker_parent_path: Option<String>,
    project_picker_dirs: Vec<DesktopProjectLocation>,
    project_picker_error: Option<String>,
    /// Reading a directory or resolving a picked project off the UI thread.
    project_picker_task: Option<Task<()>>,
    workspace_focus_handle: FocusHandle,
    palette_return_focus: Option<(AppView, FocusHandle)>,
    modal_start_focus_handle: FocusHandle,
    modal_end_focus_handle: FocusHandle,
    /// The dialogs' backdrop blur, which lives in native views, not gpui's.
    blur_scrim: BlurScrimHandle,

    /// The Import thread dialog's source, results and background work.
    import_picker: import_ui::ImportPicker,
    import_input: Entity<InputState>,

    // Child views
    command_input: Entity<InputState>,
    capture_input: Entity<InputState>,
    knowledge_title_input: Entity<InputState>,
    knowledge_body_input: Entity<InputState>,
    knowledge_tags_input: Entity<InputState>,
    clip_title_input: Entity<InputState>,
    clip_url_input: Entity<InputState>,
    clip_source_name_input: Entity<InputState>,
    clip_author_input: Entity<InputState>,
    clip_published_input: Entity<InputState>,
    clip_tags_input: Entity<InputState>,
    clip_excerpt_input: Entity<InputState>,
    clip_create_visible: bool,
    inbox_import_visible: bool,
    inbox_import_source: InboxSource,
    inbox_sender_input: Entity<InputState>,
    inbox_subject_input: Entity<InputState>,
    inbox_body_input: Entity<InputState>,
    inbox_reply_input: Entity<InputState>,
    relationship_company_input: Entity<InputState>,
    relationship_role_input: Entity<InputState>,
    relationship_email_input: Entity<InputState>,
    relationship_phone_input: Entity<InputState>,
    relationship_location_input: Entity<InputState>,
    relationship_notes_input: Entity<InputState>,
    relationship_item_input: Entity<InputState>,
    relationship_item_kind: RelationshipItemKind,
    scheduling_visible: bool,
    scheduling_link_title_input: Entity<InputState>,
    scheduling_link_duration_input: Entity<InputState>,
    scheduling_selected_task_id: Option<String>,
    life_tab: DesktopLifeTab,
    life_name_input: Entity<InputState>,
    life_value_input: Entity<InputState>,
    life_category_input: Entity<InputState>,
    task_create_title_input: Entity<InputState>,
    task_create_description_input: Entity<InputState>,
    task_create_due_input: Entity<InputState>,
    task_detail_title_input: Entity<InputState>,
    task_detail_description_input: Entity<InputState>,
    task_detail_due_input: Entity<InputState>,
    task_detail_cycle_input: Entity<InputState>,
    task_detail_waiting_input: Entity<InputState>,
    event_create_title_input: Entity<InputState>,
    event_create_start_input: Entity<InputState>,
    event_create_end_input: Entity<InputState>,
    event_create_notes_input: Entity<InputState>,
    tasks_panel: Entity<TasksPanelView>,
    diff_panel: Entity<DiffPanelView>,
    /// Meetings / people / Google-sync state from bruhs-server.
    meetings_hub: Entity<MeetingsHub>,
    meeting_dock: Option<WindowHandle<crate::views::meeting_dock::MeetingDock>>,
    meeting_transcript_view: RefCell<Option<meetings_ui::transcript_view::TranscriptViewport>>,
    live_transcript_view: RefCell<Option<meetings_ui::transcript_view::TranscriptViewport>>,
    #[cfg(test)]
    transcript_rendered_rows: std::cell::Cell<usize>,
    search_query_input: Entity<InputState>,
    /// Normalized universal-search records, rebuilt only when source data's
    /// fingerprint changes rather than on every keystroke.
    universal_search_index: RefCell<Option<(u64, bruhs_models::SearchIndex)>>,
    person_rename_input: Entity<InputState>,
    speaker_rename_input: Entity<InputState>,
    renaming_person_id: Option<String>,
    expanded_person_id: Option<String>,
    merge: Option<meetings_ui::PersonMergePanel>,
    editing_speaker: Option<meetings_ui::SpeakerRef>,
    /// Selected chunk in the live transcript. One at a time.
    live_chunk: Option<String>,
    speaker_menu: Option<meetings_ui::SpeakerRef>,
    /// Thread created by a `BRUHS_DESKTOP_SMOKE_SCENE` fixture; deleted again
    /// by `smoke_cleanup` so the smoke never touches real threads.
    smoke_thread_id: Option<String>,
    /// The thread that was active before the smoke fixture took over; the
    /// snapshot keeps pointing at it.
    smoke_previous_active_id: Option<String>,
    /// One panel for the drawer (⌘J) and the Terminal view; sessions are shared.
    terminal_panel: Entity<TerminalPanelView>,
    preview_panel: Entity<PreviewPanelView>,
    terminal_drawer_open: bool,
    terminal_drawer_height: f32,
    /// Right panel width in px, dragged at its left edge (web `RightPanelWidth`).
    right_panel_width: f32,
    right_panel_split_state: Entity<ResizableState>,
    /// Split between the transcript and the drawer; persists the drag.
    terminal_split_state: Entity<ResizableState>,
    /// Keeps the input, panel and hub subscriptions alive.
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let (persisted, persistence_error, disk_version) = load_desktop_state();
        let current_model = persisted
            .as_ref()
            .and_then(|state| state.current_model.clone())
            .unwrap_or_else(|| DEFAULT_AGENT_MODEL.to_string());
        let permission_mode = initial_permission_mode_from_persisted(persisted.as_ref());
        let mut subscriptions = Vec::new();
        let command_input = Self::new_command_input(window, cx, &mut subscriptions);
        let import_input = Self::new_import_input(window, cx, &mut subscriptions);
        let capture_input = Self::new_capture_input(window, cx, &mut subscriptions);
        let knowledge_ui::KnowledgeInputs {
            knowledge_title_input,
            knowledge_body_input,
            knowledge_tags_input,
            clip_title_input,
            clip_url_input,
            clip_source_name_input,
            clip_author_input,
            clip_published_input,
            clip_tags_input,
            clip_excerpt_input,
        } = Self::new_knowledge_inputs(window, cx, &mut subscriptions);
        let inbox_ui::InboxInputs {
            inbox_sender_input,
            inbox_subject_input,
            inbox_body_input,
            inbox_reply_input,
        } = Self::new_inbox_inputs(window, cx, &mut subscriptions);
        let scheduling_ui::SchedulingInputs {
            scheduling_link_title_input,
            scheduling_link_duration_input,
        } = Self::new_scheduling_inputs(window, cx, &mut subscriptions);
        let life_ui::LifeInputs {
            life_name_input,
            life_value_input,
            life_category_input,
        } = Self::new_life_inputs(window, cx, &mut subscriptions);
        let TaskFormInputs {
            task_create_title_input,
            task_create_description_input,
            task_create_due_input,
            task_detail_title_input,
            task_detail_description_input,
            task_detail_due_input,
            task_detail_cycle_input,
            task_detail_waiting_input,
        } = Self::new_task_form_inputs(window, cx, &mut subscriptions);
        let EventFormInputs {
            event_create_title_input,
            event_create_start_input,
            event_create_end_input,
            event_create_notes_input,
        } = Self::new_event_form_inputs(window, cx, &mut subscriptions);
        let sidebar_search = Self::new_sidebar_search(window, cx, &mut subscriptions);
        let saved_title_instructions = persisted
            .as_ref()
            .and_then(|state| state.title_instructions.clone())
            .unwrap_or_default();
        let (title_instructions_input, title_project_input, title_project_instructions_input) =
            Self::new_title_instruction_inputs(
                saved_title_instructions,
                window,
                cx,
                &mut subscriptions,
            );
        let saved_autocompact = persisted
            .as_ref()
            .and_then(|state| autocompact_flag(state.claude_autocompact.as_deref()))
            .unwrap_or_default();
        let autocompact_input =
            Self::new_autocompact_input(saved_autocompact, window, cx, &mut subscriptions);
        let keybindings = keybindings_ui::rules_from_saved(
            persisted
                .as_ref()
                .map(|state| state.keybindings.clone())
                .unwrap_or_default(),
        );
        let (keybinding_key_input, keybinding_when_input) =
            keybindings_ui::new_keybinding_inputs(&keybindings, window, cx, &mut subscriptions);
        let (modal_start_focus_handle, modal_end_focus_handle) = Self::new_modal_focus_handles(cx);
        let Panels {
            tasks_panel,
            diff_panel,
            terminal_panel,
            terminal_drawer_height,
            terminal_split_state,
            right_panel_width,
            right_panel_split_state,
            preview_panel,
        } = Self::new_panels(persisted.as_ref(), window, cx, &mut subscriptions);
        let meetings_hub =
            Self::new_meetings_hub(persisted.as_ref(), window, cx, &mut subscriptions);
        let meetings_ui::PeopleInputs {
            relationship_company_input,
            relationship_role_input,
            relationship_email_input,
            relationship_phone_input,
            relationship_location_input,
            relationship_notes_input,
            relationship_item_input,
            search_query_input,
            person_rename_input,
            speaker_rename_input,
        } = Self::new_people_inputs(window, cx, &mut subscriptions);
        let stepper_prefs = meetings_hub.read(cx).prefs.clone();
        let settings_stepper_inputs =
            Self::new_settings_stepper_inputs(&stepper_prefs, window, cx, &mut subscriptions);
        let (date_picker_hour_input, date_picker_minute_input) =
            Self::new_date_picker_inputs(window, cx, &mut subscriptions);
        let (threads, active_thread_id) = Self::restore_threads(
            persisted.as_ref(),
            &current_model,
            permission_mode,
            window,
            cx,
        );
        let RestoredRecords {
            projects,
            work_items,
            calendar_events,
            captures,
            knowledge_documents,
            source_clips,
            inbox_conversations,
            meeting_notes,
            relationships,
            scheduling_preferences,
            life_context,
            automation_state,
            active_project,
            recent_projects,
        } = RestoredRecords::from_persisted(persisted.as_ref());
        Self::restore_contact_people(&captures, &meetings_hub, cx);

        let selected_work_item_id = initial_selected_work_item_after_start(
            persisted
                .as_ref()
                .and_then(|state| state.selected_work_item_id.clone()),
        );
        if let Some(item_id) = &selected_work_item_id {
            if let Some(item) = work_items.iter().find(|item| item.id == *item_id) {
                set_task_detail_inputs(
                    [
                        &task_detail_title_input,
                        &task_detail_description_input,
                        &task_detail_due_input,
                        &task_detail_cycle_input,
                        &task_detail_waiting_input,
                    ],
                    item,
                    window,
                    cx,
                );
            }
        }

        cx.defer_in(window, |this, _, cx| this.drive_background_automations(cx));
        cx.defer_in(window, |this, _, cx| this.refresh_branch_pull(cx));
        let run_choice = persisted
            .as_ref()
            .map(|state| WorkspaceChoice::parse(&state.run_choice))
            .unwrap_or_default();
        let claude_effort = persisted.as_ref().and_then(|state| {
            state
                .claude_effort
                .as_deref()
                .and_then(bruhs_models::logic::run_place::claude_effort)
                .map(str::to_string)
        });
        let previous_worktree = persisted
            .as_ref()
            .and_then(|state| state.previous_worktree.clone());
        let follow_up_steer = persisted
            .as_ref()
            .and_then(|state| state.follow_up.as_deref())
            == Some("steer");
        let claude_autocompact = persisted
            .as_ref()
            .and_then(|state| autocompact_flag(state.claude_autocompact.as_deref()));
        let compact_idle = persisted
            .as_ref()
            .and_then(|state| state.compact_idle.as_deref())
            .map(IdleCompact::parse)
            .unwrap_or_default();
        let base_branch = String::new();
        let mut workspace = Self {
            workflow_window: window.window_handle(),
            automation_pending: false,
            sidebar_visible: initial_sidebar_visible_after_start(
                persisted.as_ref().and_then(|state| state.sidebar_visible),
            ),
            compact_sidebar_open: false,
            tasks_panel_visible: false,
            diff_panel_visible: false,
            preview_panel_visible: false,
            file_surface: false,
            open_file: None,
            file_preview: DesktopFilePreview::Empty,
            file_generation: 0,
            files_panel_visible: false,
            files_dir: String::new(),
            files_root: None,
            files_entries: Vec::new(),
            files_error: None,
            files_loading: false,
            files_generation: 0,
            pull_panel_visible: false,
            branch_pull: None,
            opened_pull_key: None,
            pull_generation: 0,
            last_panel_surface: RightPanelSurface::Tasks,
            editor_id: persisted
                .as_ref()
                .and_then(|state| state.editor_id.clone())
                .filter(|id| bruhs_models::logic::editor::editor_by_id(id).is_some())
                .unwrap_or_else(|| bruhs_models::logic::editor::DEFAULT_EDITOR_ID.to_string()),
            overlay: None,
            top_layer: None,
            selected_meeting_note_id: None,
            meeting_notes_page: 0,
            note_editor: None,
            capture_kind: CaptureKind::Task,
            capture_edit_id: None,
            capture_error: None,
            captures,
            knowledge_documents,
            source_clips,
            inbox_conversations,
            meeting_notes,
            relationships,
            scheduling_preferences,
            life_context,
            selected_search_record: None,
            search_record_focus: cx.focus_handle().tab_stop(false),
            automation_state,
            palette_mode: PaletteMode::Commands,
            automation_error: None,
            reviewed_workflow_id: None,
            mcp_bridge: mcp_bridge::WorkspaceBridge::default(),
            local_servers: Vec::new(),
            local_servers_loading: false,
            local_servers_generation: 0,
            settings_section: SettingsSection::General,
            credential_settings: cx
                .new(|cx| credential_settings::CredentialSettings::new(window, cx)),
            updates: {
                let enabled = persisted
                    .as_ref()
                    .and_then(|state| state.update_checks)
                    .unwrap_or(true);
                let updates = cx.new(|cx| updates_ui::DesktopUpdates::new(enabled, cx));
                cx.observe(&updates, |_, _, cx| cx.notify()).detach();
                updates
            },
            settings_scroll: ScrollHandle::new(),
            settings_scroll_target: None,
            settings_slider_bounds: (0..settings_ui::SettingsSlider::COUNT)
                .map(|_| Rc::new(std::cell::Cell::new(None)))
                .collect(),
            settings_slider_drag: None,
            settings_stepper_inputs,
            date_picker: None,
            date_picker_hour_input,
            date_picker_minute_input,
            active_view: initial_active_view_after_start(
                persisted
                    .as_ref()
                    .and_then(|state| state.active_view.clone()),
            ),
            command_selected_index: 0,
            command_scroll: ScrollHandle::new(),
            save_status: persistence_error.map_or(SaveStatus::Saved, SaveStatus::RecoveryRequired),
            persistence_error: None,
            #[cfg(test)]
            persistence_test_path: None,
            #[cfg(test)]
            persist_requests: 0,
            day_plan_proposal: None,
            day_plan_undo: None,
            persistence: Arc::new(PersistenceCoordinator {
                write_lock: Mutex::new(disk_version),
                ..Default::default()
            }),
            save_queue: save_queue::SaveQueue::default(),
            sidebar_tab: persisted
                .as_ref()
                .and_then(|state| state.sidebar_tab)
                .unwrap_or(DesktopSidebarTab::Chat),
            unread_thread_ids: HashSet::new(),
            sidebar_filter: sidebar_ui::SidebarFilter::All,
            sidebar_filter_open: false,
            sidebar_search,
            sidebar_settled_open: persisted
                .as_ref()
                .and_then(|state| state.sidebar_settled_open)
                .unwrap_or(false),
            sidebar_working_open: persisted
                .as_ref()
                .and_then(|state| state.sidebar_working_open)
                .unwrap_or(false),
            sidebar_snoozed_open: persisted
                .as_ref()
                .and_then(|state| state.sidebar_snoozed_open)
                .unwrap_or(false),
            sidebar_working_section: persisted
                .as_ref()
                .and_then(|state| state.sidebar_working_section)
                .unwrap_or(true),
            title_instructions: persisted
                .as_ref()
                .and_then(|state| state.title_instructions.clone())
                .unwrap_or_default(),
            title_instructions_by_project: persisted
                .as_ref()
                .map(|state| state.title_instructions_by_project.clone())
                .unwrap_or_default(),
            title_instructions_input,
            title_project_input,
            title_project_instructions_input,
            keybindings,
            keybinding_index: 0,
            keybinding_key_input,
            keybinding_when_input,
            keybinding_installed: keybindings_ui::startup_bindings(),
            thread_menu_id: None,
            sidebar_undo: None,
            sidebar_undo_generation: 0,
            threads,
            active_thread_id,
            current_model,
            permission_mode,
            run_choice,
            claude_effort,
            follow_up_steer,
            claude_autocompact,
            autocompact_input,
            compact_idle,
            page_history: PageHistory::new(),
            previous_worktree,
            base_branch,
            task_view_mode: persisted
                .as_ref()
                .and_then(|state| state.task_view_mode)
                .unwrap_or(DesktopTaskViewMode::List),
            projects,
            selected_project_id: initial_selected_project_after_start(
                persisted
                    .as_ref()
                    .and_then(|state| state.selected_project_id.clone()),
            ),
            work_items,
            selected_work_item_id,
            task_create_project_id: None,
            task_create_status: DesktopWorkItemStatus::Backlog,
            task_create_priority: DesktopPriority::None,
            task_create_error: None,
            task_detail_menu_open: None,
            calendar_mode: initial_calendar_mode_after_start(
                persisted.as_ref().and_then(|state| state.calendar_mode),
            ),
            calendar_anchor: initial_calendar_anchor_after_start(
                persisted
                    .as_ref()
                    .and_then(|state| state.calendar_anchor.as_deref()),
                today_date(),
            ),
            calendar_events,
            selected_calendar_event_id: initial_selected_calendar_event_after_start(
                persisted
                    .as_ref()
                    .and_then(|state| state.selected_calendar_event_id.clone()),
            ),
            event_create_all_day: false,
            event_create_color: "persimmon".to_string(),
            event_create_error: None,
            timegrid_drag: None,
            active_project,
            recent_projects,
            project_picker_current_path: String::new(),
            project_picker_current_name: String::new(),
            project_picker_parent_path: None,
            project_picker_dirs: Vec::new(),
            project_picker_error: None,
            project_picker_task: None,
            workspace_focus_handle: cx.focus_handle().tab_stop(false),
            palette_return_focus: None,
            modal_start_focus_handle,
            blur_scrim: BlurScrimHandle::default(),
            modal_end_focus_handle,
            import_picker: import_ui::ImportPicker::default(),
            import_input,
            command_input,
            capture_input,
            knowledge_title_input,
            knowledge_body_input,
            knowledge_tags_input,
            clip_title_input,
            clip_url_input,
            clip_source_name_input,
            clip_author_input,
            clip_published_input,
            clip_tags_input,
            clip_excerpt_input,
            clip_create_visible: false,
            inbox_import_visible: false,
            inbox_import_source: InboxSource::Email,
            inbox_sender_input,
            inbox_subject_input,
            inbox_body_input,
            inbox_reply_input,
            relationship_company_input,
            relationship_role_input,
            relationship_email_input,
            relationship_phone_input,
            relationship_location_input,
            relationship_notes_input,
            relationship_item_input,
            relationship_item_kind: RelationshipItemKind::Interaction,
            scheduling_visible: false,
            scheduling_link_title_input,
            scheduling_link_duration_input,
            scheduling_selected_task_id: None,
            life_tab: DesktopLifeTab::Finance,
            life_name_input,
            life_value_input,
            life_category_input,
            task_create_title_input,
            task_create_description_input,
            task_create_due_input,
            task_detail_title_input,
            task_detail_description_input,
            task_detail_due_input,
            task_detail_cycle_input,
            task_detail_waiting_input,
            event_create_title_input,
            event_create_start_input,
            event_create_end_input,
            event_create_notes_input,
            tasks_panel,
            diff_panel,
            meetings_hub,
            meeting_dock: None,
            meeting_transcript_view: RefCell::new(None),
            live_transcript_view: RefCell::new(None),
            #[cfg(test)]
            transcript_rendered_rows: std::cell::Cell::new(0),
            search_query_input,
            universal_search_index: RefCell::new(None),
            person_rename_input,
            speaker_rename_input,
            renaming_person_id: None,
            expanded_person_id: None,
            merge: None,
            editing_speaker: None,
            live_chunk: None,
            speaker_menu: None,
            smoke_thread_id: None,
            smoke_previous_active_id: None,
            terminal_panel,
            preview_panel,
            terminal_drawer_open: false,
            terminal_drawer_height,
            right_panel_width,
            right_panel_split_state,
            terminal_split_state,
            _subscriptions: subscriptions,
        };
        workspace.stamp_run_prefs_on_threads(cx);
        workspace.sync_keybindings(cx);
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30))
                .await;
            if this
                .update(cx, |this, cx| this.compact_idle_if_due(cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
        workspace
    }

    /// Compact the open Claude thread shortly before its prompt cache expires.
    fn compact_idle_if_due(&mut self, cx: &mut Context<'_, Self>) {
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        chat.update(cx, |chat, cx| chat.compact_idle_if_due(cx));
    }

    fn set_current_model(&mut self, model_id: impl Into<String>, cx: &mut Context<'_, Self>) {
        let model_id = model_id.into();
        self.current_model = model_id.clone();
        let chats: Vec<_> = self
            .threads
            .iter()
            .map(|thread| thread.chat.clone())
            .collect();
        for chat in chats {
            chat.update(cx, |chat, cx| {
                chat.set_model(model_id.clone(), cx);
            });
        }
        self.persist_and_notify(cx);
    }

    fn set_permission_mode(&mut self, permission_mode: PermissionMode, cx: &mut Context<'_, Self>) {
        self.permission_mode = permission_mode;
        let chats: Vec<_> = self
            .threads
            .iter()
            .map(|thread| thread.chat.clone())
            .collect();
        for chat in chats {
            chat.update(cx, |chat, cx| {
                chat.permission_mode = permission_mode;
                cx.notify();
            });
        }
        self.persist_and_notify(cx);
    }

    fn cycle_permission_mode(
        &mut self,
        _: &CyclePermissionMode,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_permission_mode(permission_mode_after_cycle(self.permission_mode), cx);
    }

    fn compact_layout(window: &Window) -> bool {
        f32::from(window.viewport_size().width) < WORKSPACE_COMPACT_BREAKPOINT_PX
    }

    fn toggle_sidebar_for_window(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if Self::compact_layout(window) {
            self.compact_sidebar_open = !self.compact_sidebar_open;
            self.close_transient_surfaces();
            cx.notify();
        } else {
            self.sidebar_visible = !self.sidebar_visible;
            self.persist_and_notify(cx);
        }
    }

    fn toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.toggle_sidebar_for_window(window, cx);
    }

    fn focus_chat_input(
        &mut self,
        _: &FocusChatInput,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if focused_input_should_receive_plain_shortcut("/", window.has_focused_input(cx))
            || self.terminal_has_focus(window, cx)
        {
            cx.propagate();
            return;
        }

        if self.active_view != AppView::Chat || self.overlay.is_some() || self.top_layer.is_some() {
            return;
        }

        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };

        chat.update(cx, |chat, cx| {
            chat.focus_composer(window, cx);
        });
    }

    fn open_task_create_shortcut(
        &mut self,
        _: &OpenTaskCreateShortcut,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if focused_input_should_receive_plain_shortcut("c", window.has_focused_input(cx))
            || self.terminal_has_focus(window, cx)
        {
            cx.propagate();
            return;
        }

        if self.active_view == AppView::Tasks {
            self.open_task_create(window, cx);
        }
    }

    fn navigate_back(&mut self, _: &NavigateBack, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.step_page(true, window, cx);
    }

    fn navigate_forward(
        &mut self,
        _: &NavigateForward,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.step_page(false, window, cx);
    }

    /// Walk the visited-page stack. The terminal keeps `Cmd+[` and `Cmd+]`.
    /// A key this build cannot parse is undone so the next visit does not
    /// drop the forward list.
    fn step_page(&mut self, back: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.terminal_has_focus(window, cx) {
            cx.propagate();
            return;
        }
        let key = if back {
            self.page_history.back().map(str::to_string)
        } else {
            self.page_history.forward().map(str::to_string)
        };
        let Some(key) = key else {
            return;
        };
        let Some(view) = page_from_key(&key) else {
            if back {
                self.page_history.forward();
            } else {
                self.page_history.back();
            }
            return;
        };
        self.active_view = view;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    fn open_chat_view(
        &mut self,
        _: &OpenChatView,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.active_view = AppView::Chat;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    fn open_today_view(
        &mut self,
        _: &OpenTodayView,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.active_view = AppView::Today;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    fn open_knowledge_view_action(
        &mut self,
        _: &OpenKnowledgeView,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_knowledge_view(window, cx);
    }

    fn open_inbox_view_action(
        &mut self,
        _: &OpenInboxView,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_inbox_view(window, cx);
    }

    fn save_knowledge_document_action(
        &mut self,
        _: &SaveKnowledgeDocument,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if matches!(self.active_view, AppView::DocumentDetail { .. }) {
            self.save_current_document(cx);
        } else {
            cx.propagate();
        }
    }

    fn open_tasks_view(
        &mut self,
        _: &OpenTasksView,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.active_view = AppView::Tasks;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    fn open_calendar_view(
        &mut self,
        _: &OpenCalendarView,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.active_view = AppView::Calendar;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    fn activate_primary_view(
        &mut self,
        view: AppView,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match view {
            AppView::Today => self.open_today_view(&OpenTodayView, window, cx),
            AppView::Chat => self.open_chat_view(&OpenChatView, window, cx),
            AppView::Tasks => self.open_tasks_view(&OpenTasksView, window, cx),
            AppView::Calendar => self.open_calendar_view(&OpenCalendarView, window, cx),
            AppView::Terminal => self.open_terminal_view(&OpenTerminalView, window, cx),
            _ => {}
        }
    }

    fn render_persistence_error(&self, error: String, cx: &Context<'_, Self>) -> AnyElement {
        let tooltip_error = error.clone();
        div()
            .id("desktop-persistence-error")
            .debug_selector(|| "desktop-persistence-error".to_string())
            .role(Role::Alert)
            .aria_label(error.clone())
            .w_full()
            .min_h(px(40.0))
            .flex_shrink_0()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(theme::durian_400().opacity(0.55))
            .bg(theme::durian_500().opacity(0.16))
            .text_size(px(12.0))
            .text_color(theme::durian_100())
            .child(Icon::new(IconName::TriangleAlert).size(px(16.0)))
            .child(
                div()
                    .id("desktop-persistence-error-message")
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .tooltip(move |window, cx| {
                        Tooltip::new(tooltip_error.clone()).build(window, cx)
                    })
                    .child(error),
            )
            .child(
                web_button(
                    "dismiss-persistence-error",
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::Enabled,
                )
                .aria_label("Dismiss persistence error")
                .on_click(cx.listener(|this, _event, _window, cx| {
                    cx.stop_propagation();
                    this.persistence_error = None;
                    cx.notify();
                }))
                .child(Icon::new(IconName::Close).size(px(14.0))),
            )
            .into_any_element()
    }

    /// With the sidebar hidden, the traffic lights still need a drag strip and
    /// there has to be a way back (the web only needs its floating "≡" on
    /// narrow viewports).
    fn render_collapsed_sidebar_strip(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("collapsed-sidebar-strip")
            .debug_selector(|| "desktop-collapsed-sidebar-strip".to_string())
            .size(px(44.0))
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(
                div()
                    .id("collapsed-sidebar-toggle")
                    .debug_selector(|| "desktop-sidebar-toggle".to_string())
                    .size(px(44.0))
                    .rounded(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::rambutan_400())
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(0.5))
                            .text_color(theme::rambutan_100())
                    })
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Toggle sidebar (⌘B)")
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        cx.stop_propagation();
                        this.toggle_sidebar_for_window(window, cx);
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle_sidebar_for_window(window, cx);
                        }
                    }))
                    .child(
                        svg()
                            .path("icons/panel-left.svg")
                            .size(px(16.0))
                            .text_color(theme::rambutan_400()),
                    ),
            )
            .into_any_element()
    }

    fn render_main_surface(&self, compact: bool, window: &Window, cx: &Context<'_, Self>) -> Div {
        match &self.active_view {
            AppView::Today => self.render_today_page(compact, cx),
            AppView::Inbox => self.render_inbox_page(cx),
            AppView::Knowledge => self.render_knowledge_page(cx),
            AppView::Chat => {
                let main: AnyElement = if let Some(thread) = self.active_thread() {
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .child(thread.chat.clone())
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .bg(cx.theme().background)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(CHAT_NO_ACTIVE_THREAD_TEXT_SIZE_PX))
                        // rambutan_500 is the contrast floor for muted text on
                        // the longan canvas (600 is 2.4:1, below WCAG AA).
                        .text_color(theme::rambutan_500())
                        .child(CHAT_NO_ACTIVE_THREAD_LABEL)
                        .into_any_element()
                };
                // ⌘J drawer: the terminal under the transcript, on a
                // draggable split whose height is remembered.
                if self.terminal_drawer_open {
                    let split = v_resizable("chat-terminal-split")
                        .with_state(&self.terminal_split_state)
                        .child(resizable_panel().child(main))
                        .child(
                            resizable_panel()
                                .size(px(self.terminal_drawer_height))
                                .size_range(
                                    px(TERMINAL_DRAWER_MIN_HEIGHT_PX)
                                        ..px(TERMINAL_DRAWER_MAX_HEIGHT_PX),
                                )
                                .child(self.terminal_panel.clone()),
                        );
                    div().flex_1().h_full().min_w_0().min_h_0().child(split)
                } else {
                    div()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(main)
                }
            }
            AppView::Terminal => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .flex()
                .flex_col()
                .child(self.render_terminal_page_header())
                .child(self.terminal_panel.clone()),
            AppView::Tasks => self.render_tasks_page(compact, cx),
            AppView::Calendar => self.render_calendar_page(cx),
            AppView::Life => self.render_life_page(window, cx),
            AppView::Automations => self.render_automations_page(window, cx),
            // Web-only surfaces. They're unreachable from the desktop command
            // bar, but shared AppView state can still land here — show an
            // explanation instead of a blank pane.
            AppView::Meetings => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .child(self.render_meetings_page(cx)),
            AppView::People => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .child(self.render_people_page(cx)),
            AppView::Search => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .child(self.render_search_page(window, cx)),
            AppView::MeetingDetail { meeting_id } => {
                let id = meeting_id.clone();
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .child(self.render_meeting_detail(&id, cx))
            }
            AppView::MeetingNoteDetail { note_id } => self.render_local_meeting_note(note_id, cx),
            AppView::DocumentDetail { document_id } => self.render_document_editor(document_id, cx),
            AppView::ClipDetail { .. } => self.render_source_clip_editor(false, cx),
            AppView::ConversationDetail { conversation_id } => {
                self.render_conversation_detail(conversation_id, cx)
            }
            AppView::Booking { .. } => self.render_calendar_page(cx),
            AppView::Settings => self.render_web_only_view(cx),
        }
    }

    /// Placeholder for AppView variants implemented only in bruhs-web
    /// (Meetings, Settings page, MeetingDetail, Search). Mirrors the
    /// no-active-thread empty-state styling.
    fn render_web_only_view(&self, cx: &Context<'_, Self>) -> Div {
        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(CHAT_NO_ACTIVE_THREAD_TEXT_SIZE_PX))
            .text_color(theme::rambutan_500())
            .child("This view is web-only for now — open bruhs.ai in your browser.")
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if !self.settings_visible() && self.credential_settings.read(cx).has_editor() {
            self.credential_settings
                .update(cx, |view, _| view.clear_editor());
        }
        for thread in &self.threads {
            let visible = self.active_view == AppView::Chat
                && self.active_thread_id.as_deref() == Some(thread.id.as_str());
            if !visible && thread.chat.read(cx).has_private_input() {
                thread.chat.update(cx, |chat, _| chat.clear_private_input());
            }
        }
        self.page_history.visit(&page_key(&self.active_view));
        let threads = self
            .threads
            .iter()
            .map(|thread| (thread.id.clone(), thread.chat.read(cx).thread_title.clone()))
            .collect();
        cx.set_global(crate::app_state::ThreadCatalog { threads });
        // A removed modal's input still owns its FocusHandle. Restore focus
        // before it leaves the dispatch tree, or global shortcuts go nowhere.
        if !self.overlay_open() {
            let previous_focus = self.palette_return_focus.take();
            if self.modal_start_focus_handle.contains_focused(window, cx) {
                let return_focus = previous_focus
                    .filter(|(view, _)| *view == self.active_view)
                    .map_or_else(|| self.workspace_focus_handle.clone(), |(_, focus)| focus);
                return_focus.focus(window, cx);
            }
        }
        let compact = Self::compact_layout(window);
        // Retain filters while the same dialog is open. On close, remove them
        // from the native layers so the Metal-backed workspace stays crisp.
        if self.top_layer.is_none() && !self.overlay.as_ref().is_some_and(Overlay::blurs_backdrop) {
            self.blur_scrim.hide();
        }
        if self.settings_visible() {
            self.sync_settings_scroll(window);
        }
        let terminal_mode = if self.active_view == AppView::Terminal {
            TerminalPanelMode::Page
        } else {
            TerminalPanelMode::Drawer
        };
        self.terminal_panel
            .update(cx, |panel, _| panel.set_mode(terminal_mode));
        self.sync_terminal_context(cx);
        // The preview's native view sits above everything gpui draws, so it
        // only shows while its surface is on screen with nothing over it.
        let preview_shown =
            self.preview_panel_visible && self.active_view == AppView::Chat && !self.overlay_open();
        self.preview_panel
            .update(cx, |panel, _| panel.set_shown(preview_shown));
        let body = div()
            .id("workspace-body")
            .role(Role::Main)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .relative()
            .overflow_hidden()
            .flex()
            .flex_row()
            .when(!compact && self.sidebar_visible, |el| {
                el.child(
                    div()
                        .w(px(WORKSPACE_SIDE_PANEL_WIDTH_PX))
                        .min_w(px(WORKSPACE_SIDE_PANEL_MIN_WIDTH_PX))
                        .h_full()
                        .flex_shrink_0()
                        .child(self.render_sidebar(cx)),
                )
            })
            .child({
                let main_column = div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.render_workspace_modes(compact, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(self.render_main_surface(compact, window, cx)),
                    )
                    .children(self.render_hub_notice(window, cx));
                if !compact && self.right_panel_open() && self.active_view == AppView::Chat {
                    // Web: the panel's left edge drags; the width is clamped
                    // to [360, viewport − 360] and remembered.
                    let viewport_width = f32::from(window.viewport_size().width);
                    let width = clamp_right_panel_width(self.right_panel_width, viewport_width);
                    let max_width =
                        (viewport_width - CHAT_COLUMN_MIN_WIDTH_PX).max(RIGHT_PANEL_MIN_WIDTH_PX);
                    div()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .min_h_0()
                        .child(
                            h_resizable("chat-panel-split")
                                .with_state(&self.right_panel_split_state)
                                .child(resizable_panel().child(main_column))
                                .child(
                                    resizable_panel()
                                        .size(px(width))
                                        .size_range(px(RIGHT_PANEL_MIN_WIDTH_PX)..px(max_width))
                                        .child(self.render_right_panel(cx)),
                                ),
                        )
                        .into_any_element()
                } else {
                    main_column.into_any_element()
                }
            })
            .children(self.render_live_meeting_pill(cx))
            .children(self.render_auto_record_prompt(cx))
            .when(compact && self.compact_sidebar_open, |el| {
                el.child(
                    div()
                        .id("compact-sidebar-backdrop")
                        .debug_selector(|| "desktop-compact-sidebar-backdrop".to_string())
                        .absolute()
                        .size_full()
                        .bg(theme::longan_950().opacity(0.58))
                        .occlude()
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            cx.stop_propagation();
                            this.compact_sidebar_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("compact-sidebar-drawer")
                        .debug_selector(|| "desktop-compact-sidebar-drawer".to_string())
                        .role(Role::Complementary)
                        .aria_label("Threads")
                        .absolute()
                        .left_0()
                        .top_0()
                        .w(px(WORKSPACE_SIDE_PANEL_WIDTH_PX))
                        .max_w_full()
                        .h_full()
                        .occlude()
                        .child(self.render_sidebar(cx)),
                )
            })
            .when(
                compact && self.right_panel_open() && self.active_view == AppView::Chat,
                |el| {
                    el.child(
                        div()
                            .id("compact-tasks-panel")
                            .debug_selector(|| "desktop-compact-tasks-panel".to_string())
                            .role(Role::Complementary)
                            .aria_label("Right panel")
                            .absolute()
                            .right_0()
                            .top_0()
                            .w(px(WORKSPACE_RIGHT_PANEL_WIDTH_PX))
                            .max_w_full()
                            .h_full()
                            .occlude()
                            .child(self.render_right_panel(cx)),
                    )
                },
            );

        div()
            .key_context("Workspace")
            .track_focus(&self.workspace_focus_handle)
            // Every bruhs type role is `line-height: 1.5`; gpui defaults to φ
            // (1.618), which made every row ~2px taller than the web's.
            .line_height(relative(WORKSPACE_LINE_HEIGHT))
            .on_action(cx.listener(Self::new_thread))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_tasks))
            .on_action(cx.listener(Self::focus_command_bar))
            .on_action(cx.listener(Self::focus_chat_input))
            .on_action(cx.listener(Self::open_task_create_shortcut))
            .on_action(cx.listener(Self::open_capture_dialog))
            .on_action(cx.listener(Self::close_overlays))
            .on_action(cx.listener(Self::open_chat_view))
            .on_action(cx.listener(Self::open_today_view))
            .on_action(cx.listener(Self::open_inbox_view_action))
            .on_action(cx.listener(Self::open_knowledge_view_action))
            .on_action(cx.listener(Self::save_knowledge_document_action))
            .on_action(cx.listener(Self::open_tasks_view))
            .on_action(cx.listener(Self::open_calendar_view))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::open_project_picker))
            .on_action(cx.listener(Self::cycle_permission_mode))
            .on_action(cx.listener(Self::pin_thread))
            .on_action(cx.listener(Self::settle_thread))
            .on_action(cx.listener(Self::copy_thread_link))
            .on_action(cx.listener(Self::copy_pull_link))
            .on_action(cx.listener(Self::copy_pull_number))
            .on_action(cx.listener(Self::toggle_diff))
            .on_action(cx.listener(Self::toggle_panel))
            .on_action(cx.listener(Self::close_panel))
            .on_action(cx.listener(Self::toggle_model_picker))
            .on_action(cx.listener(Self::toggle_workspace_menu))
            .on_action(cx.listener(Self::toggle_branch_menu))
            .on_action(cx.listener(Self::toggle_effort_menu))
            .on_action(cx.listener(Self::use_previous_worktree))
            .on_action(cx.listener(Self::prev_thread))
            .on_action(cx.listener(Self::next_thread))
            .on_action(cx.listener(Self::navigate_back))
            .on_action(cx.listener(Self::navigate_forward))
            .on_action(cx.listener(Self::jump_thread_action))
            .on_action(cx.listener(Self::mention_file))
            .on_action(cx.listener(Self::open_terminal))
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::open_terminal_view))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::open_local_servers))
            .on_action(cx.listener(Self::search_threads))
            .on_action(cx.listener(Self::undo_sidebar_action))
            .on_action(cx.listener(Self::open_project_in_editor_action))
            .on_action(cx.listener(Self::open_import_thread))
            .on_action(cx.listener(Self::copy_resume_command_action))
            .on_action(cx.listener(Self::copy_thread_markdown_action))
            .on_action(cx.listener(Self::continue_in_t3_action))
            .size_full()
            .relative()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .flex()
            .flex_col()
            .when(!self.save_status.is_saved(), |el| {
                el.child(self.render_save_status(cx))
            })
            .when_some(self.render_workflow_status(cx), |el, status| {
                el.child(status)
            })
            .when_some(self.persistence_error.clone(), |el, error| {
                el.child(self.render_persistence_error(error, cx))
            })
            .child(body)
            .when_some(self.overlay.clone(), |el, overlay| {
                el.child(match overlay {
                    Overlay::Capture => self.render_capture_dialog(cx),
                    Overlay::Settings { .. } => self.render_settings(window, cx),
                    Overlay::TaskCreate { .. } => self.render_task_create_dialog(window, cx),
                    Overlay::EventCreate { .. } => self.render_event_create_dialog(window, cx),
                    Overlay::ProjectDeleteConfirm { .. } => self.render_project_delete_confirm(cx),
                    Overlay::UpdateConfirm => self.render_update_confirm(cx),
                    Overlay::RestartConfirm => self.render_restart_confirm(cx),
                })
            })
            // Drawn after the form dialog so it sits on top of it.
            .when_some(self.top_layer, |el, top_layer| {
                el.child(match top_layer {
                    TopLayer::CommandBar => self.render_command_bar(cx),
                    TopLayer::ProjectPicker { .. } => self.render_project_picker(cx),
                    TopLayer::ImportThread => self.render_import_thread(cx),
                })
            })
    }
}

#[cfg(test)]
mod tests;
