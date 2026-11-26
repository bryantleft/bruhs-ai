//! Initial thread loading must not replace work started before storage is ready.
use leptos::prelude::*;

use crate::state::{AgentMessage, Thread, Turns};
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
    set_active: crate::state::ActiveThreadWriter,
    drafts: crate::state::Drafts,
    messages: WriteSignal<Vec<AgentMessage>>,
    turns: Turns,
    history: crate::thread_history::ThreadHistory,
    workflow_ready: RwSignal<bool>,
    saves: crate::task_saves::TaskSaves,
}

impl ThreadCatalog {
    pub fn empty_message(self) -> Option<&'static str> {
        self.status.get().empty_message()
    }

    pub fn loading(self) -> bool {
        matches!(self.status.get(), Status::Pending | Status::Loading)
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
            drafts: expect_context(),
            messages: expect_context::<crate::state::SetMessagesSignal>().0,
            turns: expect_context(),
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
        let new_id = expected.is_none().then(|| thread.id.clone());
        self.saves.edit_thread(expected, thread);
        if let Some(id) = new_id {
            self.drafts.register_new(&id);
        }
    }

    pub fn forget(self, original: Thread) {
        self.drafts.forget(&original.id);
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
            .with_untracked(|rows| initial_selection(rows, linked));
        if let Some(id) = chosen {
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

fn initial_selection(rows: &[Thread], linked: Option<&str>) -> Option<String> {
    linked
        .and_then(|id| rows.iter().find(|thread| thread.id == id))
        .or_else(|| rows.first())
        .map(|thread| thread.id.clone())
}

/// How long a load may take before the strip says so. Most loads finish in
/// a few dozen milliseconds; showing the strip for that moment moved the
/// whole page 37px when it went, so early clicks missed.
const SLOW_LOAD_MS: u64 = 400;

#[component]
pub fn ThreadCatalogStatus() -> impl IntoView {
    let catalog = expect_context::<ThreadCatalog>();
    let slow = RwSignal::new(false);
    Effect::new(move |_| {
        if catalog.status.get() == Status::Ready {
            slow.set(false);
            return;
        }
        set_timeout(
            move || {
                // Only a load still running is slow; the page may be gone.
                if catalog
                    .status
                    .try_get_untracked()
                    .is_some_and(|status| status != Status::Ready)
                {
                    let _ = slow.try_set(true);
                }
            },
            std::time::Duration::from_millis(SLOW_LOAD_MS),
        );
    });
    // A failure shows at once; loading only once it is slow.
    let shown = move || match catalog.status.get() {
        Status::Ready => false,
        Status::Failed => true,
        Status::Pending | Status::Loading => slow.get(),
    };
    view! {
        <Show when=shown>
            <section aria-label="Thread list loading status" role="status"
                class="flex shrink-0 flex-wrap items-center gap-2 border-b border-longan-700 px-3 py-2 text-body-sm text-rambutan-300 sm:px-5">
                <span>{move || if catalog.status.get() == Status::Failed {
                    "Saved threads could not be loaded. Existing records and your current work were left unchanged."
                } else { "Loading saved threads…" }}</span>
                <Show when=move || catalog.status.get() == Status::Failed>
                    <button type="button" class="bruhs-control min-h-control rounded-grape px-2 text-label-sm text-persimmon-300 hover:bg-longan-700 focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer"
                        on:click=move |_| catalog.load()>"Retry loading threads"</button>
                </Show>
            </section>
        </Show>
    }
}
