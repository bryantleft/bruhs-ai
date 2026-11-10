//! Diff surface for the right panel. Two scopes: the edits an assistant
//! turn made (rebuilt from Edit/Write tool inputs, no server round-trip)
//! and the project working tree (`git diff` from bruhs-server).

use bruhs_models::agent::Role;
use bruhs_models::logic::find::search_diff;
use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api;
use crate::components::diff_view::DiffRow;
use crate::components::find_bar::{clear_diff_mark, reveal_diff, FindBar, FindCommands};
use crate::components::timeline::changed_files;
use crate::components::timeline_view::split_path;
use crate::format::truncate_chars;
use crate::state::{
    load_json_pref, save_json_pref, ActiveProjectSignal, AgentMessage, DiffFocusPath, DiffScope,
    DiffScopeSignal, EditorSignal, GitStatusSignal, ThreadNotice, Turns,
};
use crate::ui::{Checkbox, CopyButton, Icon, Menu, MenuAlign, MenuItem};
use bruhs_models::logic::diff::{lcs_diff, parse_unified, DiffStats, Hunk};
use bruhs_models::logic::review::{
    classify_path, content_fingerprint, review_scope_key, set_viewed, viewed_count, viewed_state,
    DiffReviewFilter, ViewedBook, ViewedState,
};

/// One file in the panel, whatever the scope.
#[derive(Clone, PartialEq, Debug)]
pub struct PanelFile {
    pub path: String,
    pub added: bool,
    pub deleted: bool,
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
    pub stats: DiffStats,
}

/// `(user_message_id, label)` for every turn whose reply changed files,
/// in thread order.
pub fn turn_scopes<'a>(
    messages: impl IntoIterator<Item = &'a AgentMessage>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut last_user: Option<(String, String)> = None;
    for m in messages {
        if m.role == Role::User {
            last_user = Some((m.id.clone(), m.text()));
            continue;
        }
        if m.role != Role::Assistant || changed_files(m, None).is_empty() {
            continue;
        }
        if let Some((uid, prompt)) = &last_user {
            let first_line = prompt.trim().lines().next().unwrap_or("").trim();
            out.push((
                uid.clone(),
                format!("Turn · {}", truncate_chars(first_line, 30)),
            ));
            last_user = None;
        }
    }
    out
}

/// Files edited by the assistant turn that answered `user_message_id`.
pub fn turn_files<'a>(
    messages: impl IntoIterator<Item = &'a AgentMessage>,
    user_message_id: &str,
    root: Option<&str>,
) -> Vec<PanelFile> {
    let Some(assistant) = messages
        .into_iter()
        .skip_while(|m| m.id != user_message_id)
        .skip(1)
        .find(|m| m.role == Role::Assistant)
    else {
        return Vec::new();
    };
    changed_files(assistant, root)
        .into_iter()
        .map(|f| PanelFile {
            path: f.path,
            added: f.added,
            deleted: false,
            is_binary: false,
            hunks: f
                .edits
                .iter()
                .enumerate()
                .map(|(i, (old, new))| Hunk {
                    header: format!("@@ edit {} @@", i + 1),
                    lines: lcs_diff(old, new),
                })
                .collect(),
            stats: DiffStats {
                additions: f.additions,
                deletions: f.deletions,
            },
        })
        .collect()
}

/// Files from `git diff` text.
pub fn files_from_unified(text: &str) -> Vec<PanelFile> {
    parse_unified(text)
        .into_iter()
        .map(|fd| {
            let stats = fd.stats();
            PanelFile {
                path: fd.display_path().to_string(),
                added: fd.old_path.is_none() && fd.new_path.is_some(),
                deleted: fd.new_path.is_none() && fd.old_path.is_some(),
                is_binary: fd.is_binary,
                hunks: fd.hunks,
                stats,
            }
        })
        .filter(|f| !f.path.is_empty())
        .collect()
}

/// New-side start line of the first hunk, for "Open in editor".
pub fn first_new_line(file: &PanelFile) -> Option<u32> {
    let header = &file.hunks.first()?.header;
    header
        .split_whitespace()
        .find_map(|part| part.strip_prefix('+'))
        .and_then(|rest| rest.split(',').next())
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| *n > 0)
}

/// `(start, count)` of the old side from a `@@ -start,count +… @@` header.
pub fn hunk_old_range(header: &str) -> Option<(usize, usize)> {
    let rest = header.strip_prefix("@@ -")?;
    let spec = rest.split(' ').next()?;
    let mut it = spec.split(',');
    let start = it.next()?.parse().ok()?;
    let count = it.next().map_or(Some(1), |c| c.parse().ok())?;
    Some((start, count))
}

/// Unmodified lines skipped between the previous git hunk (or the file
/// start) and `next`, for the "N unmodified lines" separators.
pub fn unmodified_gap(prev: Option<&str>, next: &str) -> Option<usize> {
    let (start, _) = hunk_old_range(next)?;
    let after_prev = match prev {
        Some(p) => {
            let (s, c) = hunk_old_range(p)?;
            s + c
        }
        None => 1,
    };
    (start > after_prev).then(|| start - after_prev)
}

const VIEWED_KEY: &str = "bruhs.diff.viewed";

fn panel_fingerprint(file: &PanelFile) -> String {
    if file.is_binary {
        return content_fingerprint(["binary".as_bytes()]);
    }
    content_fingerprint(
        file.hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter().map(|line| line.text.as_bytes())),
    )
}

fn scope_storage_key(project: &str, scope: &DiffScope) -> String {
    let scope = match scope {
        DiffScope::Uncommitted => "uncommitted".to_string(),
        DiffScope::Turn { user_message_id } => format!("turn:{user_message_id}"),
    };
    review_scope_key(project, &scope)
}

/// Hunk line texts in the order `Cmd+F` searches and `data-diff-line` stamps.
fn hunk_line_texts(file: &PanelFile) -> Vec<String> {
    file.hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter().map(|line| line.text.clone()))
        .collect()
}

/// Totals across files.
pub fn totals(files: &[PanelFile]) -> DiffStats {
    files.iter().fold(DiffStats::default(), |acc, f| DiffStats {
        additions: acc.additions + f.stats.additions,
        deletions: acc.deletions + f.stats.deletions,
    })
}

/// Resolve the requested scope against what the thread actually has: a
/// turn that no longer exists falls back to the latest turn with changes,
/// then to the working tree.
pub fn effective_scope(requested: Option<&DiffScope>, scopes: &[(String, String)]) -> DiffScope {
    match requested {
        Some(DiffScope::Turn { user_message_id })
            if scopes.iter().any(|(id, _)| id == user_message_id) =>
        {
            DiffScope::Turn {
                user_message_id: user_message_id.clone(),
            }
        }
        Some(DiffScope::Uncommitted) => DiffScope::Uncommitted,
        _ => scopes
            .last()
            .map_or(DiffScope::Uncommitted, |(id, _)| DiffScope::Turn {
                user_message_id: id.clone(),
            }),
    }
}

#[derive(Clone, PartialEq)]
enum TreeState {
    Idle,
    Loading,
    Ready {
        files: Vec<PanelFile>,
        truncated: bool,
    },
    Failed(String),
}

#[component]
pub fn DiffPanel() -> impl IntoView {
    let messages = expect_context::<crate::state::MessagesSignal>().0;
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let active_thread_id = expect_context::<crate::state::ActiveThreadId>().0;
    let ActiveProjectSignal(active_project) = expect_context::<ActiveProjectSignal>();
    let DiffScopeSignal(scope) = expect_context::<DiffScopeSignal>();
    let DiffFocusPath(focus) = expect_context::<DiffFocusPath>();
    let GitStatusSignal(git) = expect_context::<GitStatusSignal>();

    let project_path = Signal::derive(move || {
        active_thread_id
            .get()
            .and_then(|id| {
                threads.with(|ts| {
                    ts.iter()
                        .find(|t| t.id == id)
                        .and_then(|t| t.project_path.clone())
                })
            })
            .or_else(|| active_project.get().map(|p| p.path))
    });
    // "Open" on a file header → the editor from Settings → General.
    let EditorSignal(editor_id) = expect_context::<EditorSignal>();
    let find_cmds = expect_context::<FindCommands>();
    let find_query = RwSignal::new(String::new());
    let find_index = RwSignal::new(0usize);
    let review_filter = RwSignal::new(DiffReviewFilter::All);
    // 0 means the user has not asked. An odd value collapses, an even value expands.
    let fold_cmd = RwSignal::new(0u32);
    let viewed_book = RwSignal::new(load_json_pref::<ViewedBook>(VIEWED_KEY).unwrap_or_default());
    Effect::new(move |_| {
        review_filter.track();
        find_index.set(0);
    });
    let notice = expect_context::<ThreadNotice>();
    let on_open = Callback::new(move |(rel, line): (String, Option<u32>)| {
        let full = match project_path.get_untracked() {
            Some(root) if !rel.starts_with('/') => {
                format!("{}/{}", root.trim_end_matches('/'), rel)
            }
            _ => rel,
        };
        let editor = editor_id.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let msg = match api::open_in_editor(&full, &editor, line).await {
                Ok(label) => format!("Opened in {label}"),
                Err(e) => e.to_string(),
            };
            notice.flash(msg);
        });
    });
    // The running turn's message counts too, so its edits show as they land.
    let turns = expect_context::<Turns>();
    let live_here = turns.here(active_thread_id);
    let with_transcript = move |f: &mut dyn FnMut(&mut dyn Iterator<Item = &AgentMessage>)| {
        // The reply of the turn running on screen, if there is one.
        let live = live_here
            .get()
            .then(|| active_thread_id.with_untracked(|id| turns.reply(id.as_deref()?)))
            .flatten();
        messages.with(|ms| match &live {
            Some(live) => live.with(|m| f(&mut ms.iter().chain(m.as_ref()))),
            None => f(&mut ms.iter()),
        });
    };
    let scopes = Memo::new(move |_| {
        let mut out = Vec::new();
        with_transcript(&mut |ms| out = turn_scopes(ms));
        out
    });
    let current = Memo::new(move |_| {
        let requested = scope.get();
        scopes.with(|s| effective_scope(requested.as_ref(), s))
    });
    let scope_key = Signal::derive(move || {
        scope_storage_key(project_path.get().as_deref().unwrap_or(""), &current.get())
    });
    let is_tree = Signal::derive(move || current.get() == DiffScope::Uncommitted);

    // ── Working-tree diff ────────────────────────────────────────────────
    let (tree, set_tree) = signal(TreeState::Idle);
    let request_seq = StoredValue::new(0u32);
    let reload = move || {
        let Some(path) = project_path.get_untracked() else {
            set_tree.set(TreeState::Idle);
            return;
        };
        let seq = request_seq.with_value(|s| s + 1);
        request_seq.set_value(seq);
        set_tree.set(TreeState::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            let result = api::git_diff(&path).await;
            // The panel may be gone by now; a stale or disposed sequence
            // means this response is no longer wanted.
            if request_seq.try_get_value() != Some(seq) {
                return;
            }
            let next = match result {
                Ok(resp) => TreeState::Ready {
                    files: files_from_unified(&resp.diff),
                    truncated: resp.truncated,
                },
                Err(error) => {
                    log::warn!("diff panel: git diff failed: {error}");
                    TreeState::Failed("Couldn't read the working tree.".to_string())
                }
            };
            let _ = set_tree.try_set(next);
        });
    };
    // Load when the scope becomes the tree, the project changes, or git
    // status refreshes (the composer refreshes it after every turn).
    Effect::new(move |_| {
        let tree_scope = is_tree.get();
        project_path.track();
        git.track();
        if tree_scope {
            reload();
        }
    });

    // ── Files for the active scope ───────────────────────────────────────
    let state = Memo::new(move |_| match current.get() {
        DiffScope::Turn { user_message_id } => {
            let root = project_path.get();
            let mut files = Vec::new();
            with_transcript(&mut |ms| files = turn_files(ms, &user_message_id, root.as_deref()));
            TreeState::Ready {
                files,
                truncated: false,
            }
        }
        DiffScope::Uncommitted => tree.get(),
    });

    // Scroll to the file the changed-files card asked for, once present.
    Effect::new(move |_| {
        let Some(path) = focus.get() else {
            return;
        };
        let present = state.with(
            |s| matches!(s, TreeState::Ready { files, .. } if files.iter().any(|f| f.path == path)),
        );
        if !present {
            return;
        }
        focus.set(None);
        gloo_timers::callback::Timeout::new(0, move || scroll_to_file(&path)).forget();
    });

    let find_hits = Memo::new(move |_| {
        let query = find_query.get();
        let filter = review_filter.get();
        let files = state.with(|current| match current {
            TreeState::Ready { files, .. } => files
                .iter()
                .filter(|file| filter.includes(classify_path(&file.path)))
                .cloned()
                .collect(),
            _ => Vec::new(),
        });
        let lines: Vec<Vec<String>> = files.iter().map(hunk_line_texts).collect();
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
        search_diff(&query, &pairs)
    });
    let find_count = Signal::derive(move || find_hits.with(|hits| hits.len()));
    let find_focus = Signal::derive(move || find_cmds.diff.get());
    let reveal_path = Signal::derive(move || {
        if !find_cmds.diff_open.get() {
            return None;
        }
        let len = find_hits.with(|hits| hits.len());
        if len == 0 {
            return None;
        }
        let index = find_index.get() % len;
        find_hits.with(|hits| Some(hits[index].path.clone()))
    });
    Effect::new(move |_| {
        if find_cmds.diff.get() > 0 {
            find_cmds.diff_open.set(true);
        }
    });
    Effect::new(move |_| {
        if !find_cmds.diff_open.get() {
            clear_diff_mark();
            return;
        }
        let hits = find_hits.get();
        if hits.is_empty() {
            clear_diff_mark();
            return;
        }
        let hit = hits[find_index.get() % hits.len()].clone();
        // The file section opens in its own effect. Wait for that paint.
        gloo_timers::callback::Timeout::new(50, move || reveal_diff(&hit.path, hit.line)).forget();
    });

    let wrap = RwSignal::new(true);
    let root_ref = NodeRef::<Div>::new();
    let scope_open = RwSignal::new(false);
    let scope_label = move || match current.get() {
        DiffScope::Uncommitted => {
            let n = git.with(|g| g.dirty.len());
            if n == 0 {
                "Uncommitted changes".to_string()
            } else {
                format!("Uncommitted · {n} file{}", if n == 1 { "" } else { "s" })
            }
        }
        DiffScope::Turn { user_message_id } => scopes.with(|s| {
            s.iter()
                .find(|(id, _)| *id == user_message_id)
                .map_or_else(|| "Turn".to_string(), |(_, label)| label.clone())
        }),
    };
    let icon_btn = move |active: bool| {
        format!(
            "inline-flex size-7 items-center justify-center rounded-grape transition-colors hover:bg-longan-800/60 hover:text-rambutan-100 {}",
            if active { "bg-longan-800/60 text-rambutan-100" } else { "text-rambutan-400" }
        )
    };

    view! {
        <div
            node_ref=root_ref
            data-diff-root=""
            tabindex="-1"
            class="flex h-full min-h-0 flex-col focus:outline-none"
            on:pointerdown=move |ev| {
                // A click on a line should put focus inside the diff, so the
                // next Cmd+F searches this panel. Controls keep their own focus.
                let Some(target) = ev.target() else {
                    return;
                };
                let Ok(el) = target.dyn_into::<web_sys::Element>() else {
                    return;
                };
                if el
                    .closest("input, textarea, button, a, [contenteditable='true']")
                    .ok()
                    .flatten()
                    .is_some()
                {
                    return;
                }
                if let Some(root) = root_ref.get_untracked() {
                    let _ = root.focus();
                }
            }
        >
            <div class="flex h-9 shrink-0 items-center gap-2 border-b border-longan-800/60 px-3">
                <Menu
                    open=scope_open
                    align=MenuAlign::Start
                    label="Diff scope"
                    class="min-w-[14rem]"
                    trigger=move || view! {
                        <button
                            type="button"
                            class="inline-flex h-6 max-w-[16rem] items-center gap-1 rounded-grape bg-longan-800 px-2 text-label-sm text-rambutan-100 transition-colors hover:bg-longan-800/80"
                            aria-haspopup="menu"
                            aria-expanded=move || scope_open.get().to_string()
                            on:click=move |_| scope_open.update(|o| *o = !*o)
                        >
                            <span class="truncate">{scope_label}</span>
                            <Icon name="chevron-down" size=12 class="shrink-0 text-rambutan-400" />
                        </button>
                    }
                >
                    <MenuItem
                        selected=Signal::derive(move || current.get() == DiffScope::Uncommitted)
                        on_select=Callback::new(move |_| {
                            scope.set(Some(DiffScope::Uncommitted));
                            scope_open.set(false);
                        })
                    >
                        "Uncommitted changes"
                    </MenuItem>
                    {move || scopes.get().into_iter().rev().map(|(id, label)| {
                        let id_sel = id.clone();
                        let id_set = id;
                        view! {
                            <MenuItem
                                selected=Signal::derive(move || matches!(current.get(), DiffScope::Turn { ref user_message_id } if *user_message_id == id_sel))
                                on_select=Callback::new(move |_| {
                                    scope.set(Some(DiffScope::Turn { user_message_id: id_set.clone() }));
                                    scope_open.set(false);
                                })
                            >
                                {label}
                            </MenuItem>
                        }
                    }).collect_view()}
                </Menu>
                {move || state.with(|s| match s {
                    TreeState::Ready { files, .. } if !files.is_empty() => {
                        let shown: Vec<_> = files
                            .iter()
                            .filter(|file| review_filter.get().includes(classify_path(&file.path)))
                            .cloned()
                            .collect();
                        if shown.is_empty() {
                            return None;
                        }
                        let t = totals(&shown);
                        Some(view! {
                            <span class="flex shrink-0 items-center gap-1.5 text-label-sm tabular-nums">
                                <span class="text-pandan-400">{format!("+{}", t.additions)}</span>
                                <span class="text-lychee-400">{format!("−{}", t.deletions)}</span>
                            </span>
                        })
                    }
                    _ => None,
                })}
                <span class="flex-1"></span>
                <button
                    type="button"
                    aria-pressed=move || wrap.get().to_string()
                    aria-label="Toggle line wrap"
                    title="Toggle line wrap"
                    class=move || icon_btn(wrap.get())
                    on:click=move |_| wrap.update(|w| *w = !*w)
                >
                    <Icon name="wrap-text" size=14 />
                </button>
                {move || is_tree.get().then(|| view! {
                    <button
                        type="button"
                        aria-label="Refresh diff"
                        title="Refresh"
                        class=icon_btn(false)
                        on:click=move |_| reload()
                    >
                        <Icon name="refresh-cw" size=14 />
                    </button>
                })}
            </div>
            {move || {
                let files = state.with(|current| match current {
                    TreeState::Ready { files, .. } => files.clone(),
                    _ => Vec::new(),
                });
                if files.is_empty() {
                    return None;
                }
                let filter = review_filter.get();
                let shown: Vec<_> = files
                    .iter()
                    .filter(|file| filter.includes(classify_path(&file.path)))
                    .collect();
                let key = scope_key.get();
                let pairs: Vec<(&str, String)> = shown
                    .iter()
                    .map(|file| (file.path.as_str(), panel_fingerprint(file)))
                    .collect();
                let pair_refs: Vec<(&str, &str)> = pairs
                    .iter()
                    .map(|(path, fingerprint)| (*path, fingerprint.as_str()))
                    .collect();
                let viewed_n = viewed_book.with(|book| viewed_count(book, &key, &pair_refs));
                let collapsing = !fold_cmd.get().is_multiple_of(2);
                let fold_label = if collapsing { "Expand all" } else { "Collapse all" };
                Some(view! {
                    <div
                        data-diff-review=""
                        class="flex h-8 shrink-0 items-center gap-1 border-b border-longan-800/60 px-3"
                    >
                        {DiffReviewFilter::CHOICES.into_iter().map(|choice| {
                            let pressed = filter == choice;
                            view! {
                                <button
                                    type="button"
                                    data-diff-filter=choice.label().to_ascii_lowercase()
                                    aria-pressed=pressed.to_string()
                                    class=format!(
                                        "inline-flex h-6 items-center rounded-grape px-2 text-label-sm transition-colors cursor-pointer {}",
                                        if pressed {
                                            "bg-longan-800 text-rambutan-100"
                                        } else {
                                            "text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-100"
                                        }
                                    )
                                    on:click=move |_| review_filter.set(choice)
                                >
                                    {choice.label()}
                                </button>
                            }
                        }).collect_view()}
                        <span class="flex-1"></span>
                        <span data-diff-viewed-count="" class="shrink-0 text-label-sm tabular-nums text-rambutan-400">
                            {format!("{viewed_n} of {} viewed", shown.len())}
                        </span>
                        <button
                            type="button"
                            data-diff-collapse=""
                            aria-label=fold_label
                            class="inline-flex h-6 shrink-0 items-center rounded-grape px-2 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 cursor-pointer"
                            on:click=move |_| fold_cmd.update(|cmd| *cmd = cmd.saturating_add(1))
                        >
                            {fold_label}
                        </button>
                    </div>
                })
            }}
            {move || find_cmds.diff_open.get().then(|| view! {
                <FindBar
                    label="Find in diff"
                    query=find_query
                    index=find_index
                    count=find_count
                    open=find_cmds.diff_open
                    focus_gen=find_focus
                />
            })}
            <div class="min-h-0 flex-1 overflow-auto">
                {move || match state.get() {
                    TreeState::Idle => view! {
                        <p class="px-5 py-8 text-center text-label-sm text-rambutan-500">
                            "Open a project thread to see its working tree."
                        </p>
                    }.into_any(),
                    TreeState::Loading => view! {
                        <p class="px-5 py-8 text-center text-label-sm text-rambutan-500" role="status">
                            "Reading working tree…"
                        </p>
                    }.into_any(),
                    TreeState::Failed(msg) => view! {
                        <p class="px-5 py-8 text-center text-label-sm text-lychee-300" role="alert">{msg}</p>
                    }.into_any(),
                    TreeState::Ready { files, truncated } => {
                        if files.is_empty() {
                            let text = if is_tree.get_untracked() {
                                "Working tree is clean."
                            } else {
                                "No file changes in this turn yet."
                            };
                            return view! {
                                <p class="px-5 py-8 text-center text-label-sm text-rambutan-500">{text}</p>
                            }.into_any();
                        }
                        let filter = review_filter.get();
                        let shown: Vec<_> = files
                            .into_iter()
                            .filter(|file| filter.includes(classify_path(&file.path)))
                            .collect();
                        if shown.is_empty() {
                            return view! {
                                <p class="px-5 py-8 text-center text-label-sm text-rambutan-500">
                                    {filter.empty_label()}
                                </p>
                            }.into_any();
                        }
                        let open_all = shown.len() <= 8;
                        let key = scope_key.get();
                        view! {
                            {truncated.then(|| view! {
                                <p class="px-3 py-1.5 text-label-sm text-durian-300">
                                    "Diff truncated at 2 MiB; the remaining files are not listed."
                                </p>
                            })}
                            {shown.into_iter().map(|f| {
                                let path = f.path.clone();
                                let fingerprint = panel_fingerprint(&f);
                                let path_for_mark = path.clone();
                                let fingerprint_for_mark = fingerprint.clone();
                                let viewed = Signal::derive({
                                    let path = path.clone();
                                    let fingerprint = fingerprint.clone();
                                    let key = key.clone();
                                    move || {
                                        viewed_book.with(|book| {
                                            viewed_state(book, &key, &path, &fingerprint)
                                                == ViewedState::Viewed
                                        })
                                    }
                                });
                                let changed = Signal::derive({
                                    let key = key.clone();
                                    move || {
                                        viewed_book.with(|book| {
                                            viewed_state(book, &key, &path, &fingerprint)
                                                == ViewedState::Changed
                                        })
                                    }
                                });
                                let on_viewed = Callback::new(move |checked: bool| {
                                    let key = scope_key.get_untracked();
                                    viewed_book.update(|book| {
                                        set_viewed(
                                            book,
                                            &key,
                                            &path_for_mark,
                                            &fingerprint_for_mark,
                                            checked,
                                        );
                                    });
                                    save_json_pref(VIEWED_KEY, &viewed_book.get_untracked());
                                });
                                view! {
                                    <FileSection
                                        file=f
                                        wrap=wrap
                                        initially_open=open_all
                                        reveal_path=reveal_path
                                        fold_cmd=fold_cmd.into()
                                        viewed=viewed
                                        changed=changed
                                        on_viewed=on_viewed
                                        on_open=on_open
                                    />
                                }
                            }).collect_view()}
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

fn scroll_to_file(path: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(nodes) = doc.query_selector_all("[data-diff-path]") else {
        return;
    };
    for i in 0..nodes.length() {
        let Some(node) = nodes.item(i) else {
            continue;
        };
        let Ok(el) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        if el.get_attribute("data-diff-path").as_deref() == Some(path) {
            el.scroll_into_view();
            break;
        }
    }
}

#[component]
fn FileSection(
    file: PanelFile,
    wrap: RwSignal<bool>,
    initially_open: bool,
    /// Path of the current find hit. Matching this path opens the section.
    reveal_path: Signal<Option<String>>,
    /// Odd values collapse every section. Even values above zero expand them.
    fold_cmd: Signal<u32>,
    viewed: Signal<bool>,
    changed: Signal<bool>,
    on_viewed: Callback<bool>,
    /// `(path as shown, first changed line)` → open in the editor.
    on_open: Callback<(String, Option<u32>)>,
) -> impl IntoView {
    let (open, set_open) = signal(initially_open && !viewed.get_untracked());
    let path_for_reveal = file.path.clone();
    Effect::new(move |_| {
        let cmd = fold_cmd.get();
        if cmd > 0 {
            set_open.set(cmd.is_multiple_of(2));
        }
    });
    Effect::new(move |_| {
        if reveal_path.get().as_deref() == Some(path_for_reveal.as_str()) {
            set_open.set(true);
        }
    });
    let open_line = first_new_line(&file);
    let open_path = file.path.clone();
    let open_label = format!("Open {} in editor", file.path);
    let (dir, name) = split_path(&file.path);
    let (badge, badge_cls) = if file.added {
        ("A", "w-3 shrink-0 text-pandan-400")
    } else if file.deleted {
        ("D", "w-3 shrink-0 text-lychee-400")
    } else {
        ("M", "w-3 shrink-0 text-durian-400")
    };
    let is_binary = file.is_binary;
    let lang = crate::highlight::language_for_path(&file.path);
    let highlight_ready = RwSignal::new(false);
    crate::highlight::arm_highlight(highlight_ready);
    let ready = Signal::derive(move || highlight_ready.get());
    let hunks = StoredValue::new(file.hunks);
    let path_attr = file.path.clone();
    let title = file.path.clone();

    let copy_path = file.path.clone();
    let viewed_label = format!("Mark {} viewed", file.path);
    let set_open_for_view = set_open;

    view! {
        <section
            data-diff-path=path_attr
            data-diff-kind=classify_path(&file.path).kind_name()
            class="border-b border-longan-800/60"
        >
            <div class="group/file sticky top-0 z-10 flex h-8 items-center bg-longan-900 pr-2">
                <Checkbox
                    checked=viewed
                    label=viewed_label
                    class="ml-2".to_string()
                    on_change=Callback::new(move |checked: bool| {
                        set_open_for_view.set(!checked);
                        on_viewed.run(checked);
                    })
                />
                {move || changed.get().then(|| view! {
                    <span class="shrink-0 text-label-sm text-durian-400">"Changed"</span>
                })}
                <button
                    type="button"
                    aria-expanded=move || open.get().to_string()
                    class="flex h-8 min-w-0 flex-1 items-center gap-2 px-3 text-left text-label-sm transition-colors hover:bg-longan-800/60"
                    on:click=move |_| set_open.update(|o| *o = !*o)
                >
                    {move || view! {
                        <Icon
                            name="chevron-right"
                            size=12
                            class=format!(
                                "shrink-0 text-rambutan-500 transition-transform {}",
                                if open.get() { "rotate-90" } else { "" }
                            )
                        />
                    }}
                    <span class=badge_cls>{badge}</span>
                    <span class="min-w-0 flex-1 truncate text-code-sm" title=title>
                        <span class="text-rambutan-500">{dir}</span>
                        <span class="text-rambutan-100">{name}</span>
                    </span>
                    <span class="shrink-0 tabular-nums text-pandan-400">{format!("+{}", file.stats.additions)}</span>
                    <span class="shrink-0 tabular-nums text-lychee-400">{format!("−{}", file.stats.deletions)}</span>
                </button>
                <button
                    type="button"
                    data-diff-open=""
                    aria-label=open_label
                    title="Open in editor (Settings → Editor)"
                    class="inline-flex h-6 shrink-0 items-center rounded-grape px-1.5 text-label-sm text-rambutan-400 opacity-0 transition-opacity hover:bg-longan-800/60 hover:text-rambutan-100 focus-visible:opacity-100 group-hover/file:opacity-100 cursor-pointer"
                    on:click=move |_| on_open.run((open_path.clone(), open_line))
                >
                    "Open"
                </button>
                <CopyButton
                    text=copy_path
                    label="Copy file path"
                    class="opacity-0 transition-opacity focus-visible:opacity-100 group-hover/file:opacity-100"
                />
            </div>
            {move || open.get().then(|| {
                if is_binary {
                    return view! {
                        <p class="px-3 py-2 text-label-sm text-rambutan-500">"Binary file"</p>
                    }.into_any();
                }
                let hs = hunks.get_value();
                let mut prev_header: Option<String> = None;
                let mut line_index = 0usize;
                let blocks = hs.into_iter().map(|h| {
                    let is_git = h.header.starts_with("@@ -");
                    let gap = if is_git { unmodified_gap(prev_header.as_deref(), &h.header) } else { None };
                    prev_header = Some(h.header.clone());
                    let header = h.header.clone();
                    let rows = h.lines.into_iter().map(|l| {
                        let index = line_index;
                        line_index += 1;
                        view! {
                            <div data-diff-line=index.to_string()>
                                <DiffRow line=l wrap=wrap lang=lang highlight_ready=ready />
                            </div>
                        }
                    }).collect_view();
                    view! {
                        <div>
                            {if is_git {
                                gap.map(|n| view! {
                                    <div class="flex items-center gap-2 px-3 py-1 text-label-sm text-rambutan-500">
                                        <span class="h-px flex-1 bg-longan-800/80"></span>
                                        <span class="shrink-0 tabular-nums">{format!("{n} unmodified line{}", if n == 1 { "" } else { "s" })}</span>
                                        <span class="h-px flex-1 bg-longan-800/80"></span>
                                    </div>
                                }).into_any()
                            } else {
                                view! {
                                    <div class="bg-longan-800/40 px-3 py-0.5 text-code-sm text-rambutan-500">{header}</div>
                                }.into_any()
                            }}
                            <div class=move || if wrap.get() { "" } else { "overflow-x-auto" }>
                                {rows}
                            </div>
                        </div>
                    }
                }).collect_view();
                view! { <div class="py-1 text-code-sm">{blocks}</div> }.into_any()
            })}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MessagePart, MessageStatus, ToolCallInfo};
    use serde_json::json;

    fn edit_call(path: &str) -> ToolCallInfo {
        ToolCallInfo {
            id: "1".into(),
            tool: "Edit".into(),
            input: json!({"file_path": path, "old_string": "a", "new_string": "b"}),
            output: Some("ok".into()),
            is_error: false,
            started_at_ms: None,
            ended_at_ms: None,
        }
    }

    fn thread() -> Vec<AgentMessage> {
        let u1 = AgentMessage::user("first question that is rather long indeed".into(), 1.0);
        let mut a1 = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a1.status = MessageStatus::Finished { at_ms: None };
        a1.parts = vec![MessagePart::Text("no edits".into())];
        let u2 = AgentMessage::user("second".into(), 3.0);
        let mut a2 = AgentMessage::assistant_placeholder("m".into(), 4.0);
        a2.status = MessageStatus::Finished { at_ms: None };
        a2.parts = vec![MessagePart::ToolCall(edit_call("/r/src/x.rs"))];
        vec![u1, a1, u2, a2]
    }

    #[test]
    fn turn_scopes_only_include_turns_with_changes() {
        let msgs = thread();
        let scopes = turn_scopes(&msgs);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].0, msgs[2].id);
        assert_eq!(scopes[0].1, "Turn · second");
        let long = turn_scopes(&{
            let mut m = thread();
            m[2].parts = vec![MessagePart::Text("x".repeat(40))];
            m
        });
        assert_eq!(long[0].1.chars().count(), "Turn · ".chars().count() + 31);
    }

    #[test]
    fn turn_files_build_hunks_from_edits() {
        let msgs = thread();
        let files = turn_files(&msgs, &msgs[2].id, Some("/r"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/x.rs");
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].header, "@@ edit 1 @@");
        assert_eq!((files[0].stats.additions, files[0].stats.deletions), (1, 1));
        assert!(turn_files(&msgs, "missing", None).is_empty());
    }

    #[test]
    fn effective_scope_falls_back_sensibly() {
        let scopes = vec![("u1".to_string(), "Turn · a".to_string())];
        assert_eq!(
            effective_scope(None, &scopes),
            DiffScope::Turn {
                user_message_id: "u1".into()
            }
        );
        assert_eq!(
            effective_scope(
                Some(&DiffScope::Turn {
                    user_message_id: "gone".into()
                }),
                &scopes
            ),
            DiffScope::Turn {
                user_message_id: "u1".into()
            }
        );
        assert_eq!(effective_scope(None, &[]), DiffScope::Uncommitted);
        assert_eq!(
            effective_scope(Some(&DiffScope::Uncommitted), &scopes),
            DiffScope::Uncommitted
        );
    }

    #[test]
    fn first_new_line_uses_first_hunk_new_start() {
        let file = PanelFile {
            path: "src/a.rs".into(),
            added: false,
            deleted: false,
            is_binary: false,
            hunks: vec![
                Hunk {
                    header: "@@ -12,7 +14,9 @@ fn x()".into(),
                    lines: vec![],
                },
                Hunk {
                    header: "@@ -40,2 +44,2 @@".into(),
                    lines: vec![],
                },
            ],
            stats: DiffStats::default(),
        };
        assert_eq!(first_new_line(&file), Some(14));
        let empty = PanelFile {
            hunks: vec![],
            ..file.clone()
        };
        assert_eq!(first_new_line(&empty), None);
        let tool = PanelFile {
            hunks: vec![Hunk {
                header: "edit".into(),
                lines: vec![],
            }],
            ..file
        };
        assert_eq!(first_new_line(&tool), None);
    }

    #[test]
    fn unmodified_gaps_come_from_hunk_headers() {
        assert_eq!(hunk_old_range("@@ -12,7 +12,9 @@ fn x()"), Some((12, 7)));
        assert_eq!(hunk_old_range("@@ -1 +1,2 @@"), Some((1, 1)));
        assert_eq!(hunk_old_range("@@ edit 1 @@"), None);
        assert_eq!(unmodified_gap(None, "@@ -1,3 +1,4 @@"), None);
        assert_eq!(unmodified_gap(None, "@@ -40,3 +40,4 @@"), Some(39));
        assert_eq!(
            unmodified_gap(Some("@@ -1,3 +1,4 @@"), "@@ -4,2 +5,2 @@"),
            None
        );
        assert_eq!(
            unmodified_gap(Some("@@ -1,3 +1,4 @@"), "@@ -20,2 +21,2 @@"),
            Some(16)
        );
    }

    #[test]
    fn unified_files_carry_add_delete_flags() {
        let raw = "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\ndiff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n ctx\n";
        let files = files_from_unified(raw);
        assert_eq!(files.len(), 2);
        assert!(files[0].added);
        assert_eq!(files[0].path, "new.txt");
        assert!(!files[1].added && !files[1].deleted);
        let t = totals(&files);
        assert_eq!((t.additions, t.deletions), (3, 1));
    }

    #[test]
    fn a_turn_review_is_named_by_the_user_message() {
        let turn = scope_storage_key(
            "/repo",
            &DiffScope::Turn {
                user_message_id: "u2".into(),
            },
        );
        assert_eq!(turn, review_scope_key("/repo", "turn:u2"));
        assert_eq!(
            scope_storage_key("/repo", &DiffScope::Uncommitted),
            review_scope_key("/repo", "uncommitted")
        );
        assert_eq!(classify_path("README.md").kind_name(), "docs");
        assert_eq!(DiffReviewFilter::Code.empty_label(), "No code changes.");
    }
}
