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
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use bruhs_models::imports::{
    batch_lines, export_row_detail, imported_line, importing_line, reading_line, rows_shown_line,
    valid_batch_id, ImportBatch, ImportJob, ImportJobState, ALREADY_HERE, CHOOSE_ANOTHER,
    CHOOSE_EXPORT, EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK, EXPORT_SEARCH,
    GROK_BEST_EFFORT, IMPORT_ALL, IMPORT_ALL_MAX, IMPORT_FAILED, MAX_ROWS_SHOWN, MAX_UPLOAD_FILES,
    NO_MATCHING_CHATS, TOO_MANY_FILES,
};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
use gpui::PathPromptOptions;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// How often a read is asked how it stands (D3).
const POLL: Duration = Duration::from_millis(500);
/// Web `max-h-64`.
const EXPORT_LIST_MAX_HEIGHT_PX: f32 = 256.0;
/// Web `px-4`: the list reaches the dialog's edges, as the web's `-mx-4`.
const EXPORT_EDGE_PX: f32 = 16.0;
/// Web `bg-blueberry-600/20 inset-ring-blueberry-500/30`: the selection.
const EXPORT_SELECTED_BG_OPACITY: f32 = 0.20;
const EXPORT_SELECTED_RING_OPACITY: f32 = 0.30;
/// Web `focus:inset-ring-persimmon-500/60` on the search field.
const EXPORT_SEARCH_FOCUS_OPACITY: f32 = 0.60;
/// Web `aria-disabled:opacity-50`: a button that waits keeps its focus.
const EXPORT_WAITING_OPACITY: f32 = 0.50;

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
    poll_task: Option<Task<()>>,
    import_task: Option<Task<()>>,
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

/// One line of the tab's text, findable by its words in tests.
fn export_text(kind: &'static str, text: String, color: Hsla) -> Div {
    let selector = format!("desktop-import-export-{kind}:{text}");
    div()
        .debug_selector(move || selector)
        .flex_shrink_0()
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
        .text_color(color)
        .child(text)
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

    /// The ready batch's rows not yet here that match the search.
    pub(in crate::views::workspace) fn visible_export_rows(
        &self,
        cx: &App,
    ) -> Vec<ImportCandidate> {
        let export = &self.import_picker.export;
        let Some(batch) = export.batch() else {
            return Vec::new();
        };
        let query = export
            .ui
            .as_ref()
            .map(|ui| ui.search.read(cx).value().to_string())
            .unwrap_or_default();
        export_rows(batch, &self.known_import_keys(cx), &query)
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
        if let Some(row) = rows.into_iter().take(count).nth(at) {
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
        let keys = self.known_import_keys(cx);
        let left = self
            .import_picker
            .export
            .batch()
            .is_some_and(|batch| rows_left(batch, &keys));
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
            .into_iter()
            .take(IMPORT_ALL_MAX)
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

    /// The run is over: the imported lines under the tab and as the
    /// app's notice, as a single import says what came in.
    fn finish_export_imports(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let export = &mut self.import_picker.export;
        export.importing = false;
        let line = done_line(&export.done);
        export.status.clone_from(&line);
        if let Some(line) = line {
            self.show_thread_notice(line, false, cx);
        }
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
    /// Called wherever `top_layer` is replaced.
    pub(in crate::views::workspace) fn leave_import_dialog(&mut self) {
        if self.top_layer == Some(TopLayer::ImportThread) {
            self.discard_import_batch();
        }
    }

    /// A tab button: it keeps its Tab stop while it waits, so focus never
    /// drops out of the dialog, and does nothing until the wait is over.
    fn export_button(
        label: &'static str,
        focus: &FocusHandle,
        waiting: bool,
        cx: &Context<'_, Self>,
        handler: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
    ) -> Stateful<Div> {
        let el = web_button(
            SharedString::from(format!("desktop-import-export-{label}")),
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(move || format!("desktop-import-export-button:{label}"))
        .track_focus(focus)
        .flex_shrink_0()
        .min_h(px(theme::CONTROL_PX))
        .px_3()
        .border_color(theme::longan_700())
        .text_color(theme::rambutan_200())
        .aria_label(label)
        .when(waiting, |el| {
            el.opacity(EXPORT_WAITING_OPACITY).cursor_not_allowed()
        })
        .child(label);
        on_press_view(el, cx, handler)
    }

    /// The tab's body under the dialog's header. `rows` are
    /// [`Self::visible_export_rows`].
    pub(in crate::views::workspace) fn render_export_tab(
        &self,
        rows: &[ImportCandidate],
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let export = &self.import_picker.export;
        let Some(ui) = &export.ui else {
            return div().into_any_element();
        };
        let busy = self.export_busy();
        // In a short window the tab scrolls between the dialog's header and
        // footer, so the rows keep their height.
        let body = div()
            .id("desktop-import-export")
            .debug_selector(|| "desktop-import-export".to_string())
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3();
        let body = match export.batch() {
            None => body
                .children(
                    [EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK]
                        .into_iter()
                        .map(|line| export_text("help", line.to_owned(), theme::rambutan_300())),
                )
                .child(export_text(
                    "warn",
                    GROK_BEST_EFFORT.to_owned(),
                    theme::durian_300(),
                ))
                .child(div().flex().child(Self::export_button(
                    CHOOSE_EXPORT,
                    &ui.choose,
                    export.reading() || busy,
                    cx,
                    |this, window, cx| this.choose_export_files(window, cx),
                ))),
            Some(batch) => {
                let keys = self.known_import_keys(cx);
                let body = body.children(export_lines(batch).into_iter().map(|(line, warn)| {
                    if warn {
                        export_text("warn", line, theme::durian_300())
                    } else {
                        export_text("line", line, theme::rambutan_200())
                    }
                }));
                let body = if batch.candidates.is_empty() {
                    body
                } else if rows_left(batch, &keys) {
                    body.child(self.render_export_search(ui, window, cx))
                        .child(self.render_export_list(ui, rows, cx))
                } else {
                    body.child(export_text(
                        "line",
                        ALREADY_HERE.to_owned(),
                        theme::rambutan_200(),
                    ))
                };
                body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_shrink_0()
                        .when(!rows.is_empty(), |el| {
                            el.child(Self::export_button(
                                IMPORT_ALL,
                                &ui.all,
                                busy,
                                cx,
                                |this, window, cx| this.import_all_exports(window, cx),
                            ))
                        })
                        .child(Self::export_button(
                            CHOOSE_ANOTHER,
                            &ui.another,
                            busy,
                            cx,
                            |this, window, cx| this.choose_export_files(window, cx),
                        )),
                )
            }
        };
        body.when_some(export.status.clone(), |el, line| {
            el.child(export_text("status", line, theme::rambutan_300()))
        })
        .when_some(export.error.clone(), |el, line| {
            el.child(export_text("error", line, theme::lychee_300()))
        })
        .into_any_element()
    }

    fn render_export_search(
        &self,
        ui: &ExportUi,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let focused = ui.search.focus_handle(cx).is_focused(window);
        div()
            .debug_selector(|| "desktop-import-export-search".to_string())
            .flex_shrink_0()
            .w_full()
            .rounded(px(IMPORT_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(if focused {
                theme::persimmon_500().opacity(EXPORT_SEARCH_FOCUS_OPACITY)
            } else {
                theme::longan_700()
            })
            // Web `px-2.5`, less the 1px border; a small input in 2px
            // padding is the web's `py-1.5` field.
            .px(px(9.0))
            .py(px(2.0))
            .child(
                Input::new(&ui.search)
                    .small()
                    .aria_label(EXPORT_SEARCH)
                    .appearance(false)
                    .px_0()
                    .bordered(false)
                    .focus_bordered(false)
                    .cleanable(false)
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX)),
            )
            .into_any_element()
    }

    fn render_export_list(
        &self,
        ui: &ExportUi,
        rows: &[ImportCandidate],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let export = &self.import_picker.export;
        if rows.is_empty() {
            return export_text("none", NO_MATCHING_CHATS.to_owned(), theme::rambutan_300())
                .py_2()
                .into_any_element();
        }
        let now = EpochMs::now();
        let shown = rows.len().min(MAX_ROWS_SHOWN);
        let selected = export.selected.min(shown - 1);
        div()
            .id("desktop-import-export-list")
            .debug_selector(|| "desktop-import-export-list".to_string())
            .track_focus(&ui.list)
            .role(Role::ListBox)
            .aria_label(EXPORT_TAB)
            .mx(px(-EXPORT_EDGE_PX))
            .flex_shrink_0()
            .max_h(px(EXPORT_LIST_MAX_HEIGHT_PX))
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .overflow_y_scroll()
            .track_scroll(&export.scroll)
            .children(rows.iter().take(shown).enumerate().map(|(index, row)| {
                let chosen = index == selected;
                let key = import_key(row.source, &row.id);
                let detail = export_row_detail(row, now);
                let title = bruhs_models::logic::text::plain_title(&row.title).into_owned();
                let picked = row.clone();
                div()
                    .id(("desktop-import-export-row", index))
                    .debug_selector(move || format!("desktop-import-export-row-{key}"))
                    .role(Role::ListBoxOption)
                    .aria_label(title.clone())
                    .aria_selected(chosen)
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_0p5()
                    .px(px(EXPORT_EDGE_PX - 1.0))
                    .py(px(7.0))
                    .border_1()
                    .cursor_pointer()
                    .map(|el| {
                        // Blueberry marks the selection (design rule).
                        if chosen {
                            el.bg(theme::blueberry_600().opacity(EXPORT_SELECTED_BG_OPACITY))
                                .border_color(
                                    theme::blueberry_500().opacity(EXPORT_SELECTED_RING_OPACITY),
                                )
                        } else {
                            el.border_color(gpui::transparent_black()).hover(|el| {
                                el.bg(theme::longan_700().opacity(IMPORT_HOVER_ROW_BG_OPACITY))
                            })
                        }
                    })
                    .on_hover(cx.listener(move |this, hovered, _window, cx| {
                        if *hovered && this.import_picker.export.selected != index {
                            this.import_picker.export.selected = index;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.import_export_row(picked.clone(), window, cx);
                    }))
                    .child(
                        export_text("title", title, theme::rambutan_200())
                            .w_full()
                            .min_w_0()
                            .truncate(),
                    )
                    .child(
                        export_text("detail", detail, theme::rambutan_400())
                            .w_full()
                            .min_w_0()
                            .truncate(),
                    )
            }))
            .when(rows.len() > MAX_ROWS_SHOWN, |el| {
                el.child(
                    export_text(
                        "more",
                        rows_shown_line(MAX_ROWS_SHOWN, rows.len()),
                        theme::rambutan_400(),
                    )
                    .px_4()
                    .py_2(),
                )
            })
            .vertical_scrollbar(&export.scroll)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{done_line, reading_progress};
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
