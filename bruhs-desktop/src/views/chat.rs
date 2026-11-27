use crate::views::task_registry_subscription::TaskRegistrySubscription;
use crate::views::theme_regions::ThemeRegion as _;
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::views::workspace::provider_label;
use crate::{
    app_state::{spawn_tokio, DesktopState},
    assets, theme,
};
use bruhs_core::{
    AgentTurn, OrchestratorEvent, PermissionDecision, QueryEvent, SkillRef, TaskStatus, UsageInfo,
};
use bruhs_models::agent::provider_for_model_id;
use bruhs_models::agent::AgentProvider;
#[cfg(test)]
use bruhs_models::agent::DEFAULT_AGENT_MODEL;
use bruhs_models::ids::{IdError, ThreadId};
use bruhs_models::logic::attachments::UploadedFile;
use bruhs_models::logic::commands::ThreadGoal;
use bruhs_models::logic::compact::IdleCompact;
use bruhs_models::logic::composer_text::TextAttachment;
use bruhs_models::logic::context_chip::{ChipKind, ContextChip};
use bruhs_models::logic::find::{cycle, search_texts};
use bruhs_models::logic::follow_up::QueueEdit;
use bruhs_models::logic::queue::PromptQueue;
use bruhs_models::logic::run_place::{RememberedWorktree, WorkspaceChoice};
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use bruhs_models::EpochMs;
use bruhs_ui::input::{
    Enter as InputEnter, Escape as InputEscape, Input, InputEvent, InputState,
    MoveDown as InputMoveDown, MoveUp as InputMoveUp, TextareaState,
};
use bruhs_ui::scroll::ScrollableElement;
use bruhs_ui::text::{TextView, TextViewStyle};
use bruhs_ui::zoom::px;
use bruhs_ui::ActiveTheme;
use gpui::prelude::*;
use gpui::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_stream::StreamExt;

// --- Actions ---
actions!(
    chat,
    [
        SendMessage,
        ClearInput,
        CyclePermissionMode,
        StashDraft,
        ToggleDictation,
        FindInThread
    ]
);

mod assistant_content;
/// Chat replies' Markdown style, which Knowledge previews reuse.
pub(crate) use assistant_content::desktop_markdown_style;
/// Markdown whose links follow the shared allowlist, which crew prose reuses.
pub(crate) use assistant_content::untrusted_markdown;
mod auth;
#[cfg(test)]
mod behavior_tests;
mod codex_auth;
pub(crate) mod composer_ui;
mod content_cache;
mod dictation;
mod files;
mod git_info;
mod git_ui;
mod hints_ui;
pub(crate) mod kiro;
mod linked_prompt;
mod mobile;
mod model_picker_ui;
mod native_options;
pub(crate) use mobile::mobile_label;
mod prompt_suggestion;
mod question_prompt;
pub(crate) mod response_stream;
mod restart;
mod rich_response;
pub(crate) mod screen_reader;
mod secret_prompt;
#[cfg(debug_assertions)]
mod smoke;
mod streaming;
mod svg_cache;
pub(crate) mod timeline_ui;
mod title_rename;
mod tool_output_ui;
pub(crate) mod tool_visibility;
mod viewport;
mod workspace_trust_ui;
use assistant_content::*;
#[cfg(test)]
use auth::*;
pub use composer_ui::CHAT_INPUT_PLACEHOLDER;
use composer_ui::{
    ComposerPlacement, StashEntry, CHAT_COMPOSER_BOTTOM_PADDING_PX, CHAT_COMPOSER_PAGE_X_PADDING_PX,
};
use content_cache::{BoundedCache, PreparedBlock, PreparedSvg};
#[cfg(test)]
use git_ui::*;
#[cfg(test)]
use hints_ui::*;
#[cfg(test)]
use model_picker_ui::*;
#[cfg(debug_assertions)]
pub(crate) use streaming::desktop_smoke_prompt;
#[cfg(test)]
use streaming::*;
use tool_output_ui::*;

// --- Data types ---

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComposerHintDirection {
    Previous,
    Next,
}

const CHAT_MESSAGE_LIST_MAX_WIDTH_PX: f32 = timeline_ui::TRANSCRIPT_MAX_WIDTH_PX;
const CHAT_MESSAGE_LIST_X_PADDING_PX: f32 = timeline_ui::TRANSCRIPT_X_PADDING_PX;
const CHAT_MESSAGE_LIST_Y_PADDING_PX: f32 = timeline_ui::TRANSCRIPT_TOP_PADDING_PX;
const CHAT_MESSAGE_LIST_GAP_PX: f32 = 0.0;
// Initial geometry hint: one text line, bubble padding, actions, gap and footer.
// Visible messages are always measured at their actual variable height.
const CHAT_MESSAGE_ROW_HINT_PX: f32 = timeline_ui::TRANSCRIPT_BODY_TEXT_SIZE_PX * 1.5
    + timeline_ui::USER_BUBBLE_Y_PADDING_PX * 2.0
    + timeline_ui::TRANSCRIPT_ROW_MIN_HEIGHT_PX
    + 20.0;
const CHAT_COMPOSER_MIN_ROWS: usize = 2;
const CHAT_COMPOSER_MAX_ROWS: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopMentionItem {
    path: String,
    name: String,
    is_dir: bool,
}

/// Another thread offered by the `@` menu.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ThreadHit {
    id: String,
    title: String,
}

/// The account `claude auth status` reported.
#[derive(Clone, Debug, Default, PartialEq)]
struct ClaudeAccount {
    user: Option<String>,
    subscription_type: Option<String>,
    auth_method: Option<String>,
}

/// Why the banner shows while signed out.
#[derive(Clone, Debug, Default, PartialEq)]
struct SignedOut {
    /// The status check itself failed (CLI missing, timed out, …).
    error: Option<String>,
    /// Guidance under the banner title (waiting for login, what failed).
    message: Option<String>,
    /// `claude auth login` is running.
    login_in_flight: bool,
}

/// The Claude CLI login as last checked. Until the first check answers it
/// reads as signed out, so the banner shows like the web's.
#[derive(Clone, Debug, PartialEq)]
enum DesktopAuthStatus {
    SignedIn(ClaudeAccount),
    SignedOut(SignedOut),
}

pub(crate) fn cli_health(
    cx: &App,
    prefs: bruhs_core::agent_extensions::availability::ProviderAvailability,
) -> impl std::future::Future<Output = Vec<bruhs_core::agent::health::ProviderHealth>> + Send + 'static
{
    use bruhs_core::agent::health::{self, ProviderHealth, Status};
    use bruhs_models::agent::AgentProvider;
    let claude =
        auth::auth_check_program(cx, prefs.enabled(AgentProvider::Claude)).map(|p| p.into_owned());
    let codex = codex_auth::codex_program(cx, prefs.enabled(AgentProvider::Codex));
    let external = !cfg!(test);
    #[cfg(feature = "native-performance")]
    let external = external
        && !crate::native_performance::block_service(cx, crate::native_performance::Service::Auth);
    async fn native(
        provider: AgentProvider,
        program: Option<String>,
        enabled: bool,
    ) -> ProviderHealth {
        if !enabled {
            return ProviderHealth::new(provider, Status::Disabled);
        }
        let mut row = ProviderHealth::new(provider, Status::Failed);
        let Some(program) = program else { return row };
        let (version, auth) = tokio::join!(
            health::native_cli_version(std::path::Path::new(&program)),
            async {
                if provider == AgentProvider::Claude {
                    auth::fetch_auth_status_with(&program).await
                } else {
                    codex_auth::fetch_codex_status_with(&program).await
                }
            }
        );
        row.status = match auth {
            DesktopAuthStatus::SignedIn(_) => Status::SignedIn,
            DesktopAuthStatus::SignedOut(account) if account.error.is_none() => Status::SignedOut,
            _ => version.as_ref().err().copied().unwrap_or(Status::Failed),
        };
        row.version = version.ok().flatten();
        row
    }
    async move {
        let (claude, codex, external) = tokio::join!(
            native(
                AgentProvider::Claude,
                claude,
                prefs.enabled(AgentProvider::Claude)
            ),
            native(
                AgentProvider::Codex,
                codex,
                prefs.enabled(AgentProvider::Codex)
            ),
            async {
                if external {
                    health::external_provider_health(&prefs).await
                } else {
                    Vec::new()
                }
            }
        );
        let mut rows = vec![claude, codex];
        rows.extend(external);
        rows
    }
}

/// A fork keeps the chosen reply boundary even if its parent is edited or removed.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ForkContext {
    pub parent: String,
    pub title: String,
    pub history: Vec<bruhs_models::logic::portable::PortableMessage>,
}

/// Named apart from gpui's accessibility `Role`, which views also use.
pub use bruhs_models::agent::Role as MessageRole;

/// Where a message's turn is. User and system messages are `Finished`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnState {
    /// Deltas are still arriving.
    Streaming,
    /// The turn ended on its own.
    Finished,
    /// The desktop restarted while the provider was responding.
    InterruptedByRestart,
    /// The user stopped the turn before it finished.
    Stopped,
    /// A provider or transport error ended the turn.
    Failed { error: String },
}

impl TurnState {
    /// The state saved as the state file's `stopped` / `error` pair. A
    /// record with both (never written) reads as failed: the error is shown.
    pub fn from_persisted(stopped: bool, error: Option<String>) -> Self {
        match (error, stopped) {
            (Some(error), _) => Self::Failed { error },
            (None, true) => Self::Stopped,
            (None, false) => Self::Finished,
        }
    }

    pub fn is_streaming(&self) -> bool {
        matches!(self, Self::Streaming)
    }

    pub fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped)
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error } => Some(error),
            Self::InterruptedByRestart => Some("Interrupted by restart"),
            _ => None,
        }
    }
}

/// One transcript message: data only. What the user has expanded lives on
/// `ChatView` (`expanded_thinking`, `expanded_folds`, `expanded_rows`).
#[derive(Clone, Debug)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub model: Option<String>,
    pub thinking: String,
    pub usage: Option<UsageInfo>,
    pub duration_ms: Option<u64>,
    pub started_at_ms: Option<i64>,
    /// Set by the turn's result, so it can precede the end of streaming.
    pub completed_at_ms: Option<i64>,
    pub tool_calls: Vec<DesktopToolCall>,
    /// Files this turn handed to the agent. Empty on assistant rows.
    pub files: Vec<UploadedFile>,
    pub state: TurnState,
    /// Git tree of this thread's worktree at send time.
    pub checkpoint: Option<String>,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum DesktopAssistantContentBlock {
    Markdown(String),
    Math(DesktopMathBlock),
    Mermaid(DesktopMermaidBlock),
    Rich(bruhs_models::logic::rich_response::ResponseBlock),
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopMathBlock {
    source: String,
    display: bool,
    render: DesktopMathRender,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DesktopMathRender {
    Svg(String),
    Error(String),
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopMermaidBlock {
    source: String,
    render: DesktopMermaidRender,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DesktopMermaidRender {
    Svg(String),
    Error(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DesktopToolCall {
    /// Bytes of assistant prose preceding this call. Older transcripts lack this boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_offset: Option<usize>,
    pub id: String,
    pub tool: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

impl bruhs_models::logic::tools::ToolCallView for DesktopToolCall {
    fn tool_name(&self) -> &str {
        &self.tool
    }

    fn input(&self) -> &serde_json::Value {
        &self.input
    }

    fn has_output(&self) -> bool {
        self.output.is_some()
    }

    fn is_error(&self) -> bool {
        self.is_error
    }
}

#[derive(Clone, Debug)]
struct DesktopPermissionRequest {
    id: String,
    tool: String,
    input: serde_json::Value,
    can_answer: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DesktopDiffLineKind {
    Add,
    Remove,
    Context,
}

#[derive(Clone, Debug)]
struct DesktopDiffLine {
    kind: DesktopDiffLineKind,
    content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            model: None,
            thinking: String::new(),
            usage: None,
            duration_ms: None,
            started_at_ms: Some(EpochMs::now().as_millis()),
            completed_at_ms: Some(EpochMs::now().as_millis()),
            tool_calls: Vec::new(),
            files: Vec::new(),
            state: TurnState::Finished,
            checkpoint: None,
        }
    }

    pub fn user_with_files(content: impl Into<String>, files: Vec<UploadedFile>) -> Self {
        let mut message = Self::user(content);
        message.files = files;
        message
    }

    /// The transcript includes a display label for each upload. The title
    /// writer receives those uploads separately from the user's message.
    pub(crate) fn text_for_title(&self) -> &str {
        bruhs_models::logic::composer_text::without_uploaded_file_labels(&self.content, &self.files)
    }

    pub fn assistant_streaming(started_at_ms: i64, model: Option<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: String::new(),
            model,
            thinking: String::new(),
            usage: None,
            duration_ms: None,
            started_at_ms: Some(started_at_ms),
            completed_at_ms: None,
            tool_calls: Vec::new(),
            files: Vec::new(),
            state: TurnState::Streaming,
            checkpoint: None,
        }
    }

    /// A turn that never reached the provider. The draft is already consumed.
    pub fn assistant_failed(model: Option<String>, error: impl Into<String>) -> Self {
        let now = EpochMs::now().as_millis();
        Self {
            role: MessageRole::Assistant,
            content: String::new(),
            model,
            thinking: String::new(),
            usage: None,
            duration_ms: None,
            started_at_ms: Some(now),
            completed_at_ms: Some(now),
            tool_calls: Vec::new(),
            files: Vec::new(),
            state: TurnState::Failed {
                error: error.into(),
            },
            checkpoint: None,
        }
    }
}

/// How a message another thread's agent waited on ended here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadMessageAnswer {
    Replied(String),
    Stopped,
    Failed(String),
    /// It left the queue before it ran.
    Removed,
}

enum AgentUiEvent {
    SecretRequest(bruhs_models::secrets::SecretRequest),
    SecretResolved(String),
    /// The turn's agent asks about another thread; the workspace answers.
    ThreadRequest(bruhs_models::thread_link::ThreadRequest),
    Text(String),
    Thinking(String),
    ToolCall {
        id: String,
        tool: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Usage(UsageInfo),
    ResponseMedia(std::collections::BTreeMap<String, String>),
    QuestionFiles(Vec<UploadedFile>),
    Result {
        result: String,
        duration_ms: u64,
        usage: UsageInfo,
    },
    PermissionRequest(DesktopPermissionRequest),
    PermissionCancelled(String),
    /// The provider announced the turn's session (`init`).
    Session(String),
    /// The saved session could not be resumed; the turn retries fresh.
    SessionRejected,
    QuotaBlocked(Option<i64>),
    Error(String),
    Done,
}

pub use bruhs_models::PermissionMode;

// --- ChatView entity ---

pub enum ChatViewEvent {
    CheckpointInterruptedRun,
    UsageLimitReached {
        reset_ms: Option<i64>,
        generation: u64,
    },
    RunningTasksChanged,
    OpenSettings,
    /// The access-mode menu or the Build/Plan chip picked a mode.
    SetPermissionMode(PermissionMode),
    ToggleTasks,
    ToggleDiff,
    ToggleThreadDetails,
    ForkFromReply {
        index: usize,
    },
    OpenForkSource,
    /// The header's panel-right button (⌘⌥B).
    TogglePanel,
    /// The hero heading / context strip project buttons.
    OpenProjectPicker,
    /// The user picked a model in the popover; the workspace makes it current.
    SelectModel(String),
    ToggleFavoriteModel(String),
    ModelOptionsChanged,
    ThreadOptionsChanged,
    /// A turn began: the workspace stamps model + branch on the thread card.
    TurnStarted {
        model_id: String,
        branch: Option<String>,
    },
    /// The thread needs the owner’s attention (approval or input).
    ThreadAttention(&'static str),
    /// The turn ended (done, error, or stopped).
    TurnFinished,
    /// A `/handoff` brief is ready: open a fresh thread in this thread's
    /// project with `brief` in its composer.
    Handoff {
        brief: String,
    },
    /// This thread's agent asks about, or writes to, another thread. The
    /// workspace owns the threads, so it answers.
    ThreadRequest(bruhs_models::thread_link::ThreadRequest),
    /// `/role [name]`: show, set or remove this thread's role. The workspace
    /// owns the roles, so it applies the command.
    RoleCommand(String),
    /// Something the user should know about failed (shown as a banner).
    Notice(String),
    /// The user committed a new title in the header.
    TitleRenamed,
    /// Saved chat content changed outside a turn's start
    /// or end, e.g. `/clear` or `/help`. Streaming deltas, the spinner and
    /// other view-only changes never emit this, so they never cause a save.
    MessagesChanged,
    QuestionFilesRemoved(Vec<UploadedFile>),
    /// A changed-files card's "Open diff" (web `on_open_diff`): scope the
    /// diff panel to that turn.
    OpenDiff {
        message_index: usize,
    },
    /// A draft tile or a sent file asked the right panel to read `path`.
    OpenWhiteboard {
        source: String,
    },
    OpenFile {
        path: String,
        label: String,
        project: Option<String>,
        source: bool,
    },
    /// Edit from here asked to restore this user message's worktree checkpoint.
    RevertFiles {
        index: usize,
    },
    /// A successfully created checkout is available to reuse.
    RememberWorktree(RememberedWorktree),
    /// Persist a workspace selection for this unsent thread.
    DraftPlacementChanged,
    /// A placed thread’s workspace menu edits the default for subsequent threads.
    NextThreadPlacement(WorkspaceChoice),
    /// Share effort, base branch and composer preferences.
    RunPrefs {
        effort: Option<String>,
        base_branch: String,
        composer: ComposerPrefs,
    },
    /// A newly prepared checkout is ready for background project actions.
    WorktreeReady,
    /// The first send moved this thread onto a worktree. The card follows.
    ThreadPlaced {
        project_path: String,
        branch: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DraftPlacement {
    pub choice: WorkspaceChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<RememberedWorktree>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChatWidth {
    #[default]
    Comfortable,
    Wide,
    Full,
}

impl ChatWidth {
    pub fn id(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Wide => "wide",
            Self::Full => "full",
        }
    }

    fn limit(self) -> Option<f32> {
        match self {
            Self::Comfortable => Some(CHAT_MESSAGE_LIST_MAX_WIDTH_PX),
            Self::Wide => Some(1152.0),
            Self::Full => None,
        }
    }
}

/// Which drafts need the platform modifier to send.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SendShortcut {
    #[default]
    Enter,
    ModEnterMultiline,
    ModEnter,
}

impl SendShortcut {
    pub const OPTIONS: [(Self, &'static str); 3] = [
        (Self::Enter, "enter"),
        (Self::ModEnterMultiline, "mod-enter-multiline"),
        (Self::ModEnter, "mod-enter"),
    ];

    pub fn requires_modifier(self, draft: &str) -> bool {
        match self {
            Self::Enter => false,
            Self::ModEnterMultiline => draft.contains(['\n', '\r']),
            Self::ModEnter => true,
        }
    }

    pub fn id(self) -> &'static str {
        Self::OPTIONS
            .iter()
            .find(|(mode, _)| *mode == self)
            .map_or("enter", |(_, id)| *id)
    }
}

/// Follow-up, auto-compact, and idle compact. One value is copied onto every
/// thread. `follow_up_steer` is the stored choice; Codex still queues.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerPrefs {
    pub follow_up_steer: bool,
    pub autocompact: Option<String>,
    pub compact_idle: IdleCompact,
    /// What agents may do with other threads.
    pub thread_access: bruhs_models::thread_link::ThreadAccess,
    /// Models for second-opinion reviews and meeting summaries.
    pub task_models: bruhs_models::roles::TaskModels,
}

pub struct ChatView {
    pub thread_id: Option<String>,
    pub thread_title: String,
    pub messages: Vec<Message>,
    messages_viewport: ListState,
    /// The page zoom the rows were measured at (`bruhs_ui::zoom`).
    messages_zoom: std::cell::Cell<f32>,
    pub(super) messages_focus: FocusHandle,
    pub(super) composer_area_focus: FocusHandle,
    rendered_messages: std::cell::Cell<Option<(usize, usize)>>,
    #[cfg(test)]
    rendered_message_rows: std::cell::Cell<usize>,
    pub is_streaming: bool,
    readout: screen_reader::Readout,
    pub send_shortcut: SendShortcut,
    pub(crate) slash_skills: bool,
    pub(crate) tool_visibility: tool_visibility::Visibility,
    chat_width: ChatWidth,
    pub spinner_frame: usize,
    stream_started_at: Option<Instant>,
    pub tasks_panel_visible: bool,
    pub diff_panel_visible: bool,
    pub(crate) details_trigger: FocusHandle,
    pub(crate) details_trigger_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    pub(crate) details_open: bool,
    pub(crate) details_inline_open: bool,
    restart_turn: Option<restart::Run>,
    restart_checkpoint: Option<Task<()>>,
    pub(crate) reported_limit: Option<(u64, Option<i64>)>,
    pub running_tasks_count: usize,
    running_tasks: Vec<bruhs_core::BackgroundTask>,
    pub permission_mode: PermissionMode,
    pub(crate) project_permission_override: bool,
    pub model_id: String,
    pub project_path: String,
    /// Shown in the breadcrumb, hero heading and context strip.
    pub project_name: String,
    pub project_scoped: bool,
    /// Any right-panel surface is showing (header panel toggle state).
    pub right_panel_open: bool,
    composer_options_open: bool,
    composer_options_focus: FocusHandle,
    composer_effort_focus: FocusHandle,
    composer_options_end_focus: FocusHandle,
    composer_options_trigger_focus: FocusHandle,
    composer_options_trigger_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    composer_options_scroll: ScrollHandle,
    /// A child of the options panel's body to scroll into view once the
    /// panel has laid out; a scroll area ignores requests before then.
    composer_options_reveal: std::rc::Rc<std::cell::Cell<Option<usize>>>,
    /// The transcript copy button that shows its check, like web's.
    pub(super) copy_feedback: crate::views::copy_feedback::CopyFeedback,
    /// Each turn's action row tracks a handle, keyed by (user row, index),
    /// so the row shows while focus is inside it (web `focus-within`).
    turn_actions_focus: std::cell::RefCell<std::collections::HashMap<(bool, usize), FocusHandle>>,
    composer_options_section_focus: [FocusHandle; 7],
    stash_open: bool,
    /// Parked drafts (⌘S), newest first.
    stash: Vec<StashEntry>,
    /// Prompts waiting for this thread's turn to finish.
    pub prompt_queue: PromptQueue,
    /// Interrupt, or a restored nonempty queue, waits for Resume.
    pub queue_held: bool,
    /// Queued message open in the composer, plus the draft it replaced.
    queue_edit: Option<QueueEdit>,
    linked_draft: Option<linked_prompt::LinkedDraft>,
    /// Composer text applied on the next paint. Stream tasks have no window.
    pending_composer: Option<String>,
    dictation: dictation::ComposerDictation,
    pub(crate) setup_actions: Vec<bruhs_core::git::SetupAction>,
    pub(crate) branch_naming: bruhs_models::logic::run_place::branch_naming::Naming,
    pub(crate) worktree_prefs: bruhs_models::logic::run_place::WorktreePreferences,
    pub(crate) effort_label: bool,
    pub(crate) text_generation_model: String,
    pub(crate) dictation_prefs: crate::voice_capture::DictationPreferences,
    /// Edit from here focuses the composer after that write.
    focus_pending_composer: bool,
    recall: Option<bruhs_models::logic::recall::RecallCursor>,
    /// Index of the user message whose rewind confirm is open.
    rewind_confirm: Option<usize>,
    /// The next turn starts a new provider session.
    pub fresh_session: bool,
    /// Provider session this thread resumes, saved with the thread so a
    /// restart keeps it. Sent only with a turn of `session_provider`.
    pub session_id: Option<String>,
    pub session_provider: Option<AgentProvider>,
    /// Transcript of a thread imported without a session. Turns send it
    /// ahead of the prompt until a provider session holds it.
    pub import_context: Option<String>,
    /// `import_key(source, id)` of a chat imported from an export (D8).
    pub import_key: Option<String>,
    pub(crate) fork: Option<std::sync::Arc<ForkContext>>,
    /// Provider of the newest turn, which owns the session it reports.
    turn_provider: AgentProvider,
    /// The running turn is a `/handoff`: when it ends, its reply seeds a
    /// fresh thread (web: `handoff_place`).
    pub handoff_pending: bool,
    /// Local goal. It stays until `/goal clear`.
    pub goal: Option<ThreadGoal>,
    /// Pasted text kept beside the draft until a real send. In memory only.
    text_attachments: Vec<TextAttachment>,
    /// Files picked, dropped, or pasted. They stay until a real send.
    file_offers: Vec<files::DesktopFileOffer>,
    /// Inline chips whose labels sit in the composer draft.
    context_chips: Vec<ContextChip>,
    /// Draft the chips were last aligned to. The next edit retargets from it.
    chip_draft: String,
    /// The next composer change already installed its own chips.
    chip_guard: bool,
    /// `#` query while the pull-request menu is open.
    pull_query: Option<String>,
    pull_items: Vec<bruhs_core::PullRef>,
    pull_error: Option<String>,
    pull_task: Option<Task<()>>,
    /// `$` query while the skill menu is open.
    skill_query: Option<String>,
    skill_items: Vec<SkillRef>,
    skill_error: Option<String>,
    skill_task: Option<Task<()>>,
    /// Threads matching the current `@` query, listed before files.
    thread_hits: Vec<ThreadHit>,
    /// Chips the open queue edit replaced. Restored on cancel.
    queue_prior_chips: Vec<ContextChip>,
    /// Chips applied with `pending_composer` on the next paint.
    pending_chips: Option<Vec<ContextChip>>,
    /// `Cmd+F` over this thread. Each thread has its own `ChatView`.
    find_open: bool,
    find_query: String,
    find_index: usize,
    /// Hit already scrolled into view. `None` scrolls on the next paint.
    find_pinned: Option<usize>,
    find_focus: FocusHandle,
    /// Tracks focus inside this thread so Escape can close find without
    /// taking the key from the terminal or the preview.
    chat_focus: FocusHandle,
    /// Mode to return to when leaving Plan via the Build/Plan chip.
    plan_return_mode: PermissionMode,
    pub(crate) git_dirty_count: usize,
    pub(crate) git_current_branch: Option<String>,
    git_branches: Vec<String>,
    branch_menu_open: bool,
    /// Worktree this thread runs in after the first send creates or reuses one.
    pub worktree_path: Option<String>,
    pub project_settings_root: Option<String>,
    /// Set once a send has a place, including a plain checkout.
    pub run_locked: bool,
    /// Effort on the last successful send. The chip shows `claude_effort`.
    pub effort: Option<String>,
    /// Shared prefs, copied from the workspace so the composer can draw them.
    pub run_choice: WorkspaceChoice,
    pub(crate) draft_placement: Option<DraftPlacement>,
    repo_placement_path: Option<String>,
    repo_placement_read: Option<Task<()>>,
    repo_placement: Option<WorkspaceChoice>,
    inherit_repo_placement: bool,
    pub claude_effort: Option<String>,
    /// Stored follow-up. Claude can steer; Codex still queues.
    pub follow_up_steer: bool,
    /// Claude `--autocompact`. `None` omits the flag.
    pub claude_autocompact: Option<String>,
    /// Idle compact for a Claude thread over the token floor.
    pub compact_idle: IdleCompact,
    /// What this thread's agent may do with other threads (Settings).
    pub thread_access: bruhs_models::thread_link::ThreadAccess,
    /// Models for the jobs that are not threads (Settings).
    pub task_models: bruhs_models::roles::TaskModels,
    /// Id of this thread's role: an agent started it with one, or the user
    /// set it with `/role`. Saved.
    pub role: Option<String>,
    /// Stamp of the role instructions this thread's session started with
    /// (`roles::instructions_stamp`). A session keeps those, so when they
    /// differ from the role's now, the next turn starts a new session. Saved.
    pub instructions_stamp: Option<String>,
    /// The stamp to keep once the running turn's session is announced.
    /// `None` while the turn resumes a session, which keeps what it holds.
    turn_stamp: Option<Option<String>>,
    /// That role as it runs in this thread's project, kept current by the
    /// workspace: its effort, access and instructions apply each turn.
    pub role_profile: Option<bruhs_models::roles::ResolvedRole>,
    /// This message sends in full. The return chip sets it. Not saved.
    compact_full: bool,
    pub previous_worktree: Option<RememberedWorktree>,
    /// Base branch for a new worktree. Not saved.
    pub base_branch: String,
    workspace_menu_open: bool,
    effort_menu_open: bool,
    /// A worktree is being created. The draft is already consumed.
    placing: bool,
    place_task: Option<Task<()>>,
    /// The project changed since git was last read; read it when the chat
    /// is next drawn (restoring many saved threads runs no git at all).
    git_info_stale: bool,
    /// Reading branches and status in the background.
    git_task: Option<Task<()>>,
    /// The project changed since workspace trust was last inspected.
    workspace_trust_stale: bool,
    workspace_trust: Option<bruhs_models::workspace_trust::WorkspaceTrustStatus>,
    workspace_trust_saving: bool,
    workspace_trust_error: Option<String>,
    workspace_trust_task: Option<Task<()>>,
    /// A branch switch in the background.
    checkout_task: Option<Task<()>>,
    mention_query: Option<String>,
    prompt_suggestion: bruhs_models::logic::prompt_suggestion::PromptSuggestion,
    suggestion_task: Option<Task<()>>,
    /// The pending (debounced) `@`-mention search.
    mention_task: Option<Task<()>>,
    mention_items: Vec<DesktopMentionItem>,
    composer_hint_selected_index: usize,
    composer_hint_scroll: ScrollHandle,
    composer_hints_suppressed_value: Option<String>,
    auth_status: DesktopAuthStatus,
    /// The Codex CLI login, once checked (web `CodexAuthSignal`).
    codex_auth: Option<DesktopAuthStatus>,
    initial_auth_pending: bool,
    pending_permission: Option<DesktopPermissionRequest>,
    question_prompt: Option<Entity<question_prompt::QuestionPrompt>>,
    question_response: Option<question_prompt::QuestionResponse>,
    pending_secret: Option<bruhs_models::secrets::SecretRequest>,
    secret_prompt: Option<Entity<secret_prompt::SecretPrompt>>,
    /// Settled turns whose fold the user opened (message index).
    expanded_folds: HashSet<usize>,
    /// Messages whose thinking block is open.
    expanded_thinking: HashSet<usize>,
    /// Web `Expanded`: open state of work groups, tool rows, changed-files
    /// cards, keyed by row.
    expanded_rows: HashMap<String, bool>,
    /// The chat column's painted width in CSS pixels; percentage widths inside the
    /// transcript's scroll container measure as indefinite, so the column is
    /// sized from this instead.
    measured_width: std::rc::Rc<std::cell::Cell<f32>>,
    /// Parsed assistant blocks and their SVG images, per message.
    content_blocks: content_cache::ContentBlockCache,
    response_media_bindings: std::collections::BTreeMap<String, String>,
    rich_embeds: std::rc::Rc<std::cell::RefCell<rich_response::RichEmbeds>>,
    pub model_picker_open: bool,
    model_picker_dropdown: Option<&'static str>,
    model_chip_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    pub(super) favorite_models: super::model_browser::Favorites,
    model_browser_tab: Option<String>,
    model_picker_opening_model: String,
    model_picker_expanded: std::collections::BTreeMap<String, bool>,
    model_picker_index: usize,
    model_picker_show_highlight: bool,
    /// Models each runtime's CLI listed for the account; refreshed on open.
    pub(crate) model_options: bruhs_models::delivery::generation::GenerationOptions,
    model_catalog_load: Option<Task<()>>,
    pub(crate) model_catalog: std::sync::Arc<bruhs_models::delivery::writer_catalog::WriterCatalog>,
    pub(crate) kiro_choices: kiro::KiroChoices,
    kiro_agents: Option<bruhs_core::agent_extensions::kiro::Agents>,
    kiro_agent_menu: bool,
    picker_acp_instances: HashMap<String, (String, Option<u32>)>,
    picker_availability:
        Result<bruhs_core::agent_extensions::availability::ProviderAvailability, String>,
    runtime_models: Vec<bruhs_models::runtime_models::Listing>,
    model_picker_scroll: ScrollHandle,
    model_option_scroll: ScrollHandle,
    model_option_focus: std::cell::RefCell<
        std::collections::BTreeMap<
            (bruhs_models::delivery::generation::OptionKind, usize),
            FocusHandle,
        >,
    >,
    model_picker_rail: FocusHandle,
    model_picker_query: Entity<InputState>,
    _model_picker_subscription: Subscription,
    /// The header's inline rename while it is open.
    title_rename: Option<title_rename::TitleRename>,
    pub input_state: Entity<TextareaState>,
    /// The running (or stopped but possibly still unwinding) turn's sender
    /// on the tokio runtime. A stopped turn keeps its aborted handle here so
    /// the next submit can wait for it to release the thread.
    agent_task: Option<JoinHandle<()>>,
    /// The current turn's event receiver. Replacing or dropping it cancels
    /// it, so a stopped turn's buffered events never reach a later turn.
    stream_task: Option<Task<()>>,
    /// Bumped whenever a turn starts or is abandoned; events tagged with an
    /// older generation are ignored.
    turn_generation: u64,
    pub(super) wait_on_quota: bool,
    response_stream: response_stream::Mode,
    paragraph_progress: response_stream::ParagraphProgress,
    _input_subscription: Subscription,
    _composer_key_subscription: Subscription,
    spinner_task: Option<Task<()>>,
    _registry_subscription: TaskRegistrySubscription,
    _tasks_count_task: Task<()>,
}

impl EventEmitter<ChatViewEvent> for ChatView {}

impl ChatView {
    pub fn is_working(&self) -> bool {
        self.is_streaming || self.running_tasks_count > 0
    }

    /// The turn streaming now, told apart from the thread's earlier and
    /// later turns, or `None` while none is.
    pub(crate) fn streaming_turn(&self) -> Option<u64> {
        self.is_streaming.then_some(self.turn_generation)
    }

    #[cfg(test)]
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        Self::new_with_project_path(current_dir_string(), window, cx)
    }

    #[cfg(test)]
    pub(crate) fn new_with_project_path(
        project_path: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        Self::new_with_model_and_project_path(
            DEFAULT_AGENT_MODEL.to_string(),
            project_path,
            window,
            cx,
        )
    }

    pub(crate) fn new_with_model_and_project_path(
        model_id: String,
        project_path: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let input_state =
            cx.new(|cx| composer_ui::prompt_input(window, cx, CHAT_INPUT_PLACEHOLDER));

        // Submit on Enter from the Input.
        let _input_subscription = cx.subscribe_in(
            &input_state,
            window,
            |this: &mut Self, _state, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.schedule_restart_checkpoint(cx);
                    this.clear_placed_prompt_notice(cx);
                    this.retarget_chips(cx);
                    this.sync_composer_hint_suppression(cx);
                    this.composer_hint_selected_index = 0;
                    this.composer_hint_scroll.set_offset(Point::default());
                    this.refresh_mention_hints(cx);
                    this.sync_prompt_suggestion(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { secondary, shift } => {
                    if this.dictation.active() {
                        return;
                    }
                    if *shift {
                        return;
                    }
                    if this.queue_edit.is_some() {
                        this.send_follow_up(*secondary, window, cx);
                        return;
                    }
                    if !*secondary && this.apply_selected_composer_hint(window, cx) {
                        return;
                    }
                    this.send_follow_up(*secondary, window, cx);
                }
                InputEvent::Focus | InputEvent::Blur => {
                    this.sync_prompt_suggestion(cx);
                    cx.notify();
                }
            },
        );
        let model_picker_query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search models…"));
        let _model_picker_subscription = cx.subscribe_in(
            &model_picker_query,
            window,
            |this: &mut Self, _state, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.model_picker_index = 0;
                    this.model_picker_show_highlight = false;
                    this.model_picker_scroll.set_offset(Point::default());
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.select_highlighted_model(window, cx);
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        );
        let chat = cx.weak_entity();
        let _composer_key_subscription = cx.intercept_keystrokes(move |event, window, cx| {
            let key = event.keystroke.key.as_str();
            let modifiers = event.keystroke.modifiers;
            if key == "v" && (modifiers.platform || modifiers.control) && !modifiers.alt {
                if let Some(chat) = chat.upgrade() {
                    let force_inline = modifiers.shift;
                    let handled = chat.update(cx, |this, cx| {
                        this.handle_composer_paste(force_inline, window, cx)
                    });
                    if handled {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }
                return;
            }
            if key == "tab" && !modifiers.platform && !modifiers.control && !modifiers.alt {
                if let Some(chat) = chat.upgrade() {
                    let handled = chat.update(cx, |this, cx| {
                        if !modifiers.shift && this.accept_prompt_suggestion(window, cx) {
                            return true;
                        }
                        if this.composer_options_open
                            && (this.composer_options_focus.contains_focused(window, cx)
                                || this.composer_options_trigger_focus.is_focused(window))
                        {
                            this.step_composer_options_focus(modifiers.shift, window, cx);
                            true
                        } else {
                            false
                        }
                    });
                    if handled {
                        window.prevent_default();
                        cx.stop_propagation();
                        return;
                    }
                }
            }
            if key == "escape" && !modifiers.modified() {
                if let Some(chat) = chat.upgrade() {
                    let closed = chat.update(cx, |this, cx| {
                        if this.dictation.active() && this.chat_focus.contains_focused(window, cx) {
                            this.cancel_dictation(cx);
                            return true;
                        }
                        if this.composer_options_open
                            && (this.composer_options_focus.contains_focused(window, cx)
                                || this.composer_options_trigger_focus.is_focused(window))
                        {
                            this.close_composer_options(window, cx);
                            true
                        } else {
                            false
                        }
                    });
                    if closed {
                        window.prevent_default();
                        cx.stop_propagation();
                        return;
                    }
                }
            }
            // Option/Alt+Up at the start of the draft edits the latest queued
            // message. Held anywhere else, it does not move the caret.
            let alt_up = key == "up"
                && modifiers.alt
                && !modifiers.platform
                && !modifiers.shift
                && !modifiers.control;
            if alt_up {
                let Some(chat) = chat.upgrade() else {
                    return;
                };
                let handled = chat.update(cx, |this, cx| {
                    if !this.input_state.focus_handle(cx).is_focused(window) {
                        return false;
                    }
                    if this.mention_query.is_some() || this.model_picker_open {
                        return false;
                    }
                    let range = this.input_state.read(cx).selected_range();
                    if range.start == 0 && range.end == 0 {
                        this.begin_queue_edit_latest(window, cx);
                    }
                    true
                });
                if handled {
                    window.prevent_default();
                    cx.stop_propagation();
                }
                return;
            }
            // ⌘1–9 jump to a row while the model picker is open.
            let picker_digit = if modifiers.platform && !modifiers.shift && !modifiers.alt {
                key.parse::<usize>().ok().filter(|n| (1..=9).contains(n))
            } else {
                None
            };
            let plain_nav =
                !modifiers.modified() && matches!(key, "up" | "down" | "escape" | "enter");
            let provider_cycle = modifiers.platform
                && modifiers.shift
                && !modifiers.alt
                && !modifiers.control
                && matches!(key, "up" | "down");
            let focus_provider_rail = key == "tab"
                && modifiers.shift
                && !modifiers.platform
                && !modifiers.alt
                && !modifiers.control;
            if !modifiers.platform
                && !modifiers.alt
                && !modifiers.control
                && !modifiers.shift
                && matches!(key, "backspace" | "delete")
            {
                if let Some(chat) = chat.upgrade() {
                    let handled = chat.update(cx, |this, cx| {
                        this.remove_chip_at_caret(key == "backspace", window, cx)
                    });
                    if handled {
                        window.prevent_default();
                        cx.stop_propagation();
                        return;
                    }
                }
            }
            if !modifiers.platform && !modifiers.alt && !modifiers.control {
                if let Some(composer_key) = streaming::desktop_composer_key(key, modifiers.shift) {
                    if let Some(chat) = chat.upgrade() {
                        let handled = chat.update(cx, |this, cx| {
                            this.apply_structure_key(composer_key, window, cx)
                        });
                        if handled {
                            window.prevent_default();
                            cx.stop_propagation();
                            return;
                        }
                    }
                }
            }
            if !plain_nav && picker_digit.is_none() && !provider_cycle && !focus_provider_rail {
                return;
            }
            let Some(chat) = chat.upgrade() else {
                return;
            };
            chat.update(cx, |this, cx| {
                if this.model_picker_open {
                    let search_focused =
                        this.model_picker_query.focus_handle(cx).is_focused(window);
                    let handled = if provider_cycle {
                        this.cycle_model_picker_provider(key == "down", window, cx);
                        true
                    } else if focus_provider_rail
                        && search_focused
                        && this.model_picker_browsing(cx)
                    {
                        this.model_picker_rail.focus(window, cx);
                        true
                    } else {
                        this.model_picker_dropdown.is_none()
                            && (key == "escape"
                                || picker_digit.is_some()
                                || (plain_nav && search_focused))
                            && this.handle_model_picker_key(key, picker_digit, window, cx)
                    };
                    if handled {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                    return;
                }
                if plain_nav
                    && matches!(key, "up" | "down")
                    && !this.composer_hint_popover_open(cx)
                    && this.recall_history(key == "up", window, cx)
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    return;
                }
                if plain_nav && key != "enter" && this.handle_composer_hint_key(key, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                // Escape closes find while focus is in this thread.
                if this.find_open
                    && key == "escape"
                    && !modifiers.modified()
                    && this.chat_focus.contains_focused(window, cx)
                {
                    this.find_open = false;
                    this.find_pinned = None;
                    cx.notify();
                    cx.stop_propagation();
                }
            });
        });
        let app = Arc::clone(&cx.global::<DesktopState>().app);
        let (registry_subscription, task_events) =
            TaskRegistrySubscription::new(app.registry().clone());
        let _tasks_count_task = cx.spawn(async move |chat, cx| {
            while task_events.recv().await.is_ok() {
                if chat
                    .update(cx, |chat, cx| {
                        let running_tasks: Vec<bruhs_core::BackgroundTask> = chat
                            .thread_id
                            .as_deref()
                            .map_or_else(Vec::new, |thread_id| {
                                let mut rows: Vec<_> = app
                                    .registry()
                                    .get_by_thread(thread_id)
                                    .into_iter()
                                    .filter(|task| task.status == TaskStatus::Running)
                                    .collect();
                                rows.sort_by_key(|task| task.started_at);
                                rows
                            });
                        let running_tasks_count = running_tasks.len();
                        let count_changed = chat.running_tasks_count != running_tasks_count;
                        let list_changed = count_changed
                            || chat.running_tasks.len() != running_tasks.len()
                            || chat.running_tasks.iter().zip(&running_tasks).any(|(a, b)| {
                                a.id != b.id || a.label != b.label || a.kind != b.kind
                            });
                        if list_changed {
                            chat.running_tasks = running_tasks;
                            chat.running_tasks_count = running_tasks_count;
                            if count_changed {
                                cx.emit(ChatViewEvent::RunningTasksChanged);
                            }
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let mut view = Self {
            thread_id: None,
            thread_title: "New thread".to_string(),
            messages: Vec::new(),
            messages_viewport: {
                // Match the native meeting transcript's 128px overdraw budget.
                let state = ListState::new(0, ListAlignment::Top, px(128.0));
                state.set_follow_mode(FollowMode::Tail);
                state
            },
            messages_zoom: std::cell::Cell::new(bruhs_ui::zoom::zoom()),
            messages_focus: cx.focus_handle(),
            composer_area_focus: cx.focus_handle().tab_stop(false),
            rendered_messages: std::cell::Cell::new(None),
            #[cfg(test)]
            rendered_message_rows: std::cell::Cell::new(0),
            is_streaming: false,
            spinner_frame: 0,
            stream_started_at: None,
            tasks_panel_visible: false,
            diff_panel_visible: false,
            details_trigger: cx.focus_handle().tab_stop(true),
            details_trigger_bounds: Default::default(),
            details_open: Default::default(),
            details_inline_open: false,
            restart_turn: None,
            restart_checkpoint: None,
            reported_limit: None,
            running_tasks_count: 0,
            running_tasks: Vec::new(),
            permission_mode: PermissionMode::Default,
            project_permission_override: false,
            text_generation_model:
                bruhs_models::logic::prompt_suggestion::default_suggestion_model(
                    AgentProvider::for_model(&model_id),
                )
                .to_owned(),
            model_id,
            project_path,
            project_scoped: false,
            project_name: String::new(),
            right_panel_open: false,
            composer_options_open: false,
            composer_options_focus: cx.focus_handle().tab_index(0).tab_stop(false),
            composer_effort_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            composer_options_end_focus: cx.focus_handle().tab_index(999).tab_stop(true),
            composer_options_trigger_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            composer_options_trigger_bounds: Default::default(),
            composer_options_scroll: ScrollHandle::new(),
            composer_options_reveal: Default::default(),
            copy_feedback: Default::default(),
            turn_actions_focus: Default::default(),
            composer_options_section_focus: std::array::from_fn(|_| {
                cx.focus_handle().tab_stop(false)
            }),
            stash_open: false,
            stash: Vec::new(),
            prompt_queue: PromptQueue::default(),
            queue_held: false,
            queue_edit: None,
            linked_draft: None,
            pending_composer: None,
            dictation: Default::default(),
            dictation_prefs: Default::default(),
            worktree_prefs: Default::default(),
            branch_naming: Default::default(),
            setup_actions: Vec::new(),
            effort_label: true,
            focus_pending_composer: false,
            recall: None,
            rewind_confirm: None,
            fresh_session: false,
            session_id: None,
            session_provider: None,
            import_context: None,
            import_key: None,
            fork: None,
            turn_provider: AgentProvider::Claude,
            handoff_pending: false,
            goal: None,
            text_attachments: Vec::new(),
            file_offers: Vec::new(),
            context_chips: Vec::new(),
            chip_draft: String::new(),
            chip_guard: false,
            pull_query: None,
            pull_items: Vec::new(),
            pull_error: None,
            pull_task: None,
            skill_query: None,
            skill_items: Vec::new(),
            skill_error: None,
            skill_task: None,
            thread_hits: Vec::new(),
            queue_prior_chips: Vec::new(),
            pending_chips: None,
            find_open: false,
            find_query: String::new(),
            find_index: 0,
            find_pinned: None,
            find_focus: cx.focus_handle(),
            chat_focus: cx.focus_handle().tab_stop(false),
            plan_return_mode: PermissionMode::Default,
            git_dirty_count: 0,
            git_current_branch: None,
            git_branches: Vec::new(),
            branch_menu_open: false,
            worktree_path: None,
            project_settings_root: None,
            run_locked: false,
            effort: None,
            run_choice: WorkspaceChoice::Checkout,
            draft_placement: None,
            repo_placement_path: None,
            repo_placement_read: None,
            repo_placement: None,
            inherit_repo_placement: false,
            claude_effort: None,
            readout: Default::default(),
            send_shortcut: SendShortcut::default(),
            slash_skills: true,
            tool_visibility: Default::default(),
            chat_width: ChatWidth::default(),
            follow_up_steer: false,
            claude_autocompact: None,
            compact_idle: IdleCompact::Return,
            thread_access: bruhs_models::thread_link::ThreadAccess::default(),
            task_models: bruhs_models::roles::TaskModels::default(),
            role: None,
            instructions_stamp: None,
            turn_stamp: None,
            role_profile: None,
            compact_full: false,
            previous_worktree: None,
            base_branch: String::new(),
            workspace_menu_open: false,
            effort_menu_open: false,
            placing: false,
            place_task: None,
            git_info_stale: false,
            git_task: None,
            workspace_trust_stale: false,
            workspace_trust: None,
            workspace_trust_saving: false,
            workspace_trust_error: None,
            workspace_trust_task: None,
            checkout_task: None,
            mention_query: None,
            prompt_suggestion: Default::default(),
            suggestion_task: None,
            mention_task: None,
            mention_items: Vec::new(),
            composer_hint_selected_index: 0,
            composer_hint_scroll: ScrollHandle::new(),
            composer_hints_suppressed_value: None,
            auth_status: DesktopAuthStatus::default(),
            codex_auth: None,
            initial_auth_pending: false,
            pending_permission: None,
            question_prompt: None,
            question_response: None,
            pending_secret: None,
            secret_prompt: None,
            expanded_folds: HashSet::new(),
            expanded_thinking: HashSet::new(),
            expanded_rows: HashMap::new(),
            measured_width: std::rc::Rc::new(std::cell::Cell::new(0.0)),
            content_blocks: content_cache::ContentBlockCache::default(),
            response_media_bindings: Default::default(),
            rich_embeds: Default::default(),
            model_picker_open: false,
            model_picker_dropdown: None,
            model_chip_bounds: Default::default(),
            favorite_models: Default::default(),
            model_browser_tab: None,
            model_picker_opening_model: String::new(),
            model_picker_expanded: Default::default(),
            model_picker_index: 0,
            model_picker_show_highlight: false,
            model_options: Default::default(),
            model_catalog: Default::default(),
            model_catalog_load: None,
            kiro_choices: Default::default(),
            kiro_agents: None,
            kiro_agent_menu: false,
            picker_acp_instances: HashMap::new(),
            picker_availability: Ok(Default::default()),
            runtime_models: Vec::new(),
            model_picker_scroll: ScrollHandle::new(),
            model_option_scroll: ScrollHandle::new(),
            model_option_focus: Default::default(),
            model_picker_rail: cx.focus_handle(),
            model_picker_query,
            _model_picker_subscription,
            title_rename: None,
            input_state,
            agent_task: None,
            stream_task: None,
            turn_generation: 0,
            wait_on_quota: false,
            response_stream: Default::default(),
            paragraph_progress: Default::default(),
            _input_subscription,
            _composer_key_subscription,
            spinner_task: None,
            _registry_subscription: registry_subscription,
            _tasks_count_task,
        };
        view.refresh_kiro_choices();
        view.start_initial_claude_auth_check(cx);
        if AgentProvider::for_model(&view.model_id) == AgentProvider::Codex {
            view.start_initial_codex_auth_check(cx);
        }
        view
    }

    #[cfg(test)]
    pub(crate) fn run_slash_command_for_test(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.run_slash_command(command, window, cx);
    }

    /// Fold and thinking open state is keyed by message index; drop it
    /// whenever messages are removed so a later message at the same index
    /// does not open expanded.
    fn forget_message_expansions(&mut self) {
        self.expanded_folds.clear();
        self.expanded_thinking.clear();
    }

    /// A host receipt can arrive after a newer interactive turn. Keep each
    /// prompt beside its answer and discard index-based caches after an insertion.
    pub(crate) fn insert_scheduled_exchange(&mut self, request: Message, answer: Message) {
        let end = answer.completed_at_ms.unwrap_or_default();
        let later = self.messages.iter().position(|m| {
            m.role == MessageRole::Assistant && m.started_at_ms.is_some_and(|t| t > end)
        });
        if let Some(assistant) = later {
            let index = assistant
                .checked_sub(1)
                .filter(|i| self.messages[*i].role == MessageRole::User)
                .unwrap_or(assistant);
            self.messages.splice(index..index, [request, answer]);
            self.content_blocks.truncate(index);
            self.rich_embeds.borrow_mut().truncate(index);
            self.forget_message_expansions();
            self.expanded_rows.clear();
            if self.messages_viewport.item_count() == self.messages.len() - 2 {
                self.messages_viewport.splice(index..index, 2);
            }
        } else {
            self.messages.extend([request, answer]);
        }
    }

    pub fn start_thread(
        &mut self,
        id: impl Into<String>,
        title: impl Into<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.readout.status("");
        self.cancel_dictation(cx);
        self.dismiss_prompt_suggestion(cx);
        self.thread_id = Some(id.into());
        self.thread_title = title.into();
        self.messages.clear();
        self.response_media_bindings.clear();
        self.rich_embeds.borrow_mut().clear();
        self.messages_viewport.reset(0);
        self.messages_viewport.set_follow_mode(FollowMode::Tail);
        self.prompt_queue = PromptQueue::default();
        self.queue_held = false;
        self.queue_edit = None;
        self.linked_draft = None;
        self.pending_composer = None;
        self.focus_pending_composer = false;
        self.recall = None;
        self.rewind_confirm = None;
        self.fresh_session = false;
        self.session_id = None;
        self.session_provider = None;
        self.import_context = None;
        self.import_key = None;
        self.fork = None;
        self.goal = None;
        self.worktree_path = None;
        self.project_settings_root = None;
        self.run_locked = false;
        self.draft_placement = None;
        self.repo_placement_path = None;
        self.repo_placement = None;
        self.inherit_repo_placement = false;
        self.effort = None;
        self.placing = false;
        self.place_task = None;
        self.workspace_menu_open = false;
        self.effort_menu_open = false;
        self.composer_options_open = false;
        self.stash_open = false;
        self.text_attachments.clear();
        self.file_offers.clear();
        self.context_chips.clear();
        self.chip_guard = false;
        self.chip_draft = self.input_state.read(cx).value().to_string();
        self.pull_query = None;
        self.pull_items.clear();
        self.pull_error = None;
        self.pull_task = None;
        self.skill_query = None;
        self.skill_items.clear();
        self.skill_error = None;
        self.skill_task = None;
        self.thread_hits.clear();
        self.queue_prior_chips.clear();
        self.pending_chips = None;
        self.forget_message_expansions();
        self.content_blocks.clear();
        self.is_streaming = false;
        self.spinner_frame = 0;
        self.spinner_task.take();
        self.stream_started_at = None;
        self.pending_permission = None;
        self.question_prompt = None;
        self.clear_mention_hints();
        self.abandon_turn();
        self.running_tasks = self
            .thread_id
            .as_deref()
            .map_or_else(Vec::new, |thread_id| {
                let mut rows: Vec<_> = cx
                    .global::<DesktopState>()
                    .app
                    .registry()
                    .get_by_thread(thread_id)
                    .into_iter()
                    .filter(|task| task.status == TaskStatus::Running)
                    .collect();
                rows.sort_by_key(|task| task.started_at);
                rows
            });
        self.running_tasks_count = self.running_tasks.len();
        cx.notify();
    }

    pub fn set_model(&mut self, model_id: impl Into<String>, cx: &mut Context<'_, Self>) {
        self.dismiss_prompt_suggestion(cx);
        let model_id = model_id.into();
        if model_id != self.model_id {
            self.model_options = Default::default();
        }
        self.model_id = model_id;
        self.refresh_kiro_choices();
        if !self.effort_chip_visible() {
            self.effort_menu_open = false;
        }
        if self.is_claude_model() {
            self.start_claude_auth_check(cx);
        } else if AgentProvider::for_model(&self.model_id) == AgentProvider::Codex {
            self.start_codex_auth_check(cx);
        }
        cx.notify();
    }

    /// The place is fixed after the first successful send.
    pub(super) fn thread_has_place(&self) -> bool {
        self.run_locked || self.worktree_path.is_some()
    }

    /// Model-specific reasoning settings available on this turn.
    pub(super) fn effort_chip_visible(&self) -> bool {
        !self.kiro_efforts().is_empty()
            || self.native_model().is_some_and(|m| !m.options.is_empty())
    }

    /// A draft selection survives shared preference updates and reopening the workspace.
    pub(crate) fn resolve_draft_placement(
        &mut self,
        default: Option<WorkspaceChoice>,
        cx: &mut Context<'_, Self>,
    ) -> WorkspaceChoice {
        let explicit = self
            .draft_placement
            .as_ref()
            .map(|draft| draft.choice)
            .or(default);
        self.inherit_repo_placement = explicit.is_none() && self.project_scoped;
        if self.inherit_repo_placement {
            let scope = self.settings_project_path();
            if self.repo_placement_path.as_deref() != Some(scope) {
                self.repo_placement = None;
                self.refresh_repo_placement(cx);
            }
        } else {
            self.repo_placement_path = None;
            self.repo_placement = None;
        }
        explicit.or(self.repo_placement).unwrap_or_default()
    }

    fn refresh_repo_placement(&mut self, cx: &mut Context<'_, Self>) {
        if !self.inherit_repo_placement {
            return;
        }
        let scope = self.settings_project_path().to_owned();
        self.repo_placement_path = Some(scope.clone());
        let path = PathBuf::from(&scope);
        let read = cx.background_spawn(async move {
            bruhs_core::git::project_checkout_defaults(&path).placement
        });
        self.repo_placement_read = Some(cx.spawn(async move |this, cx| {
            let value = read.await;
            let _ = this.update(cx, |chat, cx| chat.update_repo_placement(&scope, value, cx));
        }));
    }

    pub(crate) fn update_repo_placement(
        &mut self,
        scope: &str,
        value: Option<WorkspaceChoice>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.inherit_repo_placement && self.settings_project_path() == scope {
            self.repo_placement_read = None;
            self.repo_placement = value;
            self.run_choice = value.unwrap_or_default();
            cx.notify();
        }
    }

    pub(super) fn apply_run_prefs(
        &mut self,
        choice: WorkspaceChoice,
        effort: Option<String>,
        previous: Option<RememberedWorktree>,
        base_branch: String,
        composer: ComposerPrefs,
        cx: &mut Context<'_, Self>,
    ) {
        self.run_choice = choice;
        self.claude_effort = effort;
        self.previous_worktree = self
            .draft_placement
            .as_ref()
            .and_then(|draft| draft.previous.clone())
            .or(previous);
        self.base_branch = base_branch;
        self.follow_up_steer = composer.follow_up_steer;
        self.claude_autocompact = composer.autocompact;
        self.compact_idle = composer.compact_idle;
        self.thread_access = composer.thread_access;
        if self.task_models != composer.task_models {
            self.dismiss_prompt_suggestion(cx);
        }
        self.task_models = composer.task_models;
        if !self.effort_chip_visible() {
            self.effort_menu_open = false;
        }
        cx.notify();
    }

    pub(super) fn composer_prefs(&self) -> ComposerPrefs {
        ComposerPrefs {
            follow_up_steer: self.follow_up_steer,
            autocompact: self.claude_autocompact.clone(),
            compact_idle: self.compact_idle,
            thread_access: self.thread_access,
            task_models: self.task_models.clone(),
        }
    }

    pub(super) fn emit_run_prefs(&mut self, cx: &mut Context<'_, Self>) {
        cx.emit(ChatViewEvent::RunPrefs {
            effort: self.claude_effort.clone(),
            base_branch: self.base_branch.clone(),
            composer: self.composer_prefs(),
        });
    }

    #[cfg(test)]
    pub(crate) fn force_unauthenticated_for_test(&mut self, cx: &mut Context<'_, Self>) {
        self.auth_status = DesktopAuthStatus::default();
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn auth_banner_visible_for_test(&self) -> bool {
        auth_banner_visible_like_web(&self.model_id, self.auth_status.is_authenticated())
    }

    /// The agent is waiting on an approval (the sidebar's durian dot).
    pub fn has_pending_permission(&self) -> bool {
        self.pending_permission.is_some()
    }

    /// Unsaved drafts/stashes must not disappear when a workflow is undone.
    pub(crate) fn has_content_or_pending_work(&self, cx: &App) -> bool {
        !self.messages.is_empty()
            || self.is_streaming
            || self.placing
            || self.running_tasks_count > 0
            || self.pending_permission.is_some()
            || !self.stash.is_empty()
            || !self.prompt_queue.is_empty()
            || !self.input_state.read(cx).value().is_empty()
    }

    pub fn set_project_context(
        &mut self,
        project_path: impl Into<String>,
        project_name: impl Into<String>,
        project_scoped: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.project_path = project_path.into();
        self.project_name = project_name.into();
        self.project_scoped = project_scoped;
        if project_scoped {
            self.git_info_stale = true;
            self.workspace_trust_stale = true;
        } else {
            self.clear_git_info();
            self.clear_workspace_trust();
        }
        self.refresh_mention_hints(cx);
        cx.notify();
    }

    pub(crate) fn scratch_folder(&self) -> Option<&std::path::Path> {
        self.worktree_path
            .as_deref()
            .map(std::path::Path::new)
            .filter(|path| {
                !self.project_scoped && bruhs_core::worktrees::scratch::is_managed_path(path)
            })
    }

    /// Original project of a thread placed on a managed worktree.
    fn provider_enabled_for_chat(&self, provider: AgentProvider) -> bool {
        bruhs_core::agent_extensions::availability::require_enabled_for_project(
            provider,
            self.project_path.as_ref(),
        )
        .is_ok()
    }

    pub(crate) fn settings_project_path(&self) -> &str {
        self.project_settings_root
            .as_deref()
            .unwrap_or(&self.project_path)
    }

    /// Keep this conversation while changing the folder its next turn can use.
    pub fn change_project(
        &mut self,
        path: String,
        name: String,
        scoped: bool,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), &'static str> {
        if self.is_streaming
            || self.placing
            || self.running_tasks_count > 0
            || self.has_pending_permission()
        {
            return Err("Wait for this thread to finish before changing its project.");
        }
        if self.project_path != path || self.project_scoped != scoped {
            self.import_context = self
                .conversation_context(&self.messages)
                .or_else(|| self.import_context.take());
            self.session_id = None;
            self.session_provider = None;
            self.fresh_session = true;
            self.instructions_stamp = None;
            self.worktree_path = None;
            self.project_settings_root = None;
            self.run_locked = false;
            self.draft_placement = None;
            self.repo_placement_path = None;
            self.repo_placement = None;
            self.inherit_repo_placement = false;
            self.run_choice = WorkspaceChoice::Checkout;
            self.previous_worktree = None;
            self.base_branch.clear();
            self.context_chips.clear();
            self.clear_git_info();
        }
        self.set_project_context(path, name, scoped, cx);
        Ok(())
    }

    pub fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !composer_focus_allowed_like_web(self.is_streaming) {
            return;
        }

        self.input_state.update(cx, |input, cx| {
            input.focus(window, cx);
        });
    }

    pub fn append_composer_draft(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let mut draft = self.input_state.read(cx).value().to_string();
        if !draft.is_empty() {
            draft.push_str("\n\n");
        }
        draft.push_str(text);
        self.set_composer_draft(draft, window, cx);
    }

    pub fn set_composer_draft(
        &mut self,
        draft: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let cursor = draft.len();
        self.input_state.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
            input.set_selected_range(cursor..cursor, cx);
            input.focus(window, cx);
        });
        self.composer_hints_suppressed_value = None;
        self.refresh_mention_hints(cx);
        cx.notify();
    }

    pub(super) fn set_response_stream(&mut self, mode: response_stream::Mode) {
        if self.response_stream != mode {
            self.response_stream = mode;
            self.refresh_paragraph_progress(true);
            self.messages_viewport.remeasure();
        }
    }

    fn refresh_paragraph_progress(&mut self, reset: bool) {
        if reset {
            self.paragraph_progress = Default::default();
        }
        if self.response_stream == response_stream::Mode::Paragraph {
            if let Some(message) = self
                .messages
                .last()
                .filter(|m| m.role == MessageRole::Assistant && m.state.is_streaming())
            {
                self.paragraph_progress.advance(&message.content);
            }
        }
    }

    pub(super) fn set_render_math(&mut self, enabled: bool) {
        if self.content_blocks.set_render_math(enabled) {
            self.messages_viewport.remeasure();
        }
    }

    pub(super) fn set_chat_width(&mut self, width: ChatWidth) {
        if self.chat_width != width {
            self.chat_width = width;
            self.messages_viewport.remeasure();
        }
    }

    fn transcript_column_width(&self) -> f32 {
        let available = self.measured_width.get();
        let limit = self.chat_width.limit();
        if available <= 0.0 {
            limit.unwrap_or(CHAT_MESSAGE_LIST_MAX_WIDTH_PX)
        } else {
            available.min(limit.unwrap_or(available))
        }
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE=transcript`: the web's fixture thread so
    /// the two transcripts can be compared side by side.
    #[cfg(debug_assertions)]
    pub fn smoke_seed_transcript(&mut self, cx: &mut Context<'_, Self>) {
        let root = "/Users/me/code/bruhs-ai";
        self.set_project_context(root, "bruhs-ai", true, cx);
        self.thread_title = "Add --json flag to the CLI".to_string();
        self.messages = timeline_ui::transcript_fixture_messages(root);
        self.forget_message_expansions();
        self.expanded_rows.clear();
        self.is_streaming = true;
        self.stream_started_at = Some(Instant::now());
        self.scroll_messages_to_bottom();
        cx.notify();
    }

    /// Command palette "Stash prompt": same as ⌘S in the composer.
    pub fn stash_current_draft(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.stash_draft(&StashDraft, window, cx);
    }

    pub fn scroll_messages_to_bottom(&self) {
        self.messages_viewport.set_follow_mode(FollowMode::Tail);
    }

    fn open_find(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.find_open = true;
        cx.notify();
        cx.on_next_frame(window, |this, window, cx| {
            this.find_focus.focus(window, cx);
        });
    }

    fn step_find(&mut self, forward: bool, cx: &mut Context<'_, Self>) {
        let len = self.thread_hits().len();
        self.find_index = cycle(len, self.find_index, forward);
        self.find_pinned = None;
        cx.notify();
    }

    fn thread_hits(&self) -> Vec<bruhs_models::logic::find::TextHit> {
        if self.find_query.is_empty() {
            return Vec::new();
        }
        let owned: Vec<(String, String)> = self
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| (index.to_string(), searchable_text(message)))
            .collect();
        let pairs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(id, text)| (id.as_str(), text.as_str()))
            .collect();
        search_texts(&self.find_query, &pairs)
    }

    /// `(hit index, message index)` of the match the bar is on.
    fn current_find(&self) -> Option<(usize, usize)> {
        let hits = self.thread_hits();
        if hits.is_empty() {
            return None;
        }
        let hit_index = self.find_index % hits.len();
        let message_index = hits[hit_index].source_id.parse().ok()?;
        Some((hit_index, message_index))
    }

    fn on_find_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        if key == "escape" {
            window.prevent_default();
            cx.stop_propagation();
            self.find_open = false;
            self.find_pinned = None;
            cx.notify();
            return;
        }
        if key == "enter" {
            window.prevent_default();
            cx.stop_propagation();
            self.step_find(!modifiers.shift, cx);
            return;
        }
        if key == "backspace" {
            window.prevent_default();
            cx.stop_propagation();
            self.find_query.pop();
            self.find_index = 0;
            self.find_pinned = None;
            cx.notify();
            return;
        }
        if let Some(text) = event.keystroke.key_char.as_deref() {
            if !text.is_empty() && text.chars().all(|ch| !ch.is_control()) {
                window.prevent_default();
                cx.stop_propagation();
                self.find_query.push_str(text);
                self.find_index = 0;
                self.find_pinned = None;
                cx.notify();
            }
        }
    }

    fn render_find_bar(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let hits = self.thread_hits();
        let count_label = if self.find_query.is_empty() {
            String::new()
        } else if hits.is_empty() {
            "No matches".to_string()
        } else {
            format!("{} of {}", self.find_index % hits.len() + 1, hits.len())
        };
        let step_button = |label: &'static str, forward: bool, cx: &mut Context<'_, Self>| {
            div()
                .id(if forward {
                    "desktop-find-next"
                } else {
                    "desktop-find-prev"
                })
                .size(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .text_size(px(12.0))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_400())
                .cursor_pointer()
                .hover(|el| {
                    el.bg(theme::longan_800().opacity(0.6))
                        .text_color(theme::rambutan_100())
                })
                .tab_index(0)
                .role(Role::Button)
                .aria_label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.step_find(forward, cx);
                }))
                .child(if forward { "↓" } else { "↑" })
        };
        div()
            .id("desktop-find-bar")
            .track_focus(&self.find_focus)
            .h(px(36.0))
            .w_full()
            .flex_shrink_0()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .bg(theme::longan_900())
            .border_b_1()
            .border_color(theme::longan_800().opacity(0.6))
            .tab_index(0)
            .aria_label("Find in thread")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.find_focus.focus(window, cx);
                }),
            )
            .on_key_down(cx.listener(Self::on_find_key))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(14.0))
                    .font_family(crate::theme::FontFace::Code)
                    .text_color(if self.find_query.is_empty() {
                        theme::rambutan_500()
                    } else {
                        theme::rambutan_100()
                    })
                    .child(if self.find_query.is_empty() {
                        "Find in thread".to_string()
                    } else {
                        self.find_query.clone()
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .font_features(theme::tabular_nums())
                    .text_color(theme::rambutan_400())
                    .child(count_label),
            )
            .child(step_button("Previous match", false, cx))
            .child(step_button("Next match", true, cx))
            .child(
                div()
                    .id("desktop-find-close")
                    .size(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(0.6))
                            .text_color(theme::rambutan_100())
                    })
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Close find")
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.find_open = false;
                        this.find_pinned = None;
                        cx.notify();
                    }))
                    .child(
                        svg()
                            .path(assets::CLOSE_ICON_PATH)
                            .size(px(14.0))
                            .text_color(theme::rambutan_400()),
                    ),
            )
    }
}

/// Text `Cmd+F` searches: the message, its thinking, a failed turn's error,
/// and the string values of each tool call.
fn searchable_text(message: &Message) -> String {
    let mut out = String::new();
    push_block(&mut out, &message.content);
    push_block(&mut out, &message.thinking);
    if let TurnState::Failed { error } = &message.state {
        push_block(&mut out, error);
    }
    for call in &message.tool_calls {
        push_block(&mut out, &call.tool);
        push_json_strings(&mut out, &call.input);
        if let Some(output) = &call.output {
            push_block(&mut out, output);
        }
    }
    out
}

fn push_block(out: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(text);
}

fn push_json_strings(out: &mut String, value: &serde_json::Value) {
    match value {
        serde_json::Value::String(text) => push_block(out, text),
        serde_json::Value::Array(items) => {
            for item in items {
                push_json_strings(out, item);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                push_json_strings(out, item);
            }
        }
        _ => {}
    }
}

impl Drop for ChatView {
    fn drop(&mut self) {
        self.dictation.cancel();
        if let Some(task) = self.agent_task.take() {
            task.abort();
        }
    }
}

fn thread_title_after_submit_like_web(current_title: &str, _prompt: &str) -> String {
    current_title.to_string()
}

fn composer_input_disabled_like_web(_is_streaming: bool) -> bool {
    false
}

fn composer_input_tab_index_like_web(_is_streaming: bool) -> isize {
    0
}

fn composer_focus_allowed_like_web(_is_streaming: bool) -> bool {
    true
}

/// Wall time of a finished turn, when both stamps exist.
pub fn turn_duration_ms(msg: &Message) -> Option<u64> {
    match (msg.started_at_ms, msg.completed_at_ms) {
        (Some(start), Some(end)) if end > start => u64::try_from(end - start).ok(),
        _ => msg.duration_ms,
    }
}

/// Web `format::format_duration_ms`: `0.9s`, `12s`, `1m 12s`, `1h 2m`.
pub fn fold_duration_label(ms: u64) -> String {
    let secs = ms.to_f64_lossy() / 1000.0;
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else if secs < 60.0 {
        format!("{}s", secs.round().to_u64_saturating())
    } else if secs < 3600.0 {
        let m = (secs / 60.0).floor().to_u64_saturating();
        let s = (secs % 60.0).round().to_u64_saturating();
        format!("{m}m {s}s")
    } else {
        let h = (secs / 3600.0).floor().to_u64_saturating();
        let m = ((secs % 3600.0) / 60.0).floor().to_u64_saturating();
        format!("{h}h {m}m")
    }
}

/// Label of a settled turn's fold, matching bruhs-web's `fold_settled_turn`.
pub fn turn_fold_label(duration_ms: Option<u64>, stopped: bool) -> String {
    match (stopped, duration_ms) {
        (true, Some(d)) => format!("You stopped after {}", fold_duration_label(d)),
        (true, None) => "You stopped".to_string(),
        (false, Some(d)) => format!("Worked for {}", fold_duration_label(d)),
        (false, None) => "Worked".to_string(),
    }
}

/// How a login waits: the login process runs while the auth status is
/// polled, and whichever shows success first ends the wait.
#[derive(Clone, Copy, Debug)]
struct LoginTiming {
    poll_interval: Duration,
    poll_attempts: usize,
    login_timeout: Duration,
}

#[cfg(test)]
fn current_dir_string() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .unwrap_or_else(|| ".".to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopMermaidRange {
    start: usize,
    end: usize,
    source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopMathRange {
    start: usize,
    end: usize,
    source: String,
    display: bool,
}

fn element_key(value: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

impl ChatView {
    fn render_active_runs_strip(&self, cx: &Context<'_, Self>) -> AnyElement {
        let count = self.running_tasks.len();
        let summary = if count == 1 {
            match self.running_tasks[0].kind {
                bruhs_core::TaskKind::Swarm => "1 teamwork worker running".to_string(),
                bruhs_core::TaskKind::Subagent => "1 subagent running".to_string(),
                bruhs_core::TaskKind::BackgroundAgent => "1 background agent running".to_string(),
                bruhs_core::TaskKind::Shell => "1 background run".to_string(),
            }
        } else {
            format!("{count} active runs")
        };
        let dot = div()
            .size(px(7.0))
            .rounded_full()
            .flex_shrink_0()
            .bg(theme::blueberry_400());
        let indicator: AnyElement = if !crate::motion::reduce_motion(cx) {
            dot.with_animation(
                "desktop-active-runs-pulse",
                Animation::new(Duration::from_millis(1600)).repeat(),
                |el, delta| {
                    let opacity = if delta < 0.5 {
                        1.0 - delta * 1.1
                    } else {
                        0.45 + (delta - 0.5) * 1.1
                    };
                    el.opacity(opacity.clamp(0.45, 1.0))
                },
            )
            .into_any_element()
        } else {
            dot.into_any_element()
        };
        let visible = self.running_tasks.iter().take(4);
        let overflow = count.saturating_sub(4);
        div()
            .debug_selector(|| "desktop-chat-active-runs".to_string())
            .w_full()
            .px(px(CHAT_COMPOSER_PAGE_X_PADDING_PX))
            .pb_2()
            .flex_shrink_0()
            .child(
                div()
                    .w_full()
                    .mx_auto()
                    .when_some(self.chat_width.limit(), |el, limit| el.max_w(px(limit)))
                    .rounded(px(10.0))
                    .bg(theme::longan_900())
                    .border_1()
                    .border_color(theme::ring_panel())
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(indicator)
                                    .child(
                                        div()
                                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                                            .text_size(px(12.0))
                                            .text_color(theme::rambutan_200())
                                            .child(format!("Working · {summary}")),
                                    ),
                            )
                            .child(
                                web_button(
                                    "desktop-active-runs-open-tasks",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Open tasks panel")
                                .on_click(cx.listener(|_this, _, _, cx| {
                                    cx.emit(ChatViewEvent::ToggleTasks);
                                }))
                                .child("Tasks"),
                            ),
                    )
                    .children(visible.map(|task| {
                        let stop_id = task.id.clone();
                        let row_key = element_key(&task.id);
                        let (bg, fg) = match task.kind {
                            bruhs_core::TaskKind::Swarm => {
                                (theme::blueberry_400().opacity(0.16), theme::blueberry_400())
                            }
                            bruhs_core::TaskKind::Subagent => (
                                theme::mangosteen_400().opacity(0.16),
                                theme::mangosteen_400(),
                            ),
                            bruhs_core::TaskKind::BackgroundAgent => {
                                (theme::pandan_400().opacity(0.16), theme::pandan_400())
                            }
                            bruhs_core::TaskKind::Shell => {
                                (theme::longan_800(), theme::rambutan_400())
                            }
                        };
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .px_1p5()
                                    .py(px(1.0))
                                    .rounded(px(4.0))
                                    .bg(bg)
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_size(px(10.0))
                                    .text_color(fg)
                                    .child(task.kind.badge_label()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(theme::CODE_FONT_FAMILY)
                                    .text_size(px(12.0))
                                    .text_color(theme::rambutan_300())
                                    .child(task.label.clone()),
                            )
                            .child(
                                web_button(
                                    ("desktop-active-run-stop", row_key),
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label(format!("Stop {}", task.label))
                                .on_click(cx.listener(move |_this, _, _, cx| {
                                    let _ = cx
                                        .global::<DesktopState>()
                                        .app
                                        .registry()
                                        .stop_task(&stop_id);
                                }))
                                .child("Stop"),
                            )
                    }))
                    .when(overflow > 0, |el| {
                        el.child(
                            div()
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_size(px(11.0))
                                .text_color(theme::rambutan_500())
                                .child(format!("+{overflow} more in Tasks")),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.hide_blocked_prompt_suggestion(cx);
        if let Some(text) = self.pending_composer.take() {
            let focus = self.focus_pending_composer;
            let chips = self.pending_chips.take();
            self.focus_pending_composer = false;
            cx.defer_in(window, move |this, window, cx| {
                if let Some(chips) = chips {
                    this.install_chips(&text, chips, text.len(), window, cx);
                } else {
                    this.write_composer(&text, window, cx);
                }
                if focus {
                    this.input_state.update(cx, |input, cx| {
                        input.focus(window, cx);
                    });
                }
            });
        }
        // Messages can be replaced wholesale (thread restore, `/clear`).
        self.content_blocks.truncate(self.messages.len());
        self.rich_embeds.borrow_mut().truncate(self.messages.len());
        self.sync_widget_expansion(window);
        self.rich_embeds.borrow_mut().set_occluded(
            self.model_picker_open
                || self.branch_menu_open
                || self.workspace_menu_open
                || self.effort_menu_open
                || self.composer_options_open
                || self.composer_hint_popover_open(cx),
        );
        self.sync_message_viewport();
        #[cfg(test)]
        self.rendered_message_rows.set(0);
        if self.git_info_stale {
            self.refresh_git_branches(cx);
        }
        if self.workspace_trust_stale && !self.workspace_trust_saving {
            self.refresh_workspace_trust(cx);
        }
        // Web chat page: an empty thread centres the heading + composer; a
        // thread with messages keeps the transcript above a bottom composer.
        let hero = self.messages.is_empty();
        let current_find = self.find_open.then(|| self.current_find()).flatten();
        if let Some((hit_index, message_index)) = current_find {
            if self.find_pinned != Some(hit_index) {
                self.find_pinned = Some(hit_index);
                self.messages_viewport.scroll_to(ListOffset {
                    item_ix: message_index,
                    offset_in_item: px(0.0),
                });
            }
        }
        let current_message = current_find.map(|(_, message_index)| message_index);
        div()
            .track_focus(&self.chat_focus)
            .key_context("Chat")
            .on_key_down(cx.listener(Self::on_question_shortcut))
            .on_action(cx.listener(|this, _: &ToggleDictation, _, cx| {
                if this.dictation.recording() {
                    this.stop_dictation(cx);
                } else if !this.dictation.active() {
                    this.start_dictation(cx);
                }
            }))
            .on_action(cx.listener(Self::send_message))
            .on_action(cx.listener(Self::stash_draft))
            .on_action(cx.listener(|this, _: &FindInThread, window, cx| {
                this.open_find(window, cx);
            }))
            .relative()
            .child({
                let measured = self.measured_width.clone();
                let entity = cx.entity_id();
                canvas(
                    move |bounds, _window, cx| {
                        // In CSS pixels: it is compared with, and laid out
                        // beside, design widths that zoom with the page.
                        let width = bruhs_ui::zoom::css_px(bounds.size.width);
                        if (measured.get() - width).abs() > 0.5 {
                            measured.set(width);
                            cx.notify(entity);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full()
            })
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx
                .theme()
                .color_role("cssConversationBackground")
                .or_else(|| cx.theme().color_role("canvas"))
                .unwrap_or(cx.theme().background))
            .when(
                cx.theme().color_role("cssConversationBackground").is_none(),
                |el| el.theme_region("canvas", cx),
            )
            .children(crate::views::workspace::appearance::background_layer(
                "Conversation",
                cx,
            ))
            .when_some(
                cx.theme().color_role("cssConversationForeground"),
                |el, color| el.text_color(color),
            )
            .flex()
            .flex_col()
            .child(self.readout.element())
            .child(self.render_header(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(self.details_inline_open, |lane| lane.pr(px(312.)))
                    .children(self.render_fork_source(cx))
                    .when(self.find_open, |el| el.child(self.render_find_bar(cx)))
                    .when(hero, |el| {
                        el.child(
                            div()
                                .id("desktop-chat-hero-body")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .px(px(CHAT_COMPOSER_PAGE_X_PADDING_PX))
                                .flex()
                                .flex_col()
                                .items_center()
                                .when(self.pending_permission.is_some(), |el| {
                                    el.child(self.render_pending_input(window, cx))
                                })
                                .when(self.pending_secret.is_some(), |el| {
                                    el.child(self.render_secret_prompt(window, cx))
                                })
                                .when(!self.running_tasks.is_empty(), |el| {
                                    el.child(self.render_active_runs_strip(cx))
                                })
                                .child(
                                    div()
                                        .w_full()
                                        .when_some(self.chat_width.limit(), |el, limit| {
                                            el.max_w(px(limit))
                                        })
                                        .min_w_0()
                                        .my_auto()
                                        .flex_shrink_0()
                                        .child(self.render_composer_block(
                                            ComposerPlacement::Hero,
                                            window,
                                            cx,
                                        )),
                                ),
                        )
                    })
                    .when(!hero, |el| {
                        el.child(
                            div()
                                .relative()
                                .flex_1()
                                .overflow_hidden()
                                .child(self.render_message_viewport(current_message, cx))
                                .when(self.show_scroll_to_bottom_control(), |el| {
                                    el.child(self.render_scroll_to_bottom_control(cx))
                                }),
                        )
                        .when(self.pending_permission.is_some(), |el| {
                            el.child(self.render_pending_input(window, cx))
                        })
                        .when(self.pending_secret.is_some(), |el| {
                            el.child(self.render_secret_prompt(window, cx))
                        })
                        .when(!self.running_tasks.is_empty(), |el| {
                            el.child(self.render_active_runs_strip(cx))
                        })
                        .child(
                            div()
                                .w_full()
                                .px(px(CHAT_COMPOSER_PAGE_X_PADDING_PX))
                                .pb(px(CHAT_COMPOSER_BOTTOM_PADDING_PX))
                                .flex_shrink_0()
                                .child(self.render_composer_block(
                                    ComposerPlacement::Docked,
                                    window,
                                    cx,
                                )),
                        )
                    }),
            )
            .when_some(self.render_expanded_widget(cx), |element, widget| {
                element.child(widget)
            })
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod find_tests {
    use super::{searchable_text, DesktopToolCall, Message, MessageRole, TurnState};

    #[test]
    fn find_text_includes_the_message_and_tool_strings() {
        let message = Message {
            role: MessageRole::Assistant,
            content: "hello rust".into(),
            model: None,
            thinking: "ponder".into(),
            usage: None,
            duration_ms: None,
            started_at_ms: None,
            completed_at_ms: None,
            tool_calls: vec![DesktopToolCall {
                text_offset: None,
                id: "1".into(),
                tool: "Bash".into(),
                input: serde_json::json!({"command": "cargo test"}),
                output: Some("ok".into()),
                is_error: false,
            }],
            files: Vec::new(),
            state: TurnState::Failed {
                error: "boom".into(),
            },
            checkpoint: None,
        };
        let text = searchable_text(&message);
        assert!(text.contains("hello rust"));
        assert!(text.contains("ponder"));
        assert!(text.contains("boom"));
        assert!(text.contains("cargo test"));
        assert!(text.contains("ok"));
    }
}
