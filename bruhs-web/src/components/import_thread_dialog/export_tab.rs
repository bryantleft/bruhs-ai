//! The Import thread dialog's Export file tab (D1a): where to get each
//! export, choosing the files, the read's progress, its summary, the rows
//! and Import all. The read and its batch live in [`ExportState`], which
//! the dialog owns, so leaving the tab keeps them; the dialog discards the
//! batch when it closes (F23). Every call goes through the state's abort
//! signal, so a close drops an upload in flight, the poll and Import all.

use std::collections::{HashSet, VecDeque};

use bruhs_models::imports::{
    batch_lines, export_memory_from, export_row_detail, import_key, imported_line, importing_line,
    memory_found_line, reading_line, rows_shown_line, ImportBatch, ImportJob, ImportJobState,
    MoveInStart, MoveInTab, ALREADY_HERE, CHOOSE_ANOTHER, CHOOSE_EXPORT, EXPORT_HELP_CHATGPT,
    EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK, EXPORT_SEARCH, EXPORT_TAB, EXPORT_TOO_LARGE,
    GROK_BEST_EFFORT, IMPORT_ALL, IMPORT_ALL_MAX, IMPORT_FAILED, MAX_ROWS_SHOWN, MAX_UPLOAD_BYTES,
    MAX_UPLOAD_FILES, NO_MATCHING_CHATS, TOO_MANY_FILES, USE_AS_MEMORY,
};
use bruhs_models::logic::portable::{ImportCandidate, ImportSource, ImportedThread};
use bruhs_models::num::ToFloatLossy as _;
use bruhs_models::EpochMs;
use leptos::{html, prelude::*};

use crate::api::{imports as api, ApiError};
use crate::state::MoveInOpen;

/// What the file input offers: ZIPs and bare JSON files.
pub(super) const ACCEPT: &str = ".zip,.json,application/zip,application/json";
/// How often a read is asked how it stands (D3).
const POLL_MS: u32 = 500;
const BUTTON: &str = "bruhs-control min-h-control shrink-0 cursor-pointer rounded-seed px-3 text-label-sm text-rambutan-200 inset-ring inset-ring-longan-700 transition-colors duration-150 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 aria-disabled:cursor-not-allowed aria-disabled:opacity-50";
const HELP: &str = "text-label-sm text-rambutan-300";
const LINE: &str = "text-label-sm text-rambutan-200";
const WARN: &str = "text-label-sm text-durian-300";
// Live regions stay in the accessibility tree while empty, so the first
// line put in them is announced (I2).
const STATUS: &str = "text-label-sm text-rambutan-300 empty:sr-only";
const ERROR: &str = "text-label-sm text-lychee-300 empty:sr-only";
const ROW: &str = "flex w-full flex-col items-start gap-0.5 px-4 py-2 text-left cursor-pointer";

/// The control that takes focus after the tab changes under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExportFocus {
    /// **Choose export file**: after a failed read.
    Choose,
    /// The search field: once a batch is ready, and after an import.
    Search,
    /// **Choose another file**: once every chat is here.
    Another,
}

/// The Export file tab's read, rows and imports. The dialog owns it and
/// provides it as context.
#[derive(Clone, Copy)]
pub(super) struct ExportState {
    /// The read: `Reading` until it lands, then `Ready` with its batch.
    pub job: RwSignal<Option<ImportJob>>,
    /// Files being sent, before the server answers with the read.
    pub sending: RwSignal<Option<u32>>,
    pub query: RwSignal<String>,
    pub picked: RwSignal<usize>,
    pub error: RwSignal<Option<String>>,
    /// The polite status line: `reading_line` while a read runs (changed
    /// only as files finish), `importing_line` while rows come in, then
    /// `imported_line`.
    pub status: RwSignal<Option<String>>,
    /// Rows are being imported.
    pub importing: RwSignal<bool>,
    /// `import_key`s of the threads already here.
    pub known: Signal<HashSet<String>>,
    pub focus: RwSignal<Option<ExportFocus>>,
    /// The clock row details are measured against.
    pub now: fn() -> EpochMs,
    /// Aborted when the dialog closes or another file is chosen.
    abort: StoredValue<Option<web_sys::AbortController>, LocalStorage>,
    /// Rows waiting to be imported, one at a time.
    queue: StoredValue<VecDeque<ImportCandidate>>,
}

impl ExportState {
    pub(super) fn new(known: Signal<HashSet<String>>) -> Self {
        Self {
            job: RwSignal::new(None),
            sending: RwSignal::new(None),
            query: RwSignal::new(String::new()),
            picked: RwSignal::new(0),
            error: RwSignal::new(None),
            status: RwSignal::new(None),
            importing: RwSignal::new(false),
            known,
            focus: RwSignal::new(None),
            now: crate::state::now_epoch_ms,
            abort: StoredValue::new_local(None),
            queue: StoredValue::new(VecDeque::new()),
        }
    }

    /// The ready batch's id.
    fn batch_id(self) -> Option<String> {
        self.job.try_with_untracked(|job| match job {
            Some(ImportJob {
                state: ImportJobState::Ready { batch },
                ..
            }) => Some(batch.id.clone()),
            _ => None,
        })?
    }

    /// The ready batch's rows not yet here that match the search (tracked).
    fn visible(self) -> Vec<ImportCandidate> {
        self.job.with(|job| match job {
            Some(ImportJob {
                state: ImportJobState::Ready { batch },
                ..
            }) => self.known.with(|known| {
                self.query
                    .with(|query| visible_export_rows(batch, known, query))
            }),
            _ => Vec::new(),
        })
    }

    /// Whether the tab shows a row to choose (tracked), for the dialog's
    /// arrow hint (M6).
    pub(super) fn has_rows(self) -> bool {
        self.job.with(|job| match job {
            Some(ImportJob {
                state: ImportJobState::Ready { batch },
                ..
            }) => self.known.with(|known| {
                self.query.with(|query| {
                    let query = query.trim().to_lowercase();
                    batch.candidates.iter().any(|row| {
                        !known.contains(&import_key(row.source, &row.id))
                            && (query.is_empty() || row.title.to_lowercase().contains(&query))
                    })
                })
            }),
            _ => false,
        })
    }

    fn signal(self) -> Option<web_sys::AbortSignal> {
        self.abort
            .try_with_value(|abort| abort.as_ref().map(web_sys::AbortController::signal))
            .flatten()
    }

    /// The owner chose `files`: drop any earlier read and read these.
    fn choose(self, files: Vec<web_sys::File>) {
        if files.is_empty() {
            return;
        }
        let bytes: f64 = files.iter().map(|file| file.size()).sum();
        if let Some(refusal) = upload_refusal(files.len(), bytes) {
            self.error.set(Some(refusal.to_owned()));
            self.focus.set(Some(self.choose_focus()));
            return;
        }
        self.discard();
        // **Choose another file** goes with the batch; the start view's
        // **Choose export file** holds focus while the read runs.
        self.focus.set(Some(ExportFocus::Choose));
        let Ok(controller) = web_sys::AbortController::new() else {
            return;
        };
        let signal = controller.signal();
        self.abort.set_value(Some(controller));
        let count = u32::try_from(files.len()).unwrap_or(u32::MAX);
        self.sending.set(Some(count));
        self.status.set(Some(reading_line(0, count, 0)));
        wasm_bindgen_futures::spawn_local(async move {
            match self.read(files, count, &signal).await {
                Ok(()) => {}
                // The dialog closed or another file was chosen: this read's
                // state is gone, and the next one's isn't this one's to touch.
                Err(_) if signal.aborted() => {}
                Err(ApiError::Aborted) => {}
                Err(error) => self.failed(api::import_message(&error)),
            }
        });
    }

    /// Sends the files, then asks every 500 ms until the read lands.
    async fn read(
        self,
        files: Vec<web_sys::File>,
        count: u32,
        signal: &web_sys::AbortSignal,
    ) -> Result<(), ApiError> {
        let mut shown = Some((0, count));
        let still_here = || {
            if signal.aborted() {
                Err(ApiError::Aborted)
            } else {
                Ok(())
            }
        };
        let mut job = api::upload_exports(files, signal).await?;
        still_here()?;
        loop {
            match job.state {
                ImportJobState::Reading {
                    files_done,
                    files,
                    conversations,
                } => {
                    if let Some(line) =
                        reading_progress(&mut shown, files_done, files, conversations)
                    {
                        self.status.try_set(Some(line));
                    }
                }
                ImportJobState::Ready { .. } => break,
                ImportJobState::Failed { message } => {
                    let message = if message.is_empty() {
                        IMPORT_FAILED.to_owned()
                    } else {
                        message
                    };
                    self.failed(message);
                    return Ok(());
                }
            }
            self.sending.try_set(None);
            self.job.try_set(Some(job.clone()));
            gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
            still_here()?;
            job = api::import_job(&job.id, signal).await?;
            still_here()?;
        }
        self.sending.try_set(None);
        self.status.try_set(None);
        self.job.try_set(Some(job));
        self.after_change();
        Ok(())
    }

    /// A read that failed: its message, any batch it left discarded, and
    /// focus back on **Choose export file**.
    fn failed(self, message: String) {
        self.discard();
        self.error.try_set(Some(message));
        self.focus.try_set(Some(ExportFocus::Choose));
    }

    /// Focus after the rows changed: the search while rows are left, else
    /// **Choose another file**.
    fn after_change(self) {
        let left = self
            .job
            .try_with_untracked(|job| match job {
                Some(ImportJob {
                    state: ImportJobState::Ready { batch },
                    ..
                }) => self
                    .known
                    .try_with_untracked(|known| rows_left(batch, known))
                    .unwrap_or(false),
                _ => false,
            })
            .unwrap_or(false);
        let target = if left {
            ExportFocus::Search
        } else {
            ExportFocus::Another
        };
        self.focus.try_set(Some(target));
    }

    /// Where focus goes after a refused choice: the button that is drawn.
    fn choose_focus(self) -> ExportFocus {
        if self.batch_id().is_some() {
            ExportFocus::Another
        } else {
            ExportFocus::Choose
        }
    }

    /// Drops the read: aborts its calls, forgets its rows and sends
    /// `DELETE` for its batch (F23). Closing the dialog and choosing
    /// another file both come here.
    pub(super) fn discard(self) {
        if let Some(Some(controller)) = self.abort.try_update_value(Option::take) {
            controller.abort();
        }
        self.queue.try_update_value(VecDeque::clear);
        let id = self
            .job
            .try_get_untracked()
            .flatten()
            .map(|job| job.id)
            .filter(|id| bruhs_models::imports::valid_batch_id(id));
        self.job.try_set(None);
        self.sending.try_set(None);
        self.query.try_set(String::new());
        self.picked.try_set(0);
        self.error.try_set(None);
        self.status.try_set(None);
        self.importing.try_set(false);
        let Some(id) = id else {
            return;
        };
        // Its own signal: the dialog's was just aborted.
        let Ok(controller) = web_sys::AbortController::new() else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            // A batch already gone is what this asked for.
            let _ = api::discard_import(&id, &controller.signal()).await;
        });
    }

    /// Queues `row`. Answers whether nothing was importing, so the caller
    /// starts [`Self::drain`].
    pub(super) fn enqueue(self, row: ImportCandidate) -> bool {
        self.queue.update_value(|queue| queue.push_back(row));
        let idle = !self.importing.get_untracked();
        if idle {
            self.importing.set(true);
            self.error.set(None);
            self.status.set(None);
        }
        idle
    }

    /// Imports the queued rows one at a time through `import_one`, which
    /// persists each as the local tabs do, then puts the imported lines in
    /// the tab's status line, its one announcement (M10). A failure stops
    /// the run and shows under the tab; a close stops it quietly.
    pub(super) async fn drain(self, import_one: Callback<ImportedThread>) {
        let (Some(batch), Some(signal)) = (self.batch_id(), self.signal()) else {
            self.queue.try_update_value(VecDeque::clear);
            self.importing.try_set(false);
            return;
        };
        let mut done: Vec<ImportSource> = Vec::new();
        while let Some(row) = self.queue.try_update_value(VecDeque::pop_front).flatten() {
            let total = done.len() + 1 + self.queue.try_with_value(VecDeque::len).unwrap_or(0);
            if total > 1 {
                self.status
                    .try_set(Some(importing_line(done.len() + 1, total)));
            }
            match api::import_export_thread(row.source, &batch, &row.id, &signal).await {
                Ok(imported) => {
                    if import_one.try_run(imported).is_none() {
                        // The dialog is gone.
                        return;
                    }
                    done.push(row.source);
                }
                // The dialog closed or the batch went: its state may be gone,
                // and a stale line must not show on the next read (M2).
                Err(_) if signal.aborted() => return,
                Err(ApiError::Aborted) => return,
                Err(error) => {
                    self.queue.try_update_value(VecDeque::clear);
                    self.error.try_set(Some(api::import_message(&error)));
                    break;
                }
            }
        }
        if signal.aborted() {
            return;
        }
        self.importing.try_set(false);
        self.status.try_set(done_line(&done));
        self.after_change();
    }
}

/// Whether any of the batch's chats isn't here yet.
fn rows_left(batch: &ImportBatch, known: &HashSet<String>) -> bool {
    batch
        .candidates
        .iter()
        .any(|row| !known.contains(&import_key(row.source, &row.id)))
}

/// `imported_line` for each source that brought chats in, in the
/// exports' order.
fn done_line(done: &[ImportSource]) -> Option<String> {
    let lines: Vec<String> = ImportSource::EXPORTS
        .into_iter()
        .filter_map(|source| {
            let n = done.iter().filter(|&&each| each == source).count();
            (n > 0).then(|| imported_line(n, source))
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// The reading line, only when the files done (or the file count) moved
/// since `shown`, so the polite region doesn't speak on every poll (I2).
fn reading_progress(
    shown: &mut Option<(u32, u32)>,
    files_done: u32,
    files: u32,
    chats: u32,
) -> Option<String> {
    if *shown == Some((files_done, files)) {
        return None;
    }
    *shown = Some((files_done, files));
    Some(reading_line(files_done, files, chats))
}

/// Scrolls the element `selector` finds into view inside its scroller,
/// once drawn. A native test build has no page.
pub(super) fn scroll_into_view_later(selector: &'static str) {
    if cfg!(not(target_arch = "wasm32")) {
        return;
    }
    request_animation_frame(move || {
        let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.query_selector(selector).ok().flatten())
        else {
            return;
        };
        let options = web_sys::ScrollIntoViewOptions::new();
        options.set_block(web_sys::ScrollLogicalPosition::Nearest);
        options.set_inline(web_sys::ScrollLogicalPosition::Nearest);
        element.scroll_into_view_with_scroll_into_view_options(&options);
    });
}

/// Refuses a choice the server would refuse, before sending any of it.
fn upload_refusal(files: usize, bytes: f64) -> Option<&'static str> {
    if files > MAX_UPLOAD_FILES {
        Some(TOO_MANY_FILES)
    } else if bytes > MAX_UPLOAD_BYTES.to_f64_lossy() {
        Some(EXPORT_TOO_LARGE)
    } else {
        None
    }
}

/// The batch's rows whose chat isn't a thread yet, in the batch's order,
/// whose title holds `query` (any case). Uncapped: the list shows the
/// first [`MAX_ROWS_SHOWN`], and Import all takes the first
/// [`IMPORT_ALL_MAX`].
pub(super) fn visible_export_rows(
    batch: &ImportBatch,
    known: &HashSet<String>,
    query: &str,
) -> Vec<ImportCandidate> {
    let query = query.trim().to_lowercase();
    batch
        .candidates
        .iter()
        .filter(|row| !known.contains(&import_key(row.source, &row.id)))
        .filter(|row| query.is_empty() || row.title.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

/// Focuses `target` once it is drawn. A native test build has no page.
fn focus_later<E>(target: NodeRef<E>)
where
    E: leptos::tachys::html::element::ElementType,
    E::Output: wasm_bindgen::JsCast + Clone + 'static,
{
    if cfg!(not(target_arch = "wasm32")) {
        return;
    }
    request_animation_frame(move || {
        use wasm_bindgen::JsCast as _;
        if let Some(Some(element)) = target.try_get_untracked() {
            if let Ok(element) = element.dyn_into::<web_sys::HtmlElement>() {
                let _ = element.focus();
            }
        }
    });
}

#[component]
pub(super) fn ExportTab(on_import: Callback<ImportCandidate>, busy: Signal<bool>) -> impl IntoView {
    let state = expect_context::<ExportState>();
    let file_input: NodeRef<html::Input> = NodeRef::new();
    let choose_ref: NodeRef<html::Button> = NodeRef::new();
    let search_ref: NodeRef<html::Input> = NodeRef::new();
    let another_ref: NodeRef<html::Button> = NodeRef::new();
    let rows = Memo::new(move |_| state.visible());

    Effect::new(move |_| {
        let Some(target) = state.focus.get() else {
            return;
        };
        state.focus.set(None);
        match target {
            ExportFocus::Choose => focus_later(choose_ref),
            ExportFocus::Search => focus_later(search_ref),
            ExportFocus::Another => focus_later(another_ref),
        }
    });

    let pick = move || {
        if let Some(input) = file_input.get_untracked() {
            input.click();
        }
    };
    let chose = move || {
        let Some(input) = file_input.get_untracked() else {
            return;
        };
        let files: Vec<web_sys::File> = input
            .files()
            .map(|list| (0..list.length()).filter_map(|at| list.get(at)).collect())
            .unwrap_or_default();
        // The same file chosen again is a new choice.
        input.set_value("");
        state.choose(files);
    };
    let reading = move || {
        state.sending.with(Option::is_some)
            || state.job.with(|job| {
                matches!(
                    job,
                    Some(ImportJob {
                        state: ImportJobState::Reading { .. },
                        ..
                    })
                )
            })
    };
    let import_row = move |row: ImportCandidate| {
        if !busy.get_untracked() {
            on_import.run(row);
        }
    };
    let import_all = move || {
        if busy.get_untracked() {
            return;
        }
        for row in rows.get_untracked().into_iter().take(IMPORT_ALL_MAX) {
            on_import.run(row);
        }
    };
    let on_keydown = move |event: web_sys::KeyboardEvent| {
        let count = rows.with_untracked(Vec::len).min(MAX_ROWS_SHOWN);
        match event.key().as_str() {
            "ArrowDown" | "ArrowUp" => {
                event.prevent_default();
                if let Some(next) = crate::components::command_bar::move_selected_index(
                    state.picked.get_untracked(),
                    count,
                    &event.key(),
                ) {
                    state.picked.set(next);
                    scroll_into_view_later("[data-export-tab] [role=option][aria-selected=true]");
                }
            }
            "Enter" => {
                event.prevent_default();
                let at = state.picked.get_untracked().min(count.saturating_sub(1));
                if let Some(row) = rows.with_untracked(|rows| rows.get(at).cloned()) {
                    import_row(row);
                }
            }
            _ => {}
        }
    };

    let start = move || {
        view! {
            {[EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK]
                .into_iter()
                .map(|line| view! { <p data-export-help="" class=HELP>{line}</p> })
                .collect_view()}
            <p data-export-grok="" class=WARN>{GROK_BEST_EFFORT}</p>
            <div>
                <button type="button" node_ref=choose_ref data-export-choose="" data-autofocus="" class=BUTTON aria-disabled=move || reading().then_some("true") on:click=move |_| if !untrack(reading) { pick() }>{CHOOSE_EXPORT}</button>
            </div>
        }
        .into_any()
    };

    // Whether the ready batch has a chat not here yet. Redraws the list
    // only when that flips, so an import keeps the search and its focus.
    let left = Memo::new(move |_| {
        state.job.with(|job| match job {
            Some(ImportJob {
                state: ImportJobState::Ready { batch },
                ..
            }) => state.known.with(|known| rows_left(batch, known)),
            _ => false,
        })
    });
    // The picked row, as its key: each row compares itself with it, so a
    // hover or an arrow key restyles two rows, not the list (M7).
    let picked_key = Memo::new(move |_| {
        rows.with(|rows| {
            let at = state.picked.get().min(rows.len().saturating_sub(1));
            rows.get(at).map(|row| import_key(row.source, &row.id))
        })
    });
    let rows_view = move || {
        let now = (state.now)();
        let row_view = move |row: ImportCandidate| {
            let key = import_key(row.source, &row.id);
            let chosen = {
                let key = key.clone();
                Memo::new(move |_| {
                    picked_key.with(|picked| picked.as_deref() == Some(key.as_str()))
                })
            };
            let detail = export_row_detail(&row, now);
            let title = bruhs_models::logic::text::plain_title(&row.title).into_owned();
            let hover = move |_| {
                let at = rows.with_untracked(|rows| {
                    rows.iter()
                        .position(|each| import_key(each.source, &each.id) == key)
                });
                if let Some(at) = at {
                    state.picked.set(at);
                }
            };
            view! {
                <button
                    type="button"
                    role="option"
                    aria-selected=move || chosen.get().to_string()
                    on:mouseenter=hover
                    on:click=move |_| import_row(row.clone())
                    class=move || {
                        // Blueberry marks the selection (design rule).
                        if chosen.get() {
                            format!("{ROW} bg-blueberry-600/20 inset-ring inset-ring-blueberry-500/30")
                        } else {
                            format!("{ROW} hover:bg-longan-700/40")
                        }
                    }
                >
                    <span data-export-title="" class="w-full truncate text-label-sm text-rambutan-200">{title}</span>
                    <span data-export-detail="" class="w-full truncate text-label-sm text-rambutan-400">{detail}</span>
                </button>
            }
        };
        let matches = move || {
            if rows.with(Vec::is_empty) {
                return view! { <p data-export-none="" class="py-2 text-label-sm text-rambutan-300">{NO_MATCHING_CHATS}</p> }
                    .into_any();
            }
            let more = move || {
                let total = rows.with(Vec::len);
                (total > MAX_ROWS_SHOWN).then(|| {
                    view! { <p data-export-more="" class="px-4 py-2 text-label-sm text-rambutan-400">{rows_shown_line(MAX_ROWS_SHOWN, total)}</p> }
                })
            };
            view! {
                <div class="-mx-4 max-h-64 overflow-y-auto">
                    <div role="listbox" aria-label=EXPORT_TAB>
                        <For
                            each=move || rows.with(|rows| rows.iter().take(MAX_ROWS_SHOWN).cloned().collect::<Vec<_>>())
                            key=|row| import_key(row.source, &row.id)
                            children=row_view
                        />
                    </div>
                    {more}
                </div>
            }
            .into_any()
        };
        // Redrawn only when the search starts or stops matching.
        let empty = Memo::new(move |_| rows.with(Vec::is_empty));
        view! {
            <input
                node_ref=search_ref
                data-export-search=""
                type="text"
                placeholder=EXPORT_SEARCH
                aria-label=EXPORT_SEARCH
                prop:value=move || state.query.get()
                on:input=move |event| {
                    state.query.set(event_target_value(&event));
                    state.picked.set(0);
                }
                on:keydown=on_keydown
                class="w-full rounded-grape bg-longan-900 px-2.5 py-1.5 text-label-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-longan-700 focus:outline-none focus:inset-ring-persimmon-500/60"
            />
            {move || {
                empty.track();
                untrack(matches)
            }}
        }
        .into_any()
    };

    let list = move |batch: &ImportBatch| {
        if batch.candidates.is_empty() {
            return ().into_any();
        }
        (move || {
            if left.get() {
                rows_view()
            } else {
                view! { <p data-export-already="" class=LINE>{ALREADY_HERE}</p> }.into_any()
            }
        })
        .into_any()
    };
    // The export's memory of the owner, each offered as a bruh's memory:
    // it opens Move in at By prompt with the text and where it came from
    // (D14). The batch can go when this dialog closes; the text travels.
    let move_in = use_context::<MoveInOpen>().map(|open| open.0);
    let memory_view = move |batch: &ImportBatch| {
        batch
            .memory
            .iter()
            .filter(|suggestion| !suggestion.text.trim().is_empty())
            .map(|suggestion| {
                let source = suggestion.source;
                let text = suggestion.text.clone();
                let use_memory = move |_| {
                    if let Some(move_in) = move_in {
                        move_in.set(Some(MoveInStart {
                            tab: MoveInTab::ByPrompt,
                            text: Some(text.clone()),
                            from: Some(export_memory_from(source)),
                        }));
                    }
                };
                view! {
                    <div class="flex flex-wrap items-center gap-2">
                        <p data-export-memory="" class=LINE>{memory_found_line(source)}</p>
                        <button type="button" data-export-use-memory="" class=BUTTON on:click=use_memory>{USE_AS_MEMORY}</button>
                    </div>
                }
            })
            .collect_view()
    };
    let batch_view = move |batch: &ImportBatch| {
        let lines = batch_lines(batch);
        // The read names Grok's note among its warnings; it shows once, in
        // durian (D9).
        let grok = (batch.best_effort && !lines.iter().any(|line| line == GROK_BEST_EFFORT))
            .then(|| view! { <p data-export-grok="" class=WARN>{GROK_BEST_EFFORT}</p> });
        let lines = lines
            .into_iter()
            .map(|line| {
                let class = if line == GROK_BEST_EFFORT { WARN } else { LINE };
                view! { <p data-export-line="" class=class>{line}</p> }
            })
            .collect_view();
        let all = move || {
            rows.with(|rows| !rows.is_empty()).then(|| {
                view! { <button type="button" data-export-all="" class=BUTTON aria-disabled=move || busy.get().then_some("true") on:click=move |_| import_all()>{IMPORT_ALL}</button> }
            })
        };
        let memory = memory_view(batch);
        view! {
            {lines}
            {grok}
            {memory}
            {list(batch)}
            <div class="flex flex-wrap items-center gap-2">
                {all}
                <button type="button" node_ref=another_ref data-export-another="" class=BUTTON aria-disabled=move || busy.get().then_some("true") on:click=move |_| if !busy.get_untracked() { pick() }>{CHOOSE_ANOTHER}</button>
            </div>
        }
        .into_any()
    };

    // Redrawn when the read moves between states, not on every poll or
    // keystroke, so focus and the typed search stay.
    let phase = Memo::new(move |_| {
        state.job.with(|job| match job {
            Some(ImportJob {
                state: ImportJobState::Ready { batch },
                ..
            }) => Some(batch.id.clone()),
            _ => None,
        })
    });
    let body = move || match phase.get() {
        None => start(),
        Some(_) => state
            .job
            .with_untracked(|job| match job {
                Some(ImportJob {
                    state: ImportJobState::Ready { batch },
                    ..
                }) => Some(batch_view(batch)),
                _ => None,
            })
            .unwrap_or_else(start),
    };
    let status = move || state.status.get();

    view! {
        <div data-export-tab="" class="flex flex-col gap-2 px-4 py-3">
            <input node_ref=file_input type="file" multiple accept=ACCEPT tabindex="-1" aria-hidden="true" class="sr-only" on:change=move |_| chose() />
            {body}
            <p role="status" data-export-status="" class=STATUS>{status}</p>
            <p role="alert" data-export-error="" class=ERROR>{move || state.error.get()}</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::imports::BatchCounts;
    use leptos::tachys;

    const NOW: i64 = 3 * 86_400_000;

    fn row(source: ImportSource, id: &str, title: &str) -> ImportCandidate {
        ImportCandidate {
            source,
            id: id.into(),
            title: title.into(),
            cwd: None,
            updated_at: EpochMs::saturating_from_i64(86_400_000),
            provider: None,
            session_id: None,
            model: None,
        }
    }

    fn batch() -> ImportBatch {
        ImportBatch {
            id: "0123456789abcdef0123456789abcdef".into(),
            candidates: vec![
                row(ImportSource::ChatGpt, "a1", "Trip to Lisbon"),
                row(ImportSource::Grok, "g1", "Rocket notes"),
                row(ImportSource::ChatGpt, "a2", "lisbon restaurants"),
                row(ImportSource::Grok, "g2", "Tax <questions> & more"),
            ],
            counts: BatchCounts {
                conversations: 4,
                skipped_empty: 1,
                ..Default::default()
            },
            best_effort: true,
            ..Default::default()
        }
    }

    fn render(view: impl FnOnce() -> AnyView) -> String {
        untrack(|| tachys::view::RenderHtml::to_html(view()))
    }

    fn escaped(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// The text of every element carrying `marker`, in page order.
    fn texts<'a>(html: &'a str, marker: &str) -> Vec<&'a str> {
        html.match_indices(marker)
            .map(|(at, _)| {
                let start = at + html[at..].find('>').unwrap() + 1;
                let end = start + html[start..].find('<').unwrap();
                &html[start..end]
            })
            .collect()
    }

    /// The opening tag of the element carrying `marker`.
    fn tag<'a>(html: &'a str, marker: &str) -> &'a str {
        let at = html
            .find(marker)
            .unwrap_or_else(|| panic!("{marker} in {html}"));
        let start = html[..at].rfind('<').unwrap();
        let end = at + html[at..].find('>').unwrap();
        &html[start..=end]
    }

    fn state(known: &[&str]) -> ExportState {
        let known: HashSet<String> = known.iter().map(|key| (*key).to_string()).collect();
        let mut state = ExportState::new(Signal::stored(known));
        state.now = || EpochMs::saturating_from_i64(NOW);
        state
    }

    fn tab(state: ExportState) -> String {
        tab_busy(state, false)
    }

    fn tab_busy(state: ExportState, busy: bool) -> String {
        // The tab's focus effect needs an executor to be created.
        let _ = any_spawner::Executor::init_futures_executor();
        render(move || {
            provide_context(state);
            view! { <ExportTab on_import=Callback::new(|_| {}) busy=Signal::stored(busy) /> }
                .into_any()
        })
    }

    /// Whether a tag has the `disabled` attribute (not a `disabled:` class).
    fn is_disabled(tag: &str) -> bool {
        tag.match_indices(" disabled")
            .any(|(at, word)| !tag[at + word.len()..].starts_with(':'))
    }

    fn ready(state: ExportState, batch: ImportBatch) {
        state.job.set(Some(ImportJob {
            id: batch.id.clone(),
            state: ImportJobState::Ready {
                batch: Box::new(batch),
            },
        }));
    }

    #[test]
    fn choices_the_server_would_refuse_are_refused_first() {
        let cap = MAX_UPLOAD_BYTES.to_f64_lossy();
        assert_eq!(upload_refusal(MAX_UPLOAD_FILES, cap), None);
        assert_eq!(
            upload_refusal(MAX_UPLOAD_FILES + 1, 1.0),
            Some(TOO_MANY_FILES)
        );
        assert_eq!(upload_refusal(1, cap + 1.0), Some(EXPORT_TOO_LARGE));
    }

    #[test]
    fn import_all_names_each_source() {
        use ImportSource::{ChatGpt, Grok};
        assert_eq!(
            done_line(&[Grok, ChatGpt, ChatGpt]).as_deref(),
            Some("Imported 2 chats from ChatGPT. Imported 1 chat from Grok.")
        );
        assert_eq!(done_line(&[]), None);
    }

    #[test]
    fn long_batches_show_the_first_rows_and_say_so() {
        Owner::new().with(|| {
            let mut long = batch();
            long.candidates = (0..MAX_ROWS_SHOWN + 2)
                .map(|n| {
                    row(
                        ImportSource::ChatGpt,
                        &format!("c{n}"),
                        &format!("Chat {n}"),
                    )
                })
                .collect();
            let state = state(&["chatgpt:c0"]);
            ready(state, long);
            let html = tab(state);
            assert_eq!(texts(&html, "data-export-title").len(), MAX_ROWS_SHOWN);
            assert_eq!(texts(&html, "data-export-title")[0], "Chat 1");
            assert_eq!(
                texts(&html, "data-export-more"),
                [rows_shown_line(MAX_ROWS_SHOWN, MAX_ROWS_SHOWN + 1)]
            );
        });
    }

    #[test]
    fn rows_queue_one_at_a_time() {
        Owner::new().with(|| {
            let state = state(&[]);
            assert!(state.enqueue(row(ImportSource::ChatGpt, "a1", "One")));
            assert!(state.importing.get_untracked());
            // A second row waits for the running import.
            assert!(!state.enqueue(row(ImportSource::ChatGpt, "a2", "Two")));
            assert_eq!(state.queue.with_value(VecDeque::len), 2);
        });
    }

    #[test]
    fn export_rows_hide_imported_chats() {
        let batch = batch();
        let known: HashSet<String> = [import_key(ImportSource::Grok, "g1")].into();
        let ids = |rows: Vec<ImportCandidate>| -> Vec<String> {
            rows.into_iter().map(|row| row.id).collect()
        };
        assert_eq!(
            ids(visible_export_rows(&batch, &known, "")),
            ["a1", "a2", "g2"]
        );
        assert_eq!(
            ids(visible_export_rows(&batch, &known, "  LISBON ")),
            ["a1", "a2"]
        );
        // The same id from another source is another chat.
        let chatgpt_g1: HashSet<String> = [import_key(ImportSource::ChatGpt, "g1")].into();
        assert_eq!(
            ids(visible_export_rows(&batch, &chatgpt_g1, "rocket")),
            ["g1"]
        );
    }

    #[test]
    fn busy_buttons_keep_focus_and_say_they_wait() {
        Owner::new().with(|| {
            // A disabled control that holds focus drops it to the page
            // behind the dialog, so busy buttons stay enabled (I1).
            let state = state(&[]);
            state.sending.set(Some(1));
            let html = tab(state);
            let choose = tag(&html, "data-export-choose");
            assert!(!is_disabled(choose), "{choose}");
            assert!(choose.contains("aria-disabled=\"true\""), "{choose}");
            state.sending.set(None);
            let html = tab(state);
            assert!(
                !tag(&html, "data-export-choose").contains("aria-disabled=\""),
                "{html}"
            );

            ready(state, batch());
            for busy in [true, false] {
                let html = tab_busy(state, busy);
                for marker in ["data-export-all", "data-export-another"] {
                    let button = tag(&html, marker);
                    assert!(!is_disabled(button), "{button}");
                    assert_eq!(button.contains("aria-disabled=\"true\""), busy, "{button}");
                }
            }
        });
    }

    #[test]
    fn live_regions_stay_in_the_tree() {
        Owner::new().with(|| {
            let html = tab(state(&[]));
            for marker in ["data-export-status", "data-export-error"] {
                let line = tag(&html, marker);
                assert!(line.contains("empty:sr-only"), "{line}");
                assert!(!line.contains("empty:hidden"), "{line}");
            }
        });
    }

    #[test]
    fn the_reading_line_changes_with_the_files_done() {
        let mut last = None;
        assert_eq!(
            reading_progress(&mut last, 0, 2, 0),
            Some(reading_line(0, 2, 0))
        );
        // More chats on the same file: the polite region stays still.
        assert_eq!(reading_progress(&mut last, 0, 2, 40), None);
        assert_eq!(
            reading_progress(&mut last, 1, 2, 80),
            Some(reading_line(1, 2, 80))
        );
        assert_eq!(reading_progress(&mut last, 1, 2, 120), None);
    }

    #[test]
    fn a_refusal_focuses_a_control_that_is_there() {
        Owner::new().with(|| {
            let state = state(&[]);
            assert_eq!(state.choose_focus(), ExportFocus::Choose);
            ready(state, batch());
            assert_eq!(state.choose_focus(), ExportFocus::Another);
        });
    }

    #[test]
    fn the_search_uses_shared_copy_and_says_when_nothing_matches() {
        Owner::new().with(|| {
            let state = state(&[]);
            ready(state, batch());
            let html = tab(state);
            let search = tag(&html, "data-export-search");
            assert!(
                search.contains(&format!("placeholder=\"{EXPORT_SEARCH}\"")),
                "{search}"
            );
            assert!(
                search.contains(&format!("aria-label=\"{EXPORT_SEARCH}\"")),
                "{search}"
            );
            assert!(!html.contains("Search titles and folders"), "{html}");
            assert!(!html.contains(NO_MATCHING_CHATS), "{html}");
            // The selected row wears the selection tone (blueberry).
            assert!(
                tag(&html, "aria-selected=\"true\"").contains("bg-blueberry"),
                "{html}"
            );

            state.query.set("no such chat".into());
            let html = tab(state);
            assert_eq!(
                texts(&html, "data-export-none"),
                [NO_MATCHING_CHATS],
                "{html}"
            );
            assert!(tag(&html, "data-export-none").contains("text-rambutan-300"));
            assert!(!html.contains(ALREADY_HERE), "{html}");
            assert!(!html.contains(IMPORT_ALL), "{html}");
        });
    }

    #[test]
    fn an_exports_memory_is_offered_as_a_bruhs_memory() {
        use bruhs_models::imports::MemorySuggestion;
        Owner::new().with(|| {
            let state = state(&[]);
            let mut with_memory = batch();
            with_memory.memory = vec![
                MemorySuggestion {
                    source: ImportSource::ClaudeAi,
                    text: "Likes short answers.".into(),
                },
                MemorySuggestion {
                    source: ImportSource::ChatGpt,
                    text: "  ".into(),
                },
            ];
            ready(state, with_memory);
            let html = tab(state);
            assert_eq!(
                texts(&html, "data-export-memory"),
                [escaped(&memory_found_line(ImportSource::ClaudeAi))],
                "{html}"
            );
            assert_eq!(texts(&html, "data-export-use-memory"), [USE_AS_MEMORY]);
            ready(state, batch());
            assert!(!tab(state).contains(USE_AS_MEMORY));
        });
    }

    #[test]
    fn the_export_tab_reads_from_the_shared_lines() {
        Owner::new().with(|| {
            let state = state(&[]);
            let html = tab(state);
            assert_eq!(
                texts(&html, "data-export-help"),
                [EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK],
                "{html}"
            );
            assert_eq!(texts(&html, "data-export-grok"), [GROK_BEST_EFFORT]);
            assert!(
                tag(&html, "data-export-grok").contains("text-durian"),
                "{html}"
            );
            assert_eq!(texts(&html, "data-export-choose"), [CHOOSE_EXPORT]);
            let input = tag(&html, "type=\"file\"");
            assert!(input.contains(" multiple"), "{input}");
            assert!(input.contains(&format!("accept=\"{ACCEPT}\"")), "{input}");
            assert_eq!(ACCEPT, ".zip,.json,application/zip,application/json");
            assert!(!html.contains(IMPORT_ALL), "{html}");

            let batch = batch();
            ready(state, batch.clone());
            let html = tab(state);
            let lines: Vec<String> = batch_lines(&batch).iter().map(|l| escaped(l)).collect();
            assert!(!lines.is_empty());
            assert_eq!(texts(&html, "data-export-line"), lines, "{html}");
            assert_eq!(texts(&html, "data-export-grok"), [GROK_BEST_EFFORT]);
            let now = EpochMs::saturating_from_i64(NOW);
            let titles: Vec<String> = batch
                .candidates
                .iter()
                .map(|row| escaped(&row.title))
                .collect();
            let details: Vec<String> = batch
                .candidates
                .iter()
                .map(|row| export_row_detail(row, now))
                .collect();
            assert_eq!(texts(&html, "data-export-title"), titles, "{html}");
            assert_eq!(texts(&html, "data-export-detail"), details, "{html}");
            assert!(details[1].ends_with(", best effort"));
            assert_eq!(texts(&html, "data-export-all"), [IMPORT_ALL]);
            assert_eq!(texts(&html, "data-export-another"), [CHOOSE_ANOTHER]);
            assert!(texts(&html, "data-export-help").is_empty(), "{html}");
            assert!(!html.contains(ALREADY_HERE), "{html}");

            // The read's own warning carries the note: it shows once, in
            // durian, among the lines.
            let mut warned = batch.clone();
            warned.warnings = vec![GROK_BEST_EFFORT.to_string()];
            ready(state, warned.clone());
            let html = tab(state);
            let lines: Vec<String> = batch_lines(&warned).iter().map(|l| escaped(l)).collect();
            assert_eq!(texts(&html, "data-export-line"), lines, "{html}");
            assert_eq!(html.matches(GROK_BEST_EFFORT).count(), 1, "{html}");
            assert!(
                tag(&html, &format!("{GROK_BEST_EFFORT}<")).contains("text-durian"),
                "{html}"
            );

            let every: Vec<&str> = vec!["chatgpt:a1", "grok:g1", "chatgpt:a2", "grok:g2"];
            let state = self::state(&every);
            ready(state, batch);
            let html = tab(state);
            assert_eq!(
                texts(&html, "data-export-already"),
                [ALREADY_HERE],
                "{html}"
            );
            assert!(texts(&html, "data-export-title").is_empty(), "{html}");
            assert_eq!(texts(&html, "data-export-another"), [CHOOSE_ANOTHER]);
        });
    }
}
