//! App-owned, ordered task/capture writes. A failed save retains the latest draft until
//! explicit Retry; stale baselines fail closed rather than overwrite other tabs.

use bruhs_models::{KnowledgeAttachment, KnowledgeDocument, SourceClip};
use leptos::prelude::*;

use crate::durable_metadata::SaveState;
use crate::state::{CalendarEvent, CaptureRecord, InboxConversation, Project, WorkItem};
use crate::storage::metadata::{yield_before_save, SaveError};

mod queue;
use queue::Queue;
mod hydration;
pub(crate) use hydration::InitialEdits;
mod record;
pub(crate) use record::{Kind, Record};
mod attachment_recovery;
mod events;
mod knowledge;
mod people;
mod projects;
mod sources;
mod threads;

#[derive(Clone, Copy)]
pub struct TaskSaves {
    queue: RwSignal<Queue>,
    initial_edits: StoredValue<InitialEdits>,
    initial_captures: StoredValue<InitialEdits<CaptureRecord>>,
    initial_inbox: StoredValue<InitialEdits<InboxConversation>>,
    initial_events: StoredValue<InitialEdits<CalendarEvent>>,
    initial_documents: StoredValue<InitialEdits<KnowledgeDocument>>,
    initial_sources: StoredValue<InitialEdits<SourceClip>>,
    initial_projects: StoredValue<InitialEdits<Project>>,
    conversations: StoredValue<threads::Conversations>,
    banners: crate::components::banner::BannerStackSignal,
    saving: RwSignal<bool>,
    pub status: RwSignal<SaveState>,
    pub contacts_committed: RwSignal<u64>,
}

impl TaskSaves {
    pub fn install() {
        provide_context(Self {
            queue: RwSignal::new(Queue::default()),
            initial_edits: StoredValue::new(InitialEdits::default()),
            initial_captures: StoredValue::new(InitialEdits::default()),
            initial_inbox: StoredValue::new(InitialEdits::default()),
            initial_events: StoredValue::new(InitialEdits::default()),
            initial_documents: StoredValue::new(InitialEdits::default()),
            initial_sources: StoredValue::new(InitialEdits::default()),
            initial_projects: StoredValue::new(InitialEdits::default()),
            conversations: StoredValue::new(threads::Conversations::default()),
            banners: expect_context(),
            saving: RwSignal::new(false),
            status: RwSignal::new(SaveState::Saved),
            contacts_committed: RwSignal::new(0),
        });
    }

    pub(crate) fn pending(self) -> bool {
        self.queue.with(|queue| !queue.is_empty())
    }

    pub fn edit(self, id: String, expected: Option<WorkItem>, draft: Option<WorkItem>) {
        self.initial_edits
            .update_value(|edits| edits.edit(&id, draft.as_ref()));
        self.queue
            .update(|queue| queue.edit(id, expected.map(Record::Task), draft.map(Record::Task)));
        self.refresh_status();
        self.save();
    }

    pub fn finish_loading(
        self,
        loaded: Vec<WorkItem>,
        captures: Vec<CaptureRecord>,
    ) -> (Vec<WorkItem>, Vec<CaptureRecord>) {
        let mut merged = Vec::new();
        let mut merged_captures = Vec::new();
        self.initial_edits
            .update_value(|edits| merged = edits.merge(loaded));
        self.initial_captures
            .update_value(|edits| merged_captures = edits.merge_by(captures, |record| &record.id));
        (merged, merged_captures)
    }

    pub fn edit_capture(self, expected: Option<CaptureRecord>, draft: CaptureRecord) {
        self.initial_captures
            .update_value(|edits| edits.edit(&draft.id, Some(&draft)));
        self.queue.update(|queue| {
            queue.edit_capture(
                draft.id.clone(),
                expected.map(Record::Capture),
                Some(Record::Capture(draft)),
            )
        });
        self.refresh_status();
        self.save();
    }

    pub fn finish_loading_inbox(self, loaded: Vec<InboxConversation>) -> Vec<InboxConversation> {
        let mut merged = Vec::new();
        self.initial_inbox
            .update_value(|edits| merged = edits.merge_by(loaded, |record| &record.id));
        merged
    }

    pub fn edit_inbox(
        self,
        id: String,
        expected: Option<InboxConversation>,
        draft: Option<InboxConversation>,
    ) {
        self.initial_inbox
            .update_value(|edits| edits.edit(&id, draft.as_ref()));
        self.queue.update(|queue| {
            queue.edit_record(
                Kind::Inbox,
                id,
                expected.map(Record::Inbox),
                draft.map(Record::Inbox),
            );
        });
        self.refresh_status();
        self.save();
    }

    pub fn capture_inbox(self, capture: CaptureRecord, conversation: InboxConversation) {
        self.initial_inbox
            .update_value(|edits| edits.edit(&conversation.id, Some(&conversation)));
        self.initial_captures
            .update_value(|edits| edits.edit(&capture.id, Some(&capture)));
        self.queue.update(|queue| {
            let ids = [
                (Kind::Inbox, conversation.id.clone()),
                (Kind::Capture, capture.id.clone()),
            ];
            queue.edit_record(
                Kind::Inbox,
                conversation.id.clone(),
                None,
                Some(Record::Inbox(conversation)),
            );
            queue.edit_capture(capture.id.clone(), None, Some(Record::Capture(capture)));
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }

    pub fn capture_task(self, capture: CaptureRecord, task: WorkItem) {
        self.initial_edits
            .update_value(|edits| edits.edit(&task.id, Some(&task)));
        self.initial_captures
            .update_value(|edits| edits.edit(&capture.id, Some(&capture)));
        self.queue.update(|queue| {
            let ids = [
                (Kind::Task, task.id.clone()),
                (Kind::Capture, capture.id.clone()),
            ];
            queue.edit(task.id.clone(), None, Some(Record::Task(task)));
            queue.edit_capture(capture.id.clone(), None, Some(Record::Capture(capture)));
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }

    /// Publish a single logical operation to the writer. Ordinary edits to any
    /// member coalesce into this group until its atomic commit is acknowledged.
    pub fn edit_group(self, changes: Vec<(Option<WorkItem>, WorkItem)>) {
        self.initial_edits.update_value(|edits| {
            for (_, draft) in &changes {
                edits.edit(&draft.id, Some(draft));
            }
        });
        self.queue.update(|queue| {
            let ids = changes
                .iter()
                .map(|(_, draft)| draft.id.clone())
                .collect::<Vec<_>>();
            for (original, draft) in changes {
                queue.edit(
                    draft.id.clone(),
                    original.map(Record::Task),
                    Some(Record::Task(draft)),
                );
            }
            queue.group(&ids);
        });
        self.refresh_status();
        self.save();
    }

    fn retry(self) {
        self.queue.update(Queue::retry);
        self.refresh_status();
        self.save();
    }

    pub(crate) fn record_status(self, kind: Kind, id: &str) -> SaveState {
        self.queue.with(|queue| queue.status(kind, id))
    }

    fn refresh_status(self) {
        self.status.set(self.queue.with_untracked(|queue| {
            if let Some((message, conflict)) = queue.failure() {
                SaveState::Error {
                    message: message.clone(),
                    conflict: *conflict,
                }
            } else if queue.is_empty() {
                SaveState::Saved
            } else {
                SaveState::Saving
            }
        }));
    }

    fn save(self) {
        if self.saving.get_untracked() {
            return;
        }
        self.saving.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                yield_before_save().await;
                let Some(pending) = self.queue.with_untracked(Queue::next_batch) else {
                    break;
                };
                let writes = pending
                    .iter()
                    .map(|edit| crate::storage::tasks::Write {
                        kind: edit.kind,
                        id: &edit.id,
                        expected: edit.expected.as_ref(),
                        draft: edit.draft.as_ref(),
                    })
                    .collect::<Vec<_>>();
                match crate::storage::tasks::save(&writes).await {
                    Ok(()) => {
                        let contact_committed = pending.iter().any(|edit| edit.kind == Kind::Person);
                        self.queue.update(|queue| {
                            for edit in pending {
                                let notice = match &edit.draft {
                                    Some(Record::Capture(capture)) => Some(if edit.expected.is_none() {
                                        format!("Captured as {}", capture.kind.label())
                                    } else {
                                        "Capture updated".into()
                                    }),
                                    _ => None,
                                };
                                if queue.saved(edit) {
                                    if let Some(notice) = notice {
                                        self.banners.push(crate::components::banner::BannerKind::Info, notice, Some(2_000));
                                    }
                                }
                            }
                        });
                        if contact_committed {
                            self.contacts_committed.update(|version| *version = version.wrapping_add(1));
                        }
                    },
                    Err(error) => self.queue.update(|queue| {
                        let message: String = if matches!(error, SaveError::Conflict) {
                            if pending.iter().any(|edit| edit.kind == Kind::Capture) {
                                "This capture or linked record changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            } else if pending.iter().any(|edit| edit.kind == Kind::Inbox) {
                                "This conversation changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            } else if pending.iter().any(|edit| edit.kind == Kind::Event) {
                                "This event or linked task changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            } else if pending.iter().any(|edit| edit.kind == Kind::SourceClip) {
                                "This source or a citing document changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            } else if pending.iter().any(|edit| edit.kind == Kind::Person) {
                                "This contact changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            } else if pending.iter().any(|edit| edit.kind == Kind::Project) {
                                "This project or a linked task changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            } else if pending.iter().any(|edit| matches!(edit.kind, Kind::Thread | Kind::Conversation)) {
                                "This thread or its conversation changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            } else if pending.iter().any(|edit| matches!(edit.kind, Kind::Document | Kind::Attachment)) {
                                "This document, attachment, or cited source changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            } else {
                                "This task changed in storage. Your unsaved changes are kept here; copy the recovery data before reloading to review the latest version.".into()
                            }
                        } else {
                            error.to_string()
                        };
                        for edit in &pending {
                            let conflict = matches!(error, SaveError::Conflict | SaveError::Corrupt);
                            match edit.kind {
                                Kind::Task => queue.failed(&edit.id, message.clone(), conflict),
                                Kind::Capture => queue.failed_capture(&edit.id, message.clone(), conflict),
                                Kind::Inbox => queue.failed_record(Kind::Inbox, &edit.id, message.clone(), conflict),
                                Kind::Event => queue.failed_record(Kind::Event, &edit.id, message.clone(), conflict),
                                Kind::Document | Kind::Attachment => queue.failed_record(edit.kind, &edit.id, message.clone(), conflict),
                                Kind::SourceClip | Kind::Person | Kind::Project | Kind::Thread | Kind::Conversation => queue.failed_record(edit.kind, &edit.id, message.clone(), conflict),
                            }
                        }
                    }),
                }
                self.refresh_status();
            }
            self.saving.set(false);
        });
    }
}

/// An in-flow recovery rail survives route changes without covering controls.
#[component]
pub fn TaskSaveNotice() -> impl IntoView {
    let saves = expect_context::<TaskSaves>();
    let visible = RwSignal::new(false);
    Effect::new(move |_| match saves.status.get() {
        SaveState::Error { .. } => visible.set(true),
        SaveState::Saved => visible.set(false),
        _ => {}
    });
    view! {
        <Show when=move || visible.get()>
            <section aria-label="Unsaved local changes" class="max-h-[30dvh] shrink-0 overflow-y-auto">
                <crate::components::metadata_status::MetadataStatus
                    status=saves.status
                    retry=Callback::new(move |_| saves.retry())
                    recovery=Signal::derive(move || saves.queue.with(Queue::recovery_json))
                />
                <Show when=move || matches!(saves.status.get(), SaveState::Error { .. }) && saves.queue.with(|queue| !queue.recovery_attachments().is_empty())>
                    <div class="space-y-2 px-4 py-2 text-label-sm text-rambutan-300">
                        <p>"Large attachment bytes are kept separately from recovery text. Download these files and copy the recovery data before closing or reloading. Each bytes_file entry names its matching download."</p>
                        <For each=move || saves.queue.with(Queue::recovery_attachments) key=|file| file.key.clone() children=move |file| {
                            let label = format!("Download {} ({})", file.attachment.file_name, if file.key.starts_with("original-") { "original" } else { "unsaved" });
                            view! { <button type="button" class="block max-w-full break-all rounded-grape bg-longan-800 px-3 py-2 text-left hover:bg-longan-700 cursor-pointer" on:click=move |_| {
                                if let Err(message) = attachment_recovery::download(&file) {
                                    saves.banners.push(crate::components::banner::BannerKind::Error, message, None);
                                }
                            }>{label}</button> }
                        } />
                    </div>
                </Show>
            </section>
        </Show>
    }
}
