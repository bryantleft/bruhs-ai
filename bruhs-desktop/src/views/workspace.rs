use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use gpui::prelude::*;
use gpui::*;
use bruhs_ui::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use bruhs_ui::resizable::{
    h_resizable, resizable_panel, v_resizable, ResizablePanelEvent, ResizableState,
};
use bruhs_ui::scroll::ScrollableElement;
use bruhs_ui::tooltip::Tooltip;
use bruhs_ui::zoom::px;
use bruhs_ui::{ActiveTheme, Icon, IconName, WindowExt};
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
        ChatView, ChatViewEvent, ChatWidth, ComposerPrefs, CyclePermissionMode, DesktopToolCall, Message,
        MessageRole, PermissionMode, SendShortcut, TurnState,
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
use bruhs_models::thread_link::ThreadAccess;
use bruhs_models::EpochMs;
use bruhs_models::{
    backlinks_for, AutomationState, CaptureKind, CaptureRecord, ConflictResolution,
    InboxConversation, InboxMessage, InboxParticipant, InboxSource, KnowledgeCitation,
    KnowledgeDocument, KnowledgeLink, KnowledgeLinkKind, LifeContext, PersonProfile,
    RelationshipItemKind, RelationshipProfile, SchedulingLink, SchedulingPreferences, SourceClip,
    TodaySnapshot, WorkItem, WorkItemPlanning, WorkItemSource,
};
use bruhs_ui::press::on_press_view;
use bruhs_ui::text_field::TextInput;

mod app_links;
mod default_model;
mod native_model_options;
use bruhs_models::delivery::{generation::GenerationOptions,writer_catalog::WriterCatalog};
mod apps_settings;
mod automations_ui;
mod backup_settings;
mod booking_ui;
#[cfg(test)]
mod breakpoint_tests;
mod calendar_ui;
mod capture_ui;
#[cfg(test)]
mod chat_events_tests;
mod connected_inbox;
mod crew_ui;
pub(super) mod date_picker_ui;
mod event_dialog_ui;
#[cfg(test)]
mod import_tests;
mod import_ui;
mod move_in_ui;
#[cfg(test)]
mod keyboard_tests;
mod mail_compose;
mod vault_settings;
#[cfg(test)]
mod zoom_tests;
pub(crate) use import_ui::portable_messages;
mod inbox_ui;
mod keybindings_ui;
mod language;
mod linked_keybindings;
pub(crate) use linked_keybindings::{LinkedDeliveries, TaskEditorHost};
mod keybinding_file;
mod knowledge_storage;
mod knowledge_ui;
mod library_pages_ui;
mod life_ui;
mod mcp_bridge;
mod meetings_ui;
mod mobile;
mod modal_ui;
#[cfg(all(feature = "native-performance", target_os = "macos"))]
pub(crate) mod native_fixture;
mod navigation_ui;
#[cfg(test)]
mod notification_tests;
#[cfg(test)]
mod overlay_tests;
#[cfg(test)]
mod palette_tests;
mod palette_ui;
mod panels_ui;
#[cfg(test)]
mod performance_tests;
mod persisted_enum;
mod persistence;
mod archive;
mod clone_project;
mod project_defaults;
mod kiro_options;
mod project_actions;
mod project_art;
mod project_order;
mod project_grouping;
mod project_name;
mod project_removal;
mod delivery_preferences;
mod source_writer;
mod environments;
mod diagnostics;
mod device_settings;
mod telemetry;
mod maintenance;
mod font_settings;
mod voice_dictionary;
mod voice_runtime;
mod voice_models;
mod voice_cloud;
mod voice_cleanup;
mod voice_shortcut;
mod microphone_test;
mod voice_settings;
mod snapshots;
mod snooze_options;
mod quota_settings;
pub(crate) mod appearance;
mod color_sliders;
mod browser_settings;
mod thread_notifications;
mod person_merge;
#[cfg(test)]
mod project_picker_tests;
mod project_picker_ui;
mod record_picker_ui;
mod role_settings_ui;
mod room_ui;
mod save_queue;
mod save_status;
mod scheduling_ui;
mod search_record_detail;
mod license_browser;
mod settings_page_ui;
mod settings_ui;
mod sidebar_ui;
#[cfg(debug_assertions)]
mod smoke;
mod snooze_ui;
mod state_file;
#[cfg(test)]
mod state_fixture_tests;
mod task_forms_ui;
mod tasks_ui;
#[cfg(all(test, unix))]
mod thread_auth_tests;
#[cfg(test)]
mod thread_catalog_tests;
mod thread_link_ui;
mod thread_details;
mod thread_forks;
mod thread_snapshot;
mod scheduled_replies;
mod thread_titles;
mod welcome;
mod labs;
mod pull_settlement;
mod provider_health;
mod setting_source;
use setting_source::ScopedSwitch;
mod provider_editor;
mod usage_hubs;
mod text_generators;
mod quit_shortcut;
mod secret_source_settings;
mod panel_motion;
mod environment_mark;
mod fallback_model;
mod worktree_naming;
mod worktree_storage;
mod threads_ui;
mod today_ui;
mod updates_ui;
mod view_restore;
mod whiteboard_ui;
mod work_item_handoff_ui;
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
/// Escape closes overlays; shared with the `bruhs-ui` components.
pub use bruhs_ui::actions::CloseOverlays;
actions!(
    workspace,
    [
        NewThread,
        ToggleSidebar,
        ToggleTasks,
        FocusCommandBar,
        FocusChatInput,
        FocusCrew,
        StopAll,
        OpenTaskCreateShortcut,
        OpenCapture,
        OpenChatView,
        OpenTodayView,
        OpenInboxView,
        OpenKnowledgeView,
        SaveKnowledgeDocument,
        OpenTasksView,
        OpenCalendarView,
        OpenSettings,
        CaptureWindow,
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
        ContinueInWorkbench,
        ZoomIn,
        ZoomOut,
        ResetZoom
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
    ("Jump to thread 1 to 9", &["⌘1", "⌘9"]),
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
            Self::None => "",
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
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<WorkItemSource>,
    updated_at: EpochMs,
}

impl DesktopWorkItem {
    fn to_work_item(&self) -> WorkItem {
        WorkItem {
            id: self.id.clone(),
            project_id: self.project_id.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            status: self.status,
            priority: self.priority,
            due_date: self.due_date.clone(),
            planning: self.planning.clone(),
            source: self.source.clone(),
            created_at: 0,
            updated_at: self.updated_at.as_millis(),
        }
    }
}

#[derive(Clone)]
struct DesktopTaskDrag {
    item_id: String,
    title: String,
    priority: DesktopPriority,
}

/// A calendar event on the desktop grid. `start_at` and `end_at` are the
/// wall-clock view of the stored instants in the machine zone, as the web
/// shows `Date`s; the state file stores the instants (CAL-13).
#[derive(Clone, Default, Serialize)]
struct DesktopCalendarEvent {
    id: String,
    title: String,
    start_at: NaiveDateTime,
    end_at: NaiveDateTime,
    all_day: bool,
    color: Option<String>,
    notes: Option<String>,
    /// Set for a provider (Google) event, which is read only here.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<DesktopProviderEvent>,
    /// The UTC instants (ms) this event was loaded or received with. While
    /// the wall times still read the same, saving keeps them, so a time in
    /// the repeated hour of a fall-back day keeps its own occurrence.
    #[serde(skip)]
    loaded_instants: Option<(i64, i64)>,
}

/// Provider details of a read-only event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
struct DesktopProviderEvent {
    /// The provider's web link, opened instead of editing.
    html_link: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DesktopAppliedPlan {
    ownership: bruhs_models::DayPlanOwnership,
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
    Welcome,
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
    /// Every event and due task of one month day (web "Calendar day").
    CalendarDay {
        date: NaiveDate,
    },
    ProjectDeleteConfirm {
        project_id: String,
    },
    /// The task detail's "Delete task?" (web `TaskDetail`).
    TaskDeleteConfirm {
        item_id: String,
    },
    /// The Tasks header's "Projects" in a compact window, where the project
    /// sidebar cannot show (web `TasksPage`).
    TaskProjects,
    RecordDeleteConfirm {
        target: RecordDeleteTarget,
    },
    /// The sidebar Update pill's "Update N providers?".
    UpdateConfirm { from_settings: bool, details_open: bool },
    ThemeEditor(appearance::ThemeEditor),
    RemoveThemeVariants(appearance::VariantRemoval),
    /// "Restart to update" for a downloaded release.
    RestartConfirm { previous: Option<Box<Overlay>> },
    /// The thread menu's "Custom…" snooze (web "Snooze until").
    SnoozeCustom {
        thread_id: String,
        unit: bruhs_models::logic::sidebar::SnoozeUnit,
        /// The unit dropdown's listbox.
        unit_open: bool,
    },
    /// The crew rail's "New bruh" (web `CreateBruhDialog`).
    CreateBruh,
    /// New room or Room settings (web `RoomDialog`).
    RoomDialog,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecordDeleteTarget {
    Document(String),
    SourceClip(String),
    Conversation(String),
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
        thread: Option<EntityId>,
    },
    /// Import a thread from Claude Code, Codex, T3 Code or Cursor.
    ImportThread,
    /// Move in: a bruh from OpenClaw or Hermes Agent, or memory By prompt.
    MoveIn,
    /// Connect a record or cite a source on the open document (web
    /// `RecordPicker`), with the highlighted match.
    RecordPicker {
        target: record_picker_ui::RecordPickerTarget,
        highlighted: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskDetailMenu {
    Project,
    Status,
    Priority,
    Dependency,
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
    Project {path:String,name:String},
    LinkedProject(project_grouping::Member),
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
        _ => provider.label(),
    }
}

pub(crate) fn accent_dot(color: u32) -> Div {
    div()
        .size(px(12.))
        .flex_none()
        .rounded_full()
        .bg(gpui::rgb(color))
        .border_1()
        .border_color(theme::rambutan_400().opacity(0.65))
}

pub(crate) fn provider_identity_mark(
    provider: AgentProvider,
    accent: Option<u32>,
    cx: &App,
) -> AnyElement {
    match accent {
        Some(color) => accent_dot(color).into_any_element(),
        None => provider_mark(provider, cx),
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
fn redraw_on_edit<T: EventEmitter<InputEvent>>(
    input: &Entity<T>,
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
    Appearance,
    Chat,
    Diffs,
    Projects,
    SourceControl,
    Maintenance,
    Diagnostics,
    Labs,
    Worktrees,
    Browser,
    ScheduledTasks,
    Environments,
    Devices,
    Access,
    Voice,
    SnapShots,
    Threads,
    Archived,
    Updates,
    Vault,
    Apps,
    Backup,
    Calendar,
    Meetings,
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
    settled_at_ms: Option<EpochMs>,
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
    #[serde(skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    favorite_models: super::model_browser::Favorites,
    #[serde(with = "persisted_enum::optional")]
    permission_mode: Option<PermissionMode>,
    sidebar_visible: Option<bool>,
    #[serde(with = "persisted_enum::view_route")]
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
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    desktop_task_links: BTreeMap<String,BTreeMap<String,String>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    desktop_task_cursors: BTreeMap<String,i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_thread_view: Option<crate::views::reactive_automations::TaskThreadLocation>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    title_requests: BTreeMap<String, thread_titles::PendingTitle>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    archived_threads: Vec<archive::ArchivedRecord>,
    projects: Vec<PersistedProject>,
    work_items: Vec<PersistedWorkItem>,
    calendar_events: Vec<PersistedCalendarEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    day_plan_undo: Option<DesktopAppliedPlan>,
    captures: Vec<CaptureRecord>,
    knowledge_documents: Vec<KnowledgeDocument>,
    source_clips: Vec<SourceClip>,
    inbox_conversations: Vec<InboxConversation>,
    meeting_notes: Vec<bruhs_models::MeetingNote>,
    relationships: Vec<RelationshipProfile>,
    people_cache: PeopleSnapshot,
    meeting_cache: MeetingCache,
    meeting_actions: crate::views::meetings_hub::ActionCheckpoints,
    /// Read-only Google Calendar events, so they show offline and after a
    /// restart (the web keeps them in IndexedDB). They stay on this device:
    /// the agent's calendar tools read only local events, and Copy recovery
    /// leaves them out.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    google_events: Vec<bruhs_models::CalendarEvent>,
    scheduling_preferences: SchedulingPreferences,
    life_context: LifeContext,
    automation_state: AutomationState,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_start_folder: Option<String>,
    #[serde(skip_serializing_if="skip_false")]
    clone_host_folders: bool,
    #[serde(skip_serializing_if="Option::is_none")]
    idle_settle_days: Option<u16>,
    pull_preferences: pull_settlement::Preferences,
    #[serde(default, skip_serializing_if="BTreeMap::is_empty")]
    known_thread_pulls: BTreeMap<String,pull_settlement::Observation>,
    #[serde(skip_serializing_if="Option::is_none")]
    maintenance:Option<maintenance::MaintenancePreferences>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    project_defaults: BTreeMap<String, project_defaults::ProjectDefaults>,
    #[serde(skip_serializing_if="BTreeMap::is_empty")]
    project_actions: BTreeMap<String, Vec<project_actions::ProjectAction>>,
    #[serde(skip_serializing_if="Option::is_none")]
    snooze_options: Option<Vec<snooze_options::Choice>>,
    #[serde(skip_serializing_if="Option::is_none")]
    quota_prefs:Option<quota_settings::Preferences>,
    #[serde(default, skip_serializing_if="BTreeMap::is_empty")]
    quota_recoveries: BTreeMap<String, quota_settings::Recovery>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    interrupted_runs: BTreeMap<String, maintenance::restart::Interrupted>,
    #[serde(default, skip_serializing_if="GenerationOptions::is_empty")]
    default_options: GenerationOptions,
    #[serde(default, skip_serializing_if="GenerationOptions::is_empty")]
    fallback_options: GenerationOptions,
    #[serde(skip_serializing_if="Option::is_none")]
    fallback_model:Option<String>,
    #[serde(default,skip_serializing_if="super::chat::kiro::KiroChoices::is_empty")]
    default_kiro:super::chat::kiro::KiroChoices,
    #[serde(default,skip_serializing_if="super::chat::kiro::KiroChoices::is_empty")]
    fallback_kiro:super::chat::kiro::KiroChoices,
    #[serde(skip_serializing_if="BTreeMap::is_empty")]
    provider_resets:BTreeMap<String,i64>,
    #[serde(default,skip_serializing_if="project_order::ProjectList::empty")]
    project_list:project_order::ProjectList,
    #[serde(skip_serializing_if="Option::is_none")]
    response_stream:Option<super::chat::response_stream::Mode>,
    #[serde(skip_serializing_if="Option::is_none")]
    text_generation_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    browser_preferences: Option<super::browser_preferences::BrowserPreferences>,
    active_project: Option<PersistedProjectLocation>,
    recent_projects: Vec<PersistedProjectLocation>,
    sidebar_settled_open: Option<bool>,
    /// Missing means closed, matching the web default.
    sidebar_working_open: Option<bool>,
    sidebar_snoozed_open: Option<bool>,
    /// Missing means open, matching the web default.
    crew_rail_open: Option<bool>,
    /// Missing means the Working section is on.
    sidebar_working_section: Option<bool>,
    labs_sidebar: labs::Folders,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_notifications: Option<thread_notifications::NotificationSettings>,
    confirm_thread_archive: Option<bool>,
    confirm_thread_unpin: Option<bool>,
    confirm_thread_delete: Option<bool>,
    #[serde(default)]
    quit_mode:quit_shortcut::Mode,
    /// Missing means update checks are on (web `bruhs.updateChecks`).
    update_checks: Option<bool>,
    #[serde(default)]
    update_track: Option<updates_ui::UpdateTrack>,
    title_instructions: Option<String>,
    title_instructions_by_project: BTreeMap<String, String>,
    editor_id: Option<String>,
    /// Height of the ⌘J terminal drawer (web: `bruhs.terminalHeight`).
    terminal_drawer_height: Option<f32>,
    /// Web `bruhs.rightPanelWidth`.
    right_panel_width: Option<f32>,
    /// Page zoom (⌘+ / ⌘− / ⌘0), the browser zoom of the web app. Absent
    /// means 100%.
    #[serde(skip_serializing_if = "Option::is_none")]
    zoom: Option<f32>,
    /// Web `bruhs.whisper_model` / `bruhs.diarize` / `bruhs.auto_record` /
    /// `bruhs.personality` preferences.
    meeting_prefs: Option<MeetingPrefs>,
    /// `checkout`, `new_worktree`, or `previous_worktree`. Empty inherits the repository default.
    #[serde(default)]
    run_choice: String,
    /// Claude `--effort` for the next send. Codex has no effort flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_effort: Option<String>,
    /// `steer`, or absent for queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    follow_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    send_shortcut: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    slash_skills:Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    composer_rich_text: Option<bool>,
    tool_visibility: crate::views::chat::tool_visibility::Visibility,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_reader: Option<crate::views::chat::screen_reader::ReaderPreferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_width: Option<ChatWidth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    appearance: Option<appearance::DeviceAppearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_themes: Option<Arc<appearance::ThemeLibrary>>,
    #[serde(default, skip_serializing_if="BTreeMap::is_empty")]
    thread_origins: BTreeMap<String,thread_details::Origin>,
    #[serde(default, skip_serializing_if="std::collections::BTreeSet::is_empty")]
    hidden_inline_thread_details: std::collections::BTreeSet<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    fonts:Option<font_settings::DeviceFonts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshots: Option<snapshots::Preferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dictation_prefs: Option<crate::voice_capture::DictationPreferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_prefs: Option<bruhs_models::logic::run_place::WorktreePreferences>,
    worktree_retention: worktree_storage::WorktreeRetention,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending_scratch_cleanups: Vec<worktree_storage::ScratchCleanup>,
    cleanup_deleted_worktrees:bool,
    cleanup_keep_when: bruhs_models::coordination::worktree::KeepWhen,
    health_interval:u32,
    branch_naming: bruhs_models::logic::run_place::branch_naming::Naming,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_preferences: Option<crate::views::diff_panel::DiffPreferences>,
    /// Claude `--autocompact` token count. Absent omits the flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_autocompact: Option<String>,
    /// `before` or `never`. Absent means compact when you return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compact_idle: Option<String>,
    /// `read` or `off`. Absent lets agents message other threads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread_access: Option<String>,
    /// Agent-to-agent messages allowed in a row. Absent means the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread_hops: Option<u32>,
    /// The user's roles and each project's overrides. Absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roles: Option<bruhs_models::roles::RoleBook>,
    /// Models for meeting summaries and second opinions. Absent means the
    /// built-in defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_models: Option<bruhs_models::roles::TaskModels>,
    /// Live meeting suggestion settings. Absent means the defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    meeting_suggestions: Option<bruhs_models::meeting_suggestions::MeetingSuggestionPrefs>,
    /// Last worktree this client created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_worktree: Option<RememberedWorktree>,
    /// Shortcut rules. Empty means the shipped defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    keybindings: Vec<bruhs_models::logic::keybindings::Keybinding>,
    #[serde(default, skip_serializing_if = "linked_keybindings::SavedLinks::is_empty")]
    shortcut_links: linked_keybindings::SavedLinks,
    #[serde(default, skip_serializing_if = "linked_keybindings::SavedLinks::is_empty")]
    project_links: linked_keybindings::SavedLinks,
    /// Records the addition of the native Settings shortcut once per device.
    keybinding_schema: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedThread {
    #[serde(default, skip_serializing_if="GenerationOptions::is_empty")]
    model_options: GenerationOptions,
    #[serde(default,skip_serializing_if="super::chat::kiro::KiroChoices::is_empty")]
    kiro_choices: super::chat::kiro::KiroChoices,
    id: String,
    title: String,
    project_name: String,
    project_path: Option<String>,
    model_id: String,
    #[serde(default, with = "persisted_enum::required")]
    permission_mode: PermissionMode,
    #[serde(default,skip_serializing_if="skip_false")]
    project_permission_override: bool,
    messages: Vec<PersistedMessage>,
    /// Last activity. Builds before 2026-10 also wrote a placeholder
    /// `updated_at: "now"` string, which is ignored on read.
    #[serde(default, deserialize_with = "bruhs_models::time::deserialize_legacy")]
    updated_at_ms: EpochMs,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    settled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settled_at_ms: Option<EpochMs>,
    #[serde(default, skip_serializing_if = "skip_false")]
    cleanup_busy: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    import_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fork: Option<std::sync::Arc<crate::views::chat::ForkContext>>,
    /// Local `/goal` text. Missing on states saved before goals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    goal: Option<bruhs_models::logic::commands::ThreadGoal>,
    /// Worktree this thread runs in, once the first send created or reused one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    project_settings_root: Option<String>,
    /// Effort applied on the last successful send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    /// The thread keeps its checkout or worktree. Missing means it has not sent.
    #[serde(default, skip_serializing_if = "skip_false")]
    run_locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draft_placement: Option<crate::views::chat::DraftPlacement>,
    /// Id of the thread's role: an agent started it with one, or the user
    /// set it with `/role`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    /// Stamp of the role instructions the thread's session started with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instructions_stamp: Option<String>,
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
    settled_at_ms: Option<EpochMs>,
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
    #[serde(default, skip_serializing_if = "skip_false")]
    interrupted_by_restart: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<WorkItemSource>,
    /// Builds before 2026-10 wrote the placeholder `"now"`, which reads as
    /// the epoch: the time was never recorded.
    #[serde(deserialize_with = "bruhs_models::time::deserialize_legacy")]
    updated_at: EpochMs,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedCalendarEvent {
    id: String,
    title: String,
    /// The wall-clock start in the zone at save time. Older builds stored
    /// only these floating times; they still read them.
    start_at: String,
    end_at: String,
    all_day: bool,
    color: Option<String>,
    notes: Option<String>,
    /// The UTC instants, as the web stores them. Missing in files written
    /// before CAL-13; such times are read in the current zone once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_at_ms: Option<i64>,
}

const DESKTOP_DATE_FORMAT: &str = "%Y-%m-%d";
const DESKTOP_DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";
const DESKTOP_DIALOG_MD_WIDTH_PX: f32 = 448.0;
const DESKTOP_DIALOG_LG_WIDTH_PX: f32 = 576.0;
/// Web `max-w-2xl` (New bruh).
const DESKTOP_DIALOG_2XL_WIDTH_PX: f32 = 672.0;
const DESKTOP_DIALOG_MAX_WIDTH_RATIO: f32 = 0.9;
const DESKTOP_DIALOG_RADIUS_PX: f32 = 14.0;
const DESKTOP_MODAL_TITLE_TEXT_SIZE_PX: f32 = 16.0;
const DESKTOP_MODAL_OVERLAY_OPACITY: f32 = 0.70;
const DESKTOP_MODAL_HEADER_X_PADDING_PX: f32 = 16.0;
const DESKTOP_MODAL_HEADER_Y_PADDING_PX: f32 = 10.0;
/// Where top-anchored overlays hang: the command bar and the pickers whose
/// lists change length, so they do not jump while you type or browse.
const DESKTOP_DIALOG_TOP_PADDING_PX: f32 = 80.0;
const DESKTOP_DIALOG_BOTTOM_GAP_PX: f32 = 16.0;
/// Room kept above and below a centered dialog (shadcn `Dialog` centers).
const DESKTOP_DIALOG_EDGE_GAP_PX: f32 = 32.0;
/// shadcn `SidebarTrigger` (`size-7`).
const SIDEBAR_TRIGGER_PX: f32 = theme::CONTROL_SM_PX;
const DESKTOP_DIALOG_MIN_AVAILABLE_HEIGHT_PX: f32 = 240.0;
const FORM_DIALOG_PREFERRED_HEIGHT_PX: f32 = 650.0;
fn form_text_input(input: Input) -> Input {
    // `Input::h` (inherent) only applies to multi-line inputs; the `Styled`
    // height is refined last, so it sizes single-line fields too.
    let input = Styled::h(input, px(FORM_INPUT_HEIGHT_PX));
    input
        .px(px(FORM_INPUT_X_PADDING_PX))
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(FORM_INPUT_TEXT_SIZE_PX))
}

fn form_textarea<E: Styled>(input: E, height_px: f32) -> E {
    input
        .h(px(height_px))
        .px(px(FORM_INPUT_X_PADDING_PX))
        .py(px(FORM_INPUT_Y_PADDING_PX))
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(FORM_INPUT_TEXT_SIZE_PX))
}
/// bruhs type scale: every role is `line-height: 1.5`.
const WORKSPACE_LINE_HEIGHT: f32 = 1.5;
/// The web's Tailwind breakpoints (`sm`, `md`, `lg`). Its layouts change at
/// these window widths, so each desktop layout switches where its web
/// counterpart does.
const SM_BREAKPOINT_PX: f32 = 640.0;
const MD_BREAKPOINT_PX: f32 = 768.0;
const LG_BREAKPOINT_PX: f32 = 1024.0;
/// Web `w-[min(88vw,24rem)]`: the right panel's width as a drawer below `md`.
const RIGHT_PANEL_DRAWER_WIDTH_PX: f32 = 384.0;
const RIGHT_PANEL_DRAWER_MAX_WINDOW_SHARE: f32 = 0.88;
/// Web `bg-longan-950/70`: the scrim under the sidebar and right panel drawers.
const DRAWER_SCRIM_OPACITY: f32 = 0.70;
const CHAT_NO_ACTIVE_THREAD_LABEL: &str = "Pick a conversation, or start a new one.";
const CHAT_NO_ACTIVE_THREAD_TEXT_SIZE_PX: f32 = 13.0;
const WORKSPACE_SIDE_PANEL_WIDTH_PX: f32 = SIDEBAR_WIDTH_PX;
const WORKSPACE_SIDE_PANEL_MIN_WIDTH_PX: f32 = 208.0;
/// Diff rows need more room than the tasks list.
/// Web `RIGHT_PANEL_DEFAULT_WIDTH`.
const WORKSPACE_RIGHT_PANEL_WIDTH_PX: f32 = 540.0;
/// Web `RIGHT_PANEL_MIN_WIDTH`.
const RIGHT_PANEL_MIN_WIDTH_PX: f32 = 360.0;
const WORKSPACE_SIDEBAR_BORDER_OPACITY: f32 = 0.50;
const WORKSPACE_SIDEBAR_TAB_RADIUS_PX: f32 = 4.0;
const WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY: f32 = 0.30;
const FORM_FIELD_GAP_PX: f32 = 6.0;
const FORM_CONTROL_RADIUS_PX: f32 = 8.0;
const FORM_SELECT_BODY_FONT_FAMILY: &str = "Monaspace Neon";
const FORM_SELECT_BODY_TEXT_SIZE_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
const FORM_SELECT_HEIGHT_PX: f32 = theme::CONTROL_PX;
const FORM_SELECT_MENU_MAX_HEIGHT_PX: f32 = 184.0;
const FORM_INPUT_HEIGHT_PX: f32 = theme::APP_CONTROL_HEIGHT_PX;
const FORM_INPUT_TEXT_SIZE_PX: f32 = theme::APP_INPUT_TEXT_SIZE_PX;
const FORM_INPUT_X_PADDING_PX: f32 = 10.0;
const FORM_INPUT_Y_PADDING_PX: f32 = 6.0;
const FORM_THREE_ROW_TEXTAREA_HEIGHT_PX: f32 = 76.0;
const FORM_FIVE_ROW_TEXTAREA_HEIGHT_PX: f32 = 118.0;
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
    database_panel: Option<Entity<crate::views::database_panel::DatabasePanel>>,
    project_tool: panels_ui::ProjectTool,
    api_client_panel: Option<Entity<crate::views::api_client_panel::ApiClientPanel>>,
    files_dir: String,
    files_root: Option<String>,
    files_entries: Vec<bruhs_core::files::FileEntry>,
    files_error: Option<String>,
    files_loading: bool,
    files_generation: u64,
    pull_panel_visible: bool,
    branch_pull: Option<DesktopBranchPull>,
    known_thread_pulls: BTreeMap<String,pull_settlement::Observation>,
    pull_preferences: pull_settlement::Preferences,
    pull_check_times: BTreeMap<String,std::time::Instant>,
    pull_lifecycle_task: Option<Task<()>>,
    linked_pull: Option<(Option<String>, DesktopBranchPull)>,
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
    reactive_automations: Entity<crate::views::reactive_automations::ReactiveAutomations>,
    palette_mode: PaletteMode,
    selected_search_record: Option<search_record_detail::SelectedSearchRecord>,
    search_record_focus: FocusHandle,
    automation_error: Option<String>,
    reviewed_workflow_id: Option<String>,
    mcp_bridge: mcp_bridge::WorkspaceBridge,
    /// Agent workflows' scroll area.
    automations_scroll: ScrollHandle,
    /// Calendar's Week and Day grid. Like the web's, it opens two hours
    /// before now each time it is shown (`calendar_timegrid_autoscroll`).
    calendar_timegrid_scroll: ScrollHandle,
    calendar_timegrid_autoscroll: std::cell::Cell<bool>,
    /// The page's own dry run (New dry run, a suggestion's or a draft's
    /// preview), reviewed in Preview changes while it waits.
    dry_run_workflow_id: Option<String>,
    /// Opened "Review workflow" disclosures, by place and workflow.
    workflow_disclosures: HashSet<String>,
    /// The app-wide workflow notice put away, until it changes.
    dismissed_workflow_notice: Option<String>,
    /// The workflow notice's Open workflows, which Pause hands focus to.
    workflow_notice_open_focus: FocusHandle,
    /// The conversation page's Back to Inbox, which an opened row focuses.
    inbox_back_focus: FocusHandle,
    /// Each Inbox row's focus, kept while the list is hidden (an element's
    /// own handle goes with it), so Back to Inbox can return to the row.
    inbox_row_focus: RefCell<std::collections::HashMap<String, FocusHandle>>,
    /// The conversation Back to Inbox returns focus to.
    inbox_return_row: Option<String>,
    mobile: mobile::MobileUi,
    local_servers: Vec<LocalServer>,
    local_servers_loading: bool,
    #[cfg_attr(
        test,
        expect(dead_code, reason = "only the cfg(not(test)) server scan reads it")
    )]
    local_servers_generation: u64,
    settings_section: SettingsSection,
    license_browser: Option<Entity<license_browser::LicenseBrowser>>,
    license_trigger: FocusHandle,
    license_forward: bool,
    vault_settings: Entity<vault_settings::VaultSettings>,
    apps_settings: Entity<apps_settings::AppsSettings>,
    backup_settings: Entity<backup_settings::BackupSettings>,
    /// The Vault settings under the name uncommitted work still uses (landing shim).
    credential_settings: Entity<vault_settings::VaultSettings>,
    /// Provider CLI and app update checks (Settings → Updates, sidebar button).
    updates: Entity<updates_ui::DesktopUpdates>,
    settings_search: Entity<InputState>,
    settings_model_query: Entity<InputState>,
    settings_model_trigger: FocusHandle,
    settings_model_rail: FocusHandle,
    settings_model_listings: Vec<bruhs_models::runtime_models::Listing>,
    settings_models_load: Option<Task<()>>,
    thread_catalog: Arc<WriterCatalog>,
    thread_catalog_load: Option<Task<()>>,
    default_options: GenerationOptions,
    fallback_options: GenerationOptions,
    settings_search_results: Vec<SettingsHit>,
    settings_search_catalog: Vec<SettingsHit>,
    settings_search_index: usize,
    settings_search_open: bool,
    settings_nav_focus: [FocusHandle; SettingsSection::ALL.len()],
    settings_nav_scroll: ScrollHandle,
    settings_nav_reveal: Rc<std::cell::Cell<bool>>,
    settings_search_inventory: RefCell<Option<Vec<SettingsHit>>>,
    settings_search_anchor: Option<SettingsReveal>,
    /// Scroll position within the selected section.
    settings_scroll: ScrollHandle,
    /// Last-frame bounds of each settings slider track, for pointer hit-testing.
    settings_slider_bounds: Vec<Rc<std::cell::Cell<Option<Bounds<Pixels>>>>>,
    settings_slider_drag: Option<settings_ui::SettingsSlider>,
    /// Text fields of the Recording steppers (web `Stepper` input).
    settings_stepper_inputs: Vec<Entity<InputState>>,
    settings_transcription_model_input: Entity<InputState>,
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
    day_plan_error: Option<String>,
    work_item_handoff: work_item_handoff_ui::HandoffUi,
    /// An applied or undone plan waits for the workspace save that carries
    /// it; a failed save says so on Today (web `DayPlanState::busy`).
    day_plan_saving: bool,
    /// Today's recovery is reading the saved workspace again.
    day_plan_reading: bool,
    /// The saved day plan was read at startup. False only when the saved
    /// workspace could not be read, so Today offers "Retry day-plan
    /// recovery" (web `DayPlanState::loaded`).
    day_plan_loaded: bool,
    #[cfg(test)]
    day_plan_test_now: Option<chrono::DateTime<Local>>,
    persistence: Arc<PersistenceCoordinator>,
    save_queue: save_queue::SaveQueue,

    /// Title search box in the sidebar (web: the `Search` input).
    sidebar_search: Entity<InputState>,
    /// Geometry and keyboard targets for visible sidebar rows.
    sidebar_viewport: RefCell<sidebar_ui::SidebarViewport>,
    /// Threads whose turn finished while another thread was active.
    unread_thread_ids: HashSet<String>,
    sidebar_settled_open: bool,
    sidebar_working_open: bool,
    sidebar_snoozed_open: bool,
    /// The crew rail above the threads; open unless closed.
    crew_rail_open: bool,
    /// The crew rail's first row and its New bruh button, so ⌘⇧B can focus them.
    crew_row_focus: FocusHandle,
    crew_new_focus: FocusHandle,
    /// The crew rail's one scroll region, bruhs then rooms.
    crew_scroll: ScrollHandle,
    /// On unless this device saved it off.
    sidebar_working_section: bool,
    labs_sidebar: labs::Folders,
    thread_notifications: thread_notifications::NotificationSettings,
    confirm_thread_archive: bool,
    confirm_thread_unpin: bool,
    confirm_thread_delete: bool,
    quit_gesture:quit_shortcut::Gesture,
    voice_press: Option<voice_shortcut::Press>,
    title_instructions: String,
    title_instructions_by_project: BTreeMap<String, String>,
    title_instructions_input: Entity<TextareaState>,
    /// The project whose title instructions Settings shows; empty for none.
    title_project: String,
    title_project_instructions_input: Entity<TextareaState>,
    keybindings: Vec<bruhs_models::logic::keybindings::Keybinding>,
    keybinding_index: usize,
    keymap_file:keybinding_file::KeymapFile,
    linked_keybindings: linked_keybindings::LinkedKeybindings,
    linked_projects: linked_keybindings::LinkedProjects,
    keybinding_edit_open: bool,
    keybinding_draft: Option<keybindings_ui::NewBinding>,
    keybinding_when_open: Option<keybindings_ui::BindingEditor>,
    keybinding_recording: Option<keybindings_ui::BindingEditor>,
    keybinding_record_focus: FocusHandle,
    keybinding_add_focus: FocusHandle,
    keybinding_condition_focus: FocusHandle,
    keybinding_filter: Entity<InputState>,
    keybinding_key_input: Entity<InputState>,
    keybinding_when_input: Entity<InputState>,
    /// Shortcuts currently installed, so the next edit can unbind them.
    keybinding_installed: Vec<keybindings_ui::InstalledBinding>,
    /// Thread whose action menu is open.
    thread_menu_id: Option<String>,
    /// Focus targets for the thread menu's items (`ThreadMenuItem` slots).
    thread_menu_items: Rc<[FocusHandle]>,
    /// The thread menu's "Custom…" snooze fields.
    snooze_inputs: snooze_ui::SnoozeInputs,
    /// The knowledge record picker's search field and list scroll.
    record_picker: record_picker_ui::RecordPickerUi,
    /// The booking page's name field and the booking made on it.
    booking: booking_ui::BookingUi,
    sidebar_undo: Option<SidebarUndo>,
    sidebar_undo_generation: u32,
    threads: Vec<DesktopThread>,
    archived_threads: Vec<archive::ArchivedThread>,
    workflow_window: gpui::AnyWindowHandle,
    automation_pending: bool,
    active_thread_id: Option<String>,
    current_model: String,
    explicit_model: bool,
    favorite_models: super::model_browser::Favorites,
    model_browser_tab: Option<String>,
    permission_mode: PermissionMode,
    /// Where a thread that has not sent yet will run. Copied onto every chat.
    run_choice: Option<WorkspaceChoice>,
    /// Claude effort chip. `None` is Default, which omits `--effort`.
    claude_effort: Option<String>,
    /// Stored follow-up. Claude can steer a running turn. Codex still queues.
    follow_up_steer: bool,
    send_shortcut: SendShortcut,
    slash_skills:bool,
    composer_rich_text: bool,
    tool_visibility: crate::views::chat::tool_visibility::Visibility,
    screen_reader: crate::views::chat::screen_reader::ReaderPreferences,
    last_reader_title: Option<String>,
    chat_width: ChatWidth,
    appearance: appearance::DeviceAppearance,
    system_language: language::Language,
    custom_themes: Arc<appearance::ThemeLibrary>,
    theme_import_notice: Option<(String, std::time::Instant)>,
    theme_navigation: appearance::CollectionNavigation,
    theme_preview_active: bool,
    details: thread_details::DetailsUi,
    thread_origins: BTreeMap<String,thread_details::Origin>,
    fonts:font_settings::DeviceFonts,
    available_fonts:Vec<String>,
    snapshots: snapshots::Preferences,
    picking_window: bool,
    capture_binding: snapshots::CaptureBinding,
    delivery_preferences:delivery_preferences::HostPreferences,
    environments: environments::Environments,
    diagnostics: diagnostics::Monitor,
    local_tools: device_settings::Access,
    command_launcher: Option<Result<crate::launcher::Status,String>>,
    command_launcher_busy: bool,
    telemetry:telemetry::Editor,
    dictation_prefs: crate::voice_capture::DictationPreferences,
    voice_dictionary_editor: voice_dictionary::DictionaryEditor,
    voice_model_input: Entity<InputState>,
    voice_cleanup_fields: voice_cleanup::Fields,
    voice_models: voice_models::Browser,
    voice_cloud: voice_cloud::Editor,
    voice_devices: Option<Result<Vec<String>, String>>,
    worktree_prefs: bruhs_models::logic::run_place::WorktreePreferences,
    worktree_retention: worktree_storage::WorktreeRetention,
    pending_scratch_cleanups: Vec<worktree_storage::ScratchCleanup>,
    dispatched_scratch_cleanups: std::collections::BTreeSet<String>,
    cleanup_deleted_worktrees:bool,
    cleanup_keep_when: bruhs_models::coordination::worktree::KeepWhen,
    health_interval:u32,
    branch_naming: bruhs_models::logic::run_place::branch_naming::Naming,
    health_interval_input:Entity<InputState>,
    generators: text_generators::GeneratorSettings,
    acp_models: Vec<crate::views::acp_models::Instance>,
    provider_availability: Result<bruhs_core::agent_extensions::availability::ProviderAvailability, String>,
    usage_hubs: usage_hubs::UsageHubs,
    provider_editor: Option<provider_editor::ProviderEditor>,
    provider_add_focus: FocusHandle,
    provider_change_pending: bool,
    provider_override_source: String,
    settings_dropdown_trigger: Option<FocusHandle>,
    first_override_project: FocusHandle,
    checkout_defaults: bruhs_core::git::ProjectCheckoutDefaults,
    checkout_defaults_watch: Option<Task<()>>,
    provider_change_error: Option<String>,
    health_result:Option<Vec<bruhs_core::agent::health::ProviderHealth>>,
    health_timer:Option<Task<()>>,
    health_check:Option<Task<()>>,
    retention_editor: Option<worktree_storage::retention::DaysEditor>,
    location_input: Entity<InputState>,
    location_status: Option<String>,
    worktree_cleanup_status:Option<String>,
    settings_advanced: std::collections::BTreeSet<&'static str>,
    location_busy:bool,
    #[cfg(target_os = "macos")]
    fast_checkout: labs::FastCheckout,
    branch_prefix_input: Entity<InputState>,
    branch_instructions_input: Entity<TextareaState>,
    branch_editor_scope: String,
    branch_prefix_edited: bool,
    branch_instructions_edited: bool,
    voice_device_generation: u64,
    voice_testing: bool,
    microphone_test: microphone_test::MicrophoneTest,
    voice_runtime: voice_runtime::Runtime,
    voice_test_result: Option<String>,
    /// Claude `--autocompact`. `None` omits the flag.
    claude_autocompact: Option<String>,
    autocompact_input: Entity<InputState>,
    /// What to do with an idle Claude thread over the token floor.
    compact_idle: IdleCompact,
    /// What agents may do with other threads.
    thread_access: ThreadAccess,
    /// Agent-to-agent messages allowed in a row before the user is asked.
    thread_hops: u32,
    /// Tool calls held until another thread has replied.
    thread_waits: Vec<thread_link_ui::ThreadWait>,
    /// `message_bruh` calls held until the bruh's run has ended.
    bruh_waits: thread_link_ui::BruhWaits,
    /// Named model setups an agent can start a thread with.
    roles: bruhs_models::roles::RoleBook,
    /// Models for the jobs that are not threads.
    task_models: bruhs_models::roles::TaskModels,
    /// Live meeting suggestions: preferences and the current session.
    meeting_suggestions: meetings_ui::suggestions::MeetingSuggestionsUi,
    /// Id of the role open in Settings.
    role_selected: Option<String>,
    role_inputs: role_settings_ui::RoleInputs,
    /// Visited pages. `Cmd+[` and `Cmd+]` walk this stack.
    page_history: PageHistory,
    /// Offered as Previous worktree. Shared by every thread.
    previous_worktree: Option<RememberedWorktree>,
    /// Base branch for a new worktree. This session only; not saved.
    base_branch: String,
    task_view_mode: DesktopTaskViewMode,
    library_pages: library_pages_ui::LibraryPages,
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
    #[cfg(test)]
    calendar_legacy_preparation: std::cell::Cell<bool>,
    selected_calendar_event_id: Option<String>,
    event_create_all_day: bool,
    /// Set while the event dialog shows a read-only provider event.
    event_dialog_provider: Option<DesktopProviderEvent>,
    event_create_color: String,
    event_create_error: Option<String>,
    timegrid_drag: Option<DesktopTimegridDragState>,
    active_project: Option<DesktopProjectLocation>,
    recent_projects: Vec<DesktopProjectLocation>,
    project_start_folder_input: Entity<InputState>,
    clone_host_folders: bool,
    clone_editor: clone_project::CloneEditor,
    idle_settle_days: u16,
    maintenance:maintenance::Maintenance,
    project_defaults: BTreeMap<String, project_defaults::ProjectDefaults>,
    settings_project_scope: String,
    project_actions: BTreeMap<String, Vec<project_actions::ProjectAction>>,
    action_editor: project_actions::ActionEditor,
    idle_action_runs: Option<BTreeMap<String, EpochMs>>,
    snooze_editor:snooze_options::Editor,
    quota_prefs:quota_settings::Preferences,
    project_identities: project_grouping::Identities,
    expanded_project_groups: std::collections::BTreeSet<String>,
    project_names:project_name::SyncState,
    project_list:project_order::ProjectList,
    removal: project_removal::RemovalState,
    response_stream:super::chat::response_stream::Mode,
    text_generation_model: Option<String>,
    title_requests: BTreeMap<String, thread_titles::PendingTitle>,
    title_request_serial: u64,
    task_thread_view: Option<crate::views::reactive_automations::TaskThreadLocation>,
    desktop_task_cursors: BTreeMap<String,i64>,
    reading_desktop_tasks: HashSet<String>,
    desktop_task_links: BTreeMap<String,BTreeMap<String,String>>,
    quota_timers:BTreeMap<String,Task<()>>,
    foreign_runs: BTreeMap<String, maintenance::restart::Interrupted>,
    quota_recoveries: BTreeMap<String, quota_settings::Recovery>,
    quota_dispatching: BTreeMap<String, quota_settings::Recovery>,
    fallback_model:Option<String>,
    default_kiro:super::chat::kiro::KiroChoices,
    fallback_kiro:super::chat::kiro::KiroChoices,
    provider_resets:BTreeMap<String,i64>,
    capacity_task:Option<Task<()>>,
    snooze_options:Option<Vec<snooze_options::Choice>>,
    browser_search_url_input: Entity<InputState>,
    viewport_fields:[Entity<InputState>;2],
    #[cfg(target_os = "macos")]
    browser_profile_editor: browser_settings::ProfileEditor,
    project_picker_current_path: String,
    project_picker_current_name: String,
    project_picker_parent_path: Option<String>,
    project_picker_dirs: Vec<DesktopProjectLocation>,
    project_picker_error: Option<String>,
    project_picker_start_warning: Option<String>,
    /// Reading a directory or resolving a picked project off the UI thread.
    project_picker_task: Option<Task<()>>,
    project_picker_scroll: ScrollHandle,
    workspace_focus_handle: FocusHandle,
    #[cfg(target_os = "macos")]
    last_embedded_shell_focus: Option<FocusHandle>,
    palette_return_focus: Option<(AppView, FocusHandle)>,
    modal_start_focus_handle: FocusHandle,
    modal_end_focus_handle: FocusHandle,
    modal_focus: modal_ui::ModalFocus,
    modal_scroll: ScrollHandle,
    /// The dialogs' backdrop blur, which lives in native views, not gpui's.
    blur_scrim: BlurScrimHandle,

    /// The Import thread dialog's source, results and background work.
    import_picker: import_ui::ImportPicker,
    import_input: Entity<InputState>,
    move_in: move_in_ui::MoveInPicker,

    // Child views
    command_input: Entity<InputState>,
    capture_input: Entity<TextareaState>,
    capture_cancel_focus: FocusHandle,
    capture_submit_focus: FocusHandle,
    knowledge_title_input: Entity<InputState>,
    knowledge_body_input: Entity<TextareaState>,
    knowledge_tags_input: Entity<InputState>,
    clip_title_input: Entity<InputState>,
    clip_url_input: Entity<InputState>,
    clip_source_name_input: Entity<InputState>,
    clip_author_input: Entity<InputState>,
    clip_published_input: Entity<InputState>,
    clip_tags_input: Entity<InputState>,
    clip_excerpt_input: Entity<TextareaState>,
    /// The Knowledge library's search.
    knowledge: knowledge_ui::KnowledgeState,
    clip_create_visible: bool,
    /// False when the state file could not be read and no backup restored
    /// it: the Inbox then says its conversations could not be loaded.
    saved_records_loaded: bool,
    inbox_import_visible: bool,
    /// The Inbox's Import conversation button, where closing the import form
    /// returns focus when the button opened it.
    inbox_import_focus: FocusHandle,
    inbox_import_returns_focus: bool,
    inbox_mail_visible: bool,
    connected_inbox: Entity<connected_inbox::ConnectedInbox>,
    inbox_page: usize,
    inbox_filter: bruhs_models::InboxFilter,
    inbox_search_input: Entity<InputState>,
    inbox_directory: RefCell<Option<(usize, bruhs_models::InboxDirectory)>>,
    /// The conversation's Linked person list is open.
    inbox_person_menu_open: bool,
    inbox_import_source: InboxSource,
    inbox_sender_input: Entity<InputState>,
    inbox_subject_input: Entity<InputState>,
    inbox_body_input: Entity<TextareaState>,
    inbox_reply_input: Entity<TextareaState>,
    relationship_company_input: Entity<InputState>,
    relationship_role_input: Entity<InputState>,
    relationship_email_input: Entity<InputState>,
    relationship_phone_input: Entity<InputState>,
    relationship_location_input: Entity<InputState>,
    relationship_notes_input: Entity<TextareaState>,
    relationship_item_input: Entity<InputState>,
    relationship_item_kind: RelationshipItemKind,
    scheduling_visible: bool,
    scheduling_error: Option<String>,
    scheduling_scroll: ScrollHandle,
    scheduling_link_title_input: Entity<InputState>,
    scheduling_link_duration_input: Entity<InputState>,
    scheduling_selected_task_id: Option<String>,
    life_tab: DesktopLifeTab,
    /// Web's Life notice: what the last CSV import did, or why a card's
    /// entry was refused, until dismissed.
    life_notice: Option<String>,
    /// Each health card's form (web `health_tab`).
    life_health: life_ui::HealthForms,
    /// Each finance card's form (web `finance_tab`).
    life_finance: life_ui::FinanceForms,
    life_csv_input: Entity<TextareaState>,
    task_create_title_input: Entity<InputState>,
    task_create_description_input: Entity<TextareaState>,
    task_create_due_input: Entity<InputState>,
    task_detail_title_input: Entity<InputState>,
    task_detail_description_input: Entity<TextareaState>,
    task_detail_due_input: Entity<InputState>,
    task_detail_cycle_input: Entity<InputState>,
    task_detail_waiting_input: Entity<InputState>,
    event_create_title_input: Entity<InputState>,
    event_create_start_input: Entity<InputState>,
    event_create_end_input: Entity<InputState>,
    event_create_notes_input: Entity<TextareaState>,
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
    /// The Search result Up and Down select and Enter opens (web `selected`).
    search_selected: usize,
    person_rename_input: Entity<InputState>,
    speaker_rename_input: Entity<InputState>,
    renaming_person_id: Option<String>,
    expanded_person_id: Option<String>,
    merge: Option<meetings_ui::PersonMergePanel>,
    /// People page search, paging, confirmations, undo and return state.
    people_page: meetings_ui::people_view::PeoplePageState,
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
    workbench_panel: Option<Entity<crate::views::workbench::WorkbenchPanel>>,
    whiteboard_panel: Option<Entity<crate::views::whiteboard::catalog::WhiteboardLibrary>>,
    whiteboard_panels: std::collections::BTreeMap<
        String,
        Entity<crate::views::whiteboard::catalog::WhiteboardLibrary>,
    >,
    terminal_drawer_open: bool,
    terminal_drawer_height: f32,
    /// Right panel width in px, dragged at its left edge (web `RightPanelWidth`).
    right_panel_width: f32,
    sidebar_motion:panel_motion::Motion,
    right_panel_motion:panel_motion::Motion,
    right_panel_split_state: Entity<ResizableState>,
    /// Page zoom; see [`crate::zoom`].
    zoom: f32,
    /// Split between the transcript and the drawer; persists the drag.
    terminal_split_state: Entity<ResizableState>,
    /// Keeps the input, panel and hub subscriptions alive.
    _subscriptions: Vec<Subscription>,
}

#[cfg(test)]
type ThreadRestorer = fn(
    Option<&mut PersistedWorkspaceState>,
    &str,
    PermissionMode,
    &mut Window,
    &mut Context<'_, Workspace>,
) -> (Vec<DesktopThread>, Option<String>);

impl Workspace {
    /// The Vault and Apps sections share one Settings page: a change on
    /// one that the other shows has the other read its listing again
    /// (final review BC M4).
    fn sync_credential_sections(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let vault = self.vault_settings.clone();
        self._subscriptions.push(cx.subscribe_in(
            &self.apps_settings,
            window,
            move |_, _, _: &vault_settings::CredentialsChanged, window, cx| {
                vault.update(cx, |vault, cx| vault.reread(window, cx));
            },
        ));
        let apps = self.apps_settings.clone();
        self._subscriptions.push(cx.subscribe_in(
            &self.vault_settings,
            window,
            move |_, _, _: &vault_settings::CredentialsChanged, window, cx| {
                apps.update(cx, |apps, cx| apps.reread(window, cx));
            },
        ));
    }

    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        Self::from_loaded_state(
            load_desktop_state(),
            window,
            cx,
            #[cfg(test)]
            RestoredRecords::take_from_persisted,
            #[cfg(test)]
            Self::restore_threads,
        )
    }

    fn from_loaded_state(
        (mut persisted, persistence_error, disk_version): (
            Option<PersistedWorkspaceState>,
            Option<String>,
            DiskVersion,
        ),
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        #[cfg(test)] restore_records: fn(Option<&mut PersistedWorkspaceState>) -> RestoredRecords,
        #[cfg(test)] restore_threads: ThreadRestorer,
    ) -> Self {
        let untrusted_workspace_state = disk_version == DiskVersion::Untrusted;
        let favorite_models = persisted.as_ref().map(|s| s.favorite_models.clone()).unwrap_or_default();
        let explicit_model = persisted.as_ref().is_some_and(|state| state.current_model.is_some());
        let current_model = persisted
            .as_ref()
            .and_then(|state| state.current_model.clone())
            .unwrap_or_else(|| DEFAULT_AGENT_MODEL.to_string());
        let permission_mode = initial_permission_mode_from_persisted(persisted.as_ref());
        // A missing file is a new workspace with no plan to recover.
        let day_plan_loaded = persisted.is_some() || persistence_error.is_none();
        let saved_records_loaded = day_plan_loaded;
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            this.cancel_quit_on_blur(window, cx);
            if !window.is_window_active() {this.release_voice_shortcut(cx);}
        }));
        let workspace_for_appearance = cx.weak_entity();
        subscriptions.push(window.observe_window_appearance(move |window, cx| {
            let _ = workspace_for_appearance.update(cx, |this, cx| {
                if this.theme_preview_active { this.preview_theme_draft(window, cx); }
                else if this.appearance.scheme == appearance::ColorScheme::System { this.apply_device_appearance(window, cx); }
            });
        }));
        let settings_model_query = cx.new(|cx| InputState::new(window, cx).placeholder(super::model_browser::SEARCH_HINT));
        subscriptions.push(redraw_on_edit(&settings_model_query, window, cx));
        let appearance = persisted.as_ref().and_then(|state|state.appearance).unwrap_or_default().normalized();
        let system_language = language::Language::system();
        let search_placeholder = appearance.language.effective(system_language).text("Search settings");
        let settings_search = cx.new(|cx| InputState::new(window, cx).placeholder(search_placeholder));
        subscriptions.push(cx.subscribe_in(&settings_search, window, |this: &mut Self, field, event: &InputEvent, window, cx| {
            match event {
                InputEvent::Focus if !this.settings_search_open => {
                    this.settings_search_open = true;
                    cx.notify();
                }
                InputEvent::Blur if this.settings_search_open && field.read(cx).value().trim().is_empty() => {
                    this.settings_search_open = false;
                    cx.notify();
                }
                InputEvent::Change => this.refresh_settings_search(cx),
                InputEvent::PressEnter { .. } => this.open_settings_hit(this.settings_search_index, window, cx),
                _ => {}
            }
        }));
        let command_input = Self::new_command_input(window, cx, &mut subscriptions);
        let import_input = Self::new_import_input(window, cx, &mut subscriptions);
        let record_picker = record_picker_ui::RecordPickerUi::new(window, cx, &mut subscriptions);
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
        let knowledge = knowledge_ui::KnowledgeState::new(window, cx, &mut subscriptions);
        let inbox_ui::InboxInputs {
            inbox_search_input,
            inbox_sender_input,
            inbox_subject_input,
            inbox_body_input,
            inbox_reply_input,
        } = Self::new_inbox_inputs(window, cx, &mut subscriptions);
        let connected_inbox =
            cx.new(|cx| connected_inbox::ConnectedInbox::new(meetings_client(), window, cx));
        subscriptions.push(cx.subscribe_in(
            &connected_inbox,
            window,
            |this: &mut Self, _, event, window, cx| {
                match event {
                    connected_inbox::InboxEvent::ShowImports => this.inbox_mail_visible = false,
                    connected_inbox::InboxEvent::OpenConnections => {
                        this.open_settings(&OpenSettings, window, cx);
                        this.set_settings_section(SettingsSection::Vault, cx);
                    }
                }
                cx.notify();
            },
        ));
        let scheduling_ui::SchedulingInputs {
            scheduling_link_title_input,
            scheduling_link_duration_input,
        } = Self::new_scheduling_inputs(window, cx, &mut subscriptions);
        let life_ui::LifeInputs {
            life_health,
            life_finance,
            life_csv_input,
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
        let sidebar_search = Self::new_sidebar_search(appearance.language.effective(system_language), window, cx, &mut subscriptions);
        let saved_title_instructions = persisted
            .as_ref()
            .and_then(|state| state.title_instructions.clone())
            .unwrap_or_default();
        let (title_instructions_input, title_project_instructions_input) =
            Self::new_title_instruction_inputs(
                saved_title_instructions,
                window,
                cx,
                &mut subscriptions,
            );
        let browser_prefs = persisted.as_ref().and_then(|p| p.browser_preferences.clone()).unwrap_or_default();
        let browser_search_url_input = Self::new_browser_settings(browser_prefs, window, cx, &mut subscriptions);
        let viewport_fields=Self::new_viewport_fields(window,cx,&mut subscriptions);
        let health_interval=persisted.as_ref().map(|p|p.health_interval).unwrap_or(300);
        let health_interval=if health_interval==0{0}else{health_interval.clamp(30,86400)};
        let health_interval_input=Self::new_health_interval(health_interval,window,cx,&mut subscriptions);
        let branch_naming=persisted.as_ref().map(|p|p.branch_naming.clone()).unwrap_or_default();
        let voice_cleanup_fields=voice_cleanup::Fields::new(&persisted.as_ref().and_then(|state|state.dictation_prefs.as_ref()).map(|p|p.cleanup.clone()).unwrap_or_default(),window,cx,&mut subscriptions);
        let voice_cloud = voice_cloud::Editor::new(window,cx,&mut subscriptions);
        let voice_models = voice_models::Browser::new(window,cx,&mut subscriptions);
        let voice_model_input = cx.new(|cx| InputState::new(window,cx).placeholder("Model ID from your server")
            .default_value(persisted.as_ref().and_then(|state|state.dictation_prefs.as_ref()).map(|p|p.remote_model.clone()).unwrap_or_default()));
        subscriptions.push(cx.subscribe_in(&voice_model_input,window, |this: &mut Self,field,event:&InputEvent,_,cx|{
            if matches!(event,InputEvent::Change) {
                let value=field.read(cx).value().to_string();
                if this.dictation_prefs.remote_model!=value {this.dictation_prefs.remote_model=value;this.stamp_run_prefs_on_threads(cx);this.persist_and_notify(cx);}
            }
        }));
        let location_input=cx.new(|cx|InputState::new(window,cx).placeholder("Default location").default_value(bruhs_core::worktrees::locations::selected_parent().ok().flatten().map(|p|p.to_string_lossy().into_owned()).unwrap_or_default()));
        let (branch_prefix_input,branch_instructions_input)=Self::new_branch_fields(&branch_naming,window,cx,&mut subscriptions);
        let project_start_folder_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("~/").default_value(
                persisted.as_ref().and_then(|p| p.project_start_folder.clone()).unwrap_or_default()
            )
        });
        subscriptions.push(cx.subscribe(&project_start_folder_input, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.persist(cx);
            }
        }));
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
            persisted.as_ref().map_or(0, |state| state.keybinding_schema),
        );
        let keybinding_filter = cx.new(|cx| InputState::new(window,cx).placeholder("Search keybindings"));
        subscriptions.push(redraw_on_edit(&keybinding_filter, window, cx));
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
        if let Some(prefs) = persisted.as_ref().and_then(|state| state.diff_preferences) {
            diff_panel.update(cx, |panel, cx| panel.set_preferences(prefs, cx));
        }
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
            people_page,
        } = Self::new_people_inputs(window, cx, &mut subscriptions);
        let stepper_prefs = meetings_hub.read(cx).prefs.clone();
        let settings_stepper_inputs =
            Self::new_settings_stepper_inputs(&stepper_prefs, window, cx, &mut subscriptions);
        let settings_transcription_model_input =
            Self::new_transcription_model_input(&stepper_prefs, window, cx, &mut subscriptions);
        let (date_picker_hour_input, date_picker_minute_input) =
            Self::new_date_picker_inputs(window, cx, &mut subscriptions);
        #[cfg(not(test))]
        let (threads, active_thread_id) = Self::restore_threads(
            persisted.as_mut(),
            &current_model,
            permission_mode,
            window,
            cx,
        );
        #[cfg(test)]
        let (threads, active_thread_id) = restore_threads(
            persisted.as_mut(),
            &current_model,
            permission_mode,
            window,
            cx,
        );
        #[cfg(not(test))]
        let records = RestoredRecords::take_from_persisted(persisted.as_mut());
        #[cfg(test)]
        let records = restore_records(persisted.as_mut());
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
        } = records;
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

        cx.defer_in(window, |this, _, cx| {
            this.dispatch_saved_scratch(this.pending_scratch_cleanups.clone(), cx);
            this.drive_background_automations(cx);
        });
        cx.defer_in(window, |this, _, cx| this.refresh_branch_pull(cx));
        let run_choice = persisted.as_ref().filter(|s| !s.run_choice.is_empty())
            .map(|s| WorkspaceChoice::parse(&s.run_choice));
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
        let thread_access = persisted
            .as_ref()
            .and_then(|state| state.thread_access.as_deref())
            .map(ThreadAccess::parse)
            .unwrap_or_default();
        let thread_hops = persisted
            .as_ref()
            .and_then(|state| state.thread_hops)
            .filter(|hops| bruhs_models::thread_link::HOP_CHOICES.contains(hops))
            .unwrap_or(bruhs_models::thread_link::MAX_HOPS);
        let roles = persisted
            .as_ref()
            .and_then(|state| state.roles.clone())
            .unwrap_or_default()
            .sanitized();
        let task_models = persisted
            .as_ref()
            .and_then(|state| state.task_models.clone())
            .unwrap_or_default()
            .sanitized();
        crate::server_client::set_summary_model(task_models.summary.clone());
        let meeting_suggestions = meetings_ui::suggestions::MeetingSuggestionsUi::new(
            persisted
                .as_ref()
                .and_then(|state| state.meeting_suggestions.clone())
                .unwrap_or_default(),
        );
        let role_inputs = Self::new_role_inputs(window, cx, &mut subscriptions);
        let base_branch = String::new();
        let reopen_view = persisted
            .as_ref()
            .and_then(|state| state.active_view.clone());
        // Vault replaced Credentials; uncommitted work still reads the old field
        // name, so both fields hold the same Vault settings (landing shim).
        let vault_settings = {
            let settings =
                cx.new(|cx| vault_settings::VaultSettings::new(meetings_client(), window, cx));
            let sources = settings.read(cx).secret_sources();
            cx.observe(&sources, |_, _, cx| cx.notify()).detach();
            settings
        };
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
            database_panel: None,
            project_tool: panels_ui::ProjectTool::Files,
            api_client_panel: None,
            files_dir: String::new(),
            files_root: None,
            files_entries: Vec::new(),
            files_error: None,
            files_loading: false,
            files_generation: 0,
            pull_panel_visible: false,
            branch_pull: None,
            known_thread_pulls: persisted.as_ref().map(|p|p.known_thread_pulls.clone()).unwrap_or_default(),
            pull_preferences: persisted.as_ref().map(|p|p.pull_preferences).unwrap_or_default(),
            pull_check_times: Default::default(),
            pull_lifecycle_task: None,
            linked_pull: None,
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
            reactive_automations: {
                let panel = cx.new(crate::views::reactive_automations::ReactiveAutomations::new);
                subscriptions.push(cx.observe(&panel, |this,_,cx| {
                    if this.details.open_for_active_thread(this.active_thread_id.as_deref()) || (this.settings_visible() && matches!(this.settings_section, SettingsSection::General | SettingsSection::ScheduledTasks)) { cx.notify(); }
                }));
                subscriptions.push(cx.subscribe(
                    &panel,
                    |this,
                     _,
                     event: &crate::views::reactive_automations::ReactiveProjectOpened,
                     cx| {
                        this.active_project = Some(DesktopProjectLocation {
                            path: event.path.clone(),
                            name: event
                                .path
                                .rsplit('/')
                                .next()
                                .unwrap_or("Project")
                                .to_string(),
                        });
                        this.persist_and_notify(cx);
                    },
                ));
                subscriptions.push(cx.subscribe_in(
                    &panel,
                    window,
                    |this,
                     _,
                     event: &crate::views::reactive_automations::ReactiveBruhCreated,
                     window,
                     cx| {
                        this.open_created_bruh(event.agent_id.clone(), window, cx)
                    },
                ));
                subscriptions.push(cx.subscribe_in(
                    &panel,
                    window,
                    |this,
                     _,
                     event: &crate::views::reactive_automations::ReactiveRoomDone,
                     window,
                     cx| {
                        this.room_dialog_done(event.open.clone(), window, cx)
                    },
                ));
                subscriptions.push(cx.subscribe_in(
                    &panel,
                    window,
                    |this,
                     _,
                     _: &crate::views::reactive_automations::ReactiveOpenAutomations,
                     window,
                     cx| {
                        this.activate_sidebar_view(AppView::Automations, window, cx)
                    },
                ));
                subscriptions.push(cx.subscribe_in(
                    &panel,
                    window,
                    |this,
                     _,
                     event: &crate::views::reactive_automations::ReactiveOpenRoomSettings,
                     window,
                     cx| {
                        this.open_room_dialog(Some(event.room_id.clone()), window, cx)
                    },
                ));
                subscriptions.push(cx.subscribe(&panel, |this,_,activity:&crate::views::reactive_automations::DesktopTaskActivity,cx| {
                    this.read_scheduled_replies(activity,cx);
                }));
                subscriptions.push(cx.subscribe_in(&panel, window, |this, _, event: &crate::views::reactive_automations::TaskThreadLocation, window, cx| {
                    if this.desktop_task_links.contains_key(event.thread_id.as_str()) && this.threads.iter().any(|thread| thread.id == event.thread_id.as_str()) {
                        this.select_thread_by_id(event.thread_id.as_str(),cx);
                        return;
                    }
                    this.activate_sidebar_view(AppView::Bruh { agent_id: event.agent_id.clone() }, window, cx);
                    this.task_thread_view = Some(event.clone());
                    this.persist_and_notify(cx);
                }));
                subscriptions.push(cx.subscribe_in(&panel, window, |this, _, event, window, cx| {
                    this.task_thread_deletion(event,window,cx);
                }));
                // A crew change can end a held `message_bruh` call.
                subscriptions.push(cx.subscribe(
                    &panel,
                    |this, _, _: &crate::views::reactive_automations::ReactiveCrewChanged, cx| {
                        this.check_bruh_waits(cx)
                    },
                ));
                subscriptions.push(cx.subscribe(
                    &panel,
                    |this, _, event: &crate::views::reactive_automations::ReactivePreview, cx| {
                        let now = EpochMs::now().as_millis();
                        let id = this.automation_state.create_reactive_workflow(
                            &event.title,
                            event.operations.clone(),
                            &event.run_id,
                            now,
                        );
                        if this.automation_state.preview(&id, now).is_none() {
                            this.automation_error = Some(
                                "This draft could not be previewed. No records were changed."
                                    .into(),
                            );
                        }
                        this.show_dry_run(id);
                        this.persist_and_notify(cx);
                    },
                ));
                panel
            },
            palette_mode: PaletteMode::Commands,
            automation_error: None,
            reviewed_workflow_id: None,
            mcp_bridge: mcp_bridge::WorkspaceBridge::default(),
            automations_scroll: ScrollHandle::new(),
            calendar_timegrid_scroll: ScrollHandle::new(),
            calendar_timegrid_autoscroll: std::cell::Cell::new(true),
            dry_run_workflow_id: None,
            workflow_disclosures: HashSet::new(),
            dismissed_workflow_notice: None,
            workflow_notice_open_focus: cx.focus_handle().tab_stop(true),
            inbox_back_focus: cx.focus_handle().tab_stop(true),
            inbox_row_focus: RefCell::new(std::collections::HashMap::new()),
            inbox_return_row: None,
            mobile: mobile::MobileUi::new(window, cx),
            local_servers: Vec::new(),
            local_servers_loading: false,
            local_servers_generation: 0,
            thread_origins: persisted.as_ref().map(|p|p.thread_origins.clone()).unwrap_or_default(),
            details: {
                let mut details = thread_details::DetailsUi::new(cx);
                if let Some(state) = &persisted { details.hidden_inline.clone_from(&state.hidden_inline_thread_details); }
                details
            },
            settings_section: SettingsSection::General,
            license_browser: None,
            license_trigger: cx.focus_handle(),
            license_forward: false,
            vault_settings: vault_settings.clone(),
            credential_settings: vault_settings,
            apps_settings: cx
                .new(|cx| apps_settings::AppsSettings::new(meetings_client(), window, cx)),
            backup_settings: cx
                .new(|cx| backup_settings::BackupSettings::new(meetings_client(), window, cx)),
            updates: {
                let enabled = persisted
                    .as_ref()
                    .and_then(|state| state.update_checks)
                    .unwrap_or(true);
                let updates = cx.new(|cx| updates_ui::DesktopUpdates::new(enabled, persisted.as_ref().and_then(|s|s.maintenance.as_ref()).is_none_or(|m|m.provider_checks), persisted.as_ref().and_then(|s| s.update_track).unwrap_or_else(|| updates_ui::UpdateTrack::for_installed(env!("BRUHS_RELEASE_VERSION"))), cx));
                cx.subscribe(&updates, Self::combined_update_event).detach();
                let mut registry_revision = 0;
                cx.observe(&updates, move |ws, u, cx| {
                    let revision = u.read(cx).registry_revision;
                    if revision != registry_revision {
                        registry_revision = revision;
                        ws.health_check = None;
                        ws.check_provider_health(cx);
                    }
                    cx.notify();
                }).detach();
                updates
            },
            settings_search,
            settings_model_query,
            settings_model_trigger: cx.focus_handle(),
            settings_model_rail: cx.focus_handle(),
            settings_model_listings: Vec::new(),
            settings_models_load: None,
            thread_catalog: Default::default(),
            thread_catalog_load: None,
            default_options: persisted.as_ref().map(|s|s.default_options.clone()).unwrap_or_default(),
            fallback_options: persisted.as_ref().map(|s|s.fallback_options.clone()).unwrap_or_default(),
            settings_search_results: Vec::new(),
            settings_search_catalog: Vec::new(),
            settings_search_index: 0,
            settings_search_open: false,
            settings_nav_focus: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            settings_nav_scroll: ScrollHandle::new(),
            settings_nav_reveal: Default::default(),
            settings_search_inventory: RefCell::default(),
            settings_search_anchor: None,
            settings_scroll: ScrollHandle::new(),
            settings_slider_bounds: (0..settings_ui::SettingsSlider::COUNT)
                .map(|_| Rc::new(std::cell::Cell::new(None)))
                .collect(),
            settings_slider_drag: None,
            settings_stepper_inputs,
            settings_transcription_model_input,
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
            day_plan_undo: persisted
                .as_ref()
                .and_then(|state| state.day_plan_undo.clone()),
            day_plan_error: None,
            work_item_handoff: work_item_handoff_ui::HandoffUi::default(),
            day_plan_saving: false,
            day_plan_reading: false,
            day_plan_loaded,
            #[cfg(test)]
            day_plan_test_now: None,
            persistence: Arc::new(PersistenceCoordinator {
                write_lock: Mutex::new(disk_version),
                ..Default::default()
            }),
            save_queue: save_queue::SaveQueue::default(),
            unread_thread_ids: HashSet::new(),
            sidebar_search,
            sidebar_viewport: RefCell::new(sidebar_ui::SidebarViewport::new(cx)),
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
            crew_rail_open: persisted
                .as_ref()
                .and_then(|state| state.crew_rail_open)
                .unwrap_or(true),
            thread_notifications: persisted.as_ref().and_then(|state| state.thread_notifications).unwrap_or_default(),
            confirm_thread_archive: persisted.as_ref().and_then(|state| state.confirm_thread_archive).unwrap_or(false),
            confirm_thread_unpin: persisted.as_ref().and_then(|p| p.confirm_thread_unpin).unwrap_or(false),
            voice_press: None,
            quit_gesture:quit_shortcut::Gesture::new(persisted.as_ref().map(|p|p.quit_mode).unwrap_or_default()),
            confirm_thread_delete: persisted.as_ref().and_then(|p| p.confirm_thread_delete).unwrap_or(true),
            labs_sidebar: persisted.as_ref().map(|s|s.labs_sidebar.clone()).unwrap_or_default(),
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
            title_project: String::new(),
            title_project_instructions_input,
            keybindings,
            keybinding_index: 0,
            keymap_file:keybinding_file::KeymapFile::new(),
            linked_projects: linked_keybindings::LinkedProjects::restore(persisted.as_ref().map(|s| &s.project_links)),
            linked_keybindings: linked_keybindings::LinkedKeybindings::restore(persisted.as_ref().map(|s| &s.shortcut_links)),
            keybinding_edit_open: false,
            keybinding_draft: None,
            keybinding_when_open: None,
            keybinding_recording: None,
            keybinding_record_focus: cx.focus_handle().tab_stop(true),
            keybinding_add_focus: cx.focus_handle().tab_stop(true),
            keybinding_condition_focus: cx.focus_handle().tab_stop(true),
            keybinding_filter,
            keybinding_key_input,
            keybinding_when_input,
            keybinding_installed: keybindings_ui::startup_bindings(),
            crew_row_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            crew_new_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            crew_scroll: ScrollHandle::new(),
            thread_menu_id: None,
            thread_menu_items: (0..sidebar_ui::THREAD_MENU_MAX_ITEMS)
                .map(|_| cx.focus_handle().tab_index(0).tab_stop(true))
                .collect(),
            snooze_inputs: snooze_ui::SnoozeInputs::new(window, cx),
            record_picker,
            booking: booking_ui::BookingUi::new(window, cx),
            sidebar_undo: None,
            sidebar_undo_generation: 0,
            archived_threads: persisted.as_mut().map(|state| std::mem::take(&mut state.archived_threads))
                .unwrap_or_default().into_iter().map(archive::ArchivedThread::Stored).collect(),
            threads,
            active_thread_id,
            current_model,
            explicit_model,
            favorite_models,
            model_browser_tab: None,
            permission_mode,
            run_choice,
            claude_effort,
            follow_up_steer,
            dictation_prefs: persisted.as_ref().and_then(|state| state.dictation_prefs.clone()).unwrap_or_default(),
            voice_dictionary_editor: voice_dictionary::DictionaryEditor::new(window,cx),
            voice_model_input,
            voice_cleanup_fields,
            voice_models,
            voice_cloud,
            voice_devices: None,
            worktree_prefs: match persisted.as_ref() {
                Some(state) => state.worktree_prefs.unwrap_or_default(),
                None => bruhs_models::logic::run_place::WorktreePreferences {
                    submodules: bruhs_models::logic::run_place::WorktreeSubmodules::FromProject,
                    ..Default::default()
                },
            },
            voice_device_generation: 0,
            voice_testing: false,
            microphone_test: Default::default(),
            voice_runtime:Default::default(),
            voice_test_result: None,
            fonts:persisted.as_ref().and_then(|s|s.fonts.clone()).unwrap_or_default(),
            available_fonts:Vec::new(),
            snapshots: persisted.as_ref().and_then(|s| s.snapshots).unwrap_or_default(),
            picking_window: false,
            capture_binding: Default::default(),
            environments:environments::Environments::new(window,cx),
            diagnostics:Default::default(),
            local_tools: Default::default(),
            command_launcher: None,
            command_launcher_busy: false,
            telemetry:telemetry::Editor::new(window,cx),
            delivery_preferences:Default::default(),
            theme_import_notice: None,
            theme_navigation: Default::default(),
            theme_preview_active: false,
            custom_themes: persisted.as_ref().and_then(|state|state.custom_themes.clone()).unwrap_or_default(),
            appearance,
            system_language,
            chat_width: persisted.as_ref().and_then(|state| state.chat_width).unwrap_or_default(),
            last_reader_title: None,
            screen_reader: persisted.as_ref().and_then(|p|p.screen_reader).unwrap_or_default(),
            composer_rich_text: persisted.as_ref().and_then(|p| p.composer_rich_text).unwrap_or(true),
            slash_skills:persisted.as_ref().and_then(|p|p.slash_skills).unwrap_or(true),
            tool_visibility:persisted.as_ref().map(|p|p.tool_visibility).unwrap_or_default(),
            send_shortcut: persisted.as_ref().and_then(|p| {
                SendShortcut::OPTIONS.into_iter().find(|(_, id)| Some(*id) == p.send_shortcut.as_deref()).map(|(mode, _)| mode)
            }).unwrap_or_default(),
            claude_autocompact,
            autocompact_input,
            compact_idle,
            thread_access,
            thread_hops,
            thread_waits: Vec::new(),
            bruh_waits: thread_link_ui::BruhWaits::default(),
            roles,
            task_models,
            meeting_suggestions,
            role_selected: None,
            role_inputs,
            page_history: PageHistory::new(),
            previous_worktree,
            base_branch,
            task_view_mode: persisted
                .as_ref()
                .and_then(|state| state.task_view_mode)
                .unwrap_or(DesktopTaskViewMode::List),
            library_pages: Default::default(),
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
            #[cfg(test)]
            calendar_legacy_preparation: std::cell::Cell::new(false),
            selected_calendar_event_id: initial_selected_calendar_event_after_start(
                persisted
                    .as_ref()
                    .and_then(|state| state.selected_calendar_event_id.clone()),
            ),
            event_create_all_day: false,
            event_dialog_provider: None,
            event_create_color: "persimmon".to_string(),
            event_create_error: None,
            timegrid_drag: None,
            active_project,
            recent_projects,
            browser_search_url_input,
            viewport_fields,
            #[cfg(target_os = "macos")]
            browser_profile_editor: browser_settings::ProfileEditor::new(window,cx),
            project_start_folder_input,
            branch_naming, branch_prefix_input, branch_instructions_input, branch_editor_scope: String::new(), branch_prefix_edited: false, branch_instructions_edited: false,
            generators: text_generators::GeneratorSettings::new(cx),
            acp_models: crate::views::acp_models::load().unwrap_or_default(),
            provider_availability: bruhs_core::agent_extensions::availability::ProviderAvailability::load()
                .map_err(|err| err.to_string()),
            provider_change_error: None,
            usage_hubs: usage_hubs::UsageHubs::new(cx),
            provider_editor: None,
            provider_add_focus: cx.focus_handle(),
            provider_change_pending: false,
            provider_override_source: String::new(),
            settings_dropdown_trigger: None,
            first_override_project: cx.focus_handle(),
            checkout_defaults: Default::default(),
            checkout_defaults_watch: None,
            health_interval,health_interval_input,health_result:None,health_timer:None,health_check:None,
            retention_editor: None,
            worktree_retention: persisted.as_ref().map(|s| s.worktree_retention).unwrap_or_default(),
            #[cfg(target_os = "macos")]
            fast_checkout: Default::default(),
            location_input,location_status:None,location_busy:false,worktree_cleanup_status:None,settings_advanced: Default::default(),
            cleanup_keep_when: persisted.as_ref().map(|state|state.cleanup_keep_when).unwrap_or_default(),
            pending_scratch_cleanups: persisted.as_mut().map(|s| std::mem::take(&mut s.pending_scratch_cleanups)).unwrap_or_default(),
            dispatched_scratch_cleanups: Default::default(),
            cleanup_deleted_worktrees:persisted.as_ref().is_some_and(|s|s.cleanup_deleted_worktrees),
            maintenance:maintenance::Maintenance::new(persisted.as_ref().map(|state|state.maintenance.clone().unwrap_or_default()).unwrap_or_else(maintenance::MaintenancePreferences::for_new_install),window,cx,&mut subscriptions),
            idle_settle_days: persisted.as_ref().map_or(3, |state|state.idle_settle_days.unwrap_or(1)).min(365),
            clone_host_folders: persisted.as_ref().is_some_and(|p|p.clone_host_folders),
            clone_editor: clone_project::CloneEditor::new(window,cx),
            snooze_options:persisted.as_ref().and_then(|s|s.snooze_options.clone()),
            snooze_editor:snooze_options::Editor::new(window,cx),
            quota_prefs:persisted.as_ref().and_then(|s|s.quota_prefs).unwrap_or_default(),
            desktop_task_cursors: persisted.as_ref().map(|p| p.desktop_task_cursors.clone()).unwrap_or_default(),
            reading_desktop_tasks: Default::default(),
            desktop_task_links: persisted.as_ref().map(|p|p.desktop_task_links.clone()).unwrap_or_default(),
            quota_timers:BTreeMap::new(),
            foreign_runs:BTreeMap::new(),
            quota_recoveries:persisted.as_mut().map(|s|std::mem::take(&mut s.quota_recoveries)).unwrap_or_default(),
            quota_dispatching:BTreeMap::new(),
            default_kiro:persisted.as_ref().map(|p|p.default_kiro.clone()).unwrap_or_default(),
            fallback_kiro:persisted.as_ref().map(|p|p.fallback_kiro.clone()).unwrap_or_default(),
            fallback_model:persisted.as_ref().and_then(|p|p.fallback_model.clone()),
            provider_resets:persisted.as_ref().map(|p|p.provider_resets.clone()).unwrap_or_default(),
            capacity_task:None,
            project_identities: Default::default(),
            expanded_project_groups: Default::default(),
            removal: Default::default(),
            project_names:Default::default(),
            project_list:persisted.as_ref().map(|s|s.project_list.clone()).unwrap_or_default(),
            title_requests: persisted.as_mut().map(|p| std::mem::take(&mut p.title_requests)).unwrap_or_default(),
            title_request_serial:0,
            task_thread_view: persisted.as_ref().and_then(|p| p.task_thread_view.clone()),
            text_generation_model:persisted.as_ref().and_then(|s|s.text_generation_model.clone()),
            response_stream:persisted.as_ref().and_then(|s|s.response_stream).unwrap_or_default(),
            idle_action_runs: None,
            action_editor:project_actions::ActionEditor::new(window,cx),
            project_actions:persisted.as_ref().map(|s|s.project_actions.clone()).unwrap_or_default(),
            project_defaults: persisted.as_ref().map(|state| state.project_defaults.clone()).unwrap_or_default(),
            settings_project_scope: String::new(),
            project_picker_current_path: String::new(),
            project_picker_current_name: String::new(),
            project_picker_parent_path: None,
            project_picker_dirs: Vec::new(),
            project_picker_error: None,
            project_picker_start_warning: None,
            project_picker_task: None,
            project_picker_scroll: ScrollHandle::new(),
            workspace_focus_handle: cx.focus_handle().tab_stop(false),
            #[cfg(target_os = "macos")]
            last_embedded_shell_focus: None,
            palette_return_focus: None,
            modal_start_focus_handle,
            blur_scrim: BlurScrimHandle::default(),
            modal_end_focus_handle,
            modal_focus: modal_ui::ModalFocus::new(cx),
            modal_scroll: ScrollHandle::new(),
            import_picker: import_ui::ImportPicker::default(),
            import_input,
            move_in: move_in_ui::MoveInPicker::default(),
            command_input,
            capture_input,
            capture_cancel_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            capture_submit_focus: cx.focus_handle().tab_index(0).tab_stop(true),
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
            knowledge,
            clip_create_visible: false,
            saved_records_loaded,
            inbox_import_visible: false,
            inbox_import_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            inbox_import_returns_focus: false,
            inbox_mail_visible: false,
            connected_inbox,
            inbox_page: 0,
            inbox_filter: bruhs_models::InboxFilter::Inbox,
            inbox_search_input,
            inbox_directory: RefCell::new(None),
            inbox_person_menu_open: false,
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
            scheduling_error: None,
            scheduling_scroll: ScrollHandle::new(),
            scheduling_link_title_input,
            scheduling_link_duration_input,
            scheduling_selected_task_id: None,
            life_tab: DesktopLifeTab::Finance,
            life_notice: None,
            life_health,
            life_finance,
            life_csv_input,
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
            search_selected: 0,
            person_rename_input,
            speaker_rename_input,
            renaming_person_id: None,
            expanded_person_id: None,
            merge: None,
            people_page,
            editing_speaker: None,
            live_chunk: None,
            speaker_menu: None,
            smoke_thread_id: None,
            smoke_previous_active_id: None,
            terminal_panel,
            preview_panel,
            terminal_drawer_open: false,
            workbench_panel: None,
            whiteboard_panel: None,
            whiteboard_panels: std::collections::BTreeMap::new(),
            terminal_drawer_height,
            right_panel_width,
            sidebar_motion:Default::default(),right_panel_motion:Default::default(),
            right_panel_split_state,
            terminal_split_state,
            zoom: crate::zoom::restore(persisted.as_ref().and_then(|state| state.zoom)),
            _subscriptions: subscriptions,
        };
        workspace.sync_credential_sections(window, cx);
        workspace.diff_panel.update(cx,|panel,cx|panel.set_default_wrap(workspace.fonts.wrap,cx));
        workspace.restore_project_list(cx);
        workspace.refresh_project_identities(cx);
        workspace.schedule_project_refresh(cx);
        workspace.start_pull_lifecycle(cx);
        workspace.monitor_provider_capacity(cx);
        workspace.monitor_power(cx);
        workspace.schedule_health_checks(cx);
        workspace.schedule_log_cleanup(cx);
        workspace.reveal_selected_task_page();
        workspace.stamp_run_prefs_on_threads(cx);
        if untrusted_workspace_state {
            workspace.quota_recoveries.clear();
        } else {
            workspace.restore_quota_recoveries(cx);
            if let Some(state) = persisted.as_mut() {
                workspace.restore_interrupted_runs(std::mem::take(&mut state.interrupted_runs), cx);
            }
        }
        workspace.resume_title_requests(cx);
        #[cfg(debug_assertions)]
        if let (Some(prompt),Some(thread))=(crate::views::chat::desktop_smoke_prompt(),workspace.active_thread()){
            thread.chat.update(cx,|chat,cx|chat.submit_prompt(prompt,cx));
        }
        // Return to the view the app closed on, as the web does from its URL.
        if let Some(view) = reopen_view {
            cx.defer_in(window, move |this, window, cx| {
                this.reopen_view_after_start(view, window, cx);
            });
        }
        workspace.sync_keybindings(cx);
        workspace.load_generators(cx);
        workspace.refresh_keymap_file(cx);
        #[cfg(not(test))]
        workspace.watch_project_names(cx);
        workspace.apply_device_appearance(window, cx);
        workspace.apply_zoom(window, cx);
        // With nothing focused, gpui dispatches keys from the window root,
        // past every workspace action: Tab and the shortcuts would do
        // nothing until a click. Start in the workspace, as a web page
        // starts with the document focused.
        cx.defer_in(window, |this, window, cx| {
            if window.focused(cx).is_none() {
                this.workspace_focus_handle.focus(window, cx);
            }
        });
        workspace.start_meeting_suggestions(cx);
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30))
                .await;
            if this
                .update(cx, |this, cx| {
                    this.compact_idle_if_due(cx);
                    if matches!(this.active_view, AppView::Today) {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        workspace.show_update_receipt(cx);
        workspace
    }

    /// Compact the open Claude thread shortly before its prompt cache expires.
    fn compact_idle_if_due(&mut self, cx: &mut Context<'_, Self>) {
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        chat.update(cx, |chat, cx| chat.compact_idle_if_due(cx));
    }

    /// Pick the model of the open thread (the composer's chip), which new
    /// threads then start on. Each thread keeps its own model, so another
    /// thread's session still resumes on the provider that owns it.
    fn set_current_model(&mut self, model_id: impl Into<String>, cx: &mut Context<'_, Self>) {
        let model_id = model_id.into();
        if self.current_model != model_id {self.default_options = Default::default();}
        self.explicit_model = true;
        self.current_model = model_id.clone();
        if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
            chat.update(cx, |chat, cx| chat.set_model(model_id, cx));
        }
        self.persist_and_notify(cx);
    }

    /// Settings → Model: the model new threads start on. The open thread
    /// follows only while it has no turn yet (web: a thread without a model
    /// is on the default).
    fn set_default_model(&mut self, model_id: impl Into<String>, cx: &mut Context<'_, Self>) {
        self.save_default_model(Some(model_id.into()), cx);
    }

    fn save_default_model(&mut self, model: Option<String>, cx: &mut Context<'_, Self>) {
        if model.as_deref()!=Some(self.current_model.as_str()) {self.default_options=Default::default();}
        self.explicit_model = model.is_some();
        self.current_model = model.unwrap_or_else(|| DEFAULT_AGENT_MODEL.to_owned());
        if let Some(chat) = self
            .active_thread()
            .map(|thread| thread.chat.clone())
            .filter(|chat| chat.read(cx).messages.is_empty())
        {
            let path = chat.read(cx).settings_project_path();
            let (model, _) = self.new_thread_defaults(path);
            chat.update(cx, |chat, cx| chat.set_model(model, cx));
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
                chat.project_permission_override = false;
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

    /// Whether the window is narrower than the web breakpoint
    /// `breakpoint_px`: the web shows its layout without that `sm:`, `md:`
    /// or `lg:` variant.
    fn narrower_than(window: &Window, breakpoint_px: f32) -> bool {
        // In CSS pixels, so a zoomed window switches layouts at the widths a
        // zoomed browser page does.
        bruhs_ui::zoom::css_px(window.viewport_size().width) < breakpoint_px
    }

    /// Web: below `sm` the sidebar is a drawer over a scrim; from `sm` it
    /// docks. The window's 640px minimum keeps it docked outside tests.
    fn sidebar_drawer_layout(window: &Window) -> bool {
        Self::narrower_than(window, SM_BREAKPOINT_PX)
    }

    /// Web: below `md` the right panel is a drawer over a scrim; from `md`
    /// it docks beside the chat.
    fn right_panel_drawer_layout(window: &Window) -> bool {
        Self::narrower_than(window, MD_BREAKPOINT_PX)
    }

    /// ⌘+ / ⌘− / ⌘0: zoom the window's content like a browser zooms the web
    /// app, and remember the level.
    pub(crate) fn set_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.zoom = zoom;
        self.apply_zoom(window, cx);
        self.persist_and_notify(cx);
    }

    /// Draw the window's design lengths at the saved zoom (see
    /// [`bruhs_ui::zoom`]), and keep the macOS traffic lights centred in
    /// the zoomed header.
    fn apply_zoom(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        bruhs_ui::zoom::set_zoom(self.zoom, cx);
        #[cfg(target_os = "macos")]
        window.set_traffic_light_position(point(
            bruhs_ui::zoom::window_px(12.0),
            bruhs_ui::zoom::window_px(crate::zoom::traffic_light_top(
                theme::APP_HEADER_HEIGHT_PX,
                self.zoom,
            )),
        ));
        window.refresh();
    }

    /// Room for the macOS traffic lights, which stay in window points while
    /// the page zooms; elsewhere a padding that zooms with the page.
    fn traffic_light_inset(&self) -> Pixels {
        #[cfg(target_os = "macos")]
        {
            bruhs_ui::zoom::window_px(SIDEBAR_HEADER_LEFT_INSET_PX)
        }
        #[cfg(not(target_os = "macos"))]
        {
            px(SIDEBAR_HEADER_LEFT_INSET_PX)
        }
    }

    fn toggle_sidebar_for_window(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        if Self::sidebar_drawer_layout(window) {
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
        if self.settings_visible() && self.settings_section == SettingsSection::About {
            if back && self.license_browser.is_some() {
                self.return_to_about(window, cx);
                return;
            }
            if !back && self.license_forward {
                self.open_license_browser(window, cx);
                return;
            }
        }
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
    /// The sidebar toggle in the top bar: shadcn's `SidebarTrigger`, a 28 px
    /// ghost button around a 16 px icon.
    fn render_collapsed_sidebar_strip(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("collapsed-sidebar-strip")
            .debug_selector(|| "desktop-collapsed-sidebar-strip".to_string())
            .h_full()
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(
                div()
                    .id("collapsed-sidebar-toggle")
                    .debug_selector(|| "desktop-sidebar-toggle".to_string())
                    .group("desktop-sidebar-toggle")
                    .size(px(SIDEBAR_TRIGGER_PX))
                    .rounded(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|el| el.bg(theme::longan_800().opacity(0.5)))
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Toggle sidebar (⌘B)")
                    .border_1()
                    .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .map(|el| {
                        on_press_view(el, cx, |this, window, cx| {
                            this.toggle_sidebar_for_window(window, cx);
                        })
                    })
                    .child(
                        svg()
                            .path("icons/panel-left.svg")
                            .size(px(theme::ICON_PX))
                            .text_color(theme::rambutan_400())
                            .group_hover("desktop-sidebar-toggle", |el| {
                                el.text_color(theme::rambutan_100())
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_main_surface(&self, window: &Window, cx: &Context<'_, Self>) -> Div {
        match &self.active_view {
            AppView::Today => self.render_today_page(window, cx),
            AppView::Inbox => self.render_inbox_page(cx),
            AppView::Knowledge => self.render_knowledge_page(cx),
            AppView::Chat => {
                let main: AnyElement = if let Some(thread) = self.active_thread() {
                    self.conversation_with_details(thread, window, cx)
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
                        .with_handle_appearance(bruhs_ui::resize_handle::handle_appearance())
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
            AppView::Tasks => self.render_tasks_page(window, cx),
            AppView::Calendar => self.render_calendar_page(cx),
            AppView::Life => self.render_life_page(window, cx),
            AppView::Automations => self.render_automations_page(window, cx),
            AppView::Bruh { .. } => self.render_bruh_page(),
            AppView::LinkedTaskThread { .. } => self.render_linked_thread(cx),
            AppView::Room { .. } => self.render_bruh_page(),
            AppView::Workbench => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .when_some(self.workbench_panel.as_ref(), |el, panel| {
                    el.child(panel.clone())
                }),
            AppView::Whiteboards { .. } => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .when_some(self.whiteboard_panel.as_ref(), |el, panel| {
                    el.child(panel.clone())
                }),
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
                .child(self.render_people_page(window, cx)),
            AppView::Search => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .child(self.render_search_page(window, cx)),
            AppView::MeetingDetail { meeting_id } => {
                let id = meeting_id.clone();
                let detail = div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .child(self.render_meeting_detail(&id, cx));
                self.with_back_to_people(detail, cx)
            }
            AppView::MeetingNoteDetail { note_id } => self.render_local_meeting_note(note_id, cx),
            // Web: the document's metadata column docks from `lg`.
            AppView::DocumentDetail { document_id } => self.render_document_editor(
                document_id,
                Self::narrower_than(window, LG_BREAKPOINT_PX),
                cx,
            ),
            AppView::ClipDetail { .. } => self.render_source_clip_editor(false, cx),
            AppView::ConversationDetail { conversation_id } => {
                let detail = self.render_conversation_detail(
                    conversation_id,
                    Self::narrower_than(window, SM_BREAKPOINT_PX),
                    cx,
                );
                self.with_back_to_people(detail, cx)
            }
            // Web `sm:grid-cols-2`: two columns of times from `sm`.
            AppView::Booking { slug } => {
                self.render_booking_page(slug, Self::narrower_than(window, SM_BREAKPOINT_PX), cx)
            }
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
            .child("Open this view at bruhs.ai in your browser.")
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.reconcile_linked_task_editor(window,cx);
        self.reconcile_linked_thread(window,cx);
        self.reconcile_linked_delivery_dialog(window, cx);
        self.restore_theme_preview(window,cx);
        if self.settings_visible() && self.settings_section == SettingsSection::SourceControl {
            self.sync_writer_editor(window,cx);
        }
        if self.settings_visible() && self.settings_section == SettingsSection::Worktrees &&
            self.branch_editor_scope != self.settings_project_scope {
            self.commit_branch_fields(window, cx);
            self.load_branch_fields(window, cx);
        }
        #[cfg(target_os = "macos")]
        {
            let focused = window.focused(cx);
            if self.last_embedded_shell_focus != focused {
                crate::views::preview_panel::focus_native_shell(window);
                self.last_embedded_shell_focus = focused;
            }
        }
        let title = (self.screen_reader.window_title && self.active_view == AppView::Chat)
            .then(|| self.active_thread().map(|thread| thread.chat.read(cx).reader_title())).flatten();
        if self.last_reader_title != title {
            window.set_window_title(title.as_deref().unwrap_or(crate::DESKTOP_WINDOW_TITLE));
            self.last_reader_title = title;
        }
        self.sync_api_client_visibility(window, cx);
        self.sync_mobile_offer();
        // As on the web, a dry run belongs to Agent workflows: once another
        // page shows, a dry run still waiting is reviewed in its own card.
        if self.active_view != AppView::Automations {
            self.dry_run_workflow_id = None;
        }
        // The web mounts its time grid afresh whenever it appears.
        if self.active_view != AppView::Calendar || self.calendar_mode == DesktopCalendarMode::Month
        {
            self.calendar_timegrid_autoscroll.set(true);
        }
        if !self.settings_visible() || self.settings_section != SettingsSection::Vault {
            let sources = self.vault_settings.read(cx).secret_sources();
            if sources.read(cx).needs_reset() {
                sources.update(cx, |view, _| view.leave_settings());
            }
        }
        if !self.settings_visible() && self.vault_settings.read(cx).holds_visit(cx) {
            self.vault_settings
                .update(cx, |view, cx| view.left(window, cx));
        }
        if !self.settings_visible() && self.apps_settings.read(cx).has_editor() {
            self.apps_settings
                .update(cx, |view, cx| view.clear_editor(window, cx));
        }
        if !self.settings_visible() && self.backup_settings.read(cx).holds_text(cx) {
            self.backup_settings
                .update(cx, |view, cx| view.clear_fields(window, cx));
        }
        for thread in &self.threads {
            let visible = self.active_view == AppView::Chat
                && self.active_thread_id.as_deref() == Some(thread.id.as_str());
            thread.chat.update(cx, |chat, cx| {
                let details_open = visible && !self.overlay_open() && self.details.is_open(&thread.id);
                if chat.details_open != details_open { chat.details_open = details_open; cx.notify(); }
                chat.set_rich_response_shown(visible && !self.overlay_open(), cx);
                if !visible {
                    chat.cancel_dictation(cx);
                }
            });
            if !visible && thread.chat.read(cx).has_private_input() {
                thread.chat.update(cx, |chat, _| chat.clear_private_input());
            }
        }
        self.page_history.visit(&page_key(&self.active_view));
        self.forget_left_people_return();
        self.sync_thread_catalog(cx);
        self.sync_create_bruh(cx);
        self.sync_room_dialog(cx);
        self.sync_bruh_page(cx);
        self.schedule_modal_first_control(window, cx);
        self.restore_theme_dialog_focus(window, cx);
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
        let sidebar_drawer = Self::sidebar_drawer_layout(window);
        let right_panel_drawer = Self::right_panel_drawer_layout(window);
        // The thread menu draws with its card and lives while focus is in
        // it. When its card goes (the sidebar hid, a fold closed, a filter
        // changed) or focus left it (a shortcut opened another surface), it
        // closes, so its backdrop cannot swallow the next click.
        let menu_focused = self.thread_menu_has_focus(window);
        if self.thread_menu_id.is_some()
            && (!menu_focused || !self.thread_menu_card_shown(sidebar_drawer, cx))
        {
            self.thread_menu_id = None;
        }
        // A focused item that no longer draws strands the keyboard: GPUI
        // would dispatch from the window root, past every workspace action.
        if self.thread_menu_id.is_none() && menu_focused {
            cx.defer_in(window, |this, window, cx| {
                if this.thread_menu_id.is_none() && this.thread_menu_has_focus(window) {
                    this.workspace_focus_handle.focus(window, cx);
                }
            });
        }
        // Retain filters while the same dialog is open. On close, remove them
        // from the native layers so the Metal-backed workspace stays crisp.
        if self.top_layer.is_none() && self.overlay.is_none() {
            self.blur_scrim.hide();
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
        if self.active_view == AppView::Workbench && self.workbench_panel.is_none() {
            let usage = std::mem::take(&mut self.usage_hubs.open_workbench_usage);
            let panel = cx.new(|cx| crate::views::workbench::WorkbenchPanel::new(usage, window, cx));
            // Verification's "Use draft" fills the composer, as on the web.
            self._subscriptions.push(cx.subscribe_in(
                &panel,
                window,
                |this, _panel, event: &crate::views::workbench::WorkbenchEvent, window, cx| {
                    let crate::views::workbench::WorkbenchEvent::DictationDraft(text) = event;
                    this.receive_dictation_draft(text, window, cx);
                },
            ));
            self.workbench_panel = Some(panel);
        }
        if self.active_view == AppView::Workbench {
            if std::mem::take(&mut self.usage_hubs.open_workbench_usage) {
                if let Some(panel) = &self.workbench_panel {
                    panel.update(cx, |panel, cx| panel.open_usage(window, cx));
                }
            }
            if let (Some(id), Some(panel)) = (
                self.work_item_handoff.open.take(),
                self.workbench_panel.as_ref(),
            ) {
                panel.update(cx, |panel, cx| panel.open_handoff(&id, window, cx));
            }
        }
        if let AppView::Whiteboards { board_id } = self.active_view.clone() {
            let project = self.whiteboard_project(cx);
            let panel = self.whiteboard_library(project, window, cx);
            if self
                .whiteboard_panel
                .as_ref()
                .is_none_or(|current| current.entity_id() != panel.entity_id())
            {
                if let Some(id) = board_id {
                    panel.update(cx, |panel, cx| panel.open(id, window, cx));
                }
            }
            self.whiteboard_panel = Some(panel);
        }
        for panel in self.whiteboard_panels.values() {
            let shown = matches!(self.active_view, AppView::Whiteboards { .. })
                && self
                    .whiteboard_panel
                    .as_ref()
                    .is_some_and(|active| active.entity_id() == panel.entity_id());
            panel.update(cx, |panel, _| panel.set_shown(shown));
        }
        let workbench_shown = self.active_view == AppView::Workbench && !self.overlay_open();
        if let Some(panel) = self.workbench_panel.as_ref() {
            panel.update(cx, |panel, cx| panel.set_shown(workbench_shown, cx));
        }
        let booking = matches!(self.active_view, AppView::Booking { .. });
        let sidebar_docked = !sidebar_drawer && self.sidebar_visible && !booking;
        let sidebar_drawer_open = sidebar_drawer && self.compact_sidebar_open && !booking;
        let chat_panel_open = self.right_panel_open() && self.active_view == AppView::Chat;
        let right_drawer_width = (bruhs_ui::zoom::css_px(window.viewport_size().width)
            * RIGHT_PANEL_DRAWER_MAX_WINDOW_SHARE)
            .min(RIGHT_PANEL_DRAWER_WIDTH_PX);
        let now=std::time::Instant::now();
        let duration=if crate::motion::reduce_motion(cx) {0}else{self.appearance.panel_duration_ms};
        let sidebar_fraction=self.sidebar_motion.sample(sidebar_docked||sidebar_drawer_open,now,if booking{0}else{duration});
        let panel_fraction=self.right_panel_motion.sample(chat_panel_open,now,if self.active_view==AppView::Chat{duration}else{0});
        if self.sidebar_motion.running() || self.right_panel_motion.running(){window.request_animation_frame();}
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
            // Like web, the booking page is a page of its own.
            .when(!sidebar_drawer && (sidebar_docked || sidebar_fraction>0.0), |el| {
                el.child(
                    div()
                        .w(px(WORKSPACE_SIDE_PANEL_WIDTH_PX*sidebar_fraction))
                        .min_w(px(WORKSPACE_SIDE_PANEL_MIN_WIDTH_PX*sidebar_fraction))
                        .overflow_hidden()
                        .h_full()
                        .flex_shrink_0()
                        .child(self.render_sidebar(window, cx)),
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
                    .when(!booking, |el| {
                        el.child(self.render_workspace_navigation(sidebar_drawer, cx))
                    })
                    // Under the navigation, above the page, as the web places
                    // its notices: never under the macOS window buttons.
                    .when_some(self.render_save_status(cx), |el, status| el.child(status))
                    .when_some(self.persistence_error.clone(), |el, error| {
                        el.child(self.render_persistence_error(error, cx))
                    })
                    .when_some(self.render_workflow_status(cx), |el, status| {
                        el.child(status)
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(self.render_main_surface(window, cx))
                            // Notices float over the view (no layout shift).
                            .when(!self.settings_has_notice(cx), |el| el.children(self.render_hub_notice(window, cx))),
                    );
                if !right_panel_drawer && (chat_panel_open || panel_fraction>0.0) {
                    // Web: the panel's left edge drags and the width is
                    // remembered, clamped, and gives way to the chat's 360px
                    // minimum. A window resize never changes the remembered
                    // width, so the panel holds it unless its edge is dragged.
                    let viewport_width = bruhs_ui::zoom::css_px(window.viewport_size().width);
                    let sidebar_width = if sidebar_docked {
                        WORKSPACE_SIDE_PANEL_WIDTH_PX
                    } else {
                        0.0
                    };
                    let width = docked_right_panel_width(
                        self.right_panel_width,
                        viewport_width,
                        sidebar_width,
                    );
                    let (min_width, max_width) =
                        if self.right_panel_split_state.read(cx).is_resizing() {
                            docked_right_panel_range(viewport_width, sidebar_width)
                        } else {
                            (width*panel_fraction,width*panel_fraction)
                        };
                    div()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .min_h_0()
                        .child(
                            h_resizable("chat-panel-split")
                                .with_handle_appearance(
                                    bruhs_ui::resize_handle::handle_appearance(),
                                )
                                .with_state(&self.right_panel_split_state)
                                .child(resizable_panel().child(main_column))
                                .child(
                                    resizable_panel()
                                        .size(px(width*panel_fraction))
                                        .size_range(px(min_width)..px(max_width))
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
            .when(sidebar_drawer && (sidebar_drawer_open || sidebar_fraction>0.0), |el| {
                el.child(
                    div()
                        .id("compact-sidebar-backdrop")
                        .debug_selector(|| "desktop-compact-sidebar-backdrop".to_string())
                        .absolute()
                        .size_full()
                        .bg(theme::longan_950().opacity(DRAWER_SCRIM_OPACITY*sidebar_fraction))
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
                        .left(px(-WORKSPACE_SIDE_PANEL_WIDTH_PX*(1.0-sidebar_fraction)))
                        .top_0()
                        .w(px(WORKSPACE_SIDE_PANEL_WIDTH_PX))
                        .max_w_full()
                        .h_full()
                        .occlude()
                        .child(self.render_sidebar(window, cx)),
                )
            })
            .when(right_panel_drawer && (chat_panel_open || panel_fraction>0.0), |el| {
                el.child(
                    div()
                        .id("compact-right-panel-backdrop")
                        .debug_selector(|| "desktop-compact-right-panel-backdrop".to_string())
                        .absolute()
                        .size_full()
                        .bg(theme::longan_950().opacity(DRAWER_SCRIM_OPACITY*panel_fraction))
                        .occlude()
                        .on_click(cx.listener(|this, _event, window, cx| {
                            cx.stop_propagation();
                            this.close_panel(&ClosePanel, window, cx);
                        })),
                )
                .child(
                    div()
                        .id("compact-tasks-panel")
                        .debug_selector(|| "desktop-compact-tasks-panel".to_string())
                        .role(Role::Complementary)
                        .aria_label("Right panel")
                        .absolute()
                        .right(px(-right_drawer_width*(1.0-panel_fraction)))
                        .top_0()
                        .w(px(right_drawer_width))
                        .h_full()
                        .occlude()
                        .child(self.render_right_panel(cx)),
                )
            });

        div()
            .key_context("Workspace")
            .capture_key_down(cx.listener(Self::quit_down))
            .capture_key_up(cx.listener(|this, event, window, cx| {
                this.voice_shortcut_key_up(event, cx);
                this.quit_up(event, window, cx);
            }))
            .on_modifiers_changed(cx.listener(|this, event, window, cx| {
                this.voice_shortcut_modifiers(event, cx);
                this.quit_modifiers(event, window, cx);
            }))
            .text_size(px(theme::APP_INPUT_TEXT_SIZE_PX))
            .track_focus(&self.workspace_focus_handle)
            .capture_any_mouse_down(|_, window, _| {
                crate::views::preview_panel::focus_native_shell(window);
            })
            .capture_action(|_: &bruhs_ui::input::Undo, window, cx| {
                #[cfg(target_os = "macos")]
                if crate::views::preview_panel::edit_native_history(false) {
                    cx.stop_propagation();
                    return;
                }
                let _ = window;
                cx.propagate();
            })
            .capture_action(|_: &bruhs_ui::input::Redo, window, cx| {
                #[cfg(target_os = "macos")]
                if crate::views::preview_panel::edit_native_history(true) {
                    cx.stop_propagation();
                    return;
                }
                let _ = window;
                cx.propagate();
            })
            // Tab in a multi-line field moves focus, as it leaves a web
            // textarea: nothing traps the keyboard. The composer indents its
            // list items and code first (`composer_edit`), and accepts a
            // suggestion, before this runs.
            .capture_action(cx.listener(
                |this, _: &bruhs_ui::input::IndentInline, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    if this.overlay_open()
                        && this.modal_contains_focus(window, cx)
                    {
                        this.step_modal_focus(false, window, cx);
                    } else {
                        window.focus_next(cx);
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, _: &bruhs_ui::input::OutdentInline, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    if this.overlay_open()
                        && this.modal_contains_focus(window, cx)
                    {
                        this.step_modal_focus(true, window, cx);
                    } else {
                        window.focus_prev(cx);
                    }
                },
            ))
            // Every bruhs type role is `line-height: 1.5`; gpui defaults to φ
            // (1.618), which made every row ~2px taller than the web's.
            .line_height(relative(WORKSPACE_LINE_HEIGHT))
            .on_action(cx.listener(Self::new_thread))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                this.set_zoom(crate::zoom::zoom_in(this.zoom), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| {
                this.set_zoom(crate::zoom::zoom_out(this.zoom), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ResetZoom, window, cx| {
                this.set_zoom(crate::zoom::DEFAULT, window, cx);
            }))
            .on_action(cx.listener(Self::toggle_tasks))
            .on_action(cx.listener(Self::focus_command_bar))
            .on_action(cx.listener(Self::focus_chat_input))
            .on_action(cx.listener(Self::focus_crew))
            .on_action(cx.listener(Self::stop_all))
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
            .on_action(cx.listener(Self::capture_window))
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
            .on_action(cx.listener(Self::open_web_link))
            .on_action(cx.listener(Self::open_local_servers))
            .on_action(cx.listener(Self::search_threads))
            .on_action(cx.listener(Self::undo_sidebar_action))
            .on_action(cx.listener(Self::open_project_in_editor_action))
            .on_action(cx.listener(Self::open_import_thread))
            .on_action(cx.listener(Self::copy_resume_command_action))
            .on_action(cx.listener(Self::copy_thread_markdown_action))
            .on_action(cx.listener(Self::continue_in_workbench_action))
            .size_full()
            .relative()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .flex()
            .flex_col()
            .children(crate::views::workspace::appearance::background_layer("Root", cx))
            .children(self.inspection_frame(true,cx))
            .child(body)
            .when(matches!(self.overlay, Some(Overlay::ThemeEditor(_) | Overlay::RemoveThemeVariants(_))), |el| {
                el.child(self.render_settings(window, cx))
            })
            .children(self.inspection_frame(false,cx))
            .when_some(self.overlay.clone(), |el, overlay| {
                el.child(match overlay {
                    Overlay::Capture => self.render_capture_dialog(window, cx),
                    Overlay::Welcome => self.render_welcome(window, cx),
                    Overlay::Settings { .. } => self.render_settings(window, cx),
                    Overlay::TaskCreate { .. } => self.render_task_create_dialog(window, cx),
                    Overlay::EventCreate { .. } => self.render_event_create_dialog(window, cx),
                    Overlay::CalendarDay { date } => {
                        self.render_calendar_day_list(date, window, cx)
                    }
                    Overlay::ProjectDeleteConfirm { .. } => {
                        self.render_project_delete_confirm(window, cx)
                    }
                    Overlay::TaskDeleteConfirm { ref item_id } => {
                        self.render_task_delete_confirm(item_id, window, cx)
                    }
                    Overlay::TaskProjects => self.render_task_projects(window, cx),
                    Overlay::RecordDeleteConfirm { ref target } => {
                        self.render_record_delete_confirm(target, window, cx)
                    }
                    Overlay::UpdateConfirm { .. } => self.render_update_confirm(window, cx),
                    Overlay::ThemeEditor(editor) => self.render_theme_editor(editor,window,cx),
                    Overlay::RemoveThemeVariants(removal) => self.render_variant_removal(removal, window, cx),
                    Overlay::RestartConfirm { .. } => self.render_restart_confirm(window, cx),
                    Overlay::SnoozeCustom { .. } => self.render_snooze_custom(window, cx),
                    Overlay::CreateBruh => self.render_create_bruh(window, cx),
                    Overlay::RoomDialog => self.render_room_dialog(window, cx),
                })
            })
            // Drawn after the form dialog so it sits on top of it.
            .when(matches!(self.overlay,Some(Overlay::Settings { .. })), |el| el.children(self.render_theme_import_notice()))
            .when_some(self.top_layer, |el, top_layer| {
                el.child(match top_layer {
                    TopLayer::CommandBar => self.render_command_bar(window, cx),
                    TopLayer::ProjectPicker { .. } => self.render_project_picker(window, cx),
                    TopLayer::ImportThread => self.render_import_thread(window, cx),
                    TopLayer::MoveIn => self.render_move_in(window, cx),
                    TopLayer::RecordPicker {
                        target,
                        highlighted,
                    } => self.render_record_picker(target, highlighted, window, cx),
                })
            })
            // Web: a transparent full-window layer under the thread menu
            // closes it, so that click does nothing else.
            .when(self.thread_menu_id.is_some(), |el| {
                el.child(
                    div()
                        .id("desktop-thread-menu-backdrop")
                        .debug_selector(|| "desktop-thread-menu-backdrop".to_string())
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _event, window, cx| {
                                cx.stop_propagation();
                                this.close_thread_menu(window, cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, _event, window, cx| {
                                cx.stop_propagation();
                                this.close_thread_menu(window, cx);
                            }),
                        ),
                )
            })
            .when(!self.settings_visible() && ((!sidebar_docked && !sidebar_drawer_open) || self.overlay_open()), |el|
                el.children(self.floating_update_progress(cx)))
            .when_some(self.quit_hint(),|el,hint|el.child(hint))
    }
}

#[cfg(test)]
mod tests;
