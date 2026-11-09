//! Threads, messages and the running turn.

use bruhs_models::logic::attachments::UploadedFile;

use super::*;

/// Every context is a newtype, so two signals of the same shape cannot
/// collide in the Leptos context type-map and every lookup names what it
/// wants.
#[derive(Copy, Clone)]
pub struct ThreadsSignal(pub ReadSignal<Vec<Thread>>);
#[derive(Copy, Clone)]
pub struct SetThreadsSignal(pub WriteSignal<Vec<Thread>>);

/// Id of the thread on screen.
#[derive(Copy, Clone)]
pub struct ActiveThreadId(pub ReadSignal<Option<String>>);
#[derive(Copy, Clone)]
pub struct SetActiveThreadId(pub WriteSignal<Option<String>>);

/// The visible transcript (the active thread's settled messages).
#[derive(Copy, Clone)]
pub struct MessagesSignal(pub ReadSignal<Vec<AgentMessage>>);
#[derive(Copy, Clone)]
pub struct SetMessagesSignal(pub WriteSignal<Vec<AgentMessage>>);

/// A turn is running (in any thread).
#[derive(Copy, Clone)]
pub struct IsStreaming(pub ReadSignal<bool>);
#[derive(Copy, Clone)]
pub struct SetIsStreaming(pub WriteSignal<bool>);

/// Background shells the agent started (`/api/tasks/stream`).
#[derive(Copy, Clone)]
pub struct BackgroundTasksSignal(pub ReadSignal<Vec<BackgroundTask>>);
#[derive(Copy, Clone)]
pub struct SetBackgroundTasksSignal(pub WriteSignal<Vec<BackgroundTask>>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub title: String,
    /// Project display name; empty string == general (project-less) thread.
    pub project_name: String,
    /// Last activity. Older records stored it as a string: a numeric one
    /// (`"1700000000000"`) keeps its value, anything else (`"now"`) reads as
    /// the epoch, i.e. "unknown", and sorts last.
    #[serde(deserialize_with = "bruhs_models::time::deserialize_legacy")]
    pub updated_at: EpochMs,
    /// Per-thread working directory for the agent. `None` for general threads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    /// Provider session id captured from the stream, sent back as `resume`
    /// so later turns keep the conversation context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The next turn starts a new provider session. A rewind sets this so
    /// the removed turns are not resumed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fresh_session: bool,
    /// Branch the project was on when the last turn started (sidebar).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Model id of the last turn, for the sidebar's provider mark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Pinned threads stay at the top of the sidebar (⌘⇧P).
    #[serde(default)]
    pub pinned: bool,
    /// Manually settled (⌘⇧S): folds under "Settled" until new activity.
    #[serde(default)]
    pub settled: bool,
    /// Spot in the pinned list. Missing means "not dragged", which sorts
    /// above arranged pins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_rank: Option<i64>,
    /// Spot in the active list. Missing sorts above arranged threads.
    /// Settling clears it so the thread returns to the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_rank: Option<i64>,
    /// Hidden until this local instant. A thread that needs an answer
    /// comes back early without clearing the instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until_ms: Option<i64>,
    /// Local goal for this thread. It stays until `/goal clear`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<bruhs_models::logic::commands::ThreadGoal>,
    /// Worktree this thread runs in, once one was created or reused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    /// Claude effort stamped on the last turn. `None` leaves the CLI default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The thread has a place and keeps running there. A failed worktree
    /// create leaves this false so the next send can try again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub run_locked: bool,
    /// The earlier conversation of a thread imported without a session
    /// (a Cursor chat). Sent with each turn until the thread has a
    /// session, then dropped; see `portable::with_import_context`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_context: Option<String>,
}

impl Thread {
    /// A fresh thread with no session, last active at `now`.
    pub fn new(
        title: String,
        project_name: String,
        project_path: Option<String>,
        now: EpochMs,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            project_name,
            updated_at: now,
            project_path,
            session_id: None,
            fresh_session: false,
            branch: None,
            model: None,
            pinned: false,
            settled: false,
            pin_rank: None,
            active_rank: None,
            snoozed_until_ms: None,
            goal: None,
            worktree_path: None,
            effort: None,
            run_locked: false,
            import_context: None,
        }
    }

    /// True while the thread still carries a placeholder title, so the first
    /// prompt can name it.
    pub fn is_default_title(&self) -> bool {
        matches!(
            self.title.as_str(),
            "New thread" | "New session" | "New chat" | ""
        )
    }

    /// When the thread was last active, if known (legacy rows are not).
    pub fn last_active(&self) -> Option<EpochMs> {
        (self.updated_at != EpochMs::UNIX_EPOCH).then_some(self.updated_at)
    }
}

/// The browser clock as an [`EpochMs`]. `Date.now()` is always a finite,
/// in-range number; the epoch stands in if a broken host says otherwise.
pub fn now_epoch_ms() -> EpochMs {
    EpochMs::from_f64_floor(js_sys::Date::now()).unwrap_or(EpochMs::UNIX_EPOCH)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub id: String,
    pub tool: String,
    pub input: serde_json::Value,
    pub output: Option<String>,
    pub is_error: bool,
    #[serde(default)]
    pub started_at_ms: Option<f64>,
    #[serde(default)]
    pub ended_at_ms: Option<f64>,
}

/// One ordered piece of an assistant message, in arrival order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MessagePart {
    Text(String),
    Thinking(String),
    ToolCall(ToolCallInfo),
}

/// Billed tokens of the turn, plus how full the context window is after it.
/// See [`bruhs_models::api::UsageInfo`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    /// `None` on rows saved before the window was reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
}

impl UsageInfo {
    /// Tokens in the context window after this turn.
    pub fn window(&self) -> u64 {
        bruhs_models::logic::compact::window_tokens(
            self.context_tokens,
            self.input_tokens,
            self.cache_read_tokens,
            self.cache_creation_tokens,
        )
    }
}

/// Where a message is in its life.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MessageStatus {
    /// The assistant is still producing it.
    Streaming,
    /// Nothing more will arrive. `at_ms` is when it finished; `None` for
    /// records that never said (older rows, an answer cut off by a reload).
    Finished { at_ms: Option<f64> },
}

impl MessageStatus {
    pub fn is_streaming(self) -> bool {
        matches!(self, Self::Streaming)
    }

    /// When the message finished, if it has and the time is known.
    pub fn finished_at_ms(self) -> Option<f64> {
        match self {
            Self::Streaming => None,
            Self::Finished { at_ms } => at_ms,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One chat message. `parts` is the only copy of its content; the flat
/// `content` / `thinking` / `tool_calls` fields of the stored record are
/// derived from it when saving (see [`MessageRecord`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "MessageRecord", into = "MessageRecord")]
pub struct AgentMessage {
    pub id: String,
    pub role: Role,
    pub status: MessageStatus,
    pub usage: Option<UsageInfo>,
    /// Unix epoch milliseconds when this message started. For user messages
    /// this is send time; for assistant messages, the moment the placeholder
    /// was inserted / streaming began.
    pub started_at_ms: Option<f64>,
    /// Model id responsible for this assistant message, captured at submit.
    pub model: Option<String>,
    /// Ordered content, in arrival order.
    pub parts: Vec<MessagePart>,
    /// Terminal error for this turn (stream error or provider `is_error`).
    pub error: Option<String>,
    /// Files sent with this message. Empty for older records.
    pub files: Vec<UploadedFile>,
    /// Git tree of this thread's worktree at send time. Absent for a
    /// checkout thread, an old message, or a snapshot that failed.
    pub checkpoint: Option<String>,
}

/// The IndexedDB shape of an [`AgentMessage`], unchanged from before the
/// message had a status and a single copy of its content: `is_streaming` +
/// `completed_at_ms`, and flat `content` / `thinking` / `tool_calls` next to
/// `parts`. Saving writes all of them so older builds still read the record.
/// Loading accepts records without `parts` (the oldest ones), whose parts
/// are rebuilt in the order the old renderer showed them: thinking, text,
/// tools.
#[derive(Serialize, Deserialize)]
struct MessageRecord {
    id: String,
    /// Read leniently: an unknown role is shown as the assistant's.
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<ToolCallInfo>,
    #[serde(default)]
    is_streaming: bool,
    #[serde(default)]
    thinking: String,
    #[serde(default)]
    usage: Option<UsageInfo>,
    #[serde(default)]
    started_at_ms: Option<f64>,
    #[serde(default)]
    completed_at_ms: Option<f64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    parts: Vec<MessagePart>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<UploadedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoint: Option<String>,
}

impl From<MessageRecord> for AgentMessage {
    fn from(record: MessageRecord) -> Self {
        let parts = if record.parts.is_empty() {
            let mut parts = Vec::new();
            if !record.thinking.is_empty() {
                parts.push(MessagePart::Thinking(record.thinking));
            }
            if !record.content.is_empty() {
                parts.push(MessagePart::Text(record.content));
            }
            parts.extend(record.tool_calls.into_iter().map(MessagePart::ToolCall));
            parts
        } else {
            record.parts
        };
        Self {
            id: record.id,
            role: Role::parse_lossy(&record.role),
            // A record saved mid-stream says so even if it also has a time.
            status: if record.is_streaming {
                MessageStatus::Streaming
            } else {
                MessageStatus::Finished {
                    at_ms: record.completed_at_ms,
                }
            },
            usage: record.usage,
            started_at_ms: record.started_at_ms,
            model: record.model,
            parts,
            error: record.error,
            files: record.files,
            checkpoint: record.checkpoint.filter(|tree| !tree.is_empty()),
        }
    }
}

impl From<AgentMessage> for MessageRecord {
    fn from(message: AgentMessage) -> Self {
        let content = message.text();
        let thinking = message.thinking();
        let tool_calls = message.tool_calls_iter().cloned().collect();
        Self {
            id: message.id,
            role: message.role.as_str().to_string(),
            content,
            tool_calls,
            is_streaming: message.status.is_streaming(),
            thinking,
            usage: message.usage,
            started_at_ms: message.started_at_ms,
            completed_at_ms: message.status.finished_at_ms(),
            model: message.model,
            parts: message.parts,
            error: message.error,
            files: message.files,
            checkpoint: message.checkpoint.filter(|tree| !tree.is_empty()),
        }
    }
}

impl AgentMessage {
    pub fn user(text: String, now_ms: f64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role: Role::User,
            status: MessageStatus::Finished {
                at_ms: Some(now_ms),
            },
            usage: None,
            started_at_ms: Some(now_ms),
            model: None,
            parts: vec![MessagePart::Text(text)],
            error: None,
            files: Vec::new(),
            checkpoint: None,
        }
    }

    pub fn user_with_files(text: String, now_ms: f64, files: Vec<UploadedFile>) -> Self {
        let mut message = Self::user(text, now_ms);
        message.files = files;
        message
    }

    pub fn assistant_placeholder(model: String, now_ms: f64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role: Role::Assistant,
            status: MessageStatus::Streaming,
            usage: None,
            started_at_ms: Some(now_ms),
            model: Some(model),
            parts: vec![],
            error: None,
            files: Vec::new(),
            checkpoint: None,
        }
    }

    /// A finished assistant message that is just `text` (local notes such as
    /// `/help`).
    pub fn assistant_note(id: String, text: String, now_ms: f64) -> Self {
        Self {
            id,
            role: Role::Assistant,
            status: MessageStatus::Finished {
                at_ms: Some(now_ms),
            },
            usage: None,
            started_at_ms: Some(now_ms),
            model: None,
            parts: vec![MessagePart::Text(text)],
            error: None,
            files: Vec::new(),
            checkpoint: None,
        }
    }

    pub fn is_streaming(&self) -> bool {
        self.status.is_streaming()
    }

    /// All text parts concatenated.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    /// All thinking parts concatenated.
    pub fn thinking(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Thinking(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn tool_calls_iter(&self) -> impl Iterator<Item = &ToolCallInfo> {
        self.parts.iter().filter_map(|p| match p {
            MessagePart::ToolCall(c) => Some(c),
            _ => None,
        })
    }
}

/// A background shell the agent started, as `/api/tasks/stream` reports it.
pub use bruhs_models::api::BackgroundTask;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub tool: String,
    pub input: serde_json::Value,
}

/// Permission requests the running turn is waiting on, oldest first, at most
/// one per id. A provider can ask for several at once; each is answered and
/// cleared by its own id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingPermissions(Vec<PermissionRequest>);

impl PendingPermissions {
    /// Add `request`, or replace the one with the same id in place.
    pub fn insert(&mut self, request: PermissionRequest) {
        match self.0.iter_mut().find(|r| r.id == request.id) {
            Some(slot) => *slot = request,
            None => self.0.push(request),
        }
    }

    /// Remove the request `id`; `false` if there was none.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|r| r.id != id);
        self.0.len() != before
    }

    /// The request to ask about now (the oldest).
    pub fn first(&self) -> Option<&PermissionRequest> {
        self.0.first()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// App-wide [`PendingPermissions`].
#[derive(Copy, Clone)]
pub struct PendingPermissionsSignal(pub RwSignal<PendingPermissions>);

/// Request metadata only. Private input stays in its mounted form, never app state.
#[derive(Copy, Clone)]
pub struct PendingSecretsSignal(pub RwSignal<Vec<bruhs_models::secrets::SecretRequest>>);

/// Settled messages of the thread whose turn is currently streaming (up to
/// and including the prompt). `messages` is that thread's transcript while it
/// is the active one, so switching threads mid-turn neither corrupts the
/// visible list nor loses the turn. The running turn's own message is in
/// [`LiveMessageSignal`]; it joins `messages` here when the turn ends.
#[derive(Clone, Default, PartialEq)]
pub struct StreamBuffer {
    pub thread_id: Option<String>,
    pub messages: Vec<AgentMessage>,
}

#[derive(Copy, Clone)]
pub struct StreamBufferSignal(pub RwSignal<StreamBuffer>);

/// The assistant message of the running turn, patched in place per token. It
/// belongs to [`StreamBuffer::thread_id`]. Kept apart from the transcript so
/// a token re-renders only that message, not the whole thread.
#[derive(Copy, Clone)]
pub struct LiveMessageSignal(pub RwSignal<Option<AgentMessage>>);

/// Whether the running turn belongs to the `active` thread. Changes when a
/// turn starts or ends or the thread changes, never per token.
pub fn live_turn_here(
    active: ReadSignal<Option<String>>,
    buffer: RwSignal<StreamBuffer>,
) -> Memo<bool> {
    Memo::new(move |_| {
        let active = active.get();
        active.is_some() && buffer.with(|b| b.thread_id == active)
    })
}

/// Id of the thread with a running turn, or `None` when idle.
#[derive(Copy, Clone)]
pub struct StreamingThreadId(pub Memo<Option<String>>);

/// Threads whose last turn finished while another thread was active.
#[derive(Copy, Clone)]
pub struct UnreadThreads(pub RwSignal<HashSet<String>>);

/// One-line notice shown near the composer (e.g. "A turn is already
/// running in another thread"). Cleared by the next successful submit.
/// [`ThreadNotice::flash`] shows a confirmation for a moment; any newer
/// notice cancels that flash's reset, so it is never wiped early.
#[derive(Copy, Clone)]
pub struct ThreadNotice {
    text: RwSignal<Option<String>>,
    reset: crate::ui::ResetTimer,
}

impl ThreadNotice {
    /// How long a flashed confirmation stays.
    pub const FLASH_MS: u32 = 2_000;

    pub fn new() -> Self {
        Self {
            text: RwSignal::new(None),
            reset: crate::ui::ResetTimer::new(),
        }
    }

    /// The notice showing now (tracked).
    pub fn get(self) -> Option<String> {
        self.text.get()
    }

    /// Show `notice` until something replaces it (`None` clears it).
    pub fn set(self, notice: Option<String>) {
        self.reset.cancel();
        self.text.try_set(notice);
    }

    /// Show `notice` for [`Self::FLASH_MS`].
    pub fn flash(self, notice: impl Into<String>) {
        self.text.try_set(Some(notice.into()));
        let text = self.text;
        self.reset.after(Self::FLASH_MS, move || {
            text.try_set(None);
        });
    }
}

impl Default for ThreadNotice {
    fn default() -> Self {
        Self::new()
    }
}

/// Prompts waiting for their thread's running turn to finish.
#[derive(Copy, Clone)]
pub struct PromptQueueSignal(pub RwSignal<crate::queue::PromptQueue>);

/// Queued messages stay put after Interrupt or a reload until Resume or a
/// new send. A turn that finishes on its own clears this and drains.
#[derive(Copy, Clone)]
pub struct QueueHeldSignal(pub RwSignal<bool>);

/// The composer draft. Shared so ⌘S (stash) and the palette can reach it.
#[derive(Copy, Clone)]
pub struct ComposerDraft(pub RwSignal<String>);

/// Inline context chips for [`ComposerDraft`]. Labels live in the draft.
#[derive(Copy, Clone)]
pub struct ComposerChips(pub RwSignal<Vec<bruhs_models::logic::context_chip::ContextChip>>);

/// Set while a chip insert replaces the draft, so the composer does not
/// retarget those chips a second time.
#[derive(Copy, Clone)]
pub struct ChipRetargetGuard(pub StoredValue<bool>);

/// Stashed prompts (⌘S), persisted as `bruhs.promptStash`.
#[derive(Copy, Clone)]
pub struct PromptStashSignal(pub RwSignal<crate::stash::PromptStash>);

/// Open state of the composer's stash menu.
#[derive(Copy, Clone)]
pub struct StashMenuOpen(pub RwSignal<bool>);

/// Make `id` the active thread and load its transcript: from the stream
/// buffer when that thread's turn is still running, otherwise from IndexedDB.
pub fn activate_thread(
    history: crate::thread_history::ThreadHistory,
    active: ReadSignal<Option<String>>,
    id: String,
    set_active: WriteSignal<Option<String>>,
    set_messages: WriteSignal<Vec<AgentMessage>>,
    stream_buffer: RwSignal<StreamBuffer>,
    unread: RwSignal<HashSet<String>>,
) {
    set_active.set(Some(id.clone()));
    unread.update(|s| {
        s.remove(&id);
    });
    history.load(id, active, set_messages, stream_buffer);
}

/// "New thread" from the sidebar, header, palette or ⌘⇧N: when projects are
/// known open the picker so the thread gets a workspace; otherwise create a
/// general thread immediately.
pub fn new_thread_or_pick(
    catalog: crate::thread_catalog::ThreadCatalog,
    has_projects: bool,
    set_threads: WriteSignal<Vec<Thread>>,
    set_active: WriteSignal<Option<String>>,
    set_messages: WriteSignal<Vec<AgentMessage>>,
    set_picker_open: WriteSignal<bool>,
) {
    if has_projects {
        set_picker_open.set(true);
    } else {
        create_thread(
            catalog,
            set_threads,
            set_active,
            set_messages,
            String::new(),
            None,
        );
    }
}

/// Toggle a thread's pin and persist it. Returns the new state.
pub fn toggle_pinned(
    catalog: crate::thread_catalog::ThreadCatalog,
    set_threads: WriteSignal<Vec<Thread>>,
    id: &str,
) -> Option<bool> {
    let mut now = None;
    set_threads.update(|ts| {
        if let Some(t) = ts.iter_mut().find(|t| t.id == id) {
            let original = t.clone();
            super::set_pinned(t, !t.pinned);
            now = Some(t.pinned);
            catalog.persist(Some(original), t.clone());
        }
    });
    now
}

/// Toggle a thread's manual "settled" state; settling also clears unread.
/// Returns the new state.
pub fn toggle_settled(
    catalog: crate::thread_catalog::ThreadCatalog,
    set_threads: WriteSignal<Vec<Thread>>,
    unread: RwSignal<std::collections::HashSet<String>>,
    id: &str,
) -> Option<bool> {
    let mut now = None;
    set_threads.update(|ts| {
        if let Some(t) = ts.iter_mut().find(|t| t.id == id) {
            let original = t.clone();
            super::set_settled(t, !t.settled);
            now = Some(t.settled);
            catalog.persist(Some(original), t.clone());
        }
    });
    if now == Some(true) {
        unread.update(|s| {
            s.remove(id);
        });
    }
    now
}

/// Global thread-title instructions plus an optional line per project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleInstructions {
    #[serde(default)]
    pub global: String,
    #[serde(default)]
    pub by_project: BTreeMap<String, String>,
}

#[derive(Copy, Clone)]
pub struct TitleInstructionsSignal(pub RwSignal<TitleInstructions>);

/// Folds and the Working-section switch. One copy for the sidebar, the
/// shell's keyboard order, and Settings.
#[derive(Copy, Clone)]
pub struct SidebarChrome {
    pub working_open: RwSignal<bool>,
    pub snoozed_open: RwSignal<bool>,
    pub settled_open: RwSignal<bool>,
    /// On unless the device has saved `"0"`.
    pub working_section: RwSignal<bool>,
}

/// Something the five-second Undo notification can put back.
#[derive(Clone, Debug)]
pub struct UndoEntry {
    pub message: String,
    pub restore: UndoRestore,
}

#[derive(Clone, Debug)]
pub enum UndoRestore {
    Thread(Box<Thread>),
    Draft {
        text: String,
        chips: Vec<bruhs_models::logic::context_chip::ContextChip>,
    },
}

/// In-memory undo. A newer action replaces the previous one.
#[derive(Copy, Clone)]
pub struct SidebarUndo {
    entry: RwSignal<Option<(u32, UndoEntry)>>,
    generation: RwSignal<u32>,
    reset: crate::ui::ResetTimer,
}

impl SidebarUndo {
    pub fn new() -> Self {
        Self {
            entry: RwSignal::new(None),
            generation: RwSignal::new(0),
            reset: crate::ui::ResetTimer::new(),
        }
    }

    pub fn current(self) -> Option<UndoEntry> {
        self.entry.get().map(|(_, entry)| entry)
    }

    pub fn push(self, entry: UndoEntry) {
        let generation = self.generation.get_untracked().wrapping_add(1);
        self.generation.set(generation);
        self.entry.set(Some((generation, entry)));
        let stored = self.entry;
        let delay = u32::try_from(bruhs_models::logic::sidebar::UNDO_MS).unwrap_or(5_000);
        self.reset.after(delay, move || {
            stored.update(|current| {
                if current.as_ref().is_some_and(|(id, _)| *id == generation) {
                    *current = None;
                }
            });
        });
    }

    pub fn take(self) -> Option<UndoEntry> {
        self.reset.cancel();
        self.entry
            .try_update(|current| current.take().map(|(_, entry)| entry))
            .flatten()
    }
}

impl Default for SidebarUndo {
    fn default() -> Self {
        Self::new()
    }
}

/// First line of the prompt, capped at 48 characters, for auto-naming a
/// thread that still has its placeholder title.
pub fn auto_title(prompt: &str) -> String {
    // A message from another thread's agent is named after what it says.
    let prompt = bruhs_models::thread_link::delivery_body(prompt);
    let first = prompt.trim().lines().next().unwrap_or("").trim();
    crate::format::truncate_chars(first, 48)
}

/// Create a thread, make it active with an empty transcript, and persist it.
/// Returns the new id. `project_name` empty == general thread.
pub fn create_thread(
    catalog: crate::thread_catalog::ThreadCatalog,
    set_threads: WriteSignal<Vec<Thread>>,
    set_active: WriteSignal<Option<String>>,
    set_messages: WriteSignal<Vec<AgentMessage>>,
    project_name: String,
    project_path: Option<String>,
) -> String {
    let thread = Thread::new(
        "New thread".to_string(),
        project_name,
        project_path,
        now_epoch_ms(),
    );
    let id = thread.id.clone();
    set_threads.update(|ts| ts.insert(0, thread.clone()));
    set_active.set(Some(id.clone()));
    set_messages.set(vec![]);
    catalog.persist(None, thread);
    id
}

/// Threads, the visible transcript, the running turn and the composer's
/// shared state (draft, queue, stash).
pub(super) fn provide() {
    let (threads, set_threads) = signal(Vec::<Thread>::new());
    let (active_thread_id, set_active_thread_id) = signal(Option::<String>::None);
    let (messages, set_messages) = signal(Vec::<AgentMessage>::new());
    let (is_streaming, set_is_streaming) = signal(false);
    let (tasks, set_tasks) = signal(Vec::<BackgroundTask>::new());

    // Each thread keeps its model, so opening one puts its model in the
    // composer and its session resumes on the provider that owns it. A thread with no turn yet is on the default.
    let crate::state::SetCurrentModel(set_current_model) =
        expect_context::<crate::state::SetCurrentModel>();
    let crate::state::DefaultModel(default_model) = expect_context::<crate::state::DefaultModel>();
    let active_model = Memo::new(move |_| {
        let id = active_thread_id.get()?;
        threads.with(|ts| ts.iter().find(|t| t.id == id)?.model.clone())
    });
    Effect::new(move |_| {
        let model = active_model
            .get()
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| default_model.get());
        set_current_model.set(model);
    });

    let stream_buffer = RwSignal::new(StreamBuffer::default());
    let streaming_thread_id = Memo::new(move |_| {
        if is_streaming.get() {
            stream_buffer.with(|b| b.thread_id.clone())
        } else {
            None
        }
    });
    // Queued prompts survive a reload; they are sent when their thread is idle.
    let prompt_queue = RwSignal::new(
        load_json_pref::<crate::queue::PromptQueue>("bruhs.promptQueue").unwrap_or_default(),
    );
    Effect::new(move |_| prompt_queue.with(|q| save_json_pref("bruhs.promptQueue", q)));
    // A reload is a restart: queued rows wait for Resume or the next send.
    let queue_held = RwSignal::new(prompt_queue.with_untracked(|queue| !queue.is_empty()));
    provide_context(QueueHeldSignal(queue_held));
    let prompt_stash = RwSignal::new(
        load_json_pref::<crate::stash::PromptStash>("bruhs.promptStash").unwrap_or_default(),
    );
    Effect::new(move |_| prompt_stash.with(|s| save_json_pref("bruhs.promptStash", s)));

    provide_context(ThreadsSignal(threads));
    provide_context(SetThreadsSignal(set_threads));
    provide_context(ActiveThreadId(active_thread_id));
    provide_context(SetActiveThreadId(set_active_thread_id));
    provide_context(MessagesSignal(messages));
    provide_context(SetMessagesSignal(set_messages));
    provide_context(IsStreaming(is_streaming));
    provide_context(SetIsStreaming(set_is_streaming));
    provide_context(BackgroundTasksSignal(tasks));
    provide_context(SetBackgroundTasksSignal(set_tasks));
    provide_context(PendingPermissionsSignal(RwSignal::new(
        PendingPermissions::default(),
    )));
    provide_context(PendingSecretsSignal(RwSignal::new(Vec::new())));
    provide_context(StreamBufferSignal(stream_buffer));
    provide_context(LiveMessageSignal(RwSignal::new(
        Option::<AgentMessage>::None,
    )));
    provide_context(StreamingThreadId(streaming_thread_id));
    provide_context(UnreadThreads(RwSignal::new(HashSet::<String>::new())));
    provide_context(ThreadNotice::new());
    provide_context(PromptQueueSignal(prompt_queue));
    provide_context(ComposerDraft(RwSignal::new(String::new())));
    provide_context(ComposerChips(RwSignal::new(Vec::new())));
    provide_context(ChipRetargetGuard(StoredValue::new(false)));
    provide_context(PromptStashSignal(prompt_stash));
    provide_context(StashMenuOpen(RwSignal::new(false)));

    let working_section =
        RwSignal::new(load_pref("bruhs.sidebarWorkingSection").as_deref() != Some("0"));
    let working_open = RwSignal::new(load_pref("bruhs.sidebarWorkingOpen").as_deref() == Some("1"));
    let snoozed_open = RwSignal::new(load_pref("bruhs.sidebarSnoozedOpen").as_deref() == Some("1"));
    let settled_open = RwSignal::new(load_pref("bruhs.sidebarSettledOpen").as_deref() == Some("1"));
    Effect::new(move |_| {
        save_pref(
            "bruhs.sidebarWorkingSection",
            if working_section.get() { "1" } else { "0" },
        );
    });
    Effect::new(move |_| {
        save_pref(
            "bruhs.sidebarWorkingOpen",
            if working_open.get() { "1" } else { "0" },
        );
    });
    Effect::new(move |_| {
        save_pref(
            "bruhs.sidebarSnoozedOpen",
            if snoozed_open.get() { "1" } else { "0" },
        );
    });
    Effect::new(move |_| {
        save_pref(
            "bruhs.sidebarSettledOpen",
            if settled_open.get() { "1" } else { "0" },
        );
    });
    provide_context(SidebarChrome {
        working_open,
        snoozed_open,
        settled_open,
        working_section,
    });
    let title_instructions = RwSignal::new(
        load_json_pref::<TitleInstructions>("bruhs.threadTitleInstructions").unwrap_or_default(),
    );
    Effect::new(move |_| {
        title_instructions.with(|value| save_json_pref("bruhs.threadTitleInstructions", value));
    });
    provide_context(TitleInstructionsSignal(title_instructions));
    provide_context(SidebarUndo::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str, tool: &str) -> PermissionRequest {
        PermissionRequest {
            id: id.into(),
            tool: tool.into(),
            input: serde_json::Value::Null,
        }
    }

    /// Several requests can be outstanding; answering or withdrawing one
    /// leaves the others, oldest first.
    #[test]
    fn pending_permissions_are_keyed_by_id_in_arrival_order() {
        let mut pending = PendingPermissions::default();
        pending.insert(request("a", "Bash"));
        pending.insert(request("b", "Edit"));
        pending.insert(request("a", "Write"));
        assert_eq!(pending.0.len(), 2);
        assert_eq!(pending.first().map(|r| r.tool.as_str()), Some("Write"));
        assert!(!pending.remove("missing"));
        assert!(pending.remove("a"));
        assert_eq!(pending.first().map(|r| r.id.as_str()), Some("b"));
        pending.clear();
        assert!(pending.is_empty());
    }

    /// The oldest IndexedDB rows have no `parts`; they load with parts in
    /// the order the old renderer showed (thinking, text, tools).
    #[test]
    fn legacy_record_gains_ordered_parts() {
        let m: AgentMessage = serde_json::from_value(serde_json::json!({
            "id": "1",
            "role": "assistant",
            "content": "hello",
            "tool_calls": [{
                "id": "t1", "tool": "Read", "input": {"file_path": "a.rs"},
                "output": "x", "is_error": false
            }],
            "is_streaming": false,
            "thinking": "hmm"
        }))
        .unwrap();
        assert_eq!(m.role, Role::Assistant);
        assert_eq!(m.status, MessageStatus::Finished { at_ms: None });
        assert_eq!(m.parts.len(), 3);
        assert!(matches!(m.parts[0], MessagePart::Thinking(ref t) if t == "hmm"));
        assert!(matches!(m.parts[1], MessagePart::Text(ref t) if t == "hello"));
        assert!(matches!(m.parts[2], MessagePart::ToolCall(ref c) if c.id == "t1"));
    }

    /// A message saved mid-stream loads as streaming; a finished one keeps
    /// its time; an unknown role reads as the assistant's.
    #[test]
    fn legacy_status_fields_load_as_a_status() {
        let load = |v: serde_json::Value| serde_json::from_value::<AgentMessage>(v).unwrap();
        let streaming = load(serde_json::json!({
            "id": "a", "role": "assistant", "content": "", "tool_calls": [],
            "is_streaming": true, "completed_at_ms": null
        }));
        assert_eq!(streaming.status, MessageStatus::Streaming);
        let done = load(serde_json::json!({
            "id": "b", "role": "user", "content": "hi", "tool_calls": [],
            "is_streaming": false, "started_at_ms": 1.0, "completed_at_ms": 2.0,
            "parts": [{"kind": "text", "value": "hi"}]
        }));
        assert_eq!(done.role, Role::User);
        assert_eq!(done.status, MessageStatus::Finished { at_ms: Some(2.0) });
        assert_eq!(done.text(), "hi");
        let odd = load(serde_json::json!({"id": "c", "role": "system"}));
        assert_eq!(odd.role, Role::Assistant);
    }

    /// Saving still writes the flat fields and the streaming flag older
    /// builds read, derived from the parts and the status, and reloading
    /// gives the same message back.
    #[test]
    fn saved_messages_keep_the_old_record_shape() {
        let mut m = AgentMessage::assistant_placeholder("claude-sonnet-4-6".into(), 1.0);
        m.parts = vec![
            MessagePart::Thinking("a".into()),
            MessagePart::Text("x".into()),
            MessagePart::ToolCall(ToolCallInfo {
                id: "t".into(),
                tool: "Bash".into(),
                input: serde_json::Value::Null,
                output: Some("ok".into()),
                is_error: false,
                started_at_ms: None,
                ended_at_ms: None,
            }),
            MessagePart::Thinking("b".into()),
            MessagePart::Text("y".into()),
        ];
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "xy");
        assert_eq!(json["thinking"], "ab");
        assert_eq!(json["tool_calls"][0]["id"], "t");
        assert_eq!(json["is_streaming"], true);
        assert_eq!(json["completed_at_ms"], serde_json::Value::Null);
        assert_eq!(serde_json::from_value::<AgentMessage>(json).unwrap(), m);

        m.status = MessageStatus::Finished { at_ms: Some(9.0) };
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["is_streaming"], false);
        assert_eq!(json["completed_at_ms"], 9.0);
        assert_eq!(serde_json::from_value::<AgentMessage>(json).unwrap(), m);
    }

    #[test]
    fn text_and_thinking_concatenate_their_parts() {
        let mut m = AgentMessage::assistant_placeholder("claude-sonnet-4-6".into(), 0.0);
        m.parts.push(MessagePart::Text("a".into()));
        m.parts.push(MessagePart::ToolCall(ToolCallInfo {
            id: "t".into(),
            tool: "Bash".into(),
            input: serde_json::Value::Null,
            output: None,
            is_error: false,
            started_at_ms: None,
            ended_at_ms: None,
        }));
        m.parts.push(MessagePart::Text("b".into()));
        assert_eq!(m.text(), "ab");
        assert_eq!(m.thinking(), "");
        assert_eq!(m.tool_calls_iter().count(), 1);
    }

    /// Thread rows written before `updated_at` was a number: numeric strings
    /// keep their value, `"now"` reads as unknown, numbers read as is.
    #[test]
    fn legacy_thread_json_loads() {
        let t: Thread = serde_json::from_str(
            r#"{"id":"x","title":"New thread","project_name":"","updated_at":"now"}"#,
        )
        .unwrap();
        assert_eq!(t.project_path, None);
        assert_eq!(t.session_id, None);
        assert!(t.is_default_title());
        assert_eq!(t.updated_at, EpochMs::UNIX_EPOCH);
        assert_eq!(t.last_active(), None);

        let t: Thread = serde_json::from_str(
            r#"{"id":"x","title":"t","project_name":"p","updated_at":"1700000000123"}"#,
        )
        .unwrap();
        assert_eq!(t.updated_at.as_millis(), 1_700_000_000_123);
        let t: Thread = serde_json::from_str(
            r#"{"id":"x","title":"t","project_name":"p","updated_at":"1700000000123.4"}"#,
        )
        .unwrap();
        assert_eq!(t.updated_at.as_millis(), 1_700_000_000_123);

        // New rows store a number and read back unchanged.
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["updated_at"], 1_700_000_000_123_i64);
        assert_eq!(serde_json::from_value::<Thread>(json).unwrap(), t);
    }

    #[test]
    fn auto_title_trims_and_caps() {
        assert_eq!(
            auto_title("  Fix the login bug\nplease  "),
            "Fix the login bug"
        );
        let long = "x".repeat(80);
        assert_eq!(auto_title(&long).chars().count(), 49); // 48 + ellipsis
    }

    #[test]
    fn message_part_round_trips_through_json() {
        let part = MessagePart::Text("hi".into());
        let json = serde_json::to_string(&part).unwrap();
        assert_eq!(json, r#"{"kind":"text","value":"hi"}"#);
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }
}
