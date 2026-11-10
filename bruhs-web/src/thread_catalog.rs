//! Initial thread loading must not replace work started before storage is ready.
use leptos::prelude::*;

use crate::state::{AgentMessage, Thread, Turns, WorkspaceMode};
use crate::task_saves::InitialEdits;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Default, PartialEq)]
enum Status {
    #[default]
    Pending,
    Loading,
    Failed,
    Ready,
}

impl Status {
    fn empty_message(self) -> Option<&'static str> {
        match self {
            Self::Pending | Self::Loading => Some("Loading saved threads…"),
            Self::Failed => Some("Saved threads couldn’t be loaded."),
            Self::Ready => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ThreadCatalog {
    status: RwSignal<Status>,
    edits: StoredValue<InitialEdits<Thread>>,
    touched: StoredValue<bool>,
    threads: ReadSignal<Vec<Thread>>,
    set_threads: WriteSignal<Vec<Thread>>,
    active: ReadSignal<Option<String>>,
    set_active: WriteSignal<Option<String>>,
    messages: WriteSignal<Vec<AgentMessage>>,
    turns: Turns,
    mode: WorkspaceMode,
    history: crate::thread_history::ThreadHistory,
    workflow_ready: RwSignal<bool>,
    saves: crate::task_saves::TaskSaves,
}

impl ThreadCatalog {
    pub fn empty_message(self) -> Option<&'static str> {
        self.status.get().empty_message()
    }

    pub fn install() {
        provide_context(Self {
            status: RwSignal::new(Status::Pending),
            edits: StoredValue::new(InitialEdits::default()),
            touched: StoredValue::new(false),
            threads: expect_context::<crate::state::ThreadsSignal>().0,
            set_threads: expect_context::<crate::state::SetThreadsSignal>().0,
            active: expect_context::<crate::state::ActiveThreadId>().0,
            set_active: expect_context::<crate::state::SetActiveThreadId>().0,
            messages: expect_context::<crate::state::SetMessagesSignal>().0,
            turns: expect_context(),
            mode: expect_context(),
            history: expect_context(),
            workflow_ready: expect_context::<crate::workflow_executor::WorkflowExecutor>()
                .threads_hydrated,
            saves: expect_context(),
        });
    }

    // The journal protects initial publication; the app-owned writer owns
    // durable acknowledgment, ordered retries and recovery independently.
    pub fn persist(self, expected: Option<Thread>, thread: Thread) {
        self.touched.set_value(true);
        self.edits
            .update_value(|edits| edits.edit(&thread.id, Some(&thread)));
        self.saves.edit_thread(expected, thread);
    }

    pub fn forget(self, original: Thread) {
        self.touched.set_value(true);
        self.edits
            .update_value(|edits| edits.edit(&original.id, None));
        self.saves.delete_thread(original);
    }

    pub fn load(self) {
        if matches!(self.status.get_untracked(), Status::Loading | Status::Ready) {
            return;
        }
        self.status.set(Status::Loading);
        leptos::task::spawn_local(async move {
            match crate::storage::load_threads().await {
                Ok(loaded) => {
                    let mut merged = Vec::new();
                    self.edits.update_value(|edits| {
                        merged = edits.merge_by(loaded, |thread| &thread.id);
                    });
                    let restore_focus = self.active.get_untracked().map(|id| {
                        crate::thread_history::focus::retain_composer_focus(self.active, id)
                    });
                    self.set_threads.set(merged);
                    self.activate_initial();
                    self.status.set(Status::Ready);
                    self.workflow_ready.set(true);
                    if let Some(restore_focus) = restore_focus {
                        restore_focus();
                    }
                }
                Err(error) => {
                    log::warn!("Saved thread list loading failed: {error:?}");
                    self.status.set(Status::Failed);
                }
            }
        });
    }

    fn activate_initial(self) {
        let active = self.active.get_untracked();
        if self.threads.with_untracked(|rows| {
            active
                .as_ref()
                .is_some_and(|id| rows.iter().any(|thread| &thread.id == id))
        }) {
            return;
        }
        let linked = window().location().hash().unwrap_or_default();
        let linked = linked.strip_prefix("#thread/");
        let chosen = self
            .threads
            .with_untracked(|rows| initial_selection(rows, linked, self.mode.0.get_untracked()));
        if let Some((id, linked_mode)) = chosen {
            if let Some(mode) = linked_mode {
                self.mode.0.set(mode);
            }
            self.set_active.set(Some(id.clone()));
            self.history
                .load(id, self.active, self.messages, self.turns);
        } else if !self.touched.get_value() {
            // Only a successfully decoded, committed empty snapshot may create
            // a default. Failure and intentional local deletion stay distinct.
            let thread = Thread::new(
                "New thread".into(),
                String::new(),
                None,
                crate::state::now_epoch_ms(),
            );
            self.set_active.set(Some(thread.id.clone()));
            self.set_threads.update(|rows| rows.push(thread.clone()));
            self.messages.set(Vec::new());
            self.persist(None, thread);
        }
    }
}

fn initial_selection(
    rows: &[Thread],
    linked: Option<&str>,
    mode: bruhs_models::SidebarTab,
) -> Option<(String, Option<bruhs_models::SidebarTab>)> {
    if let Some(thread) = linked.and_then(|id| rows.iter().find(|thread| thread.id == id)) {
        return Some((
            thread.id.clone(),
            Some(bruhs_models::SidebarTab::for_project(&thread.project_name)),
        ));
    }
    rows.iter()
        .find(|thread| mode.matches_project(&thread.project_name))
        .or_else(|| rows.first())
        .map(|thread| (thread.id.clone(), None))
}

#[component]
pub fn ThreadCatalogStatus() -> impl IntoView {
    let catalog = expect_context::<ThreadCatalog>();
    view! {
        <Show when=move || catalog.status.get() != Status::Ready>
            <section aria-label="Thread list loading status" role="status"
                class="flex shrink-0 flex-wrap items-center gap-2 border-b border-longan-700 px-3 py-2 text-body-sm text-rambutan-300 sm:px-5">
                <span>{move || if catalog.status.get() == Status::Failed {
                    "Saved threads could not be loaded. Existing records and your current work were left unchanged."
                } else { "Loading saved threads…" }}</span>
                <Show when=move || catalog.status.get() == Status::Failed>
                    <button type="button" class="min-h-11 rounded-grape px-2 text-label-sm text-persimmon-300 hover:bg-longan-700 focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer"
                        on:click=move |_| catalog.load()>"Retry loading threads"</button>
                </Show>
            </section>
        </Show>
    }
}
