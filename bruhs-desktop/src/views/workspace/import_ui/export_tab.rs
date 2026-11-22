//! The Import thread dialog's Export file tab (D1a), the desktop half of
//! the web's `import_thread_dialog/export_tab.rs`: where to get each
//! export, choosing files or folders in the native picker, the read's
//! progress and summary, the rows and Import all. The server reads the
//! picked paths in place (D19); only it owns `<core>/imports`.
//!
//! The read and its rows live in [`ExportPicker`], so leaving the tab
//! keeps them. An import keeps the dialog open on the batch, and the
//! dialog discards the batch whenever it closes (F23).

use super::*;
use bruhs_models::imports::{
    batch_lines, export_row_detail, imported_line, importing_line, reading_line, rows_shown_line,
    valid_batch_id, ImportBatch, ImportJob, ImportJobState, ALREADY_HERE, CHOOSE_ANOTHER,
    CHOOSE_EXPORT, EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK, EXPORT_SEARCH,
    GROK_BEST_EFFORT, IMPORT_ALL, IMPORT_ALL_MAX, IMPORT_FAILED, MAX_ROWS_SHOWN, MAX_UPLOAD_FILES,
    NO_MATCHING_CHATS, TOO_MANY_FILES,
};
use gpui::PathPromptOptions;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

mod render;

/// How often a read is asked how it stands (D3).
const POLL: Duration = Duration::from_millis(500);

/// One server call behind the tab, answered on the UI thread.
pub(in crate::views::workspace) type ExportCall<A, T> =
    Rc<dyn Fn(A, &App) -> Task<Result<T, String>>>;

/// The server calls the tab makes, as one seam: the app uses the server,
/// and tests answer in its place. Each failure is the line the tab shows.
#[derive(Clone)]
pub(in crate::views::workspace) struct ExportCalls {
    /// `POST /api/import/files`.
    pub(in crate::views::workspace) read: ExportCall<Vec<String>, ImportJob>,
    /// `GET /api/import/uploads/{id}`.
    pub(in crate::views::workspace) job: ExportCall<String, ImportJob>,
    /// `GET /api/import/thread`: source, batch id, conversation id.
    pub(in crate::views::workspace) thread:
        ExportCall<(ImportSource, String, String), ImportedThread>,
    /// `DELETE /api/import/uploads/{id}`, sent and not waited on: closing
    /// the dialog needs no answer, and a batch already gone is what it
    /// asked for.
    pub(in crate::views::workspace) discard: Rc<dyn Fn(String)>,
}

impl ExportCalls {
    /// The calls through the app's server client, on the desktop's tokio
    /// runtime.
    fn server(cx: &App) -> Self {
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let client = crate::server_client::ServerClient::workbench_client;
        Self {
            read: Rc::new(move |paths: Vec<String>, cx: &App| {
                let client = client();
                crate::app_state::spawn_tokio(
                    cx,
                    async move { client.read_export_files(paths).await },
                )
            }),
            job: Rc::new(move |id: String, cx: &App| {
                let client = client();
                crate::app_state::spawn_tokio(cx, async move { client.import_job(&id).await })
            }),
            thread: Rc::new(
                move |(source, batch, id): (ImportSource, String, String), cx: &App| {
                    let client = client();
                    crate::app_state::spawn_tokio(cx, async move {
                        client.export_thread(source, &batch, &id).await
                    })
                },
            ),
            discard: Rc::new(move |id: String| {
                let client = client();
                drop(runtime.spawn(async move {
                    let _gone = client.discard_import(&id).await;
                }));
            }),
        }
    }
}

/// The tab's own controls, made once the dialog first opens.
pub(in crate::views::workspace) struct ExportUi {
    pub(in crate::views::workspace) search: Entity<InputState>,
    pub(in crate::views::workspace) choose: FocusHandle,
    /// The rows, one Tab stop: ↑, ↓ and Enter work there and in the search.
    pub(in crate::views::workspace) list: FocusHandle,
    pub(in crate::views::workspace) all: FocusHandle,
    pub(in crate::views::workspace) another: FocusHandle,
    _search_changes: Subscription,
}

/// The control that takes focus after the tab changes under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::views::workspace) enum ExportFocus {
    /// **Choose export file**: while a read runs, and after a failed one.
    Choose,
    /// The search: once a batch is ready, and after an import.
    Search,
    /// **Choose another file**: once every chat is here.
    Another,
}

/// The Export file tab's read, rows and imports.
pub(in crate::views::workspace) struct ExportPicker {
    /// The read: `Reading` until it lands, then `Ready` with its batch.
    pub(in crate::views::workspace) job: Option<ImportJob>,
    /// The picked paths are on their way, before the server answers.
    pub(in crate::views::workspace) sending: bool,
    pub(in crate::views::workspace) selected: usize,
    pub(in crate::views::workspace) error: Option<String>,
    /// `reading_line` while a read runs (changed only as files finish),
    /// `importing_line` while rows come in, then `imported_line`.
    pub(in crate::views::workspace) status: Option<String>,
    /// Rows are being imported.
    pub(in crate::views::workspace) importing: bool,
    /// The server, or a test's stand-in. Set when the dialog first opens.
    pub(in crate::views::workspace) calls: Option<ExportCalls>,
    pub(in crate::views::workspace) ui: Option<ExportUi>,
    /// Moves on every new read and every discard: an answer for an older
    /// one is dropped.
    generation: u64,
    /// The files done (and file count) the status line shows.
    shown: Option<(u32, u32)>,
    /// Rows waiting to be imported, one at a time, and the sources of
    /// those done.
    queue: VecDeque<ImportCandidate>,
    done: Vec<ImportSource>,
    scroll: ScrollHandle,
    /// The scrolling part of the tab: the summary and the rows.
    body_scroll: ScrollHandle,
    /// The rows and whether any chat is left, kept until the batch, the
    /// search or the threads change: a draw never rebuilds them (M3).
    rows_cache: RefCell<Option<RowsCache>>,
    poll_task: Option<Task<()>>,
    import_task: Option<Task<()>>,
}

/// Which import keys the threads here carry: how many, and a sum of
/// their hashes that is the same in any order. Deleting one imported
/// thread and importing another between two draws changes it, where a
/// count of threads would not.
type ImportsHere = (usize, u64);

pub(in crate::views::workspace) fn imports_here<'a>(
    keys: impl IntoIterator<Item = &'a str>,
) -> ImportsHere {
    use std::hash::{BuildHasher as _, BuildHasherDefault, DefaultHasher};
    let hasher = BuildHasherDefault::<DefaultHasher>::default();
    keys.into_iter().fold((0, 0), |(count, sum), key| {
        (count + 1, sum.wrapping_add(hasher.hash_one(key)))
    })
}

/// What [`ExportPicker::rows_cache`] was built from, and what it holds.
struct RowsCache {
    batch: String,
    query: String,
    /// Which imported threads are here (see [`imports_here`]).
    threads: ImportsHere,
    rows: Rc<Vec<ImportCandidate>>,
    left: bool,
}

impl Default for ExportPicker {
    fn default() -> Self {
        Self {
            job: None,
            sending: false,
            selected: 0,
            error: None,
            status: None,
            importing: false,
            calls: None,
            ui: None,
            generation: 0,
            shown: None,
            queue: VecDeque::new(),
            done: Vec::new(),
            scroll: ScrollHandle::new(),
            body_scroll: ScrollHandle::new(),
            rows_cache: RefCell::new(None),
            poll_task: None,
            import_task: None,
        }
    }
}

impl ExportPicker {
    /// The ready batch.
    fn batch(&self) -> Option<&ImportBatch> {
        match &self.job {
            Some(ImportJob {
                state: ImportJobState::Ready { batch },
                ..
            }) => Some(batch.as_ref()),
            _ => None,
        }
    }

    /// A read is on its way or running.
    fn reading(&self) -> bool {
        self.sending
            || matches!(
                self.job,
                Some(ImportJob {
                    state: ImportJobState::Reading { .. },
                    ..
                })
            )
    }
}

/// The batch's rows whose chat isn't a thread yet, in the batch's order,
/// whose title holds `query` (any case). Uncapped: the list shows the
/// first [`MAX_ROWS_SHOWN`], and Import all takes the first
/// [`IMPORT_ALL_MAX`].
pub(in crate::views::workspace) fn export_rows(
    batch: &ImportBatch,
    known_keys: &HashSet<String>,
    query: &str,
) -> Vec<ImportCandidate> {
    let query = query.trim().to_lowercase();
    visible_candidates(&batch.candidates, &HashSet::new(), known_keys)
        .into_iter()
        .filter(|row| query.is_empty() || row.title.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

/// The ready batch's summary lines, each with whether it shows in durian.
/// The read names Grok's note among its warnings; it shows once (D9).
pub(in crate::views::workspace) fn export_lines(batch: &ImportBatch) -> Vec<(String, bool)> {
    let mut lines: Vec<(String, bool)> = batch_lines(batch)
        .into_iter()
        .map(|line| {
            let warn = line == GROK_BEST_EFFORT;
            (line, warn)
        })
        .collect();
    if batch.best_effort && !lines.iter().any(|(line, _)| line == GROK_BEST_EFFORT) {
        lines.push((GROK_BEST_EFFORT.to_owned(), true));
    }
    lines
}

/// `imported_line` for each source that brought chats in, in the exports'
/// order.
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
/// since `shown`, as the web's polite line.
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

/// Whether any of the batch's chats isn't here yet.
fn rows_left(batch: &ImportBatch, known_keys: &HashSet<String>) -> bool {
    batch
        .candidates
        .iter()
        .any(|row| !known_keys.contains(&import_key(row.source, &row.id)))
}

impl Workspace {
    /// The tab's controls and calls, made once.
    pub(in crate::views::workspace) fn prepare_export_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.import_picker.export.calls.is_none() {
            self.import_picker.export.calls = Some(ExportCalls::server(cx));
        }
        if self.import_picker.export.ui.is_some() {
            return;
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(EXPORT_SEARCH));
        let search_changes = cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, _input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let export = &mut this.import_picker.export;
                    export.selected = 0;
                    export.scroll.set_offset(Point::default());
                    cx.notify();
                }
            },
        );
        self.import_picker.export.ui = Some(ExportUi {
            search,
            choose: cx.focus_handle().tab_stop(true),
            list: cx.focus_handle().tab_stop(true),
            all: cx.focus_handle().tab_stop(true),
            another: cx.focus_handle().tab_stop(true),
            _search_changes: search_changes,
        });
    }

    /// Focuses `target` once the tab has drawn it, while the tab is shown.
    pub(in crate::views::workspace) fn focus_export_control(
        &self,
        target: ExportFocus,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.import_picker.tab != ImportTab::Export {
            return;
        }
        let Some(ui) = &self.import_picker.export.ui else {
            return;
        };
        let handle = match target {
            ExportFocus::Choose => ui.choose.clone(),
            ExportFocus::Search => ui.search.focus_handle(cx),
            ExportFocus::Another => ui.another.clone(),
        };
        cx.defer_in(window, move |_this, window, cx| handle.focus(window, cx));
    }

    /// The search field the dialog's keys go through on this tab.
    pub(in crate::views::workspace) fn active_import_input(&self) -> Entity<InputState> {
        match (&self.import_picker.tab, &self.import_picker.export.ui) {
            (ImportTab::Export, Some(ui)) => ui.search.clone(),
            _ => self.import_input.clone(),
        }
    }

    /// Whether ↑, ↓ and Enter choose and import a row: always on a local
    /// tab, and on the Export file tab from its search or its rows.
    pub(in crate::views::workspace) fn import_keys_reach_rows(
        &self,
        window: &Window,
        cx: &App,
    ) -> bool {
        if self.import_picker.tab != ImportTab::Export {
            return true;
        }
        self.import_picker.export.ui.as_ref().is_some_and(|ui| {
            ui.search.focus_handle(cx).is_focused(window) || ui.list.is_focused(window)
        })
    }

    fn export_busy(&self) -> bool {
        self.import_picker.importing || self.import_picker.export.importing
    }

    /// The ready batch's rows not yet here that match the search, and
    /// whether any of its chats isn't here yet. Rebuilt only when the
    /// batch, the search or the threads change.
    pub(in crate::views::workspace) fn export_view(
        &self,
        cx: &App,
    ) -> (Rc<Vec<ImportCandidate>>, bool) {
        let export = &self.import_picker.export;
        let Some(batch) = export.batch() else {
            return (Rc::new(Vec::new()), false);
        };
        let query = export
            .ui
            .as_ref()
            .map(|ui| ui.search.read(cx).value().to_string())
            .unwrap_or_default();
        let threads = imports_here(
            self.threads
                .iter()
                .filter_map(|thread| thread.chat.read(cx).import_key.as_deref()),
        );
        if let Some(cache) = export.rows_cache.borrow().as_ref() {
            if cache.batch == batch.id && cache.query == query && cache.threads == threads {
                return (Rc::clone(&cache.rows), cache.left);
            }
        }
        let keys = self.known_import_keys(cx);
        let rows = Rc::new(export_rows(batch, &keys, &query));
        let left = rows_left(batch, &keys);
        *export.rows_cache.borrow_mut() = Some(RowsCache {
            batch: batch.id.clone(),
            query,
            threads,
            rows: Rc::clone(&rows),
            left,
        });
        (rows, left)
    }

    /// The ready batch's rows not yet here that match the search.
    pub(in crate::views::workspace) fn visible_export_rows(
        &self,
        cx: &App,
    ) -> Rc<Vec<ImportCandidate>> {
        self.export_view(cx).0
    }

    pub(in crate::views::workspace) fn move_export_selection(
        &mut self,
        key: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let count = self.visible_export_rows(cx).len().min(MAX_ROWS_SHOWN);
        let export = &mut self.import_picker.export;
        if let Some(next) = Self::move_command_selected_index(export.selected, count, key) {
            export.selected = next;
            export.scroll.scroll_to_item(next);
            cx.notify();
        }
    }

    pub(in crate::views::workspace) fn import_selected_export(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let rows = self.visible_export_rows(cx);
        let count = rows.len().min(MAX_ROWS_SHOWN);
        let at = Self::clamp_command_selected_index(self.import_picker.export.selected, count);
        if let Some(row) = rows.iter().take(count).nth(at).cloned() {
            self.import_export_row(row, window, cx);
        }
    }

    /// **Choose export file**: the native picker, files and folders, then
    /// the read of what was picked.
    pub(in crate::views::workspace) fn choose_export_files(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.export_busy() || self.import_picker.export.reading() {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some(CHOOSE_EXPORT.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picker.await else {
                return;
            };
            let _closed = this.update_in(cx, |this, window, cx| {
                if this.top_layer == Some(TopLayer::ImportThread) {
                    this.read_export_paths(paths, window, cx);
                }
            });
        })
        .detach();
    }

    /// Drops any earlier read and asks the server to read `paths` in place.
    pub(in crate::views::workspace) fn read_export_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if paths.is_empty() || self.export_busy() || self.import_picker.export.reading() {
            return;
        }
        if paths.len() > MAX_UPLOAD_FILES {
            // Refused before anything is sent; focus goes to the button
            // that is drawn.
            let target = if self.import_picker.export.batch().is_some() {
                ExportFocus::Another
            } else {
                ExportFocus::Choose
            };
            self.import_picker.export.error = Some(TOO_MANY_FILES.to_owned());
            self.focus_export_control(target, window, cx);
            cx.notify();
            return;
        }
        self.discard_import_batch();
        let export = &mut self.import_picker.export;
        let Some(calls) = export.calls.clone() else {
            return;
        };
        let generation = export.generation;
        let count = u32::try_from(paths.len()).unwrap_or(u32::MAX);
        export.sending = true;
        export.shown = Some((0, count));
        export.status = Some(reading_line(0, count, 0));
        let paths = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let answer = (calls.read)(paths, cx);
        // **Choose another file** goes with the batch; the start view's
        // **Choose export file** holds focus while the read runs.
        self.focus_export_control(ExportFocus::Choose, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _closed = this.update_in(cx, |this, window, cx| {
                this.export_job_answered(generation, true, answer, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Asks again in 500 ms how read `id` stands.
    pub(in crate::views::workspace) fn poll_import_job(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let export = &mut self.import_picker.export;
        let Some(calls) = export.calls.clone() else {
            return;
        };
        let generation = export.generation;
        export.poll_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(POLL).await;
            let Ok(answer) = this.update(cx, |_this, cx| (calls.job)(id, cx)) else {
                return;
            };
            let answer = answer.await;
            let _closed = this.update_in(cx, |this, window, cx| {
                this.export_job_answered(generation, false, answer, window, cx);
            });
        }));
    }

    /// The server's answer for read `generation`: the first one, from
    /// `read`, or a poll's.
    fn export_job_answered(
        &mut self,
        generation: u64,
        first: bool,
        answer: Result<ImportJob, String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let open = self.top_layer == Some(TopLayer::ImportThread);
        let export = &mut self.import_picker.export;
        if !open || generation != export.generation {
            // The dialog closed, or another file was chosen, before the
            // server named this read: nobody can discard it but here.
            if let (true, Ok(job), Some(calls)) = (first, &answer, &export.calls) {
                if valid_batch_id(&job.id) {
                    (calls.discard)(job.id.clone());
                }
            }
            return;
        }
        let job = match answer {
            Ok(job) => job,
            Err(message) => {
                self.failed_export(message, window, cx);
                return;
            }
        };
        match job.state {
            ImportJobState::Reading {
                files_done,
                files,
                conversations,
            } => {
                export.sending = false;
                if let Some(line) =
                    reading_progress(&mut export.shown, files_done, files, conversations)
                {
                    export.status = Some(line);
                }
                let id = job.id.clone();
                export.job = Some(job);
                self.poll_import_job(id, window, cx);
            }
            ImportJobState::Ready { .. } => {
                export.sending = false;
                export.status = None;
                export.selected = 0;
                export.scroll.set_offset(Point::default());
                export.body_scroll.set_offset(Point::default());
                export.job = Some(job);
                if let Some(ui) = &export.ui {
                    let search = ui.search.clone();
                    search.update(cx, |input, cx| input.set_value("", window, cx));
                }
                self.after_export_change(window, cx);
            }
            ImportJobState::Failed { message } => {
                let message = if message.is_empty() {
                    IMPORT_FAILED.to_owned()
                } else {
                    message
                };
                self.failed_export(message, window, cx);
            }
        }
        cx.notify();
    }

    /// A read that failed: its line, and focus back on **Choose export
    /// file**.
    fn failed_export(&mut self, message: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.discard_import_batch();
        self.import_picker.export.error = Some(message);
        self.focus_export_control(ExportFocus::Choose, window, cx);
        cx.notify();
    }

    /// Focus after the rows changed: the search while chats are left, else
    /// **Choose another file**.
    fn after_export_change(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let left = self.export_view(cx).1;
        let target = if left {
            ExportFocus::Search
        } else {
            ExportFocus::Another
        };
        self.focus_export_control(target, window, cx);
    }

    fn import_export_row(
        &mut self,
        row: ImportCandidate,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.export_busy() {
            self.import_export_rows(vec![row], window, cx);
        }
    }

    /// **Import all**: the first [`IMPORT_ALL_MAX`] rows shown, in order.
    pub(in crate::views::workspace) fn import_all_exports(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.export_busy() {
            return;
        }
        let rows = self
            .visible_export_rows(cx)
            .iter()
            .take(IMPORT_ALL_MAX)
            .cloned()
            .collect();
        self.import_export_rows(rows, window, cx);
    }

    fn import_export_rows(
        &mut self,
        rows: Vec<ImportCandidate>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let export = &mut self.import_picker.export;
        if rows.is_empty() || export.batch().is_none() {
            return;
        }
        export.queue.extend(rows);
        export.done.clear();
        export.importing = true;
        export.error = None;
        export.status = None;
        self.drain_export(window, cx);
    }

    /// Imports the next queued row through the same path as a local
    /// import, behind the dialog. A failure stops the run and shows under
    /// the tab; a close stops it quietly.
    fn drain_export(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let export = &mut self.import_picker.export;
        let batch = export.batch().map(|batch| batch.id.clone());
        let (Some(batch), Some(calls)) = (batch, export.calls.clone()) else {
            export.queue.clear();
            export.importing = false;
            return;
        };
        let Some(row) = export.queue.pop_front() else {
            self.finish_export_imports(window, cx);
            return;
        };
        let total = export.done.len() + 1 + export.queue.len();
        if total > 1 {
            export.status = Some(importing_line(export.done.len() + 1, total));
        }
        let generation = export.generation;
        let source = row.source;
        let answer = (calls.thread)((row.source, batch, row.id), cx);
        export.import_task = Some(cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _closed = this.update_in(cx, |this, window, cx| {
                if this.top_layer != Some(TopLayer::ImportThread)
                    || generation != this.import_picker.export.generation
                {
                    return;
                }
                match answer {
                    Ok(imported) => {
                        this.open_imported_thread(imported, window, cx);
                        this.import_picker.export.done.push(source);
                        this.drain_export(window, cx);
                    }
                    Err(message) => {
                        this.import_picker.export.queue.clear();
                        this.import_picker.export.error = Some(message);
                        this.finish_export_imports(window, cx);
                    }
                }
            });
        }));
        cx.notify();
    }

    /// The run is over: the imported lines under the tab, its one
    /// announcement, as on the web (M10).
    fn finish_export_imports(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let export = &mut self.import_picker.export;
        export.importing = false;
        export.status = done_line(&export.done);
        self.after_export_change(window, cx);
        cx.notify();
    }

    /// Drops the read: stops its calls, forgets its rows and sends
    /// `DELETE` for its batch (F23). Closing the dialog and choosing
    /// another file both come here.
    pub(in crate::views::workspace) fn discard_import_batch(&mut self) {
        let export = &mut self.import_picker.export;
        export.generation = export.generation.wrapping_add(1);
        export.poll_task = None;
        export.import_task = None;
        export.queue.clear();
        export.done.clear();
        export.sending = false;
        export.selected = 0;
        export.error = None;
        export.status = None;
        export.importing = false;
        export.shown = None;
        let id = export
            .job
            .take()
            .map(|job| job.id)
            .filter(|id| valid_batch_id(id));
        if let (Some(id), Some(calls)) = (id, &export.calls) {
            (calls.discard)(id);
        }
    }

    /// The dialog is going, for any reason: its batch goes with it (F23).
    /// Called wherever `top_layer` is replaced. Move in goes the same way:
    /// its scans, its crew watch and any pasted answer are dropped, as the
    /// web unmounts its body.
    pub(in crate::views::workspace) fn leave_import_dialog(&mut self) {
        if self.top_layer == Some(TopLayer::ImportThread) {
            self.discard_import_batch();
        }
        if self.top_layer == Some(TopLayer::MoveIn) {
            self.move_in.open = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{done_line, imports_here, reading_progress};
    use bruhs_models::imports::reading_line;
    use bruhs_models::logic::portable::ImportSource;

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
    fn the_rows_follow_which_imports_are_here_not_how_many() {
        let before = imports_here(["grok:g1", "chatgpt:c1"]);
        // The same keys in another order are the same threads.
        assert_eq!(imports_here(["chatgpt:c1", "grok:g1"]), before);
        // One imported thread deleted and another imported between two
        // draws: the counts match, the rows must not stay as they were.
        let after = imports_here(["grok:g1", "chatgpt:c2"]);
        assert_eq!(after.0, before.0);
        assert_ne!(after, before);
        assert_eq!(imports_here([]), (0, 0));
    }

    #[test]
    fn the_reading_line_changes_with_the_files_done() {
        let mut last = None;
        assert_eq!(
            reading_progress(&mut last, 0, 2, 0),
            Some(reading_line(0, 2, 0))
        );
        assert_eq!(reading_progress(&mut last, 0, 2, 40), None);
        assert_eq!(
            reading_progress(&mut last, 1, 2, 80),
            Some(reading_line(1, 2, 80))
        );
    }
}
