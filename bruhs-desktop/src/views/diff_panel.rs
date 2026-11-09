//! Diff surface for the right panel: the active project's working tree via
//! `bruhs_core::git` (status + unified diff), rendered as per-file sections
//! with "N unmodified lines" separators, like bruhs-web's `diff_panel.rs`.

use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use bruhs_models::logic::diff::{self, lcs_diff, DiffLine, Hunk, LineKind};
use bruhs_models::logic::find::{cycle, search_diff, DiffHit};
use bruhs_models::logic::git::GitStatus;
use bruhs_models::logic::review::{
    classify_path, content_fingerprint, review_scope_key, set_viewed, viewed_count, viewed_state,
    DiffReviewFilter, ViewedBook, ViewedState,
};
pub use bruhs_models::logic::tools::relative_path;
use gpui::prelude::*;
use gpui::*;
use gpui_component::scroll::ScrollableElement;
use gpui_component::tooltip::Tooltip;

use crate::views::chat::{DesktopToolCall, Message, MessageRole};
use crate::views::dropdown::{
    dropdown_list, DropdownGroup, DropdownOption, SelectHandler, ToggleHandler,
};
use crate::views::web_button::is_web_button_activation_key;
use crate::{assets, theme};

const PANEL_BODY_FONT_FAMILY: &str = "Monaspace Neon";
const PANEL_LABEL_FONT_FAMILY: &str = "Monaspace Argon";
/// Web toolbar: `h-9`.
const TOOLBAR_HEIGHT_PX: f32 = 36.0;
const TOOLBAR_TEXT_SIZE_PX: f32 = 12.0;
const FILE_HEADER_HEIGHT_PX: f32 = 32.0;
const FILE_HEADER_TEXT_SIZE_PX: f32 = 12.0;
const ROW_TEXT_SIZE_PX: f32 = 12.0;
const GUTTER_WIDTH_PX: f32 = 40.0;
const SIGN_WIDTH_PX: f32 = 12.0;
const ROW_Y_PADDING_PX: f32 = 1.0;
const ROW_BG_OPACITY: f32 = 0.10;
const GAP_TEXT_SIZE_PX: f32 = 12.0;
const ACTION_SIZE_PX: f32 = 28.0;
const RADIUS_PX: f32 = 8.0;
const DIVIDER_OPACITY: f32 = 0.50;
const HOVER_BG_OPACITY: f32 = 0.50;

actions!(diff_panel, [FindInDiff]);
const EMPTY_TEXT_SIZE_PX: f32 = 12.0;
const ICON_SIZE_PX: f32 = 14.0;

const REFRESH_LABEL: &str = "Refresh diff";
const OPEN_LABEL: &str = "Open in editor";
const NO_PROJECT_LABEL: &str = "Open a project thread to see its working tree.";
const LOADING_LABEL: &str = "Reading git…";
const EMPTY_LABEL: &str = "No changes in the working tree";
/// Web `scope_label`: "Uncommitted changes" when clean, else "Uncommitted · N files".
const SCOPE_LABEL: &str = "Uncommitted";
const SCOPE_DROPDOWN_ID: &str = "desktop-diff-scope";
/// Web scope chip: `h-6 max-w-[16rem] gap-1 rounded-grape bg-longan-800 px-2`.
const SCOPE_CHIP_HEIGHT_PX: f32 = 24.0;
const SCOPE_CHIP_MAX_WIDTH_PX: f32 = 256.0;
const WRAP_LABEL: &str = "Toggle line wrap";
const SCOPE_LIST_WIDTH_PX: f32 = 300.0;
const TURN_EMPTY_LABEL: &str = "No file changes in this turn";

fn element_key(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
pub(crate) fn element_key_for_test(value: &str) -> u64 {
    element_key(value)
}

// ── Pure diff model (port of bruhs-web `diff.rs` / `diff_panel.rs`) ──────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopDiffFile {
    pub path: String,
    pub added: bool,
    pub deleted: bool,
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
}

impl DesktopDiffFile {
    fn from_file_diff(file: diff::FileDiff) -> Option<Self> {
        let path = file.new_path.clone().or_else(|| file.old_path.clone())?;
        Some(Self {
            path,
            added: file.is_added(),
            deleted: file.is_deleted(),
            is_binary: file.is_binary,
            hunks: file.hunks,
        })
    }

    pub fn additions(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Add)
            .count()
    }

    pub fn deletions(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Remove)
            .count()
    }
}

/// Parse `git diff` output into the panel's files. A file block without
/// any path (malformed input) is dropped.
pub fn parse_unified(text: &str) -> Vec<DesktopDiffFile> {
    diff::parse_unified(text)
        .into_iter()
        .filter_map(DesktopDiffFile::from_file_diff)
        .collect()
}

/// `(start, count)` of the old side from a `@@ -start,count +… @@` header.
pub fn hunk_old_range(header: &str) -> Option<(usize, usize)> {
    let rest = header.strip_prefix("@@ -")?;
    let spec = rest.split(' ').next()?;
    let mut it = spec.split(',');
    let start = it.next()?.parse().ok()?;
    let count = it.next().map_or(1, |c| c.parse().unwrap_or(1));
    Some((start, count))
}

/// Lines of the old file skipped between two consecutive hunks.
pub fn unmodified_gap(prev: Option<&str>, next: &str) -> Option<usize> {
    let (prev_start, prev_count) = hunk_old_range(prev?)?;
    let (next_start, _) = hunk_old_range(next)?;
    let gap = next_start.saturating_sub(prev_start + prev_count);
    (gap > 0).then_some(gap)
}

/// New-side start line of the first hunk, for "Open in editor".
pub fn first_new_line(file: &DesktopDiffFile) -> Option<u32> {
    let header = &file.hunks.first()?.header;
    header
        .split_whitespace()
        .find_map(|part| part.strip_prefix('+'))
        .and_then(|rest| rest.split(',').next())
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| *n > 0)
}

/// `(additions, deletions)` across files.
pub fn diff_totals<'a>(files: impl IntoIterator<Item = &'a DesktopDiffFile>) -> (usize, usize) {
    files.into_iter().fold((0, 0), |(add, del), file| {
        (add + file.additions(), del + file.deletions())
    })
}

/// `"src/"` + `"main.rs"` for dim-directory / bright-name rendering.
pub fn split_path(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (format!("{dir}/"), name.to_string()),
        None => (String::new(), path.to_string()),
    }
}

// ── Per-turn scopes (port of bruhs-web `timeline::changed_files`, `diff::lcs_diff`) ──

/// Files an assistant turn wrote or edited, rebuilt from tool inputs: one
/// hunk per Edit (`@@ edit N @@`), Write as a whole-file addition.
pub fn changed_files_from_tool_calls(
    calls: &[DesktopToolCall],
    root: Option<&str>,
) -> Vec<DesktopDiffFile> {
    let mut files: Vec<DesktopDiffFile> = Vec::new();
    let mut push = |path: String, added: bool, old: &str, new: &str| {
        let hunk_index = files
            .iter()
            .find(|f| f.path == path)
            .map_or(0, |f| f.hunks.len());
        let hunk = Hunk {
            header: format!("@@ edit {} @@", hunk_index + 1),
            lines: lcs_diff(old, new),
        };
        if let Some(f) = files.iter_mut().find(|f| f.path == path) {
            f.added |= added;
            f.hunks.push(hunk);
        } else {
            files.push(DesktopDiffFile {
                path,
                added,
                deleted: false,
                is_binary: false,
                hunks: vec![hunk],
            });
        }
    };
    for call in calls {
        if call.output.is_none() || call.is_error {
            continue;
        }
        let field = |k: &str| {
            call.input
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let raw_path = if field("file_path").is_empty() {
            field("notebook_path")
        } else {
            field("file_path")
        };
        if raw_path.is_empty() {
            continue;
        }
        let path = relative_path(&raw_path, root);
        match call.tool.as_str() {
            "Write" => {
                let content = field("content");
                push(path, true, "", &content);
            }
            "Edit" | "MultiEdit" | "NotebookEdit" => {
                if let Some(edits) = call.input.get("edits").and_then(|v| v.as_array()) {
                    for e in edits {
                        let old = e.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                        let new = e.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
                        push(path.clone(), false, old, new);
                    }
                } else if !field("old_string").is_empty() || !field("new_string").is_empty() {
                    push(path, false, &field("old_string"), &field("new_string"));
                } else if !field("new_source").is_empty() {
                    push(path, false, "", &field("new_source"));
                }
            }
            _ => {}
        }
    }
    files
}

/// One assistant turn's file changes, labelled by the prompt that started it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopTurnFiles {
    pub label: String,
    /// Full prompt. The desktop review key uses this text.
    pub prompt: String,
    pub files: Vec<DesktopDiffFile>,
}

fn truncate_label(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

/// Turns of `messages` that changed files, in thread order.
pub fn turn_files_from_messages(messages: &[Message], root: Option<&str>) -> Vec<DesktopTurnFiles> {
    let mut out = Vec::new();
    let mut last_user: Option<String> = None;
    for message in messages {
        match message.role {
            MessageRole::User => {
                last_user = Some(message.content.clone());
                continue;
            }
            MessageRole::Assistant => {}
            MessageRole::Tool => continue,
        }
        let files = changed_files_from_tool_calls(&message.tool_calls, root);
        if files.is_empty() {
            continue;
        }
        let prompt = last_user.take().unwrap_or_default();
        let first_line = prompt.trim().lines().next().unwrap_or("").trim();
        out.push(DesktopTurnFiles {
            label: format!("Turn · {}", truncate_label(first_line, 30)),
            prompt,
            files,
        });
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffScope {
    WorkingTree,
    Turn(usize),
}

impl DiffScope {
    fn value(self) -> String {
        match self {
            Self::WorkingTree => "working-tree".to_string(),
            Self::Turn(i) => format!("turn-{i}"),
        }
    }

    fn parse(value: &str) -> Self {
        value
            .strip_prefix("turn-")
            .and_then(|n| n.parse::<usize>().ok())
            .map_or(Self::WorkingTree, Self::Turn)
    }
}

// ── View ─────────────────────────────────────────────────────────────────

pub enum DiffPanelEvent {
    OpenInEditor { path: String, line: Option<u32> },
}

pub struct DiffPanelView {
    project_path: Option<String>,
    status: Option<GitStatus>,
    files: Vec<DesktopDiffFile>,
    truncated: bool,
    collapsed: HashSet<String>,
    review_filter: DiffReviewFilter,
    viewed_book: ViewedBook,
    loading: bool,
    error: Option<String>,
    /// Bumped per refresh so late results from an older run are dropped.
    generation: u64,
    /// Per-turn file changes of the active thread (Edit/Write tool inputs).
    turns: Vec<DesktopTurnFiles>,
    /// What the user asked for; `effective_scope` resolves it against the
    /// turns the thread has (web `effective_scope`).
    requested_scope: Option<DiffScope>,
    scope_open: bool,
    /// Web `wrap` (default on): soft-wrap long diff lines.
    wrap: bool,
    find_open: bool,
    find_query: String,
    find_index: usize,
    /// Hit already scrolled into view. `None` scrolls on the next paint.
    find_pinned: Option<usize>,
    find_focus: FocusHandle,
    panel_focus: FocusHandle,
    files_scroll: ScrollHandle,
}

impl EventEmitter<DiffPanelEvent> for DiffPanelView {}

impl DiffPanelView {
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        Self {
            project_path: None,
            status: None,
            files: Vec::new(),
            truncated: false,
            collapsed: HashSet::new(),
            review_filter: DiffReviewFilter::All,
            viewed_book: load_viewed(),
            loading: false,
            error: None,
            generation: 0,
            turns: Vec::new(),
            requested_scope: None,
            scope_open: false,
            wrap: true,
            find_open: false,
            find_query: String::new(),
            find_index: 0,
            find_pinned: None,
            find_focus: cx.focus_handle(),
            panel_focus: cx.focus_handle().tab_stop(false),
            files_scroll: ScrollHandle::new(),
        }
    }

    fn open_find(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.find_open = true;
        self.find_pinned = None;
        cx.notify();
        cx.on_next_frame(window, |this, window, cx| {
            this.find_focus.focus(window, cx);
        });
    }

    fn step_find(&mut self, forward: bool, cx: &mut Context<'_, Self>) {
        let files = self.reviewed_files();
        let len = find_in_diff_files(&self.find_query, &files).len();
        self.find_index = cycle(len, self.find_index, forward);
        self.find_pinned = None;
        cx.notify();
    }

    fn current_find(&self) -> Option<(usize, DiffHit)> {
        let files = self.reviewed_files();
        let hits = find_in_diff_files(&self.find_query, &files);
        if hits.is_empty() {
            return None;
        }
        let index = self.find_index % hits.len();
        Some((index, hits[index].clone()))
    }

    fn on_find_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        if key == "escape" {
            window.prevent_default();
            cx.stop_propagation();
            self.find_open = false;
            self.find_pinned = None;
            cx.notify();
            return;
        }
        if key == "enter" {
            window.prevent_default();
            cx.stop_propagation();
            self.step_find(!modifiers.shift, cx);
            return;
        }
        if key == "backspace" {
            window.prevent_default();
            cx.stop_propagation();
            self.find_query.pop();
            self.find_index = 0;
            self.find_pinned = None;
            cx.notify();
            return;
        }
        if let Some(text) = event.keystroke.key_char.as_deref() {
            if !text.is_empty() && text.chars().all(|ch| !ch.is_control()) {
                window.prevent_default();
                cx.stop_propagation();
                self.find_query.push_str(text);
                self.find_index = 0;
                self.find_pinned = None;
                cx.notify();
            }
        }
    }

    /// Replace the per-turn scopes; a selected turn that vanished falls back
    /// to the working tree.
    pub fn set_turns(&mut self, turns: Vec<DesktopTurnFiles>, cx: &mut Context<'_, Self>) {
        self.turns = turns;
        cx.notify();
    }

    /// Web `on_open_diff`: scope the panel to one turn.
    pub fn select_turn(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        self.requested_scope = Some(DiffScope::Turn(index));
        self.scope_open = false;
        cx.notify();
    }

    /// Web `effective_scope`: a requested turn that still exists, an explicit
    /// working tree, otherwise the latest turn, otherwise the working tree.
    fn effective_scope(&self) -> DiffScope {
        match self.requested_scope {
            Some(DiffScope::Turn(i)) if i < self.turns.len() => DiffScope::Turn(i),
            Some(DiffScope::WorkingTree) => DiffScope::WorkingTree,
            _ => match self.turns.len() {
                0 => DiffScope::WorkingTree,
                n => DiffScope::Turn(n - 1),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn scope_for_test(&self) -> DiffScope {
        self.effective_scope()
    }

    fn visible_files(&self) -> &[DesktopDiffFile] {
        match self.effective_scope() {
            DiffScope::WorkingTree => &self.files,
            DiffScope::Turn(i) => self.turns.get(i).map_or(&[], |t| t.files.as_slice()),
        }
    }

    /// Files the active All / Code / Tests / Docs filter still shows.
    fn reviewed_files(&self) -> Vec<DesktopDiffFile> {
        self.visible_files()
            .iter()
            .filter(|file| file_matches_filter(&file.path, self.review_filter))
            .cloned()
            .collect()
    }

    fn scope_key(&self) -> String {
        let scope = match self.effective_scope() {
            DiffScope::WorkingTree => "uncommitted".to_string(),
            DiffScope::Turn(index) => {
                let prompt = self
                    .turns
                    .get(index)
                    .map_or("", |turn| turn.prompt.as_str());
                format!("turn:{prompt}")
            }
        };
        review_scope_key(self.project_path.as_deref().unwrap_or(""), &scope)
    }

    fn set_review_filter(&mut self, filter: DiffReviewFilter, cx: &mut Context<'_, Self>) {
        if self.review_filter == filter {
            return;
        }
        self.review_filter = filter;
        self.find_index = 0;
        self.find_pinned = None;
        cx.notify();
    }

    fn set_file_viewed(
        &mut self,
        path: &str,
        fingerprint: &str,
        viewed: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let key = self.scope_key();
        set_viewed(&mut self.viewed_book, &key, path, fingerprint, viewed);
        if viewed {
            self.collapsed.insert(path.to_string());
        } else {
            self.collapsed.remove(path);
        }
        persist_viewed(&self.viewed_book);
        cx.notify();
    }

    /// Collapse every file in the current filter, or expand them when they
    /// are already collapsed. Folding does not mark a file viewed.
    fn toggle_reviewed_fold(&mut self, cx: &mut Context<'_, Self>) {
        let paths: Vec<String> = self
            .reviewed_files()
            .iter()
            .map(|file| file.path.clone())
            .collect();
        let expand = !paths.is_empty() && paths.iter().all(|path| self.collapsed.contains(path));
        if expand {
            for path in &paths {
                self.collapsed.remove(path);
            }
        } else {
            for path in paths {
                self.collapsed.insert(path);
            }
        }
        cx.notify();
    }

    fn toggle_scope_menu(&mut self, cx: &mut Context<'_, Self>) {
        self.scope_open = !self.scope_open;
        cx.notify();
    }

    fn set_scope(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        self.requested_scope = Some(DiffScope::parse(value));
        self.scope_open = false;
        cx.notify();
    }

    fn scope_groups(&self) -> Vec<DropdownGroup> {
        let mut options = vec![DropdownOption {
            value: "working-tree",
            label: SCOPE_LABEL.to_string(),
            description: Some(format!(
                "{} file{}",
                self.files.len(),
                if self.files.len() == 1 { "" } else { "s" }
            )),
        }];
        for (i, turn) in self.turns.iter().enumerate() {
            // Leak per-turn values: the dropdown wants `&'static str`; turns are few.
            let value: &'static str = Box::leak(format!("turn-{i}").into_boxed_str());
            options.push(DropdownOption {
                value,
                label: turn.label.clone(),
                description: Some(format!(
                    "{} file{}",
                    turn.files.len(),
                    if turn.files.len() == 1 { "" } else { "s" }
                )),
            });
        }
        vec![DropdownGroup {
            label: None,
            mark: None,
            options,
        }]
    }

    fn scope_handlers(&self, cx: &Context<'_, Self>) -> (ToggleHandler, SelectHandler) {
        let entity = cx.entity().downgrade();
        let toggle_entity = entity.clone();
        let on_toggle: ToggleHandler = Rc::new(move |_window, cx| {
            let _ = toggle_entity.update(cx, |this, cx| this.toggle_scope_menu(cx));
        });
        let on_select: SelectHandler = Rc::new(move |value, _window, cx| {
            let _ = entity.update(cx, |this, cx| this.set_scope(value, cx));
        });
        (on_toggle, on_select)
    }

    #[cfg(test)]
    pub(crate) fn project_path(&self) -> Option<&str> {
        self.project_path.as_deref()
    }

    /// Point the panel at another project (clears stale results).
    pub fn set_project(&mut self, path: Option<String>, cx: &mut Context<'_, Self>) {
        if self.project_path == path {
            return;
        }
        self.project_path = path;
        self.files.clear();
        self.status = None;
        self.error = None;
        self.truncated = false;
        self.collapsed.clear();
        cx.notify();
    }

    /// Re-read status + diff on the desktop's tokio runtime.
    pub fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        let Some(path) = self.project_path.clone() else {
            self.files.clear();
            self.status = None;
            self.loading = false;
            cx.notify();
            return;
        };
        self.start_refresh(path, cx);
    }

    /// gpui's test scheduler rejects wake-ups from the tokio runtime thread;
    /// tests inject diffs with `set_diff_text_for_test` instead.
    #[cfg(test)]
    fn start_refresh(&mut self, _path: String, cx: &mut Context<'_, Self>) {
        self.loading = false;
        cx.notify();
    }

    #[cfg(not(test))]
    fn start_refresh(&mut self, path: String, cx: &mut Context<'_, Self>) {
        use crate::app_state::DesktopState;
        use bruhs_core::git::{self, GitDiff};
        use std::path::PathBuf;
        use std::sync::Arc;

        let Some(state) = cx.try_global::<DesktopState>() else {
            return;
        };
        let runtime = Arc::clone(&state.runtime);
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        cx.notify();

        let (tx, rx) = async_channel::bounded::<Result<(GitStatus, GitDiff), String>>(1);
        runtime.spawn(async move {
            let dir = PathBuf::from(&path);
            let result = match (git::status(&dir).await, git::diff(&dir).await) {
                (Ok(status), Ok(diff)) => Ok((status, diff)),
                (Err(e), _) | (_, Err(e)) => Err(e.to_string()),
            };
            let _ = tx.send(result).await;
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.recv().await else {
                return;
            };
            let _ = this.update(cx, |panel, cx| {
                if panel.generation != generation {
                    return;
                }
                panel.loading = false;
                match result {
                    Ok((status, diff)) => {
                        panel.status = Some(status);
                        panel.files = parse_unified(&diff.diff);
                        panel.truncated = diff.truncated;
                        panel.error = None;
                    }
                    Err(err) => panel.error = Some(err),
                }
                cx.notify();
            });
        })
        .detach();
    }

    #[cfg(test)]
    pub(crate) fn set_diff_text_for_test(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        // Invalidate any in-flight refresh so its result cannot overwrite this.
        self.generation += 1;
        self.files = parse_unified(text);
        self.loading = false;
        self.error = None;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn file_count_for_test(&self) -> usize {
        self.files.len()
    }

    fn toggle_file(&mut self, path: &str, cx: &mut Context<'_, Self>) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        cx.notify();
    }

    fn render_toolbar(&self, cx: &Context<'_, Self>) -> Div {
        let scope_files = self.visible_files();
        let scope_count = scope_files.len();
        let reviewed: Vec<&DesktopDiffFile> = scope_files
            .iter()
            .filter(|file| file_matches_filter(&file.path, self.review_filter))
            .collect();
        let show_stats = !reviewed.is_empty();
        let (additions, deletions) = diff_totals(reviewed);
        let (scope_toggle, _) = self.scope_handlers(cx);
        let scope_label = match self.effective_scope() {
            DiffScope::WorkingTree => SCOPE_LABEL.to_string(),
            DiffScope::Turn(i) => self
                .turns
                .get(i)
                .map_or_else(|| SCOPE_LABEL.to_string(), |t| t.label.clone()),
        };
        let scope_toggle_for_key = scope_toggle.clone();
        div()
            .h(px(TOOLBAR_HEIGHT_PX))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::longan_800().opacity(DIVIDER_OPACITY))
            .font_family(PANEL_LABEL_FONT_FAMILY)
            .text_size(px(TOOLBAR_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .child(
                // Web scope menu trigger: a 24px chip on longan-800.
                div()
                    .id(SCOPE_DROPDOWN_ID)
                    .debug_selector(|| SCOPE_DROPDOWN_ID.to_string())
                    .h(px(SCOPE_CHIP_HEIGHT_PX))
                    .max_w(px(SCOPE_CHIP_MAX_WIDTH_PX))
                    .px_2()
                    .gap_1()
                    .rounded(px(RADIUS_PX))
                    .bg(theme::longan_800())
                    .flex()
                    .items_center()
                    .text_color(theme::rambutan_100())
                    .tab_index(0)
                    .role(Role::ComboBox)
                    .aria_label("Diff scope")
                    .aria_expanded(self.scope_open)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800().opacity(0.8)))
                    .on_click(move |_ev, window, cx| scope_toggle(window, cx))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            scope_toggle_for_key(window, cx);
                        }
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(match self.effective_scope() {
                                DiffScope::WorkingTree if scope_count == 0 => {
                                    "Uncommitted changes".to_string()
                                }
                                DiffScope::WorkingTree => format!(
                                    "{scope_label} · {scope_count} file{}",
                                    if scope_count == 1 { "" } else { "s" }
                                ),
                                DiffScope::Turn(_) => scope_label,
                            }),
                    )
                    .child(
                        svg()
                            .path(assets::CHEVRON_DOWN_ICON_PATH)
                            .size(px(12.0))
                            .flex_shrink_0()
                            .text_color(theme::rambutan_400()),
                    ),
            )
            .when(show_stats, |el| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-diff-stats".to_string())
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .flex_shrink_0()
                        .font(theme::tabular_system_font())
                        .child(
                            div()
                                .text_color(theme::pandan_400())
                                .child(format!("+{additions}")),
                        )
                        .child(
                            div()
                                .text_color(theme::lychee_400())
                                .child(format!("−{deletions}")),
                        ),
                )
            })
            .child(div().flex_1())
            .child(self.render_toolbar_icon_button(
                "desktop-diff-wrap",
                WRAP_LABEL,
                assets::WRAP_TEXT_ICON_PATH,
                self.wrap,
                cx.listener(|this, _ev: &ClickEvent, _window, cx| this.toggle_wrap(cx)),
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.toggle_wrap(cx);
                    }
                }),
            ))
            .when(scope_count > 0, |el| {
                el.child(self.render_toolbar_icon_button(
                    "desktop-diff-refresh",
                    REFRESH_LABEL,
                    assets::REFRESH_ICON_PATH,
                    false,
                    cx.listener(|this, _ev: &ClickEvent, _window, cx| this.refresh(cx)),
                    cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.refresh(cx);
                        }
                    }),
                ))
            })
    }

    fn toggle_wrap(&mut self, cx: &mut Context<'_, Self>) {
        self.wrap = !self.wrap;
        cx.notify();
    }

    /// Web `icon_btn`: `size-7 rounded-grape`, `bg-longan-800/60 text-rambutan-100`
    /// while active, `text-rambutan-400` otherwise, both on hover.
    fn render_toolbar_icon_button(
        &self,
        id: &'static str,
        label: &'static str,
        icon_path: &'static str,
        active: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        on_key_down: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let color = if active {
            theme::rambutan_100()
        } else {
            theme::rambutan_400()
        };
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .size(px(ACTION_SIZE_PX))
            .rounded(px(RADIUS_PX))
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .when(active, |el| {
                el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY))
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY)))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .on_click(on_click)
            .on_key_down(on_key_down)
            .child(
                svg()
                    .path(icon_path)
                    .size(px(ICON_SIZE_PX))
                    .text_color(color),
            )
            .into_any_element()
    }

    fn render_review_bar(&self, cx: &Context<'_, Self>) -> impl IntoElement {
        let files = self.reviewed_files();
        let key = self.scope_key();
        let pairs: Vec<(String, String)> = files
            .iter()
            .map(|file| (file.path.clone(), file_fingerprint(file)))
            .collect();
        let refs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(path, fingerprint)| (path.as_str(), fingerprint.as_str()))
            .collect();
        let viewed_n = viewed_count(&self.viewed_book, &key, &refs);
        let total = files.len();
        let expand =
            !files.is_empty() && files.iter().all(|file| self.collapsed.contains(&file.path));
        let fold_label = if expand { "Expand all" } else { "Collapse all" };
        div()
            .id("desktop-diff-review")
            .debug_selector(|| "desktop-diff-review".to_string())
            .h(px(32.0))
            .px_3()
            .flex()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::longan_800().opacity(DIVIDER_OPACITY))
            .child(self.render_filter_chip(DiffReviewFilter::All, "desktop-diff-filter-all", cx))
            .child(self.render_filter_chip(DiffReviewFilter::Code, "desktop-diff-filter-code", cx))
            .child(self.render_filter_chip(
                DiffReviewFilter::Tests,
                "desktop-diff-filter-tests",
                cx,
            ))
            .child(self.render_filter_chip(DiffReviewFilter::Docs, "desktop-diff-filter-docs", cx))
            .child(div().flex_1().min_w_0())
            .child(
                div()
                    .id("desktop-diff-viewed-count")
                    .debug_selector(|| "desktop-diff-viewed-count".to_string())
                    .flex_shrink_0()
                    .min_w_0()
                    .truncate()
                    .text_size(px(TOOLBAR_TEXT_SIZE_PX))
                    .font_family(PANEL_LABEL_FONT_FAMILY)
                    .font(theme::tabular_system_font())
                    .text_color(theme::rambutan_400())
                    .child(format!("{viewed_n} of {total} viewed")),
            )
            .child(self.render_fold_button(fold_label, cx))
    }

    fn render_filter_chip(
        &self,
        filter: DiffReviewFilter,
        id: &'static str,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let active = self.review_filter == filter;
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .h(px(SCOPE_CHIP_HEIGHT_PX))
            .px_2()
            .rounded(px(RADIUS_PX))
            .flex()
            .flex_shrink_0()
            .items_center()
            .text_size(px(TOOLBAR_TEXT_SIZE_PX))
            .font_family(PANEL_LABEL_FONT_FAMILY)
            .when(active, |el| {
                el.bg(theme::longan_800()).text_color(theme::rambutan_100())
            })
            .when(!active, |el| el.text_color(theme::rambutan_400()))
            .tab_index(0)
            .role(Role::Button)
            .aria_label(filter.label())
            .aria_toggled(if active {
                Toggled::True
            } else {
                Toggled::False
            })
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| {
                el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY))
                    .text_color(theme::rambutan_100())
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_review_filter(filter, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.set_review_filter(filter, cx);
                }
            }))
            .child(filter.label())
    }

    fn render_fold_button(&self, label: &'static str, cx: &Context<'_, Self>) -> impl IntoElement {
        div()
            .id("desktop-diff-collapse")
            .debug_selector(|| "desktop-diff-collapse".to_string())
            .h(px(SCOPE_CHIP_HEIGHT_PX))
            .px_2()
            .rounded(px(RADIUS_PX))
            .flex()
            .flex_shrink_0()
            .items_center()
            .text_size(px(TOOLBAR_TEXT_SIZE_PX))
            .font_family(PANEL_LABEL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| {
                el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY))
                    .text_color(theme::rambutan_100())
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle_reviewed_fold(cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.toggle_reviewed_fold(cx);
                }
            }))
            .child(label)
    }

    fn render_find_bar(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let files = self.reviewed_files();
        let hits = find_in_diff_files(&self.find_query, &files);
        let count_label = if self.find_query.is_empty() {
            String::new()
        } else if hits.is_empty() {
            "No matches".to_string()
        } else {
            format!("{} of {}", self.find_index % hits.len() + 1, hits.len())
        };
        div()
            .id("desktop-diff-find-bar")
            .track_focus(&self.find_focus)
            .h(px(36.0))
            .w_full()
            .flex_shrink_0()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .bg(theme::longan_900())
            .border_b_1()
            .border_color(theme::longan_800().opacity(0.6))
            .tab_index(0)
            .aria_label("Find in diff")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.find_focus.focus(window, cx);
                }),
            )
            .on_key_down(cx.listener(Self::on_find_key))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(14.0))
                    .font_family(PANEL_BODY_FONT_FAMILY)
                    .text_color(if self.find_query.is_empty() {
                        theme::rambutan_500()
                    } else {
                        theme::rambutan_100()
                    })
                    .child(if self.find_query.is_empty() {
                        "Find in diff".to_string()
                    } else {
                        self.find_query.clone()
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.0))
                    .font_family(PANEL_LABEL_FONT_FAMILY)
                    .font(theme::tabular_system_font())
                    .text_color(theme::rambutan_400())
                    .child(count_label),
            )
            .child(self.find_step_button("Previous match", false, cx))
            .child(self.find_step_button("Next match", true, cx))
            .child(
                div()
                    .id("desktop-diff-find-close")
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .text_size(px(12.0))
                    .font_family(PANEL_LABEL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(0.6))
                            .text_color(theme::rambutan_100())
                    })
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Close find")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.find_open = false;
                        this.find_pinned = None;
                        cx.notify();
                    }))
                    .child("×"),
            )
    }

    fn find_step_button(
        &self,
        label: &'static str,
        forward: bool,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        div()
            .id(if forward {
                "desktop-diff-find-next"
            } else {
                "desktop-diff-find-prev"
            })
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .text_size(px(12.0))
            .font_family(PANEL_LABEL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .cursor_pointer()
            .hover(|el| {
                el.bg(theme::longan_800().opacity(0.6))
                    .text_color(theme::rambutan_100())
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.step_find(forward, cx);
            }))
            .child(if forward { "↓" } else { "↑" })
    }

    fn render_empty(&self, label: impl Into<SharedString>, tone: Hsla) -> Div {
        div()
            .debug_selector(|| "desktop-diff-empty".to_string())
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .px_4()
            .text_size(px(EMPTY_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(tone)
            .child(label.into())
    }

    /// `highlight` is `Some(None)` for a path match and `Some(Some(line))`
    /// for a hunk line. `None` means this file is not the current match.
    fn render_file(
        &self,
        file: &DesktopDiffFile,
        highlight: Option<Option<usize>>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let key = element_key(&file.path);
        let expanded = !self.collapsed.contains(&file.path);
        let (dir, name) = split_path(&file.path);
        let (badge, badge_color) = if file.added {
            ("A", theme::pandan_400())
        } else if file.deleted {
            ("D", theme::lychee_400())
        } else {
            ("M", theme::durian_400())
        };
        let additions = file.additions();
        let deletions = file.deletions();
        let path_for_toggle = file.path.clone();
        let path_for_toggle_key = file.path.clone();
        let path_for_open = file.path.clone();
        let open_line = first_new_line(file);
        let fingerprint = file_fingerprint(file);
        let mark = viewed_state(
            &self.viewed_book,
            &self.scope_key(),
            &file.path,
            &fingerprint,
        );
        let viewed = mark == ViewedState::Viewed;
        let changed = mark == ViewedState::Changed;
        let path_for_mark = file.path.clone();
        let path_for_mark_key = file.path.clone();
        let fingerprint_for_mark = fingerprint.clone();
        let fingerprint_for_key = fingerprint;
        let viewed_box = div()
            .id(("desktop-diff-viewed", key))
            .debug_selector(move || format!("desktop-diff-viewed-{key}"))
            .size(px(24.0))
            .flex_shrink_0()
            .rounded(px(4.0))
            .flex()
            .items_center()
            .justify_center()
            .tab_index(0)
            .role(Role::CheckBox)
            .aria_label(format!("Mark {} viewed", file.path))
            .aria_toggled(if viewed {
                Toggled::True
            } else {
                Toggled::False
            })
            .when(viewed, |el| {
                el.bg(theme::persimmon_500())
                    .text_color(theme::longan_950())
            })
            .when(!viewed, |el| {
                el.bg(theme::longan_800())
                    .border_1()
                    .border_color(theme::rambutan_600())
            })
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.set_file_viewed(&path_for_mark, &fingerprint_for_mark, !viewed, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.set_file_viewed(&path_for_mark_key, &fingerprint_for_key, !viewed, cx);
                }
            }))
            .child(
                svg()
                    .path(assets::CHECK_ICON_PATH)
                    .size(px(14.0))
                    .text_color(if viewed {
                        theme::longan_950()
                    } else {
                        theme::longan_800()
                    }),
            );
        let row_group: SharedString = format!("desktop-diff-file-{key}").into();
        let open_group = row_group.clone();

        let header = div()
            .id(("desktop-diff-file", key))
            .debug_selector(move || format!("desktop-diff-file-{key}"))
            .group(row_group)
            .h(px(FILE_HEADER_HEIGHT_PX))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .bg(theme::longan_900())
            .text_size(px(FILE_HEADER_TEXT_SIZE_PX))
            .tab_index(0)
            .role(Role::Button)
            .when(highlight == Some(None), |el| {
                el.border_1().border_color(theme::persimmon_400())
            })
            .aria_expanded(expanded)
            .aria_label(format!("Toggle {}", file.path))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY)))
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.toggle_file(&path_for_toggle, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.toggle_file(&path_for_toggle_key, cx);
                }
            }))
            .child(viewed_box)
            .when(changed, |el| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.0))
                        .font_family(PANEL_LABEL_FONT_FAMILY)
                        .text_color(theme::durian_400())
                        .child("Changed"),
                )
            })
            .child(
                div()
                    .w(px(SIGN_WIDTH_PX))
                    .flex_shrink_0()
                    .text_color(theme::rambutan_500())
                    .child(if expanded { "⌄" } else { "›" }),
            )
            .child(
                div()
                    .w(px(SIGN_WIDTH_PX))
                    .flex_shrink_0()
                    .text_color(badge_color)
                    .child(badge),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .font_family(PANEL_BODY_FONT_FAMILY)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::rambutan_500())
                            .child(dir),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .truncate()
                            .text_color(theme::rambutan_100())
                            .child(name),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .gap_1()
                    .font(theme::tabular_system_font())
                    .child(
                        div()
                            .text_color(theme::pandan_400())
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_color(theme::lychee_400())
                            .child(format!("−{deletions}")),
                    ),
            )
            .child(
                div()
                    .id(("desktop-diff-open", key))
                    .debug_selector(move || format!("desktop-diff-open-{key}"))
                    .flex_shrink_0()
                    .rounded(px(RADIUS_PX))
                    .px_2()
                    .py_1()
                    .text_color(theme::rambutan_400())
                    // Web: `opacity-0 group-hover/file:opacity-100`.
                    .invisible()
                    .group_hover(open_group, |el| el.visible())
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Open {} in editor", file.path))
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::longan_700().opacity(HOVER_BG_OPACITY))
                            .text_color(theme::rambutan_100())
                    })
                    .tooltip(|window, cx| Tooltip::new(OPEN_LABEL).build(window, cx))
                    .on_click(cx.listener(move |_this, _ev, _window, cx| {
                        cx.stop_propagation();
                        cx.emit(DiffPanelEvent::OpenInEditor {
                            path: path_for_open.clone(),
                            line: open_line,
                        });
                    }))
                    .child("Open"),
            );

        let body = if !expanded {
            None
        } else if file.is_binary {
            Some(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(EMPTY_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child("Binary file")
                    .into_any_element(),
            )
        } else {
            let mut prev_header: Option<&str> = None;
            let mut blocks: Vec<AnyElement> = Vec::new();
            let mut line_index = 0usize;
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                if let Some(gap) = unmodified_gap(prev_header, &hunk.header) {
                    blocks.push(
                        div()
                            .debug_selector(move || format!("desktop-diff-gap-{key}-{hunk_index}"))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py_1()
                            .text_size(px(GAP_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_500())
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(1.0))
                                    .bg(theme::longan_800().opacity(0.8)),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font(theme::tabular_system_font())
                                    .child(format!(
                                        "{gap} unmodified line{}",
                                        if gap == 1 { "" } else { "s" }
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(1.0))
                                    .bg(theme::longan_800().opacity(0.8)),
                            )
                            .into_any_element(),
                    );
                }
                prev_header = Some(&hunk.header);
                if !hunk.header.starts_with("@@ -") {
                    // Web: per-edit hunks show their `@@ edit N @@` header
                    // (`bg-longan-800/40 px-3 py-0.5 text-code-sm text-rambutan-500`).
                    let header = hunk.header.clone();
                    blocks.push(
                        div()
                            .debug_selector(move || format!("desktop-diff-hunk-{key}-{hunk_index}"))
                            .bg(theme::longan_800().opacity(0.4))
                            .px_3()
                            .py(px(2.0))
                            .font_family(PANEL_BODY_FONT_FAMILY)
                            .text_size(px(GAP_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_500())
                            .child(header)
                            .into_any_element(),
                    );
                }
                for line in &hunk.lines {
                    let current = highlight == Some(Some(line_index));
                    line_index += 1;
                    blocks.push(self.render_row(line, current).into_any_element());
                }
            }
            Some(
                div()
                    .debug_selector(move || format!("desktop-diff-body-{key}"))
                    .py_1()
                    .font_family(PANEL_BODY_FONT_FAMILY)
                    .children(blocks)
                    .into_any_element(),
            )
        };

        div()
            .border_b_1()
            .border_color(theme::longan_800().opacity(DIVIDER_OPACITY))
            .flex()
            .flex_col()
            .child(header)
            .children(body)
            .into_any_element()
    }

    fn render_row(&self, line: &DiffLine, current: bool) -> Div {
        let (sign, color, background) = match line.kind {
            LineKind::Add => (
                "+",
                theme::pandan_300(),
                Some(theme::pandan_400().opacity(ROW_BG_OPACITY)),
            ),
            LineKind::Remove => (
                "−",
                theme::lychee_300(),
                Some(theme::lychee_400().opacity(ROW_BG_OPACITY)),
            ),
            LineKind::Context => (" ", theme::rambutan_400(), None),
        };
        let gutter = |n: Option<usize>| {
            div()
                .w(px(GUTTER_WIDTH_PX))
                .flex_shrink_0()
                .pr_1()
                .text_align(TextAlign::Right)
                .text_color(theme::rambutan_500())
                .font(theme::tabular_system_font())
                .child(n.map_or_else(String::new, |n| n.to_string()))
        };
        div()
            .flex()
            .items_start()
            .when(current, |el| {
                el.border_1().border_color(theme::persimmon_400())
            })
            .when_some(background, |el, bg| el.bg(bg))
            .py(px(ROW_Y_PADDING_PX))
            .text_size(px(ROW_TEXT_SIZE_PX))
            .text_color(color)
            .child(gutter(line.old_no))
            .child(gutter(line.new_no))
            .child(
                div()
                    .w(px(SIGN_WIDTH_PX))
                    .flex_shrink_0()
                    .text_color(color.opacity(0.6))
                    .child(sign),
            )
            .child(
                div()
                    .min_w_0()
                    .pr_3()
                    .when(!self.wrap, |el| el.whitespace_nowrap().overflow_hidden())
                    .child(line.text.clone()),
            )
    }
}

impl Render for DiffPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let turn_scope = matches!(self.effective_scope(), DiffScope::Turn(_));
        let current = self.find_open.then(|| self.current_find()).flatten();
        if let Some((_, hit)) = &current {
            self.collapsed.remove(&hit.path);
        }
        let scope_files = self.visible_files().to_vec();
        let reviewed = self.reviewed_files();
        if let Some((hit_index, hit)) = &current {
            if self.find_pinned != Some(*hit_index) {
                if let Some(index) = reviewed.iter().position(|file| file.path == hit.path) {
                    self.find_pinned = Some(*hit_index);
                    self.files_scroll.scroll_to_item(index);
                }
            }
        }
        let body: AnyElement = if self.project_path.is_none() && !turn_scope {
            self.render_empty(NO_PROJECT_LABEL, theme::rambutan_500())
                .into_any_element()
        } else if let (Some(err), false) = (self.error.clone(), turn_scope) {
            self.render_empty(err, theme::lychee_300())
                .into_any_element()
        } else if self.loading && scope_files.is_empty() && !turn_scope {
            self.render_empty(LOADING_LABEL, theme::rambutan_500())
                .into_any_element()
        } else if scope_files.is_empty() {
            self.render_empty(
                if turn_scope {
                    TURN_EMPTY_LABEL
                } else {
                    EMPTY_LABEL
                },
                theme::rambutan_500(),
            )
            .into_any_element()
        } else if reviewed.is_empty() {
            self.render_empty(self.review_filter.empty_label(), theme::rambutan_500())
                .into_any_element()
        } else {
            let files: Vec<AnyElement> = reviewed
                .iter()
                .map(|file| {
                    let highlight = current
                        .as_ref()
                        .and_then(|(_, hit)| (hit.path == file.path).then_some(hit.line));
                    self.render_file(file, highlight, cx)
                })
                .collect();
            div()
                .id("desktop-diff-files")
                .flex_1()
                .min_h_0()
                .track_scroll(&self.files_scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.files_scroll)
                .flex()
                .flex_col()
                .children(files)
                .when(self.truncated, |el| {
                    el.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(EMPTY_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::durian_400())
                            .child("Diff truncated — the working tree is large."),
                    )
                })
                .into_any_element()
        };

        let (_, scope_select) = self.scope_handlers(cx);
        let scope_list = self.scope_open.then(|| {
            div()
                .absolute()
                .top(px(TOOLBAR_HEIGHT_PX))
                .left_3()
                .w(px(SCOPE_LIST_WIDTH_PX))
                .occlude()
                .child(dropdown_list(
                    SCOPE_DROPDOWN_ID,
                    "Diff scope",
                    &self.effective_scope().value(),
                    self.scope_groups(),
                    scope_select,
                ))
        });
        div()
            .debug_selector(|| "desktop-diff-panel".to_string())
            .track_focus(&self.panel_focus)
            .key_context("Diff")
            .on_action(cx.listener(|this, _: &FindInDiff, window, cx| {
                this.open_find(window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.panel_focus.focus(window, cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.find_open
                    && event.keystroke.key == "escape"
                    && !event.keystroke.modifiers.modified()
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.find_open = false;
                    this.find_pinned = None;
                    cx.notify();
                }
            }))
            .relative()
            .w_full()
            .h_full()
            .bg(theme::longan_950())
            .text_color(theme::rambutan_300())
            .font_family(PANEL_BODY_FONT_FAMILY)
            .flex()
            .flex_col()
            .child(self.render_toolbar(cx))
            .when(!self.visible_files().is_empty(), |el| {
                el.child(self.render_review_bar(cx))
            })
            .when(self.find_open, |el| el.child(self.render_find_bar(cx)))
            .child(body)
            .children(scope_list)
    }
}

fn file_fingerprint(file: &DesktopDiffFile) -> String {
    if file.is_binary {
        return content_fingerprint(["binary".as_bytes()]);
    }
    content_fingerprint(
        file.hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter().map(|line| line.text.as_bytes())),
    )
}

fn file_matches_filter(path: &str, filter: DiffReviewFilter) -> bool {
    filter.includes(classify_path(path))
}

/// Viewed marks stay out of the user's real file while tests run.
fn load_viewed() -> ViewedBook {
    #[cfg(test)]
    {
        ViewedBook::default()
    }
    #[cfg(not(test))]
    {
        let Some(path) = viewed_store_path() else {
            return ViewedBook::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return ViewedBook::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }
}

fn persist_viewed(book: &ViewedBook) {
    #[cfg(test)]
    {
        let _ = book;
    }
    #[cfg(not(test))]
    {
        let Some(path) = viewed_store_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(book) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(not(test))]
fn viewed_store_path() -> Option<std::path::PathBuf> {
    crate::paths::app_data_dir().map(|dir| dir.join("diff-viewed.json"))
}

fn find_in_diff_files(query: &str, files: &[DesktopDiffFile]) -> Vec<DiffHit> {
    let lines: Vec<Vec<String>> = files
        .iter()
        .map(|file| {
            file.hunks
                .iter()
                .flat_map(|hunk| hunk.lines.iter().map(|line| line.text.clone()))
                .collect()
        })
        .collect();
    let borrowed: Vec<(&str, Vec<&str>)> = files
        .iter()
        .zip(&lines)
        .map(|(file, texts)| {
            (
                file.path.as_str(),
                texts.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    let pairs: Vec<(&str, &[&str])> = borrowed
        .iter()
        .map(|(path, texts)| (*path, texts.as_slice()))
        .collect();
    search_diff(query, &pairs)
}

#[cfg(test)]
mod tests {
    // Explicit imports: `super::*` would pull in gpui's `test` macro.
    use super::{
        changed_files_from_tool_calls, diff_totals, file_matches_filter, find_in_diff_files,
        first_new_line, hunk_old_range, lcs_diff, parse_unified, relative_path, split_path,
        turn_files_from_messages, unmodified_gap, DiffScope, LineKind,
    };
    use crate::views::chat::{DesktopToolCall, Message, MessageRole, TurnState};
    use bruhs_models::logic::review::{review_scope_key, DiffReviewFilter};
    use serde_json::json;

    fn call(tool: &str, input: serde_json::Value) -> DesktopToolCall {
        DesktopToolCall {
            id: format!("{tool}-{}", input.to_string().len()),
            tool: tool.to_string(),
            input,
            output: Some("ok".to_string()),
            is_error: false,
        }
    }

    fn assistant_with(calls: Vec<DesktopToolCall>) -> Message {
        Message {
            role: MessageRole::Assistant,
            content: "done".to_string(),
            model: None,
            thinking: String::new(),
            usage: None,
            duration_ms: None,
            started_at_ms: None,
            completed_at_ms: None,
            tool_calls: calls,
            files: Vec::new(),
            state: TurnState::Finished,
            checkpoint: None,
        }
    }

    #[test]
    fn a_removed_line_starting_with_dashes_is_a_removal_not_a_header() {
        // Removing the SQL comment `-- old` prints `--- old` in the hunk.
        let diff = "diff --git a/q.sql b/q.sql\n--- a/q.sql\n+++ b/q.sql\n@@ -1,2 +1,1 @@\n--- old\n keep\n";
        let files = parse_unified(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "q.sql");
        assert!(!files[0].added && !files[0].deleted);
        let lines = &files[0].hunks[0].lines;
        assert_eq!(lines[0].kind, LineKind::Remove);
        assert_eq!(lines[0].text, "-- old");
        assert_eq!(lines[1].kind, LineKind::Context);
    }

    #[test]
    fn lcs_rows_numbering() {
        let rows = lcs_diff(
            "fn a() {}\nfn b() {}\nfn c() {}",
            "fn a() {}\nfn B() {}\nfn c() {}",
        );
        let kinds: Vec<LineKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                LineKind::Context,
                LineKind::Remove,
                LineKind::Add,
                LineKind::Context
            ]
        );
        assert_eq!((rows[1].old_no, rows[1].new_no), (Some(2), None));
        assert_eq!((rows[2].old_no, rows[2].new_no), (None, Some(2)));
        assert_eq!((rows[3].old_no, rows[3].new_no), (Some(3), Some(3)));
        assert!(lcs_diff("", "a\nb").iter().all(|r| r.kind == LineKind::Add));
        assert!(lcs_diff("same", "same")
            .iter()
            .all(|r| r.kind == LineKind::Context));
    }

    #[test]
    fn changed_files_from_tool_calls_handles_edit_multiedit_write() {
        let calls = vec![
            call(
                "Edit",
                json!({ "file_path": "/r/src/a.rs", "old_string": "old();", "new_string": "new();" }),
            ),
            call(
                "MultiEdit",
                json!({ "file_path": "/r/src/a.rs", "edits": [
                    { "old_string": "x", "new_string": "y" },
                    { "old_string": "p", "new_string": "q" }
                ]}),
            ),
            call(
                "Write",
                json!({ "file_path": "/r/new.txt", "content": "hello\nworld" }),
            ),
            call("Read", json!({ "file_path": "/r/ignored.rs" })),
        ];
        let files = changed_files_from_tool_calls(&calls, Some("/r"));
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].hunks.len(), 3);
        assert_eq!(files[0].hunks[2].header, "@@ edit 3 @@");
        assert!(!files[0].added);
        assert_eq!(files[1].path, "new.txt");
        assert!(files[1].added);
        assert_eq!(files[1].additions(), 2);
        let mut failed = call(
            "Edit",
            json!({ "file_path": "/r/x.rs", "old_string": "a", "new_string": "b" }),
        );
        failed.is_error = true;
        assert!(changed_files_from_tool_calls(&[failed], Some("/r")).is_empty());
        assert_eq!(relative_path("/r/a.rs", Some("/r/")), "a.rs");
        assert_eq!(
            relative_path("/elsewhere/a.rs", Some("/r")),
            "/elsewhere/a.rs"
        );
    }

    #[test]
    fn turn_files_from_messages_labels_turns_by_prompt() {
        let mut user = Message::user("Fix the parser\nplease");
        user.content = "Fix the parser\nplease".to_string();
        let messages = vec![
            user,
            assistant_with(vec![call(
                "Write",
                json!({ "file_path": "/r/p.rs", "content": "x" }),
            )]),
            Message::user("just chat"),
            assistant_with(vec![]),
        ];
        let turns = turn_files_from_messages(&messages, Some("/r"));
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].label, "Turn · Fix the parser");
        assert_eq!(turns[0].prompt, "Fix the parser\nplease");
        assert_eq!(
            review_scope_key("/repo", &format!("turn:{}", turns[0].prompt)),
            format!("/repo\u{1f}turn:{}", turns[0].prompt)
        );
        assert_eq!(turns[0].files[0].path, "p.rs");
        assert!(file_matches_filter("src/a.rs", DiffReviewFilter::Code));
        assert!(!file_matches_filter("README.md", DiffReviewFilter::Code));
        assert!(file_matches_filter(
            "src/a.test.ts",
            DiffReviewFilter::Tests
        ));
        assert!(file_matches_filter("README.md", DiffReviewFilter::Docs));
        assert!(file_matches_filter("pnpm-lock.yaml", DiffReviewFilter::All));
        assert!(!file_matches_filter(
            "pnpm-lock.yaml",
            DiffReviewFilter::Code
        ));
        assert_eq!(DiffReviewFilter::Docs.empty_label(), "No doc changes.");
        assert_eq!(DiffScope::parse("turn-2"), DiffScope::Turn(2));
        assert_eq!(DiffScope::parse("working-tree"), DiffScope::WorkingTree);
        assert_eq!(DiffScope::Turn(1).value(), "turn-1");
    }

    const RAW: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    more();\n }\n@@ -12,2 +13,2 @@ fn x()\n a\n b\ndiff --git a/img.png b/img.png\nBinary files a/img.png and b/img.png differ\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n";

    #[test]
    fn parses_multi_file_unified_diff() {
        let files = parse_unified(RAW);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].hunks.len(), 2);
        assert_eq!(files[0].hunks[0].header, "@@ -1,3 +1,4 @@");
        assert_eq!((files[0].additions(), files[0].deletions()), (2, 1));
        let rows = &files[0].hunks[0].lines;
        assert_eq!(rows[0].kind, LineKind::Context);
        assert_eq!((rows[0].old_no, rows[0].new_no), (Some(1), Some(1)));
        assert_eq!(rows[1].kind, LineKind::Remove);
        assert_eq!(rows[1].old_no, Some(2));
        assert_eq!(rows[2].kind, LineKind::Add);
        assert_eq!(rows[2].new_no, Some(2));
        assert!(files[1].is_binary);
        assert!(!files[1].added && !files[1].deleted);
    }

    #[test]
    fn unified_files_carry_add_delete_flags() {
        let files = parse_unified(RAW);
        assert!(files[2].added);
        assert!(!files[2].deleted);
        assert_eq!(files[2].path, "new.txt");
        let deleted = parse_unified(
            "diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-bye\n",
        );
        assert!(deleted[0].deleted);
        assert_eq!(deleted[0].path, "gone.rs");
        assert_eq!(diff_totals(&files), (4, 1));
    }

    #[test]
    fn unmodified_gaps_come_from_hunk_headers() {
        assert_eq!(hunk_old_range("@@ -12,7 +12,9 @@ fn x()"), Some((12, 7)));
        assert_eq!(hunk_old_range("@@ -5 +5 @@"), Some((5, 1)));
        assert_eq!(hunk_old_range("edit"), None);
        assert_eq!(unmodified_gap(None, "@@ -1,3 +1,4 @@"), None);
        assert_eq!(
            unmodified_gap(Some("@@ -1,3 +1,4 @@"), "@@ -12,2 +13,2 @@"),
            Some(8)
        );
        assert_eq!(
            unmodified_gap(Some("@@ -1,3 +1,4 @@"), "@@ -4,2 +5,2 @@"),
            None
        );
    }

    #[test]
    fn first_new_line_and_split_path() {
        let files = parse_unified(RAW);
        assert_eq!(first_new_line(&files[0]), Some(1));
        assert_eq!(first_new_line(&files[1]), None);
        assert_eq!(
            split_path("src/views/a.rs"),
            ("src/views/".to_string(), "a.rs".to_string())
        );
        assert_eq!(
            split_path("README.md"),
            (String::new(), "README.md".to_string())
        );
    }

    #[test]
    fn find_checks_the_path_then_each_hunk_line() {
        use bruhs_models::logic::diff::{DiffLine, Hunk, LineKind};
        let file = super::DesktopDiffFile {
            path: "src/renamed.rs".into(),
            added: false,
            deleted: false,
            is_binary: false,
            hunks: vec![Hunk {
                header: "@@ -1,2 +1,2 @@".into(),
                lines: vec![
                    DiffLine {
                        kind: LineKind::Context,
                        old_no: Some(1),
                        new_no: Some(1),
                        text: "fn untouched() {}".into(),
                    },
                    DiffLine {
                        kind: LineKind::Add,
                        old_no: None,
                        new_no: Some(2),
                        text: "fn renamed() {}".into(),
                    },
                ],
            }],
        };
        let hits = find_in_diff_files("renam", &[file]);
        assert_eq!(hits[0].line, None);
        assert_eq!(hits[1].line, Some(1));
        assert_eq!(hits[1].start, 3);
    }
}
