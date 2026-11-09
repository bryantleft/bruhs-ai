//! Renders `timeline::TimelineRow`s: user bubbles, assistant prose, grouped
//! tool activity with a live shimmer, thinking, working, changed-files cards,
//! error rows and per-turn metadata.
//!
//! The settled transcript and the running turn arrive separately, so a
//! streaming tick touches only the running turn: its rows are rebuilt and
//! every other turn's memo stays put. Within a turn every row re-renders from
//! a per-row `Memo`, so only rows whose data changed re-render. Expansion
//! state lives in a keyed map (not per-component signals) so a row that does
//! re-render keeps whatever the user opened.

use bruhs_models::logic::context_chip::ChipKind;
use bruhs_models::num::ToIntSaturating;
use std::collections::HashMap;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::diff_view::DiffView;
use crate::components::mention_editor::insert_chip_at_caret;
use crate::components::terminal_output::TerminalOutput;
use crate::components::timeline::{
    work_summary, ChangedFile, TimelineRow, ToolKind, ToolState, ToolView, Turn,
};
use crate::format::{format_duration_ms, format_tokens, truncate_chars};
use crate::markdown;
use crate::state::{OpenFileSignal, RightPanelSignal, UsageInfo};
use crate::ui::{CopyButton, Icon, Tooltip};

/// Long prompts start collapsed behind a "Show full message" toggle.
pub fn should_collapse_user_message(text: &str) -> bool {
    text.lines().count() > 8 || text.chars().count() > 600
}

/// Whether expanding a tool row would reveal anything.
pub fn tool_has_body(tool: &ToolView) -> bool {
    let input = &tool.call.input;
    let has = |k: &str| {
        input
            .get(k)
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
    };
    let has_output = tool
        .call
        .output
        .as_deref()
        .is_some_and(|o| !o.trim().is_empty());
    match tool.kind {
        ToolKind::Bash => has("command") || has_output,
        ToolKind::Edit => {
            has("old_string")
                || has("new_string")
                || has("new_source")
                || input
                    .get("edits")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty())
        }
        ToolKind::Write => has("content"),
        ToolKind::Read => has_output,
        ToolKind::Search | ToolKind::Web | ToolKind::Other => {
            has_output || input.as_object().is_some_and(|o| !o.is_empty())
        }
    }
}

/// `3:07 PM` style clock label from local hours/minutes.
pub fn format_clock(hours: u32, minutes: u32) -> String {
    let (h12, suffix) = match hours {
        0 => (12, "AM"),
        1..=11 => (hours, "AM"),
        12 => (12, "PM"),
        _ => (hours - 12, "PM"),
    };
    format!("{h12}:{minutes:02} {suffix}")
}

/// Elapsed label for the working row: whole seconds under a minute, then
/// the shared duration format.
pub fn working_label(elapsed_ms: f64) -> String {
    let secs = (elapsed_ms / 1000.0).max(0.0).floor();
    if secs < 60.0 {
        format!("Working for {}s", secs.to_u64_saturating())
    } else {
        format!("Working for {}", format_duration_ms(elapsed_ms))
    }
}

/// `(dir/, name)` split for a changed-file path.
pub fn split_path(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (format!("{dir}/"), name.to_string()),
        None => (String::new(), path.to_string()),
    }
}

fn clock_time(at_ms: f64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(at_ms));
    format_clock(d.get_hours(), d.get_minutes())
}

pub fn kind_icon(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "eye",
        ToolKind::Edit => "square-pen",
        ToolKind::Write => "file-plus",
        ToolKind::Bash => "terminal",
        ToolKind::Search => "search",
        ToolKind::Web => "globe",
        ToolKind::Other => "wrench",
    }
}

/// Expansion state shared across re-renders, keyed by row key / tool id.
#[derive(Copy, Clone)]
struct Expanded(RwSignal<HashMap<String, bool>>);

impl Expanded {
    fn is(&self, key: &str, default: bool) -> bool {
        self.0.with(|m| m.get(key).copied().unwrap_or(default))
    }
    fn toggle(&self, key: &str, default: bool) {
        let next = !self.is(key, default);
        self.0.update(|m| {
            m.insert(key.to_string(), next);
        });
    }
}

#[derive(Copy, Clone)]
struct RowCtx {
    expanded: Expanded,
    now_ms: ReadSignal<f64>,
    awaiting: Signal<bool>,
    on_retry: Callback<(String, String)>,
    on_open_diff: Callback<(String, Option<String>)>,
    can_rewind: Signal<bool>,
    /// This thread has its own worktree, so Edit from here can restore files.
    in_worktree: Signal<bool>,
    /// `(message id, revert files too)`.
    on_edit_from_here: Callback<(String, bool)>,
}

#[component]
pub fn Timeline(
    /// Turns of the settled transcript; changes when a turn starts or ends.
    settled: Memo<Vec<Turn>>,
    /// The running turn's message, rebuilt per token.
    live: Memo<Option<Turn>>,
    #[prop(into)] is_streaming: Signal<bool>,
    /// True while the turn is blocked on a permission prompt.
    #[prop(optional, into)]
    awaiting_approval: Signal<bool>,
    /// `(failed assistant message id, prompt to resend)`.
    on_retry: Callback<(String, String)>,
    /// The thread is idle, so Edit from here can rewind it.
    can_rewind: Signal<bool>,
    /// This thread has its own worktree, so Edit from here can restore files.
    in_worktree: Signal<bool>,
    /// `(message id, revert files too)`.
    on_edit_from_here: Callback<(String, bool)>,
    /// `(user message id of the turn, optional file path to focus)`.
    on_open_diff: Callback<(String, Option<String>)>,
) -> impl IntoView {
    let expanded = Expanded(RwSignal::new(HashMap::new()));

    // One 1s ticker for every "Working for …" row; only advances mid-turn.
    let (now_ms, set_now_ms) = signal(js_sys::Date::now());
    let ticker = StoredValue::new_local(Some(gloo_timers::callback::Interval::new(
        1000,
        move || {
            if is_streaming.try_get_untracked().unwrap_or(false) {
                let _ = set_now_ms.try_set(js_sys::Date::now());
            }
        },
    )));
    on_cleanup(move || {
        ticker.try_update_value(|t| {
            t.take();
        });
    });
    Effect::new(move |_| {
        if is_streaming.get() {
            set_now_ms.set(js_sys::Date::now());
        }
    });

    let ctx = RowCtx {
        expanded,
        now_ms,
        awaiting: awaiting_approval,
        on_retry,
        on_open_diff,
        can_rewind,
        in_worktree,
        on_edit_from_here,
    };
    let cite_offer = install_cite_listener();
    // Changes only when a turn starts or ends, not per token.
    let live_id = Memo::new(move |_| live.with(|t| t.as_ref().map(|t| t.message_id.clone())));
    let turn_list = Memo::new(move |_| {
        let mut ids = settled.with(|ts| {
            ts.iter()
                .map(|t| (t.message_id.clone(), t.is_user))
                .collect::<Vec<_>>()
        });
        if let Some(id) = live_id.get() {
            if !ids.iter().any(|(mid, _)| *mid == id) {
                ids.push((id, false));
            }
        }
        ids
    });
    // Where each settled turn sits, so a turn finds its rows by index.
    let settled_index = Memo::new(move |_| {
        settled.with(|ts| {
            ts.iter()
                .enumerate()
                .map(|(i, t)| (t.message_id.clone(), i))
                .collect::<HashMap<_, _>>()
        })
    });

    view! {
        <div class="mx-auto flex w-full max-w-3xl flex-col px-4 pt-4 pb-6">
            <For
                each=move || turn_list.get()
                key=|(id, _)| id.clone()
                children=move |(mid, is_user)| {
                    let mid_rows = mid.clone();
                    // Reads `live` only while this is the running turn, so
                    // settled turns do not re-run per token.
                    let turn_rows = Memo::new(move |_| {
                        if live_id.with(|id| id.as_deref() == Some(mid_rows.as_str())) {
                            return live.with(|t| t.as_ref().map(|t| t.rows.clone()).unwrap_or_default());
                        }
                        let at = settled_index.with(|ix| ix.get(&mid_rows).copied());
                        settled.with(|ts| {
                            at.and_then(|i| ts.get(i))
                                .map(|t| t.rows.clone())
                                .unwrap_or_default()
                        })
                    });
                    let cls = if is_user { "flex flex-col" } else { "group/turn flex flex-col" };
                    view! {
                        <div class=cls data-message-id=mid>
                            <For
                                each=move || turn_rows.get()
                                key=|r| r.key().to_string()
                                children=move |row| {
                                    let key = row.key().to_string();
                                    let row_memo = Memo::new(move |_| {
                                        turn_rows.with(|rs| rs.iter().find(|r| r.key() == key).cloned())
                                    });
                                    view! { {move || row_memo.get().map(|r| render_row(r, ctx))} }
                                }
                            />
                        </div>
                    }
                }
            />
            {move || {
                cite_offer.get().map(|offer| {
                    let quote = offer.quote.clone();
                    let message_id = offer.message_id.clone();
                    view! {
                        <button
                            type="button"
                            data-cite-in-composer=""
                            aria-label="Cite in composer"
                            class="fixed z-50 h-6 rounded-seed bg-longan-900 px-1.5 text-label-sm text-persimmon-400 inset-ring inset-ring-white/5 transition-colors hover:bg-longan-800 cursor-pointer"
                            style=format!("top:{:.0}px;left:{:.0}px", offer.top, offer.left)
                            on:mousedown=|ev| ev.prevent_default()
                            on:click=move |_| {
                                insert_chip_at_caret(ChipKind::Citation {
                                    message_id: message_id.clone(),
                                    quote: quote.clone(),
                                    comment: String::new(),
                                });
                                if let Some(selection) = web_sys::window()
                                    .and_then(|window| window.get_selection().ok().flatten())
                                {
                                    selection.remove_all_ranges().ok();
                                }
                                cite_offer.set(None);
                            }
                        >
                            "Cite in composer"
                        </button>
                    }
                })
            }}
        </div>
    }
}

#[derive(Clone)]
struct CiteOffer {
    message_id: String,
    quote: String,
    top: f64,
    left: f64,
}

/// Show "Cite in composer" while a selection sits inside one assistant
/// response. The button's mousedown keeps that selection.
fn install_cite_listener() -> RwSignal<Option<CiteOffer>> {
    let cite_offer = RwSignal::new(None::<CiteOffer>);
    {
        use std::cell::RefCell;
        use std::rc::Rc;
        type SelectionClosure = Closure<dyn FnMut()>;
        let shared: Rc<RefCell<Option<SelectionClosure>>> = Rc::new(RefCell::new(None));
        let shared_cleanup = StoredValue::new_local(Rc::clone(&shared));
        let offer = cite_offer;
        let cl = Closure::<dyn FnMut()>::new(move || {
            let _ = offer.try_set(read_cite_offer());
        });
        if let Some(doc) = web_sys::window().and_then(|window| window.document()) {
            let _ = doc
                .add_event_listener_with_callback("selectionchange", cl.as_ref().unchecked_ref());
        }
        *shared.borrow_mut() = Some(cl);
        on_cleanup(move || {
            let rc = shared_cleanup.try_get_value();
            let Some(rc) = rc else {
                return;
            };
            let cb_opt = rc.borrow();
            if let Some(cl) = cb_opt.as_ref() {
                if let Some(doc) = web_sys::window().and_then(|window| window.document()) {
                    let _ = doc.remove_event_listener_with_callback(
                        "selectionchange",
                        cl.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }
    cite_offer
}

fn read_cite_offer() -> Option<CiteOffer> {
    let window = web_sys::window()?;
    let selection = window.get_selection().ok().flatten()?;
    if selection.is_collapsed() || selection.range_count() == 0 {
        return None;
    }
    let range = selection.get_range_at(0).ok()?;
    let anchor = closest_prose(selection.anchor_node())?;
    let focus = closest_prose(selection.focus_node())?;
    if !anchor.is_same_node(Some(focus.as_ref())) {
        return None;
    }
    let message_id = assistant_turn_id(&anchor)?;
    let quote = range
        .to_string()
        .as_string()
        .unwrap_or_default()
        .trim()
        .to_string();
    if quote.is_empty() {
        return None;
    }
    let rect = range.get_bounding_client_rect();
    let viewport_h = window
        .inner_height()
        .ok()
        .and_then(|value| value.as_f64())
        .unwrap_or(800.0);
    let viewport_w = window
        .inner_width()
        .ok()
        .and_then(|value| value.as_f64())
        .unwrap_or(1200.0);
    let top = if rect.bottom() + 32.0 > viewport_h {
        (rect.top() - 28.0).max(8.0)
    } else {
        rect.bottom() + 4.0
    };
    let left = rect.left().clamp(8.0, (viewport_w - 160.0).max(8.0));
    Some(CiteOffer {
        message_id,
        quote,
        top,
        left,
    })
}

fn closest_prose(node: Option<web_sys::Node>) -> Option<web_sys::Element> {
    let mut el = host_element(node)?;
    loop {
        if el.class_name().split_whitespace().any(|c| c == "prose-msg") {
            return Some(el);
        }
        el = el.parent_element()?;
    }
}

fn assistant_turn_id(prose: &web_sys::Element) -> Option<String> {
    let mut el = Some(prose.clone());
    while let Some(node) = el {
        if node.has_attribute("data-message-id") {
            if node
                .class_name()
                .split_whitespace()
                .any(|c| c == "group/turn")
            {
                return node.get_attribute("data-message-id");
            }
            return None;
        }
        el = node.parent_element();
    }
    None
}

fn host_element(node: Option<web_sys::Node>) -> Option<web_sys::Element> {
    let node = node?;
    if node.node_type() == web_sys::Node::ELEMENT_NODE {
        return node.dyn_into::<web_sys::Element>().ok();
    }
    node.parent_element()
}

fn render_row(row: TimelineRow, ctx: RowCtx) -> AnyView {
    match row {
        TimelineRow::User {
            text,
            at_ms,
            message_id,
            files,
            checkpoint,
            ..
        } => view! {
            <UserRow
                text=text
                at_ms=at_ms
                message_id=message_id
                files=files
                checkpoint=checkpoint
                can_rewind=ctx.can_rewind
                in_worktree=ctx.in_worktree
                on_edit_from_here=ctx.on_edit_from_here
            />
        }
        .into_any(),
        TimelineRow::AssistantText {
            markdown, streaming, ..
        } => view! { <AssistantTextRow markdown=markdown streaming=streaming /> }.into_any(),
        TimelineRow::Thinking {
            key, text, live, ..
        } => view! { <ThinkingRow key=key text=text live=live expanded=ctx.expanded /> }.into_any(),
        TimelineRow::Work {
            key,
            tools,
            collapsed_by_default,
            ..
        } => view! {
            <WorkGroup key=key tools=tools collapsed_by_default=collapsed_by_default expanded=ctx.expanded />
        }
        .into_any(),
        TimelineRow::Fold {
            key, label, rows, ..
        } => fold_row(key, label, rows, ctx),
        TimelineRow::Working {
            started_at_ms,
            thinking_only,
            ..
        } => view! {
            <WorkingRow
                started_at_ms=started_at_ms
                thinking_only=thinking_only
                now_ms=ctx.now_ms
                awaiting=ctx.awaiting
            />
        }
        .into_any(),
        TimelineRow::ChangedFiles {
            key,
            user_message_id,
            files,
            latest,
            ..
        } => view! {
            <ChangedFilesCard
                key=key
                user_message_id=user_message_id
                files=files
                latest=latest
                expanded=ctx.expanded
                on_open_diff=ctx.on_open_diff
            />
        }
        .into_any(),
        TimelineRow::Error {
            message_id,
            message,
            retry_prompt,
            ..
        } => view! {
            <ErrorRow message_id=message_id message=message retry_prompt=retry_prompt on_retry=ctx.on_retry />
        }
        .into_any(),
        TimelineRow::AssistantMeta {
            model,
            duration_ms,
            usage,
            at_ms,
            text,
            ..
        } => view! {
            <AssistantMetaRow model=model duration_ms=duration_ms usage=usage at_ms=at_ms text=text />
        }
        .into_any(),
    }
}

// ── Fold ─────────────────────────────────────────────────────────────────────

/// "Worked for 3m 43s ›" — a settled turn's activity, expandable in place.
fn fold_row(key: String, label: String, rows: Vec<TimelineRow>, ctx: RowCtx) -> AnyView {
    let k = StoredValue::new(key);
    let is_open = move || ctx.expanded.is(&k.get_value(), false);
    let rows_sv = StoredValue::new(rows);
    view! {
        <div class="pb-2">
            <button
                type="button"
                class="flex min-h-6 items-center gap-1 rounded-grape px-1 py-0.5 text-body-sm text-rambutan-400 transition-colors hover:text-rambutan-200"
                aria-expanded=move || is_open().to_string()
                on:click=move |_| ctx.expanded.toggle(&k.get_value(), false)
            >
                <span>{label}</span>
                {move || view! {
                    <Icon
                        name="chevron-right"
                        size=12
                        class=format!(
                            "text-rambutan-500 transition-transform duration-200 {}",
                            if is_open() { "rotate-90" } else { "" }
                        )
                    />
                }}
            </button>
            {move || is_open().then(|| view! {
                <div class="mt-1 flex flex-col border-l border-longan-800/70 ps-2">
                    {rows_sv.get_value().into_iter().map(|r| render_row(r, ctx)).collect_view()}
                </div>
            })}
        </div>
    }
    .into_any()
}

// ── User ─────────────────────────────────────────────────────────────────────

#[component]
fn UserRow(
    text: String,
    at_ms: Option<f64>,
    message_id: String,
    files: Vec<bruhs_models::logic::attachments::UploadedFile>,
    checkpoint: Option<String>,
    can_rewind: Signal<bool>,
    in_worktree: Signal<bool>,
    on_edit_from_here: Callback<(String, bool)>,
) -> impl IntoView {
    let OpenFileSignal(open_file) = expect_context::<crate::state::OpenFileSignal>();
    let RightPanelSignal(panel) = expect_context::<crate::state::RightPanelSignal>();
    // A message another thread's agent sent shows who sent it, not the
    // header written for the model. Copy still takes the whole prompt.
    let copy_text = text.clone();
    let (from_thread, text) = match bruhs_models::thread_link::parse_delivery(&text) {
        Some(delivery) => (Some(delivery.caption()), delivery.body.to_string()),
        None => (None, text),
    };
    let can_collapse = should_collapse_user_message(&text);
    let (collapsed, set_collapsed) = signal(can_collapse);
    let (confirming, set_confirming) = signal(false);
    let time = at_ms.map(clock_time).unwrap_or_default();
    let rewind_id = message_id.clone();
    let has_checkpoint = checkpoint.as_deref().is_some_and(|tree| !tree.is_empty());
    view! {
        <div class="group/user flex flex-col items-end gap-1 pb-4">
            <div class="relative w-fit max-w-[85%] rounded-lychee bg-longan-800 px-3.5 py-2.5 text-body-sm text-rambutan-100">
                {from_thread.map(|from| view! {
                    <div data-from-thread="" class="mb-1.5 text-label-sm text-blueberry-400">{from}</div>
                })}
                <div class=move || {
                    if collapsed.get() {
                        "msg-collapsed max-h-44 overflow-hidden whitespace-pre-wrap break-words"
                    } else {
                        "whitespace-pre-wrap break-words"
                    }
                }>{text}</div>
                {(!files.is_empty()).then(|| view! {
                    <div class="mt-2 flex flex-col gap-1">
                        {files.into_iter().map(|file| {
                            let name = file.name.clone();
                            let path = file.path;
                            view! {
                                <button
                                    type="button"
                                    data-sent-file=name.clone()
                                    class="flex h-8 items-center rounded-seed bg-longan-900/50 px-2 text-left text-label-sm text-rambutan-300 hover:text-rambutan-100 cursor-pointer"
                                    on:click=move |_| {
                                        crate::components::file_attachments::show_open_file(
                                            open_file,
                                            panel,
                                            path.clone(),
                                            name.clone(),
                                            None,
                                            false,
                                        );
                                    }
                                >
                                    <span class="truncate">{name.clone()}</span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                })}
                {can_collapse.then(|| view! {
                    <button
                        type="button"
                        class="-ml-1 mt-1 h-6 rounded-seed px-1.5 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100"
                        on:click=move |_| set_collapsed.update(|c| *c = !*c)
                    >
                        {move || if collapsed.get() { "Show full message" } else { "Show less" }}
                    </button>
                })}
            </div>
            <div class=move || {
                let shown = confirming.get();
                format!(
                    "flex w-full max-w-[85%] items-center justify-end gap-2 pe-1 text-label-sm tabular-nums text-rambutan-500 transition-opacity focus-within:opacity-100 {}",
                    if shown { "opacity-100" } else { "opacity-0 group-hover/user:opacity-100" }
                )
            }>
                {move || {
                    if !can_rewind.get() {
                        return ().into_any();
                    }
                    if confirming.get() {
                        let rewind_id = rewind_id.clone();
                        let in_worktree = in_worktree.get();
                        let copy = if in_worktree && has_checkpoint {
                            "Revert files too also undoes your own edits."
                        } else if in_worktree {
                            "No checkpoint for this message."
                        } else {
                            "The next turn starts a new session. Files stay as they are."
                        };
                        let keep = if in_worktree {
                            "Revert and keep changes"
                        } else {
                            "Rewind"
                        };
                        let files_id = rewind_id.clone();
                        return view! {
                            <div class="flex flex-wrap items-center justify-end gap-2" role="group" aria-label="Rewind conversation">
                                <span class="text-rambutan-400">{copy}</span>
                                <button
                                    type="button"
                                    class="h-6 rounded-seed px-1.5 text-label-sm text-persimmon-400 transition-colors hover:bg-longan-700/60 hover:text-persimmon-300"
                                    on:click=move |_| {
                                        set_confirming.set(false);
                                        on_edit_from_here.run((rewind_id.clone(), false));
                                    }
                                >
                                    {keep}
                                </button>
                                { (in_worktree && has_checkpoint).then(|| view! {
                                    <button
                                        type="button"
                                        class="h-6 rounded-seed px-1.5 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100"
                                        on:click=move |_| {
                                            set_confirming.set(false);
                                            on_edit_from_here.run((files_id.clone(), true));
                                        }
                                    >
                                        "Revert files too"
                                    </button>
                                }) }
                                <button
                                    type="button"
                                    class="h-6 rounded-seed px-1.5 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100"
                                    on:click=move |_| set_confirming.set(false)
                                >
                                    "Cancel"
                                </button>
                            </div>
                        }.into_any();
                    }
                    let message_id = message_id.clone();
                    view! {
                        <button
                            type="button"
                            class="h-6 rounded-seed px-1.5 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100"
                            data-edit-from-here=message_id
                            aria-label="Edit from here"
                            on:click=move |_| set_confirming.set(true)
                        >
                            "Edit from here"
                        </button>
                    }.into_any()
                }}
                <CopyButton text=copy_text label="Copy message" />
                <span>{time}</span>
            </div>
        </div>
    }
}

// ── Assistant prose + meta ───────────────────────────────────────────────────

#[component]
fn AssistantTextRow(markdown: String, streaming: bool) -> impl IntoView {
    if !streaming && markdown.trim().is_empty() {
        return view! {
            <p class="px-1 pb-2 text-body-sm text-rambutan-500">"(empty response)"</p>
        }
        .into_any();
    }
    let html = markdown::render_untrusted(&markdown);
    view! {
        <div class="min-w-0 px-1 py-0.5 pb-2">
            <div class="prose-msg text-body-sm leading-relaxed text-rambutan-200" inner_html=html></div>
        </div>
    }
    .into_any()
}

#[component]
fn AssistantMetaRow(
    model: Option<String>,
    duration_ms: Option<f64>,
    usage: Option<UsageInfo>,
    at_ms: Option<f64>,
    text: String,
) -> impl IntoView {
    let time = at_ms.map(clock_time);
    let tokens = usage.as_ref().map(|u| {
        (
            format!(
                "{} in · {} out",
                format_tokens(u.input_tokens),
                format_tokens(u.output_tokens)
            ),
            format!(
                "in {} · out {} · cache read {} · cache write {} · context {}",
                format_tokens(u.input_tokens),
                format_tokens(u.output_tokens),
                format_tokens(u.cache_read_tokens),
                format_tokens(u.cache_creation_tokens),
                format_tokens(u.window())
            ),
        )
    });
    let has_text = !text.trim().is_empty();
    view! {
        <div class="flex min-h-6 items-center gap-2 px-1 pb-4 text-label-sm tabular-nums text-rambutan-500 opacity-0 transition-opacity focus-within:opacity-100 group-hover/turn:opacity-100">
            {has_text.then(|| view! { <CopyButton text=text.clone() label="Copy response" /> })}
            {model.map(|m| view! { <span class="truncate">{m}</span> })}
            {duration_ms.map(|d| view! { <span>{format_duration_ms(d)}</span> })}
            {tokens.map(|(short, long)| view! {
                <Tooltip tip=long><span>{short}</span></Tooltip>
            })}
            {time.map(|t| view! { <span>{t}</span> })}
        </div>
    }
}

// ── Thinking ─────────────────────────────────────────────────────────────────

#[component]
fn ThinkingRow(key: String, text: String, live: bool, expanded: Expanded) -> impl IntoView {
    let k = StoredValue::new(key);
    let is_open = move || expanded.is(&k.get_value(), false);
    let label = if live { "Thinking" } else { "Thought process" };
    let label_cls = if live {
        "live-shimmer min-w-0 flex-1 truncate text-left text-body-sm text-rambutan-400"
    } else {
        "min-w-0 flex-1 truncate text-left text-body-sm text-rambutan-400"
    };
    let text = StoredValue::new(text);
    view! {
        <div class="pb-1">
            <button
                type="button"
                class="flex min-h-6 w-full items-center gap-1.5 rounded-grape px-0.5 py-0.5 text-left transition-colors hover:bg-longan-800/30"
                aria-expanded=move || is_open().to_string()
                on:click=move |_| expanded.toggle(&k.get_value(), false)
            >
                <span class="flex size-6 shrink-0 items-center justify-center text-rambutan-500">
                    <Icon name="brain" size=14 />
                </span>
                <span class=label_cls>{label}</span>
                <span class="flex size-4 shrink-0 items-center justify-center" aria-hidden="true">
                    {move || view! {
                        <Icon
                            name="chevron-down"
                            size=12
                            class=format!(
                                "text-rambutan-500 opacity-70 transition-transform duration-200 {}",
                                if is_open() { "rotate-180" } else { "" }
                            )
                        />
                    }}
                </span>
            </button>
            {move || is_open().then(|| view! {
                <div class="ms-7 mt-1 whitespace-pre-wrap break-words border-l border-longan-800/70 ps-3 text-body-sm text-rambutan-400">
                    {text.get_value()}
                </div>
            })}
        </div>
    }
}

// ── Tools ────────────────────────────────────────────────────────────────────

#[component]
fn WorkGroup(
    key: String,
    tools: Vec<ToolView>,
    collapsed_by_default: bool,
    expanded: Expanded,
) -> impl IntoView {
    let mut tools = tools;
    // A group of one renders as its tool row.
    if let [_] = tools.as_slice() {
        if let Some(tool) = tools.pop() {
            return view! {
                <div class="pb-2">
                    <ToolRow tool=tool key_prefix=key expanded=expanded />
                </div>
            }
            .into_any();
        }
    }

    let k = StoredValue::new(key.clone());
    let default_open = !collapsed_by_default;
    let is_open = move || expanded.is(&k.get_value(), default_open);
    let summary = work_summary(&tools);
    let icon = match tools.first() {
        Some(first) if tools.iter().all(|t| t.kind == first.kind) => kind_icon(first.kind),
        _ => "hammer",
    };
    let any_running = tools.iter().any(|t| t.state == ToolState::Running);
    let summary_cls = if any_running {
        "live-shimmer min-w-0 flex-1 truncate text-body-sm text-rambutan-400"
    } else {
        "min-w-0 flex-1 truncate text-body-sm text-rambutan-400"
    };
    let tools_sv = StoredValue::new(tools);
    let prefix = StoredValue::new(key);

    view! {
        <div class="pb-2">
            <button
                type="button"
                class="group/tool-group flex min-h-6 w-full items-center gap-1.5 rounded-grape px-0.5 py-0.5 text-left transition-colors hover:bg-longan-800/30"
                aria-expanded=move || is_open().to_string()
                on:click=move |_| expanded.toggle(&k.get_value(), default_open)
            >
                <span class="flex size-6 shrink-0 items-center justify-center text-rambutan-500">
                    <Icon name=icon size=14 />
                </span>
                <span class=summary_cls>{summary}</span>
                <span class="flex size-4 shrink-0 items-center justify-center" aria-hidden="true">
                    {move || view! {
                        <Icon
                            name="chevron-down"
                            size=12
                            class=format!(
                                "text-rambutan-500 opacity-70 transition-transform duration-200 {}",
                                if is_open() { "rotate-180" } else { "" }
                            )
                        />
                    }}
                </span>
            </button>
            {move || is_open().then(|| view! {
                <section aria-label="Activity" class="ms-3 mt-0.5 space-y-px border-l border-longan-800/70 ps-1.5">
                    {tools_sv
                        .get_value()
                        .into_iter()
                        .map(|t| view! { <ToolRow tool=t key_prefix=prefix.get_value() expanded=expanded /> })
                        .collect_view()}
                </section>
            })}
        </div>
    }
    .into_any()
}

#[component]
fn ToolRow(tool: ToolView, key_prefix: String, expanded: Expanded) -> impl IntoView {
    let can_expand = tool_has_body(&tool);
    let k = StoredValue::new(format!("{key_prefix}:{}", tool.call.id));
    let is_open = move || can_expand && expanded.is(&k.get_value(), false);
    let live = tool.state == ToolState::Running;
    let icon = kind_icon(tool.kind);
    let (icon_cls, label_cls) = match tool.state {
        ToolState::Error => ("text-lychee-400/70", "text-lychee-300"),
        ToolState::Interrupted => ("text-rambutan-500", "text-rambutan-500"),
        _ => ("text-rambutan-500", "text-rambutan-400"),
    };
    let label = tool.label.clone();
    let title = match tool.state {
        ToolState::Interrupted => format!("{label} (interrupted)"),
        ToolState::Error => format!("{label} (failed)"),
        _ => label.clone(),
    };
    let is_error = tool.state == ToolState::Error;
    let body = StoredValue::new(tool);
    let toggle = move || {
        if can_expand {
            expanded.toggle(&k.get_value(), false);
        }
    };
    let container_cls = if can_expand {
        "flex cursor-pointer flex-col rounded-grape px-0.5 py-0.5 transition-colors hover:bg-longan-800/30"
    } else {
        "flex flex-col rounded-grape px-0.5 py-0.5"
    };

    view! {
        <div
            class=container_cls
            role=if can_expand { Some("button") } else { None }
            tabindex=if can_expand { Some("0") } else { None }
            aria-expanded=move || can_expand.then(|| is_open().to_string())
            on:click=move |_| toggle()
            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                if ev.key() == "Enter" || ev.key() == " " {
                    ev.prevent_default();
                    toggle();
                }
            }
        >
            <div class="flex min-h-6 items-center gap-1.5">
                <span class=format!("flex size-6 shrink-0 items-center justify-center {icon_cls}")>
                    <Icon name=icon size=14 />
                </span>
                <span
                    class=move || format!(
                        "min-w-0 flex-1 text-body-sm {} {} {}",
                        if is_open() { "whitespace-pre-wrap break-words" } else { "truncate" },
                        label_cls,
                        if live { "live-shimmer" } else { "" },
                    )
                    title=title
                >
                    {label}
                </span>
                {is_error.then(|| view! { <Icon name="x" size=12 class="shrink-0 text-lychee-400/70" /> })}
                <span
                    class=format!(
                        "flex size-4 shrink-0 items-center justify-center {}",
                        if can_expand { "" } else { "invisible" }
                    )
                    aria-hidden="true"
                >
                    {move || view! {
                        <Icon
                            name="chevron-down"
                            size=12
                            class=format!(
                                "text-rambutan-500 opacity-70 transition-transform duration-200 {}",
                                if is_open() { "rotate-180" } else { "" }
                            )
                        />
                    }}
                </span>
            </div>
            {move || is_open().then(|| view! {
                <div class="ms-7 mt-1 cursor-default" on:click=|ev| ev.stop_propagation()>
                    {tool_body(&body.get_value())}
                </div>
            })}
        </div>
    }
}

const PRE_CLS: &str = "max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-grape bg-longan-950 px-3 py-2 text-code-sm text-rambutan-300 inset-ring inset-ring-white/5";

fn tool_body(tool: &ToolView) -> AnyView {
    let input = &tool.call.input;
    let s = |k: &str| {
        input
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let output = tool.call.output.clone().unwrap_or_default();
    match tool.kind {
        ToolKind::Bash => {
            view! { <TerminalOutput command=s("command") output=output /> }.into_any()
        }
        ToolKind::Edit => {
            let mut pairs: Vec<(String, String)> = Vec::new();
            if let Some(edits) = input.get("edits").and_then(|v| v.as_array()) {
                for e in edits {
                    let old = e.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                    let new = e.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
                    pairs.push((old.to_string(), new.to_string()));
                }
            } else if !s("old_string").is_empty() || !s("new_string").is_empty() {
                pairs.push((s("old_string"), s("new_string")));
            } else if !s("new_source").is_empty() {
                pairs.push((String::new(), s("new_source")));
            }
            view! {
                <div class="flex flex-col gap-1">
                    {pairs
                        .into_iter()
                        .map(|(old, new)| view! { <DiffView old=old new=new /> })
                        .collect_view()}
                    {(tool.state == ToolState::Error && !output.trim().is_empty()).then(|| view! {
                        <pre class=PRE_CLS>{truncate_chars(&output, 4000)}</pre>
                    })}
                </div>
            }
            .into_any()
        }
        ToolKind::Write => view! { <DiffView old=String::new() new=s("content") /> }.into_any(),
        ToolKind::Read => {
            let total = output.lines().count();
            let shown: String = output.lines().take(60).collect::<Vec<_>>().join("\n");
            let note = (total > 60).then(|| format!("… {} more lines", total - 60));
            view! {
                <div class="flex flex-col gap-1">
                    <pre class=PRE_CLS>{shown}</pre>
                    {note.map(|n| view! { <p class="px-1 text-label-sm text-rambutan-500">{n}</p> })}
                </div>
            }
            .into_any()
        }
        ToolKind::Search | ToolKind::Web | ToolKind::Other => {
            let params = input
                .as_object()
                .filter(|o| !o.is_empty())
                .map(|_| serde_json::to_string_pretty(input).unwrap_or_default())
                .unwrap_or_default();
            view! {
                <div class="flex flex-col gap-1">
                    {(!params.is_empty()).then(|| view! {
                        <pre class=PRE_CLS>{truncate_chars(&params, 4000)}</pre>
                    })}
                    {(!output.trim().is_empty()).then(|| view! {
                        <pre class=PRE_CLS>{truncate_chars(&output, 4000)}</pre>
                    })}
                </div>
            }
            .into_any()
        }
    }
}

// ── Working ──────────────────────────────────────────────────────────────────

#[component]
fn WorkingRow(
    started_at_ms: Option<f64>,
    thinking_only: bool,
    now_ms: ReadSignal<f64>,
    awaiting: Signal<bool>,
) -> impl IntoView {
    let label = move || {
        if awaiting.get() {
            return "Waiting for your approval".to_string();
        }
        match started_at_ms {
            Some(start) => working_label(now_ms.get() - start),
            None => "Working".to_string(),
        }
    };
    let dot = move || {
        if awaiting.get() {
            "relative inline-flex size-1.5 rounded-orb bg-durian-400"
        } else {
            "relative inline-flex size-1.5 rounded-orb bg-blueberry-400"
        }
    };
    let ping = move || {
        if awaiting.get() {
            "hidden"
        } else {
            "absolute inline-flex size-full animate-ping rounded-orb bg-blueberry-400 opacity-60"
        }
    };
    view! {
        <div class="pb-2 pt-0.5">
            {thinking_only.then(|| view! {
                <div class="flex min-h-6 items-center gap-1.5 px-0.5">
                    <span class="flex size-6 shrink-0 items-center justify-center text-rambutan-500">
                        <Icon name="brain" size=14 />
                    </span>
                    <span class="live-shimmer text-body-sm text-rambutan-400">"Thinking"</span>
                </div>
            })}
            <div
                class="flex h-6 items-center gap-2 px-1.5 text-body-sm tabular-nums text-rambutan-400"
                role="status"
                aria-live="polite"
            >
                <span class="relative flex size-2 items-center justify-center" aria-hidden="true">
                    <span class=ping></span>
                    <span class=dot></span>
                </span>
                <span>{label}</span>
            </div>
        </div>
    }
}

// ── Changed files ────────────────────────────────────────────────────────────

#[component]
fn ChangedFilesCard(
    key: String,
    user_message_id: String,
    files: Vec<ChangedFile>,
    latest: bool,
    expanded: Expanded,
    on_open_diff: Callback<(String, Option<String>)>,
) -> impl IntoView {
    let k = StoredValue::new(key);
    let is_open = move || expanded.is(&k.get_value(), latest);
    let n = files.len();
    let adds: usize = files.iter().map(|f| f.additions).sum();
    let dels: usize = files.iter().map(|f| f.deletions).sum();
    let umid = StoredValue::new(user_message_id);
    let files_sv = StoredValue::new(files);
    let count_label = format!("{n} changed file{}", if n == 1 { "" } else { "s" });

    view! {
        <div class="mb-4 mt-1 rounded-lychee bg-longan-900 p-2 inset-ring inset-ring-white/5">
            <div class="flex items-center justify-between gap-2">
                <button
                    type="button"
                    aria-expanded=move || is_open().to_string()
                    class="group flex min-w-0 flex-1 items-center gap-1.5 rounded-grape px-2 py-1.5 text-left transition-colors hover:bg-longan-800/40"
                    on:click=move |_| expanded.toggle(&k.get_value(), latest)
                >
                    {move || view! {
                        <Icon
                            name="chevron-right"
                            size=14
                            class=format!(
                                "shrink-0 text-rambutan-500 transition-transform {}",
                                if is_open() { "rotate-90" } else { "" }
                            )
                        />
                    }}
                    <span class="flex shrink-0 items-center gap-1.5 whitespace-nowrap text-label-sm text-rambutan-100">
                        {count_label}
                        <span class="text-pandan-400 tabular-nums">{format!("+{adds}")}</span>
                        <span class="text-lychee-400 tabular-nums">{format!("−{dels}")}</span>
                    </span>
                    <span class="ml-1 hidden min-w-0 flex-1 truncate text-label-sm text-rambutan-500 group-hover:text-rambutan-300 sm:inline">
                        {move || if is_open() { "Hide files" } else { "Show files" }}
                    </span>
                </button>
                <button
                    type="button"
                    class="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-grape px-2.5 text-label-sm text-rambutan-300 inset-ring inset-ring-white/10 transition-colors hover:bg-longan-800/60 hover:text-rambutan-100"
                    on:click=move |_| on_open_diff.run((umid.get_value(), None))
                >
                    <Icon name="file-diff" size=12 />
                    <span class="hidden sm:inline">"Open diff"</span>
                </button>
            </div>
            {move || is_open().then(|| view! {
                <ul class="mt-1 flex flex-col gap-px px-1">
                    {files_sv.get_value().into_iter().map(|f| {
                        let (dir, name) = split_path(&f.path);
                        let path = f.path.clone();
                        let kind_cls = if f.added {
                            "w-3 shrink-0 text-label-sm text-pandan-400"
                        } else {
                            "w-3 shrink-0 text-label-sm text-durian-400"
                        };
                        view! {
                            <li>
                                <button
                                    type="button"
                                    class="flex w-full items-center gap-2 rounded-grape px-2 py-1 text-left text-code-sm transition-colors hover:bg-longan-800/40"
                                    on:click=move |_| on_open_diff.run((umid.get_value(), Some(path.clone())))
                                >
                                    <span class=kind_cls>{if f.added { "A" } else { "M" }}</span>
                                    <span class="min-w-0 flex-1 truncate">
                                        <span class="text-rambutan-500">{dir}</span>
                                        <span class="text-rambutan-100">{name}</span>
                                    </span>
                                    <span class="text-pandan-400 tabular-nums">{format!("+{}", f.additions)}</span>
                                    <span class="text-lychee-400 tabular-nums">{format!("−{}", f.deletions)}</span>
                                </button>
                            </li>
                        }
                    }).collect_view()}
                </ul>
            })}
        </div>
    }
}

// ── Error ────────────────────────────────────────────────────────────────────

#[component]
fn ErrorRow(
    message_id: String,
    message: String,
    retry_prompt: Option<String>,
    on_retry: Callback<(String, String)>,
) -> impl IntoView {
    let mid = StoredValue::new(message_id);
    let prompt = StoredValue::new(retry_prompt);
    let can_retry = prompt.get_value().is_some();
    view! {
        <div
            class="mb-4 flex items-start gap-2 rounded-grape bg-lychee-500/10 px-3 py-2 text-label-sm text-lychee-200 inset-ring inset-ring-lychee-400/30"
            role="alert"
        >
            <Icon name="circle-alert" size=14 class="mt-0.5 shrink-0 text-lychee-400" />
            <p class="min-w-0 flex-1 whitespace-pre-wrap break-words">{message}</p>
            {can_retry.then(|| view! {
                <button
                    type="button"
                    class="inline-flex min-h-11 shrink-0 items-center gap-1 rounded-seed px-2 text-lychee-100 transition-colors hover:bg-lychee-500/20 sm:min-h-6"
                    on:click=move |_| {
                        if let Some(p) = prompt.get_value() {
                            on_retry.run((mid.get_value(), p));
                        }
                    }
                >
                    <Icon name="rotate-ccw" size=12 />
                    "Retry"
                </button>
            })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ToolCallInfo;
    use bruhs_models::logic::tools::{tool_kind, tool_state};
    use serde_json::json;

    fn view_of(tool: &str, input: serde_json::Value, output: Option<&str>) -> ToolView {
        let call = ToolCallInfo {
            id: "1".into(),
            tool: tool.into(),
            input,
            output: output.map(Into::into),
            is_error: false,
            started_at_ms: None,
            ended_at_ms: None,
        };
        ToolView {
            kind: tool_kind(tool),
            state: tool_state(&call, false),
            label: String::new(),
            call,
        }
    }

    #[test]
    fn long_user_messages_collapse() {
        assert!(!should_collapse_user_message("short"));
        assert!(should_collapse_user_message(&"x\n".repeat(9)));
        assert!(should_collapse_user_message(&"x".repeat(601)));
    }

    #[test]
    fn tool_body_presence() {
        assert!(tool_has_body(&view_of(
            "Bash",
            json!({"command":"ls"}),
            None
        )));
        assert!(!tool_has_body(&view_of("Bash", json!({}), Some("  "))));
        assert!(tool_has_body(&view_of(
            "Edit",
            json!({"file_path":"a","old_string":"x","new_string":"y"}),
            Some("ok")
        )));
        assert!(!tool_has_body(&view_of(
            "Edit",
            json!({"file_path":"a"}),
            Some("ok")
        )));
        assert!(tool_has_body(&view_of(
            "Write",
            json!({"file_path":"a","content":"x"}),
            None
        )));
        assert!(!tool_has_body(&view_of(
            "Read",
            json!({"file_path":"a"}),
            None
        )));
        assert!(tool_has_body(&view_of(
            "Read",
            json!({"file_path":"a"}),
            Some("1\tfn")
        )));
        assert!(tool_has_body(&view_of(
            "Grep",
            json!({"pattern":"x"}),
            None
        )));
        assert!(!tool_has_body(&view_of("TodoWrite", json!({}), None)));
    }

    #[test]
    fn clock_and_working_labels() {
        assert_eq!(format_clock(0, 5), "12:05 AM");
        assert_eq!(format_clock(9, 30), "9:30 AM");
        assert_eq!(format_clock(12, 0), "12:00 PM");
        assert_eq!(format_clock(15, 7), "3:07 PM");
        assert_eq!(working_label(-500.0), "Working for 0s");
        assert_eq!(working_label(4_900.0), "Working for 4s");
        assert_eq!(working_label(75_000.0), "Working for 1m 15s");
    }

    #[test]
    fn split_path_separates_dir_and_name() {
        assert_eq!(split_path("src/main.rs"), ("src/".into(), "main.rs".into()));
        assert_eq!(split_path("Cargo.toml"), ("".into(), "Cargo.toml".into()));
    }
}
