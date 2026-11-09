//! App-owned executor: survives route changes; reload resumes only opted-in jobs.

use crate::durable_metadata::DurableMetadata;
use crate::state::{
    AutomationStateSignal, LifeContextSignal, SetCalendarEventsSignal, SetCapturesSignal,
    SetInboxConversationsSignal, SetKnowledgeDocumentsSignal, SetProjectsSignal,
    SetSourceClipsSignal, SetWorkItemsSignal, Thread,
};
use crate::storage::{
    self,
    metadata::{Metadata, SaveError},
    workflow::{is_life_domain, rollback_needs_life, CreatedRecord},
};
use bruhs_models::{AgentDomain, AutomationState, BackgroundJobStatus, LifeContext};
use leptos::prelude::*;
mod contact_events;
mod contact_merge;
mod contact_reconcile;
mod contact_sync;
pub mod note_editor;
mod people;
mod people_cache;

#[derive(Clone, Copy)]
pub struct WorkflowExecutor {
    automation: DurableMetadata<AutomationState>,
    life: DurableMetadata<LifeContext>,
    tasks: SetWorkItemsSignal,
    events: SetCalendarEventsSignal,
    captures: SetCapturesSignal,
    knowledge: SetKnowledgeDocumentsSignal,
    sources: SetSourceClipsSignal,
    projects: SetProjectsSignal,
    inbox: SetInboxConversationsSignal,
    pub meeting_notes: RwSignal<Vec<bruhs_models::MeetingNote>>,
    pub note_editor: note_editor::NoteEditor,
    threads: WriteSignal<Vec<Thread>>,
    active_thread: ReadSignal<Option<String>>,
    streaming_thread: crate::state::StreamingThreadId,
    prompt_queue: crate::state::PromptQueueSignal,
    relationships: crate::relationship_store::RelationshipStore,
    people: crate::state::PeopleSignal,
    set_people: crate::state::SetPeopleSignal,
    pub people_hydrated: RwSignal<bool>,
    pub people_cache_error: RwSignal<Option<String>>,
    pub people_refresh_error: RwSignal<Option<String>>,
    people_loading: RwSignal<bool>,
    contact_syncing: RwSignal<bool>,
    contact_sync_again: RwSignal<bool>,
    contact_merging: RwSignal<bool>,
    people_refreshing: RwSignal<bool>,
    people_refresh_again: RwSignal<bool>,
    people_refresh_deferred: RwSignal<bool>,
    person_refreshes: RwSignal<std::collections::BTreeMap<String, i64>>,
    person_refresh_running: RwSignal<bool>,
    pub merge_record: RwSignal<Option<storage::people::merge::Record>>,
    pub merge_error: RwSignal<Option<String>>,
    merge_loaded: RwSignal<bool>,
    pub contact_sync_error: RwSignal<Option<String>>,
    banners: crate::components::banner::BannerStackSignal,
    pub threads_hydrated: RwSignal<bool>,
    pub ready: RwSignal<bool>,
    pub loading: RwSignal<bool>,
    pub load_error: RwSignal<Option<String>>,
}

impl WorkflowExecutor {
    pub fn install() {
        let executor = Self {
            automation: expect_context::<AutomationStateSignal>().0,
            life: expect_context::<LifeContextSignal>().0,
            tasks: expect_context(),
            events: expect_context(),
            captures: expect_context(),
            knowledge: expect_context(),
            sources: expect_context(),
            projects: expect_context(),
            inbox: expect_context(),
            meeting_notes: RwSignal::new(Vec::new()),
            note_editor: note_editor::NoteEditor::new(),
            threads: expect_context::<crate::state::SetThreadsSignal>().0,
            active_thread: expect_context::<crate::state::ActiveThreadId>().0,
            streaming_thread: expect_context(),
            prompt_queue: expect_context(),
            relationships: expect_context(),
            people: expect_context(),
            set_people: expect_context(),
            people_hydrated: RwSignal::new(false),
            people_cache_error: RwSignal::new(None),
            people_refresh_error: RwSignal::new(None),
            people_loading: RwSignal::new(false),
            contact_syncing: RwSignal::new(false),
            contact_sync_again: RwSignal::new(false),
            contact_merging: RwSignal::new(false),
            people_refreshing: RwSignal::new(false),
            people_refresh_again: RwSignal::new(false),
            people_refresh_deferred: RwSignal::new(false),
            person_refreshes: RwSignal::new(Default::default()),
            person_refresh_running: RwSignal::new(false),
            merge_record: RwSignal::new(None),
            merge_error: RwSignal::new(None),
            merge_loaded: RwSignal::new(false),
            contact_sync_error: RwSignal::new(None),
            banners: expect_context(),
            threads_hydrated: RwSignal::new(false),
            ready: RwSignal::new(false),
            loading: RwSignal::new(false),
            load_error: RwSignal::new(None),
        };
        provide_context(executor);
        executor.load_merge();
        executor.load();
        let task_saves = expect_context::<crate::task_saves::TaskSaves>();
        Effect::new(move |_| {
            if task_saves.contacts_committed.get() != 0 {
                executor.retry_contact_sync();
            }
        });
        Effect::new(move |_| {
            if executor.people_refresh_deferred.get()
                && executor.people_hydrated.get()
                && executor.merge_loaded.get()
                && executor.relationships.hydrated()
                && !executor.contact_syncing.get()
                && !executor.contact_merging.get()
            {
                executor.people_refresh_deferred.set(false);
                wasm_bindgen_futures::spawn_local(async move {
                    executor.refresh_people().await;
                });
            }
        });
        Effect::new(move |_| {
            if !executor.ready.get() || !executor.automation.settled() {
                return;
            }
            let next = executor.automation.with(|state| {
                state
                    .jobs
                    .iter()
                    .find(|job| job.auto_run && job.status == BackgroundJobStatus::Running)
                    .map(|job| job.id.clone())
            });
            if let Some(id) = next {
                let needs_threads = executor.automation.with(|state| {
                    state
                        .pending_operation(&id)
                        .is_some_and(|operation| operation.domain() == AgentDomain::Chat)
                });
                if needs_threads && !executor.threads_hydrated.get() {
                    return;
                }
                let needs_life = executor.automation.with(|state| {
                    state
                        .pending_operation(&id)
                        .is_some_and(|operation| is_life_domain(operation.domain()))
                });
                if needs_life && !executor.life.settled() {
                    return;
                }
                let needs_people = executor.automation.with(|state| {
                    state
                        .pending_operation(&id)
                        .is_some_and(|operation| operation.domain() == AgentDomain::People)
                });
                if needs_people
                    && (!executor.people_hydrated.get() || !executor.relationships.settled())
                {
                    return;
                }
                if let Err(error) = executor.run(id) {
                    executor.automation.report(error);
                }
            }
        });
    }

    pub fn load(self) {
        // This is initial hydration/Retry, not a refresh that may replace live
        // edits. Repeated Retry clicks must not publish competing snapshots.
        if self.loading.get_untracked() || self.ready.get_untracked() {
            return;
        }
        self.loading.set(true);
        self.load_error.set(None);
        let task_saves = expect_context::<crate::task_saves::TaskSaves>();
        wasm_bindgen_futures::spawn_local(async move {
            let (tasks, events, captures, knowledge, inbox, notes, sources, projects) = futures_util::join!(
                storage::tasks::load(),
                storage::load_events(),
                storage::load_captures(),
                storage::load_knowledge_documents(),
                storage::load_inbox_conversations(),
                storage::load_meeting_notes(),
                storage::load_source_clips(),
                storage::load_projects()
            );
            // Drain every request before reporting an error; dropping pending
            // IDB futures can leave their browser callbacks invalidated.
            match tasks.and_then(|tasks| {
                Ok((
                    tasks, events?, captures?, knowledge?, inbox?, notes?, sources?, projects?,
                ))
            }) {
                Ok((tasks, events, captures, knowledge, inbox, notes, sources, projects)) => {
                    let (tasks, captures) = task_saves.finish_loading(tasks, captures);
                    self.tasks.0.set(tasks);
                    self.events.0.set(task_saves.finish_loading_events(events));
                    self.captures.0.set(captures);
                    self.knowledge
                        .0
                        .set(task_saves.finish_loading_documents(knowledge));
                    self.inbox.0.set(task_saves.finish_loading_inbox(inbox));
                    self.meeting_notes.set(notes);
                    self.sources
                        .0
                        .set(task_saves.finish_loading_sources(sources));
                    self.projects
                        .0
                        .set(task_saves.finish_loading_projects(projects));
                    self.ready.set(true);
                }
                Err(error) => self.load_error.set(Some(format!(
                    "Local workflow records could not be loaded. No workflow has run. ({error})"
                ))),
            }
            self.loading.set(false);
        });
    }

    pub fn run(self, job_id: String) -> Result<(), SaveError> {
        if !self.ready.get_untracked() {
            return Err(SaveError::Unavailable);
        }
        if !self.threads_hydrated.get_untracked()
            && self.automation.with(|state| {
                state
                    .pending_operation(&job_id)
                    .is_some_and(|operation| operation.domain() == AgentDomain::Chat)
            })
        {
            return Err(SaveError::Rejected("Chat history has not loaded. Reload to retry before creating a workflow chat. No records were changed."));
        }
        let mut relationship_tx = self.prepare_person_step(&job_id)?;
        let (expected, expected_life) = self.begin(|state| {
            state
                .pending_operation(&job_id)
                .is_some_and(|operation| is_life_domain(operation.domain()))
        })?;
        wasm_bindgen_futures::spawn_local(async move {
            match storage::workflow::execute_step(
                &expected,
                expected_life.as_ref(),
                relationship_tx
                    .as_ref()
                    .map(|transaction| transaction.expected()),
                &job_id,
                crate::clock::now_ms(),
            )
            .await
            {
                Ok(commit) => {
                    match commit.result {
                        CreatedRecord::Thread(thread) => {
                            self.threads.update(|items| items.push(*thread))
                        }
                        CreatedRecord::Task(item) => self.tasks.0.update(|items| items.push(*item)),
                        CreatedRecord::Event(event) => {
                            self.events.0.update(|events| events.push(*event))
                        }
                        CreatedRecord::Capture(capture) => {
                            self.captures.0.update(|captures| captures.push(*capture))
                        }
                        CreatedRecord::Knowledge(document) => {
                            self.knowledge.0.update(|items| items.insert(0, *document))
                        }
                        CreatedRecord::Inbox(conversation) => {
                            self.inbox.0.update(|items| items.insert(0, *conversation))
                        }
                        CreatedRecord::Finance(_) | CreatedRecord::Workout(_) => {}
                        CreatedRecord::MeetingNote(note) => {
                            self.meeting_notes.update(|notes| notes.insert(0, *note))
                        }
                        CreatedRecord::Relationship(profile) => {
                            if let Some(transaction) = relationship_tx.take() {
                                transaction.created(*profile);
                            }
                        }
                    }
                    if let Some(life) = commit.life {
                        self.life.committed(life);
                    }
                    self.automation.committed(commit.automation);
                }
                Err(error) => self.failed(error, expected_life),
            }
        });
        Ok(())
    }

    pub fn rollback(self, workflow_id: String) -> Result<(), SaveError> {
        if !self.ready.get_untracked() {
            return Err(SaveError::Unavailable);
        }
        if !self.threads_hydrated.get_untracked()
            && self.automation.with(|state| {
                state
                    .workflows
                    .iter()
                    .find(|workflow| workflow.id == workflow_id)
                    .is_some_and(|workflow| {
                        workflow
                            .steps
                            .iter()
                            .any(|step| step.operation.domain() == AgentDomain::Chat)
                    })
            })
        {
            return Err(SaveError::Rejected("Chat history has not loaded. Reload to retry before undoing this workflow. No records were changed."));
        }
        if self.active_thread.with_untracked(|active| {
            self.automation.with(|state| {
                state
                    .workflows
                    .iter()
                    .find(|workflow| workflow.id == workflow_id)
                    .is_some_and(|workflow| {
                        workflow.steps.iter().any(|step| {
                            step.operation.domain() == AgentDomain::Chat
                                && step.target_id.is_some()
                                && (step.target_id == *active
                                    || step.target_id == self.streaming_thread.0.get_untracked()
                                    || step.target_id.as_deref().is_some_and(|id| {
                                        self.prompt_queue
                                            .0
                                            .with_untracked(|queue| queue.has_for(id))
                                    }))
                        })
                    })
            })
        }) {
            return Err(SaveError::Rejected("Switch away from the created chat and finish or cancel its pending prompts before undoing this workflow. No records were changed."));
        }
        let people_ids = self
            .automation
            .with(|state| storage::workflow::rollback_people_ids(state, &workflow_id));
        let relationship_tx = if people_ids.is_empty() {
            None
        } else {
            Some(self.relationships.begin_transaction(&people_ids)?)
        };
        let (expected, expected_life) =
            self.begin(|state| rollback_needs_life(state, &workflow_id))?;
        wasm_bindgen_futures::spawn_local(async move {
            match storage::workflow::rollback(
                &expected,
                expected_life.as_ref(),
                relationship_tx
                    .as_ref()
                    .map(|transaction| transaction.expected()),
                &workflow_id,
                crate::clock::now_ms(),
            )
            .await
            {
                Ok(commit) => {
                    for (domain, id) in commit.result.targets {
                        match domain {
                            AgentDomain::Chat => self
                                .threads
                                .update(|items| items.retain(|item| item.id != id)),
                            AgentDomain::Tasks => self
                                .tasks
                                .0
                                .update(|items| items.retain(|item| item.id != id)),
                            AgentDomain::Calendar => self
                                .events
                                .0
                                .update(|items| items.retain(|item| item.id != id)),
                            AgentDomain::Capture => self
                                .captures
                                .0
                                .update(|items| items.retain(|item| item.id != id)),
                            AgentDomain::Knowledge => self
                                .knowledge
                                .0
                                .update(|items| items.retain(|item| item.id != id)),
                            AgentDomain::Inbox => self
                                .inbox
                                .0
                                .update(|items| items.retain(|item| item.id != id)),
                            AgentDomain::Meetings => self
                                .meeting_notes
                                .update(|notes| notes.retain(|note| note.id != id)),
                            _ => {}
                        }
                    }
                    if let Some(transaction) = relationship_tx {
                        transaction.rolled_back(commit.result.people);
                    }
                    if let Some(life) = commit.life {
                        self.life.committed(life);
                    }
                    self.automation.committed(commit.automation);
                }
                Err(error) => self.failed(error, expected_life),
            }
        });
        Ok(())
    }

    fn begin(
        self,
        needs_life: impl FnOnce(&AutomationState) -> bool,
    ) -> Result<(Metadata<AutomationState>, Option<Metadata<LifeContext>>), SaveError> {
        let expected = self.automation.begin_transaction()?;
        let life = if needs_life(&expected.value) {
            match self.life.begin_transaction() {
                Ok(life) => Some(life),
                Err(error) => {
                    self.automation.committed(expected);
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok((expected, life))
    }

    fn failed(self, error: SaveError, original_life: Option<Metadata<LifeContext>>) {
        if let Some(original) = original_life {
            if matches!(error, SaveError::Conflict) {
                self.life.report_error(&error);
            } else {
                // Nothing was published before commit, so an aborted transaction
                // leaves the original saved Life state usable. The workflow reports failure.
                self.life.committed(original);
            }
        }
        self.automation.report(error);
    }
}
