//! [`AppState`]: the state one bruhs process shares between its surfaces
//! (server routes, the desktop app, the MCP server): chat threads, agent
//! turns, background tasks, meetings, calendar sync and terminals.

use crate::agent::{
    AgentOrchestrator, AgentTurn, OrchestratorEventStream, PermissionDecision,
    PermissionResolveError, SendMessageError, ThreadStatus,
};
use crate::google_calendar::GoogleCalendarHandle;
use crate::meetings::{
    default_meeting_store, mark_orphaned_live_as_failed, MeetingStore, MeetingStoreError,
};
use crate::registry::BackgroundTaskRegistry;
use crate::terminal::TerminalManager;
use bruhs_models::api::Thread;
use bruhs_models::ids::{MeetingId, ThreadId};
use bruhs_models::{EpochMs, Meeting, MeetingEvent, TranscriptSegment};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, RwLock};

/// Why a meeting chat thread was not created.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CreateThreadError {
    #[error("meeting store: {0}")]
    MeetingStore(#[from] MeetingStoreError),
    #[error("meeting not found: {0}")]
    MeetingNotFound(MeetingId),
}

fn build_meeting_chat_prompt(meeting: &Meeting, segments: &[TranscriptSegment]) -> String {
    let mut s = String::new();
    s.push_str("You are a helpful assistant for a meeting that already happened. ");
    s.push_str("The user is asking follow-up questions about this meeting. ");
    s.push_str("Answer concisely; cite specific transcript moments when relevant.\n\n");
    // Writing into a `String` cannot fail, so the `fmt::Result`s below are
    // always `Ok`.
    let _ = writeln!(s, "Meeting: {}", meeting.title);
    if let Some(summary) = &meeting.summary {
        let _ = writeln!(s, "\n## Summary\n{}", summary.tldr);
        if !summary.action_items.is_empty() {
            s.push_str("\n## Action items\n");
            for ai in &summary.action_items {
                let _ = writeln!(s, "- {}", ai.text);
            }
        }
        if !summary.decisions.is_empty() {
            s.push_str("\n## Decisions\n");
            for d in &summary.decisions {
                let _ = writeln!(s, "- {d}");
            }
        }
        if !summary.topics.is_empty() {
            s.push_str("\n## Topics\n");
            for t in &summary.topics {
                let _ = writeln!(s, "### {}", t.title);
                for b in &t.bullets {
                    let _ = writeln!(s, "- {b}");
                }
            }
        }
    }
    s.push_str("\n## Transcript\n");
    for seg in segments {
        let speaker = seg.speaker_id.as_deref().unwrap_or("?");
        let _ = writeln!(s, "[{speaker}] {}", seg.text);
    }
    s
}

/// Capacity for capture and server meeting events. Lagging SSE consumers receive
/// a reconciliation hint; this channel is not a durable event log.
const MEETING_EVENT_CHANNEL_CAPACITY: usize = 64;

fn now_epoch_ms() -> EpochMs {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        });
    // The system clock is far inside EpochMs's ±285,000-year range.
    EpochMs::new(ms).unwrap_or_default()
}

/// A new, general (project-less) thread.
fn new_thread(title: String) -> Thread {
    let now = now_epoch_ms();
    Thread {
        id: ThreadId::new_v4(),
        title,
        project_path: None,
        project_name: None,
        created_at: now,
        updated_at: now,
        system_prompt: None,
        meeting_id: None,
        session_id: None,
        branch: None,
        model: None,
        pinned: false,
        settled: false,
    }
}

/// Shared state of one bruhs process. Build it with [`AppState::builder`]
/// (or [`AppState::new`] for the defaults).
pub struct AppState {
    /// Threads created in this process. Clients (web IndexedDB, desktop
    /// files) own thread persistence; this map lives only as long as the
    /// process.
    threads: RwLock<HashMap<ThreadId, Thread>>,
    registry: Arc<BackgroundTaskRegistry>,
    meetings: Arc<dyn MeetingStore>,
    google_calendar: Arc<GoogleCalendarHandle>,
    /// Broadcast channel shared by the process-owned capture pump and routes.
    /// HTTP SSE subscribers observe this channel without starting side effects.
    meeting_events_tx: broadcast::Sender<MeetingEvent>,
    terminals: TerminalManager,
    orchestrator: AgentOrchestrator,
}

/// Builds an [`AppState`]; every part not given gets its default.
#[derive(Default)]
pub struct AppStateBuilder {
    secret_store: Option<crate::secrets::SecretStore>,
    meetings: Option<Arc<dyn MeetingStore>>,
    google_calendar: Option<Arc<GoogleCalendarHandle>>,
}

impl AppStateBuilder {
    pub fn secret_store(mut self, store: crate::secrets::SecretStore) -> Self {
        self.secret_store = Some(store);
        self
    }
    /// The meeting store (default: an empty in-memory store).
    pub fn meetings(mut self, store: Arc<dyn MeetingStore>) -> Self {
        self.meetings = Some(store);
        self
    }

    /// The Google Calendar handle (default:
    /// [`GoogleCalendarHandle::from_env`]).
    pub fn google_calendar(mut self, handle: Arc<GoogleCalendarHandle>) -> Self {
        self.google_calendar = Some(handle);
        self
    }

    /// Pure: no I/O and nothing spawned, so it is safe outside a tokio
    /// runtime. Call [`AppState::start_background`] from the process that
    /// should run the background loops.
    pub fn build(self) -> AppState {
        let registry = Arc::new(BackgroundTaskRegistry::new());
        let orchestrator = AgentOrchestrator::new(registry.clone()).with_secret_store(
            self.secret_store
                .unwrap_or_else(crate::secrets::SecretStore::os),
        );
        let (meeting_events_tx, _) = broadcast::channel(MEETING_EVENT_CHANNEL_CAPACITY);
        AppState {
            threads: RwLock::new(HashMap::new()),
            registry,
            meetings: self.meetings.unwrap_or_else(default_meeting_store),
            google_calendar: self
                .google_calendar
                .unwrap_or_else(|| Arc::new(GoogleCalendarHandle::from_env())),
            meeting_events_tx,
            terminals: TerminalManager::new(),
            orchestrator,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    /// An `AppState` with every default; see [`AppState::builder`].
    pub fn new() -> Self {
        Self::builder().build()
    }

    pub fn builder() -> AppStateBuilder {
        AppStateBuilder::default()
    }

    /// Start the background loops (today: Google Calendar sync, when OAuth
    /// is configured) on `runtime`. Idempotent.
    pub fn start_background(&self, runtime: &tokio::runtime::Handle) {
        self.google_calendar.start_background(runtime);
    }

    /// Background shells started by agent turns.
    pub fn registry(&self) -> &Arc<BackgroundTaskRegistry> {
        &self.registry
    }

    pub fn secrets(&self) -> &crate::secrets::SecretManager {
        self.orchestrator.secrets()
    }

    /// Meetings, transcripts and people.
    pub fn meetings(&self) -> &Arc<dyn MeetingStore> {
        &self.meetings
    }

    pub fn google_calendar(&self) -> &Arc<GoogleCalendarHandle> {
        &self.google_calendar
    }

    /// Shell sessions shared by the server routes and the desktop app.
    pub fn terminals(&self) -> &TerminalManager {
        &self.terminals
    }

    /// Subscribe to server-injected meeting events. The SSE route calls this
    /// once per connection to receive events emitted by `emit_meeting_event`.
    pub fn subscribe_meeting_events(&self) -> broadcast::Receiver<MeetingEvent> {
        self.meeting_events_tx.subscribe()
    }

    /// How many meeting-event subscribers are connected.
    pub fn meeting_event_subscribers(&self) -> usize {
        self.meeting_events_tx.receiver_count()
    }

    /// Publish a server-injected meeting event to all SSE subscribers.
    pub fn emit_meeting_event(&self, event: MeetingEvent) {
        // Best effort by design: with no subscriber connected there is
        // nobody to tell, and reconnecting clients reload anyway.
        let _ = self.meeting_events_tx.send(event);
    }

    /// Create a new thread.
    pub async fn create_thread(&self, title: &str) -> Thread {
        let thread = new_thread(title.to_string());
        self.threads
            .write()
            .await
            .insert(thread.id.clone(), thread.clone());
        thread
    }

    /// Create a thread linked to a meeting, including a system prompt built
    /// from the meeting metadata and full transcript. The meeting is linked
    /// first; the thread exists only once that succeeded, so a failure
    /// leaves no orphan thread behind.
    pub async fn create_thread_for_meeting(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Thread, CreateThreadError> {
        let meeting = self
            .meetings
            .get_meeting(meeting_id.as_str())
            .await?
            .ok_or_else(|| CreateThreadError::MeetingNotFound(meeting_id.clone()))?;
        let segments = self.meetings.list_segments(meeting_id.as_str()).await?;

        let mut thread = new_thread(format!("Chat: {}", meeting.title));
        thread.meeting_id = Some(meeting_id.clone());
        thread.system_prompt = Some(build_meeting_chat_prompt(&meeting, &segments));

        self.meetings
            .patch_meeting_thread_id(meeting_id, &thread.id)
            .await?;
        self.threads
            .write()
            .await
            .insert(thread.id.clone(), thread.clone());
        Ok(thread)
    }

    /// Every thread, oldest first.
    pub async fn get_threads(&self) -> Vec<Thread> {
        let mut threads: Vec<Thread> = self.threads.read().await.values().cloned().collect();
        threads.sort_by(|a, b| (a.created_at, a.id.as_str()).cmp(&(b.created_at, b.id.as_str())));
        threads
    }

    /// The thread `id`, if this process created it.
    pub async fn thread(&self, id: &ThreadId) -> Option<Thread> {
        self.threads.read().await.get(id).cloned()
    }

    /// Answer a pending tool-permission prompt raised by a running turn.
    pub async fn resolve_permission(
        &self,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<(), PermissionResolveError> {
        self.orchestrator
            .resolve_permission(request_id, decision)
            .await
    }

    /// Start an agent turn. A thread created for a meeting carries that
    /// meeting's context as the system prompt.
    ///
    /// # Errors
    ///
    /// [`SendMessageError::ThreadBusy`] when the thread already has a turn
    /// in flight.
    pub async fn send_message(
        &self,
        turn: AgentTurn,
    ) -> Result<OrchestratorEventStream, SendMessageError> {
        let system_prompt = self
            .threads
            .read()
            .await
            .get(&turn.thread_id)
            .and_then(|t| t.system_prompt.clone());
        self.orchestrator
            .send_message(turn, system_prompt.as_deref())
    }

    /// Write `prompt` into the Claude turn streaming on `thread_id`.
    pub async fn steer(
        &self,
        thread_id: &ThreadId,
        prompt: &str,
    ) -> Result<(), crate::agent::SteerError> {
        self.orchestrator.steer(thread_id, prompt).await
    }

    /// Whether `thread_id` has a turn in flight.
    pub fn thread_status(&self, thread_id: &ThreadId) -> ThreadStatus {
        self.orchestrator.thread_status(thread_id)
    }

    /// Query the capture daemon for its current recording status, then flip
    /// any `Live` meetings that are NOT currently being recorded to `Failed`.
    ///
    /// Called on server startup. If the daemon is unreachable (still
    /// starting, not installed) the reconcile is skipped silently so we
    /// don't accidentally mark everything Failed just because the daemon
    /// hasn't come up yet.
    pub async fn reconcile_orphaned_meetings(&self) {
        let active_id: Option<String> = match crate::capture_client::fetch_status().await {
            Ok(status) => status.meeting_id,
            Err(e) => {
                tracing::info!(
                    "reconcile_orphaned_meetings: daemon unreachable ({e}), skipping reconcile"
                );
                return;
            }
        };

        let active_ref = active_id.as_deref();
        match mark_orphaned_live_as_failed(self.meetings.as_ref(), active_ref).await {
            Ok(0) => {}
            Ok(n) => tracing::info!("reconcile_orphaned_meetings: marked {n} meeting(s) Failed"),
            Err(e) => tracing::warn!("reconcile_orphaned_meetings: store error: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::MeetingRecords;
    use crate::meetings::SqliteMeetingStore;

    #[tokio::test]
    async fn test_create_thread() {
        let state = AppState::new();

        let thread = state.create_thread("My Thread").await;
        assert_eq!(thread.title, "My Thread");
        assert!(thread.project_path.is_none() && thread.meeting_id.is_none());

        let threads = state.get_threads().await;
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, thread.id);
        assert_eq!(state.thread(&thread.id).await, Some(thread));
    }

    #[tokio::test]
    async fn threads_list_oldest_first() {
        let state = AppState::new();
        let mut created = Vec::new();
        for title in ["a", "b", "c"] {
            created.push(state.create_thread(title).await);
        }
        let mut expected = created.clone();
        expected.sort_by(|a, b| (a.created_at, a.id.as_str()).cmp(&(b.created_at, b.id.as_str())));
        assert_eq!(state.get_threads().await, expected);
    }

    /// The JSON the server returns for a thread keeps the shape the old core
    /// `Thread` had: integer times, `null` project fields, optional fields
    /// omitted when unset.
    #[tokio::test]
    async fn thread_json_keeps_the_old_shape() {
        let state = AppState::new();
        let thread = state.create_thread("x").await;
        let json = serde_json::to_value(&thread).unwrap();
        let object = json.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "created_at",
                "id",
                "project_name",
                "project_path",
                "title",
                "updated_at"
            ]
        );
        assert!(json["created_at"].is_i64() && json["project_path"].is_null());
        let old = r#"{"id":"t-1","title":"x","project_path":null,"project_name":null,"created_at":1,"updated_at":2,"system_prompt":"p","meeting_id":"m-1"}"#;
        let back: Thread = serde_json::from_str(old).unwrap();
        assert_eq!(back.meeting_id.as_ref().map(MeetingId::as_str), Some("m-1"));
    }

    #[tokio::test]
    async fn create_thread_for_meeting_links_meeting() {
        let state = AppState::new();
        let m = Meeting::new("Standup", 0.0);
        let mid = MeetingId::try_from(m.id.clone()).unwrap();
        state.meetings().create_meeting(m).await.unwrap();

        let thread = state.create_thread_for_meeting(&mid).await.unwrap();
        assert_eq!(thread.meeting_id.as_ref(), Some(&mid));
        assert!(thread
            .system_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.contains("Standup")));

        let updated = state
            .meetings()
            .get_meeting(mid.as_str())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.thread_id.as_deref(), Some(thread.id.as_str()));

        let threads = state.get_threads().await;
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, thread.id);
    }

    #[tokio::test]
    async fn create_thread_for_meeting_missing_returns_error() {
        let state = AppState::new();
        let err = state
            .create_thread_for_meeting(&MeetingId::try_from("ghost-id").unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, CreateThreadError::MeetingNotFound(_)));
        assert!(err.to_string().contains("ghost-id"));
    }

    /// When linking the meeting fails, no thread may be left behind.
    #[tokio::test]
    async fn a_failed_meeting_link_leaves_no_orphan_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.sqlite3");
        let store = SqliteMeetingStore::open(&path).await.unwrap();
        let meeting = store
            .create_meeting(Meeting::new("Standup", 0.0))
            .await
            .unwrap();
        // Make every later meeting write fail.
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER refuse_meeting_writes BEFORE UPDATE ON meetings \
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        let state = AppState::builder().meetings(Arc::new(store)).build();

        let mid = MeetingId::try_from(meeting.id).unwrap();
        let err = state.create_thread_for_meeting(&mid).await.unwrap_err();
        assert!(matches!(err, CreateThreadError::MeetingStore(_)), "{err}");
        assert!(state.get_threads().await.is_empty());
    }

    #[test]
    fn meeting_prompt_lists_summary_and_transcript() {
        let mut meeting = Meeting::new("Standup", 0.0);
        meeting.summary = Some(bruhs_models::MeetingSummary {
            tldr: "Shipped it".into(),
            action_items: Vec::new(),
            decisions: vec!["Ship Friday".into()],
            topics: Vec::new(),
            generated_by: String::new(),
            generated_at: 0,
        });
        let prompt = build_meeting_chat_prompt(&meeting, &[]);
        assert!(prompt.contains("Meeting: Standup\n"));
        assert!(prompt.contains("\n## Summary\nShipped it\n"));
        assert!(prompt.contains("\n## Decisions\n- Ship Friday\n"));
        assert!(prompt.ends_with("\n## Transcript\n"));
    }
}
