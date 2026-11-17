//! The web side of the local MCP workspace connection
//! (`docs/local-mcp-workspace.md`, "Web connection").
//!
//! Agent workflows → Enable for this session binds a session at bruhs-server
//! to this tab's live stream (`bruhs_models::workspace_bridge::relay`). The
//! server writes the private connection file; the tab only copies its path.
//! Each relayed request is answered here, from this tab's workspace, with the
//! logic the native app answers with, after the grant check it requires.
//! Writes become previewed workflows that wait for the user in Agent
//! workflows; approved steps run through the workflow executor, whose
//! IndexedDB transactions save each record together with its checkpoint.
//!
//! The session ends when the user disables it, when another tab enables one,
//! and when this tab's live stream ends (the tab closed, reloaded or was
//! hidden, or the server stopped). Its review state goes with it, and nothing
//! reconnects by itself.

use std::collections::{BTreeMap, HashMap};

use bruhs_models::workspace_bridge::{
    self as bridge,
    relay::{DisableSession, EnableSession, RelayFrame, RelayReply, SessionEnabled},
    BridgeRequest, BridgeResponse, BridgeReviews, ChatMessageRecord, ChatThreadSummary,
    ChatToolCallRecord, ChatUsageRecord, Proposed, RollbackReview, WorkflowProposal,
};
use bruhs_models::{
    AgentDomain, AutomationState, BackgroundJobStatus, CalendarEvent, CaptureRecord,
    InboxConversation, KnowledgeDocument, LifeContext, Meeting, PersonProfile,
    PersonReminderRollback, RelationshipProfile, TranscriptSegment, TranscriptValidationError,
    WorkItem, WorkflowStatus, WorkflowStepStatus,
};
use leptos::prelude::*;
use serde_json::Value;

use crate::durable_metadata::DurableMetadata;
use crate::live_events::LiveEvents;
use crate::state::{AgentMessage, Thread, Turns};
use crate::workflow_executor::WorkflowExecutor;

/// A request that arrived before this tab finished loading what it reads.
const LOADING: &str =
    "This tab is still loading its workspace. Retry shortly; nothing was read or changed.";
/// Enabling needs this tab's live stream.
pub const UNAVAILABLE: &str = "bruhs-server is not connected to this tab, so the MCP connection is unavailable. It can be enabled once the server reconnects.";
const STREAM_RESTARTED: &str = "The MCP connection ended because this tab reconnected to bruhs-server. Enable it again and copy the new MCP config.";
const STREAM_LOST: &str = "The MCP connection ended because this tab lost its connection to bruhs-server or was hidden. Enable it again once the server is back.";
const REPLACED_ELSEWHERE: &str =
    "The session another tab held has ended; this tab now holds the MCP connection.";
const CANNOT_SAVE: &str =
    "This tab cannot save workflows right now. Review Agent workflows in the app; nothing was submitted.";

/// Where this tab's MCP connection stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Off,
    Starting,
    On {
        session_id: String,
        connection_path: String,
    },
    /// Enabling failed; nothing is connected.
    Failed(String),
}

#[derive(Clone, Copy)]
pub struct WorkspaceBridge {
    /// This tab's live stream, from its hello frame; `None` while it is down.
    stream_id: RwSignal<Option<String>>,
    pub connection: RwSignal<Connection>,
    /// Why the last session ended, until the user enables another.
    pub notice: RwSignal<Option<String>>,
    /// The session's proposals and undo requests, shared logic with desktop.
    pub reviews: RwSignal<BridgeReviews>,
    automation: DurableMetadata<AutomationState>,
    life: DurableMetadata<LifeContext>,
    executor: WorkflowExecutor,
    threads: ReadSignal<Vec<Thread>>,
    active: ReadSignal<Option<String>>,
    messages: ReadSignal<Vec<AgentMessage>>,
    turns: Turns,
    queue: RwSignal<crate::queue::PromptQueue>,
    saves: crate::task_saves::TaskSaves,
    work_items: ReadSignal<Vec<WorkItem>>,
    events: ReadSignal<Vec<CalendarEvent>>,
    captures: ReadSignal<Vec<CaptureRecord>>,
    documents: ReadSignal<Vec<KnowledgeDocument>>,
    conversations: ReadSignal<Vec<InboxConversation>>,
    people: ReadSignal<Vec<PersonProfile>>,
    relationships: ReadSignal<Vec<RelationshipProfile>>,
    relationship_store: crate::relationship_store::RelationshipStore,
    meetings: ReadSignal<Vec<Meeting>>,
    set_meetings: WriteSignal<Vec<Meeting>>,
    segments: ReadSignal<BTreeMap<String, Vec<TranscriptSegment>>>,
}

impl WorkspaceBridge {
    /// Needs the workflow executor and the chat, People and Meetings stores.
    pub fn install() {
        let crate::state::PromptQueueSignal(queue) = expect_context();
        let bridge = Self {
            stream_id: RwSignal::new(None),
            connection: RwSignal::new(Connection::Off),
            notice: RwSignal::new(None),
            reviews: RwSignal::new(BridgeReviews::default()),
            automation: expect_context::<crate::state::AutomationStateSignal>().0,
            life: expect_context::<crate::state::LifeContextSignal>().0,
            executor: expect_context(),
            threads: expect_context::<crate::state::ThreadsSignal>().0,
            active: expect_context::<crate::state::ActiveThreadId>().0,
            messages: expect_context::<crate::state::MessagesSignal>().0,
            turns: expect_context(),
            queue,
            saves: expect_context(),
            work_items: expect_context::<crate::state::WorkItemsSignal>().0,
            events: expect_context::<crate::state::CalendarEventsSignal>().0,
            captures: expect_context::<crate::state::CapturesSignal>().0,
            documents: expect_context::<crate::state::KnowledgeDocumentsSignal>().0,
            conversations: expect_context::<crate::state::InboxConversationsSignal>().0,
            people: expect_context::<crate::state::PeopleSignal>().0,
            relationships: expect_context::<crate::state::RelationshipsSignal>().0,
            relationship_store: expect_context(),
            meetings: expect_context::<crate::state::MeetingsSignal>().0,
            set_meetings: expect_context::<crate::state::SetMeetingsSignal>().0,
            segments: expect_context::<crate::state::MeetingSegmentsSignal>().0,
        };
        provide_context(bridge);
        // An undo the user saves, through either Undo button, settles that
        // workflow's undo request, as on desktop.
        Effect::new(move |_| {
            let settled = bridge.automation.with(|state| {
                bridge
                    .reviews
                    .with(|reviews| rolled_back_reviews(reviews, state))
            });
            if !settled.is_empty() {
                bridge.reviews.update(|reviews| {
                    for id in &settled {
                        reviews.finish_rollback_review(id);
                    }
                });
            }
        });
    }

    /// Listen for this tab's relay frames on the live stream.
    pub fn subscribe(self, live: &mut LiveEvents) {
        live.on(
            bruhs_models::api::live_events::WORKSPACE_BRIDGE,
            move |data| match serde_json::from_str::<RelayFrame>(&data) {
                Ok(frame) => self.receive(frame),
                Err(error) => log::warn!("unreadable workspace relay frame: {error}"),
            },
        );
        live.on_lost(move || self.stream_lost());
    }

    /// Whether a session can be enabled now: the live stream is open. Tracked.
    pub fn available(self) -> bool {
        self.stream_id.with(Option::is_some)
    }

    fn session_id_untracked(self) -> Option<String> {
        match self.connection.get_untracked() {
            Connection::On { session_id, .. } => Some(session_id),
            _ => None,
        }
    }

    fn receive(self, frame: RelayFrame) {
        match frame {
            RelayFrame::Hello { stream_id } => {
                self.stream_id.set(Some(stream_id));
                // A new stream never carries a session over.
                if matches!(
                    self.connection.get_untracked(),
                    Connection::On { .. } | Connection::Starting
                ) {
                    self.end(Some(STREAM_RESTARTED));
                }
            }
            RelayFrame::Request {
                session_id,
                request_id,
                request,
            } => {
                if self.session_id_untracked().as_deref() != Some(session_id.as_str()) {
                    return;
                }
                let Some(stream_id) = self.stream_id.get_untracked() else {
                    return;
                };
                leptos::task::spawn_local(async move {
                    let response = self.answer(request).await;
                    let reply = RelayReply {
                        stream_id,
                        session_id,
                        request_id,
                        response,
                    };
                    if let Err(error) = crate::api::reply_workspace_request(&reply).await {
                        // The call timed out or the session ended; nobody waits.
                        log::warn!("workspace request answer was not delivered: {error}");
                    }
                });
            }
            RelayFrame::Revoked { session_id, reason } => {
                if self.session_id_untracked().as_deref() == Some(session_id.as_str()) {
                    self.end(Some(reason.message()));
                }
            }
        }
    }

    fn stream_lost(self) {
        self.stream_id.set(None);
        if matches!(
            self.connection.get_untracked(),
            Connection::On { .. } | Connection::Starting
        ) {
            self.end(Some(STREAM_LOST));
        }
    }

    /// Review state is per session: it never outlives one.
    fn end(self, notice: Option<&str>) {
        self.connection.set(Connection::Off);
        self.reviews.set(BridgeReviews::default());
        self.notice.set(notice.map(str::to_owned));
    }

    /// Enable for this session.
    pub fn enable(self) {
        if matches!(
            self.connection.get_untracked(),
            Connection::Starting | Connection::On { .. }
        ) {
            return;
        }
        let Some(stream_id) = self.stream_id.get_untracked() else {
            self.connection.set(Connection::Failed(UNAVAILABLE.into()));
            return;
        };
        self.connection.set(Connection::Starting);
        self.notice.set(None);
        leptos::task::spawn_local(async move {
            let enabled = crate::api::enable_workspace_session(&EnableSession {
                stream_id: stream_id.clone(),
            })
            .await;
            match enabled {
                Ok(SessionEnabled {
                    session_id,
                    connection_path,
                    replaced,
                }) => {
                    // Disabled, or the stream changed, while this was in flight:
                    // the new session must not linger on the server.
                    if self.connection.get_untracked() != Connection::Starting
                        || self.stream_id.get_untracked().as_deref() != Some(stream_id.as_str())
                    {
                        let body = DisableSession {
                            stream_id,
                            session_id,
                        };
                        if let Err(error) = crate::api::disable_workspace_session(&body).await {
                            log::warn!("a superseded MCP session was not revoked: {error}");
                        }
                        return;
                    }
                    self.reviews.set(BridgeReviews::default());
                    self.connection.set(Connection::On {
                        session_id,
                        connection_path,
                    });
                    if replaced {
                        self.notice.set(Some(REPLACED_ELSEWHERE.into()));
                    }
                    self.warm_meetings();
                }
                Err(error) => {
                    log::warn!("MCP session could not start: {error}");
                    self.connection
                        .set(Connection::Failed(bridge::CONNECTION_FAILED_NOTE.into()));
                }
            }
        });
    }

    /// Disable the connection; every client loses it.
    pub fn disable(self) {
        let previous = self.connection.get_untracked();
        self.end(None);
        let (Connection::On { session_id, .. }, Some(stream_id)) =
            (previous, self.stream_id.get_untracked())
        else {
            return;
        };
        leptos::task::spawn_local(async move {
            let body = DisableSession {
                stream_id,
                session_id,
            };
            if let Err(error) = crate::api::disable_workspace_session(&body).await {
                // Closing the stream revokes it too; the server never keeps
                // a session its tab has left.
                log::warn!("MCP session revoke was not delivered: {error}");
            }
        });
    }

    /// Copy MCP config: the file's path, never the token.
    pub fn copy_config(self) -> bool {
        let Connection::On {
            connection_path, ..
        } = self.connection.get_untracked()
        else {
            return false;
        };
        crate::ui::copy_to_clipboard(&bridge::mcp_client_config(&connection_path).to_string());
        true
    }

    /// The web loads meetings when their page opens. A session reads the
    /// same cache, so fill it once the way the page does (server, then the
    /// saved copy) instead of answering "not loaded" for every meeting.
    fn warm_meetings(self) {
        if !self.meetings.with_untracked(Vec::is_empty) {
            return;
        }
        leptos::task::spawn_local(async move {
            if let Ok(fetched) = crate::api::fetch_meetings().await {
                if !fetched.is_empty() {
                    if self.meetings.with_untracked(Vec::is_empty) {
                        self.set_meetings.set(fetched);
                    }
                    return;
                }
            }
            if let Ok(local) = crate::storage::load_meetings().await {
                if self.meetings.with_untracked(Vec::is_empty) {
                    self.set_meetings.set(local);
                }
            }
        });
    }

    /// The workflow whose review the session waits on. Tracked.
    pub fn pending_review_id(self) -> Option<String> {
        self.reviews.with(|reviews| {
            self.automation
                .with(|state| reviews.pending_review_id(state).map(str::to_owned))
        })
    }

    /// Whether the session proposed `workflow_id`. Tracked.
    pub fn owns(self, workflow_id: &str) -> bool {
        self.reviews
            .with(|reviews| reviews.proposals.values().any(|(id, _)| id == workflow_id))
    }

    /// Whether an undo request for `workflow_id` waits on the user. Tracked.
    pub fn rollback_pending(self, workflow_id: &str) -> bool {
        self.reviews
            .with(|reviews| reviews.rollback_pending(workflow_id))
            && self.automation.with(|state| {
                state.workflows.iter().any(|workflow| {
                    workflow.id == workflow_id && workflow.status != WorkflowStatus::RolledBack
                })
            })
    }

    /// Keep changes: decline the undo request. Records stay as they are.
    pub fn keep_changes(self, workflow_id: &str) {
        self.reviews.update(|reviews| {
            reviews.decline_rollback(workflow_id);
        });
    }

    fn policy<R>(self, read: impl FnOnce(&bruhs_models::AgentPermissionPolicy) -> R) -> R {
        self.automation
            .with_untracked(|state| read(&state.permissions))
    }

    fn require(self, ready: bool) -> Result<(), String> {
        if ready {
            Ok(())
        } else {
            Err(LOADING.into())
        }
    }

    /// One request, answered from this tab's current workspace.
    async fn answer(self, request: BridgeRequest) -> BridgeResponse {
        if !self.automation.ready_untracked() {
            return BridgeResponse::Error(LOADING.into());
        }
        match request {
            BridgeRequest::ReadChatMessages { thread_id, offset } => {
                self.read_chat_messages(&thread_id, offset).await.into()
            }
            BridgeRequest::RequestRollback {
                workflow_id,
                dry_run,
            } => self.request_rollback(&workflow_id, dry_run).await.into(),
            request => self.answer_now(request).into(),
        }
    }

    fn answer_now(self, request: BridgeRequest) -> Result<Value, String> {
        match request {
            BridgeRequest::ReadFinance {
                section,
                query,
                offset,
            } => {
                self.policy(|policy| bridge::require_read(policy, AgentDomain::Finance))?;
                self.require(self.life.ready_untracked())?;
                self.life.with_untracked(|life| {
                    self.policy(|policy| {
                        bridge::read_finance(policy, life, section, &query, offset)
                    })
                })
            }
            BridgeRequest::ReadHealth {
                section,
                query,
                offset,
            } => {
                self.policy(|policy| bridge::require_read(policy, AgentDomain::Health))?;
                self.require(self.life.ready_untracked())?;
                self.life.with_untracked(|life| {
                    self.policy(|policy| bridge::read_health(policy, life, section, &query, offset))
                })
            }
            BridgeRequest::ReadMeetingNotes { query, offset } => {
                self.policy(|policy| bridge::require_read(policy, AgentDomain::Meetings))?;
                self.require(self.executor.ready.get_untracked())?;
                self.executor.meeting_notes.with_untracked(|notes| {
                    self.policy(|policy| bridge::read_meeting_notes(policy, notes, &query, offset))
                })
            }
            BridgeRequest::ReadMeetingNote { note_id } => {
                self.policy(|policy| bridge::require_read(policy, AgentDomain::Meetings))?;
                self.require(self.executor.ready.get_untracked())?;
                self.executor.meeting_notes.with_untracked(|notes| {
                    self.policy(|policy| bridge::read_meeting_note(policy, notes, &note_id))
                })
            }
            BridgeRequest::ReadMeeting {
                meeting_id,
                section,
                offset,
            } => self.meetings.with_untracked(|meetings| {
                self.segments.with_untracked(|segments| {
                    self.policy(|policy| {
                        bridge::read_meeting(
                            policy,
                            meetings.iter(),
                            |id| segments.get(id).map(Vec::as_slice),
                            |id| validate_segments(segments, id),
                            &meeting_id,
                            section,
                            offset,
                        )
                    })
                })
            }),
            BridgeRequest::ReadPerson {
                person_id,
                section,
                offset,
            } => {
                // Grant first: a loading tab must not reveal anything either.
                self.policy(|policy| bridge::require_read(policy, AgentDomain::People))?;
                self.require(self.people_ready())?;
                self.people.with_untracked(|people| {
                    self.relationships.with_untracked(|relationships| {
                        self.policy(|policy| {
                            bridge::read_person(
                                policy,
                                people.iter(),
                                relationships.iter(),
                                &person_id,
                                section,
                                offset,
                            )
                        })
                    })
                })
            }
            BridgeRequest::Read {
                domain,
                query,
                offset,
            } => {
                self.policy(|policy| bridge::require_read(policy, domain))?;
                self.read_domain(domain, &query, offset)
            }
            BridgeRequest::Preview { operation } => {
                self.policy(|policy| bridge::preview_operation(policy, &operation))
            }
            BridgeRequest::PreviewWorkflow { proposal } => {
                self.policy(|policy| bridge::preview_workflow(policy, &proposal))
            }
            BridgeRequest::Propose {
                request_id,
                operation,
            } => self.propose(request_id, WorkflowProposal::single(operation)),
            BridgeRequest::ProposeWorkflow {
                request_id,
                proposal,
            } => self.propose(request_id, proposal),
            BridgeRequest::Status { workflow_id } => self.status(&workflow_id),
            // Answered in `answer`, which may wait on IndexedDB.
            BridgeRequest::ReadChatMessages { .. } | BridgeRequest::RequestRollback { .. } => {
                Err("This request is answered elsewhere.".into())
            }
        }
    }

    fn people_ready(self) -> bool {
        self.executor.people_hydrated.get_untracked()
            && self.relationship_store.hydrated_untracked()
    }

    /// The generic domain reads, after the caller checked the Read grant.
    fn read_domain(self, domain: AgentDomain, query: &str, offset: usize) -> Result<Value, String> {
        let records = self.executor.ready.get_untracked();
        match domain {
            AgentDomain::Chat => {
                self.require(self.executor.threads_hydrated.get_untracked())?;
                self.threads.with_untracked(|threads| {
                    bridge::read_chat_threads(
                        threads.iter(),
                        |thread| {
                            let thread = *thread;
                            ChatThreadSummary {
                                id: &thread.id,
                                title: &thread.title,
                                project_name: &thread.project_name,
                                updated_at_ms: thread.updated_at.as_millis(),
                                pinned: thread.pinned,
                                settled: thread.settled,
                                streaming: self.turns.is_running_untracked(&thread.id),
                            }
                        },
                        query,
                        offset,
                    )
                })
            }
            AgentDomain::Meetings => self
                .meetings
                .with_untracked(|meetings| bridge::read_meetings(meetings.iter(), query, offset)),
            AgentDomain::People => {
                self.require(self.executor.people_hydrated.get_untracked())?;
                self.people
                    .with_untracked(|people| bridge::read_people(people.iter(), query, offset))
            }
            AgentDomain::Finance => self.answer_now(BridgeRequest::ReadFinance {
                section: Default::default(),
                query: query.into(),
                offset,
            }),
            AgentDomain::Health => self.answer_now(BridgeRequest::ReadHealth {
                section: Default::default(),
                query: query.into(),
                offset,
            }),
            AgentDomain::Tasks => {
                self.require(records)?;
                self.work_items
                    .with_untracked(|items| bridge::read_records(items, query, offset))
            }
            AgentDomain::Calendar => {
                self.require(records)?;
                self.events
                    .with_untracked(|events| bridge::read_records(events, query, offset))
            }
            AgentDomain::Capture => {
                self.require(records)?;
                self.captures
                    .with_untracked(|captures| bridge::read_records(captures, query, offset))
            }
            AgentDomain::Knowledge => {
                self.require(records)?;
                self.documents
                    .with_untracked(|documents| bridge::read_records(documents, query, offset))
            }
            AgentDomain::Inbox => {
                self.require(records)?;
                self.conversations
                    .with_untracked(|items| bridge::read_records(items, query, offset))
            }
        }
    }

    /// A thread's completed messages, wherever their newest copy is: its
    /// running turn, the open transcript, an unsaved edit, or IndexedDB.
    async fn read_chat_messages(self, thread_id: &str, offset: usize) -> Result<Value, String> {
        self.policy(|policy| bridge::authorize_chat_messages(policy, thread_id, offset))?;
        self.require(self.executor.threads_hydrated.get_untracked())?;
        if !self
            .threads
            .with_untracked(|threads| threads.iter().any(|thread| thread.id == thread_id))
        {
            return Err(bridge::CHAT_THREAD_NOT_FOUND.into());
        }
        let messages = self.settled_messages(thread_id).await?;
        // The grant is checked again: it may have been revoked during the load.
        self.policy(|policy| bridge::authorize_chat_messages(policy, thread_id, offset))?;
        bridge::read_chat_messages(
            messages.iter().filter(|message| !message.is_streaming()),
            message_record,
            thread_id,
            offset,
        )
    }

    async fn settled_messages(self, id: &str) -> Result<Vec<AgentMessage>, String> {
        if let Some(messages) = self.turns.messages(id) {
            return Ok(messages);
        }
        if self
            .active
            .with_untracked(|active| active.as_deref() == Some(id))
        {
            return Ok(self.messages.get_untracked());
        }
        match crate::storage::load_messages(id).await {
            Ok(loaded) => Ok(self.saves.loaded_conversation(id, loaded)),
            Err(error) => {
                log::warn!("MCP chat read could not load saved messages: {error:?}");
                Err(
                    "That thread's saved messages could not be loaded. No records were changed."
                        .into(),
                )
            }
        }
    }

    fn status(self, workflow_id: &str) -> Result<Value, String> {
        let durable = self.automation.settled_untracked();
        self.reviews.with_untracked(|reviews| {
            self.automation
                .with_untracked(|state| reviews.status(state, durable, workflow_id))
        })
    }

    /// Admit a proposal for review. It is saved like any workflow edit, so
    /// status stays provisional until IndexedDB acknowledges it.
    fn propose(self, request_id: String, proposal: WorkflowProposal) -> Result<Value, String> {
        let saved = self.automation.settled_untracked();
        let now = crate::clock::now_ms();
        let admitted = self.automation.try_update(|state| {
            self.reviews
                .try_update(|reviews| reviews.propose(state, saved, request_id, proposal, now))
        });
        let (Proposed::Existing(id) | Proposed::Created(id)) = match admitted {
            Some(Some(result)) => result?,
            _ => return Err(CANNOT_SAVE.into()),
        };
        self.status(&id)
    }

    /// `bruhs_request_workflow_rollback`: preview eligibility, or open an undo
    /// review. Never deletes anything; the user's Undo does that.
    async fn request_rollback(self, workflow_id: &str, dry_run: bool) -> Result<Value, String> {
        let begin = |bridge: Self| {
            let saved = bridge.automation.settled_untracked();
            bridge.reviews.with_untracked(|reviews| {
                bridge.automation.with_untracked(|state| {
                    reviews.begin_rollback_request(state, saved, workflow_id)
                })
            })
        };
        let request = begin(self)?;
        let completed_steps = if request.needs_targets() {
            self.rollback_targets(workflow_id).await?
        } else {
            0
        };
        // Loading chat history may have yielded; nothing may have changed
        // ownership, grants or the save meanwhile.
        let request = begin(self)?;
        let completed_steps = if request.needs_targets() {
            completed_steps
        } else {
            0
        };
        self.automation
            .with_untracked(|state| {
                self.reviews.try_update(|reviews| {
                    reviews.finish_rollback_request(
                        state,
                        request,
                        workflow_id,
                        dry_run,
                        completed_steps,
                    )
                })
            })
            .ok_or_else(|| CANNOT_SAVE.to_owned())?
            .map(|(response, _opened)| response)
    }

    /// How many completed steps an undo would revert now: the same checks the
    /// executor repeats inside its IndexedDB transaction, from this tab's state.
    async fn rollback_targets(self, workflow_id: &str) -> Result<usize, String> {
        if self.automation.with_untracked(|state| {
            state.jobs.iter().any(|job| {
                job.workflow_id == workflow_id
                    && job.auto_run
                    && job.status == BackgroundJobStatus::Running
            })
        }) {
            return Err("Pause the background workflow in the app before reviewing undo. No records were changed.".into());
        }
        self.require(self.executor.ready.get_untracked())?;
        let steps = self
            .automation
            .with_untracked(|state| {
                state
                    .workflows
                    .iter()
                    .find(|workflow| workflow.id == workflow_id)
                    .map(|workflow| workflow.steps.clone())
            })
            .ok_or("Workflow no longer exists.")?;
        let completed = |domain: AgentDomain| {
            steps.iter().any(|step| {
                step.operation.domain() == domain && step.status == WorkflowStepStatus::Completed
            })
        };
        if completed(AgentDomain::Chat) {
            self.require(self.executor.threads_hydrated.get_untracked())?;
        }
        if completed(AgentDomain::People) {
            self.require(self.people_ready())?;
        }
        if (completed(AgentDomain::Finance) || completed(AgentDomain::Health))
            && !self.life.ready_untracked()
        {
            return Err(LOADING.into());
        }
        // A created chat must still be empty, closed, idle and unqueued.
        let mut untouched_chats = HashMap::new();
        for step in &steps {
            if step.operation.domain() != AgentDomain::Chat
                || step.status != WorkflowStepStatus::Completed
            {
                continue;
            }
            if let Some(id) = step.target_id.as_deref() {
                let untouched = self.chat_untouched(id).await;
                untouched_chats.insert(id.to_owned(), untouched);
            }
        }
        let people = self
            .relationships
            .with_untracked(|profiles| PersonReminderRollback::prepare(&steps, profiles))
            .map_err(|error| error.message().to_owned())?;
        let targets = self.automation.with_untracked(|state| {
            state.rollback_targets(workflow_id, |domain, id| {
                self.current_snapshot(domain, id, &people, &untouched_chats)
            })
        });
        targets
            .map(|targets| targets.len())
            .map_err(|error| error.message().to_owned())
    }

    async fn chat_untouched(self, id: &str) -> bool {
        let busy = self.active.get_untracked().as_deref() == Some(id)
            || self.turns.is_busy_untracked(id)
            || self.queue.with_untracked(|queue| queue.has_for(id));
        if busy {
            return false;
        }
        match self.settled_messages(id).await {
            Ok(messages) => messages.is_empty(),
            Err(_) => false,
        }
    }

    /// The current saved form of a record a workflow created, as its receipt
    /// recorded it, or `None` if it changed, went away or is in use.
    fn current_snapshot(
        self,
        domain: AgentDomain,
        id: &str,
        people: &PersonReminderRollback,
        untouched_chats: &HashMap<String, bool>,
    ) -> Option<String> {
        fn encoded<T: serde::Serialize>(
            records: &[T],
            matches: impl Fn(&T) -> bool,
        ) -> Option<String> {
            let mut found = records.iter().filter(|record| matches(record));
            let record = found.next()?;
            if found.next().is_some() {
                return None;
            }
            serde_json::to_string(record).ok()
        }
        match domain {
            AgentDomain::People => people.snapshot(id),
            AgentDomain::Chat => {
                if !untouched_chats.get(id).copied().unwrap_or(false) {
                    return None;
                }
                self.threads
                    .with_untracked(|threads| encoded(threads, |thread| thread.id == id))
            }
            AgentDomain::Tasks => self
                .work_items
                .with_untracked(|items| encoded(items, |item| item.id == id)),
            AgentDomain::Calendar => self
                .events
                .with_untracked(|events| encoded(events, |event| event.id == id)),
            AgentDomain::Capture => self
                .captures
                .with_untracked(|captures| encoded(captures, |capture| capture.id == id)),
            AgentDomain::Knowledge => self
                .documents
                .with_untracked(|documents| encoded(documents, |document| document.id == id)),
            AgentDomain::Inbox => self
                .conversations
                .with_untracked(|items| encoded(items, |item| item.id == id)),
            AgentDomain::Meetings => self
                .executor
                .meeting_notes
                .with_untracked(|notes| encoded(notes, |note| note.id == id)),
            AgentDomain::Finance | AgentDomain::Health => self
                .life
                .with_untracked(|life| crate::storage::workflow::life_snapshot(life, domain, id)),
        }
    }
}

/// Pending undo requests whose workflow the user has already undone.
fn rolled_back_reviews(reviews: &BridgeReviews, state: &AutomationState) -> Vec<String> {
    reviews
        .rollback_reviews
        .iter()
        .filter(|(_, review)| **review == RollbackReview::Pending)
        .filter(|(id, _)| {
            state.workflows.iter().any(|workflow| {
                workflow.id == **id && workflow.status == WorkflowStatus::RolledBack
            })
        })
        .map(|(id, _)| id.clone())
        .collect()
}

fn validate_segments(
    segments: &BTreeMap<String, Vec<TranscriptSegment>>,
    meeting_id: &str,
) -> Result<(), TranscriptValidationError> {
    let records = segments
        .get(meeting_id)
        .ok_or(TranscriptValidationError::InvalidSegments)?;
    bruhs_models::validate_transcript_records(meeting_id, records)
}

fn epoch_ms(ms: f64) -> i64 {
    bruhs_models::EpochMs::saturating_from_f64(ms).as_millis()
}

/// A web message in the record shape desktop returns. The web keeps no
/// per-message stop marker, so `stopped` is false; the duration is the
/// time from start to finish when both are known.
fn message_record(message: &AgentMessage) -> ChatMessageRecord<'_> {
    let started = message.started_at_ms.map(epoch_ms);
    let finished = message.status.finished_at_ms().map(epoch_ms);
    ChatMessageRecord {
        role: message.role.as_str(),
        content: message.text().into(),
        model: message.model.as_deref(),
        thinking: message.thinking().into(),
        usage: message.usage.as_ref().map(|usage| ChatUsageRecord {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_input_tokens: usage.cache_read_tokens,
            cache_creation_input_tokens: usage.cache_creation_tokens,
            context_tokens: usage.context_tokens,
        }),
        duration_ms: started
            .zip(finished)
            .and_then(|(started, finished)| u64::try_from(finished.checked_sub(started)?).ok()),
        started_at_ms: started,
        completed_at_ms: finished,
        tool_calls: message
            .tool_calls_iter()
            .map(|call| ChatToolCallRecord {
                id: &call.id,
                tool: &call.tool,
                input: &call.input,
                output: call.output.as_deref(),
                is_error: call.is_error,
            })
            .collect(),
        stopped: false,
        error: message.error.as_deref(),
    }
}

#[cfg(test)]
mod tests;
