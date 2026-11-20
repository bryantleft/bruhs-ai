//! MentionEditor — a textarea with @ file/thread picker, # pull requests,
//! $ skills, and / slash commands.
//!
//! The draft text is the chip label. Selecting a file, thread, or pull
//! request replaces the trigger token with that label and records the chip.
//! A skill is text: `$name`.
//!
//! Behavior:
//! - Type `@` (at start or after whitespace) → threads, then files.
//! - Type `#` the same way → pull requests in the current project.
//!   `# ` with a space stays a heading.
//! - Type `$` the same way as `@` → skills for the selected provider.
//! - Type `/` at the start of a line → slash command popover.
//! - With `members` set (a room), `@` lists only those bruhs and the other
//!   triggers stay closed.
//! - ↑/↓ navigate, Enter selects, Esc closes.
//! - Enter submits. In a list, quote, or code block it edits the draft instead.

use bruhs_models::agent::AgentProvider;
use bruhs_models::crew::room::{member_hits, mention_query, Room, ROOM_LEAD};
use bruhs_models::crew::{face_view, BruhLook, BruhState, BruhStatus, RAIL_FACE_PX};
use bruhs_models::logic::commands::{insert_skill_mention, COMMAND_HELP, SLASH_COMMANDS};
use bruhs_models::logic::composer_text::{
    composer_edit, fences, overlay_html_with_chips, paste_outcome, set_fence_language,
    ComposerEdit, ComposerKey, PasteOutcome, FENCE_LANGUAGES,
};
use bruhs_models::logic::context_chip::{
    backspace_chip, delete_forward_chip, insert_chip, markdown_for_copy, replace_range, retarget,
    ChipClipboard, ChipKind, CHIP_CLIPBOARD_MIME,
};
use bruhs_models::num::ToFloatLossy;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use web_sys::{HtmlTextAreaElement, KeyboardEvent};

use crate::api::{list_files, list_pulls, list_skills, FileEntry, PullHit, SkillHit};
use crate::state::{
    ActiveThreadId, AgentMessage, AppView, ChipRetargetGuard, ComposerChips, ComposerDraft,
    SetActiveView, SetModelSelectOpen, Thread, ThreadsSignal,
};
use crate::ui::{BrailleBruh, Icon};

mod autosize;
mod caret;
mod suggestion;

#[derive(Clone, Debug, PartialEq, Eq)]
enum PopoverKind {
    Files,
    Pulls,
    Skills,
    Slash,
    /// A room's members.
    Members,
}

#[derive(Clone, PartialEq, Eq)]
enum Pick {
    File(String),
    Thread { id: String, title: String },
    Pull(PullHit),
    Skill(String),
    Slash(&'static str),
    Member { agent_id: String, name: String },
}

#[derive(Clone, PartialEq, Eq)]
struct PickRow {
    label: String,
    sub: String,
    icon: &'static str,
    /// A bruh's face, drawn in place of the icon.
    face: Option<(BruhLook, BruhStatus)>,
    pick: Pick,
}

/// The `@` rows of a room: its members the query names, the lead marked.
fn member_rows(members: &[BruhState], lead: Option<&str>, query: &str) -> Vec<PickRow> {
    let room = Room {
        id: String::new(),
        name: String::new(),
        members: members.iter().map(|bruh| bruh.agent_id.clone()).collect(),
        lead: None,
    };
    member_hits(&room, members, query)
        .into_iter()
        .map(|bruh| PickRow {
            label: bruh.name.clone(),
            sub: if lead == Some(bruh.agent_id.as_str()) {
                ROOM_LEAD.into()
            } else {
                String::new()
            },
            icon: "",
            face: Some((bruh.look.clone(), bruh.status)),
            pick: Pick::Member {
                agent_id: bruh.agent_id.clone(),
                name: bruh.name.clone(),
            },
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SlashItem {
    name: &'static str,
    desc: &'static str,
}

fn slash_commands() -> Vec<SlashItem> {
    SLASH_COMMANDS
        .iter()
        .map(|command| SlashItem {
            name: command.name,
            desc: command.description,
        })
        .collect()
}

/// Detect which popover (if any) should be open based on textarea contents
/// and caret position. Returns the active kind plus the query string after
/// the trigger character.
#[cfg(test)]
fn detect_trigger(text: &str, caret: usize) -> Option<(PopoverKind, String)> {
    detect_trigger_in(false, text, caret)
}

/// [`detect_trigger`], or in a room (`members_mode`) only the members' `@`,
/// by the rule every app shares.
fn detect_trigger_in(
    members_mode: bool,
    text: &str,
    caret: usize,
) -> Option<(PopoverKind, String)> {
    let caret = caret.min(text.len());
    if members_mode {
        return mention_query(text, caret).map(|(_, query)| (PopoverKind::Members, query));
    }
    let before = &text[..caret];

    // Find the last `@` or `/` token in `before`.
    // - `@` triggers anywhere if preceded by start-of-string or whitespace.
    // - `/` triggers only at the start of a line.
    // We walk backward to find the nearest trigger char that's still part of
    // the current word (no whitespace between trigger and caret).
    for (i, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            break;
        }
        if ch == '@' {
            let preceding = before[..i].chars().last();
            if preceding.is_none() || preceding.is_some_and(|c| c.is_whitespace()) {
                let query = before[i + 1..].to_string();
                return Some((PopoverKind::Files, query));
            }
            return None;
        }
        if ch == '#' {
            let preceding = before[..i].chars().last();
            if preceding.is_none() || preceding.is_some_and(|c| c.is_whitespace()) {
                let query = before[i + 1..].to_string();
                return Some((PopoverKind::Pulls, query));
            }
            return None;
        }
        if ch == '$' {
            let preceding = before[..i].chars().last();
            if preceding.is_none() || preceding.is_some_and(|c| c.is_whitespace()) {
                let query = before[i + 1..].to_string();
                if query.starts_with(|ch: char| ch.is_ascii_digit()) {
                    return None;
                }
                return Some((PopoverKind::Skills, query));
            }
            return None;
        }
        if ch == '/' {
            let preceding = before[..i].chars().last();
            if preceding.is_none() || preceding == Some('\n') {
                let query = before[i + 1..].to_string();
                return Some((PopoverKind::Slash, query));
            }
            return None;
        }
    }
    None
}

/// Replace the trigger token (e.g. `@par` or `/cl`) with `replacement`.
/// Returns the new full text plus the new caret position (end of replacement).
fn replace_trigger(text: &str, caret: usize, replacement: &str) -> (String, usize) {
    let caret = caret.min(text.len());
    let before = &text[..caret];
    let after = &text[caret..];

    // Find trigger position by walking back from caret.
    let mut trigger_pos = before.len();
    for (i, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            break;
        }
        trigger_pos = i;
        if ch == '@' || ch == '#' || ch == '/' || ch == '$' {
            break;
        }
    }

    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&before[..trigger_pos]);
    out.push_str(replacement);
    let new_caret = out.len();
    // Append a trailing space if the existing remainder doesn't already start
    // with whitespace — keeps tokens easy to keep typing after.
    if !after.starts_with(|c: char| c.is_whitespace()) {
        out.push(' ');
    }
    out.push_str(after);
    (
        out,
        new_caret
            + if !after.starts_with(|c: char| c.is_whitespace()) {
                1
            } else {
                0
            },
    )
}

/// Byte range of the `@`, `#`, `$`, or `/` token that ends at `caret`.
fn trigger_bounds(text: &str, caret: usize) -> Option<(usize, usize)> {
    let caret = caret.min(text.len());
    let before = &text[..caret];
    for (index, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            return None;
        }
        if ch == '@' || ch == '#' || ch == '$' || ch == '/' {
            return Some((index, caret));
        }
    }
    None
}

fn matching_threads(
    threads: &[Thread],
    query: &str,
    current: Option<&str>,
) -> Vec<(String, String)> {
    let query = query.to_lowercase();
    threads
        .iter()
        .filter(|thread| {
            Some(thread.id.as_str()) != current
                && (query.is_empty() || thread.title.to_lowercase().contains(&query))
        })
        .take(8)
        .map(|thread| (thread.id.clone(), thread.title.clone()))
        .collect()
}

/// Height for an auto-growing textarea: the content height clamped to
/// `[min, max]`, plus whether the box needs its own scrollbar.
pub fn grow_height(scroll_height: i32, min: i32, max: i32) -> (i32, bool) {
    let h = scroll_height.clamp(min, max);
    (h, scroll_height > max)
}

/// Default auto-grow bounds.
pub const EDITOR_MIN_HEIGHT_PX: i32 = 52;
pub const EDITOR_MAX_HEIGHT_PX: i32 = 200;

#[component]
pub fn MentionEditor(
    value: ReadSignal<String>,
    set_value: WriteSignal<String>,
    /// Project the `@` search runs in; `None` searches the home directory.
    #[prop(into)]
    project_path: Signal<Option<String>>,
    on_submit: Callback<()>,
    /// Cmd/Ctrl+Enter. The composer uses this for the opposite follow-up.
    #[prop(optional)]
    on_opposite: Option<Callback<()>>,
    /// Option/Alt+Up at the start of the draft edits the latest queued message.
    #[prop(optional)]
    on_edit_queued: Option<Callback<()>>,
    /// Plain ArrowUp / ArrowDown. The bools are `(older, caret on the edge
    /// for that direction)`. Returning true consumes the key.
    #[prop(optional)]
    on_recall: Option<Callback<(bool, bool), bool>>,
    /// A paste that becomes a text attachment instead of draft text.
    #[prop(optional)]
    on_attach_paste: Option<Callback<String>>,
    /// A pasted or dropped file. The editor does not also insert text.
    #[prop(optional)]
    on_attach_file: Option<Callback<web_sys::File>>,
    /// A click on an inline chip. The argument is the chip id.
    #[prop(optional)]
    on_chip: Option<Callback<String>>,
    /// `/goal` or `/compact` chosen from the slash menu. The editor clears
    /// the token after this runs.
    #[prop(optional)]
    on_command: Option<Callback<&'static str>>,
    #[prop(optional, into)] placeholder: String,
    #[prop(optional)] rows: Option<u32>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] suggestion_blocked: Signal<bool>,
    /// `false` drops the textarea's own ring/background and auto-grows it,
    /// for hosting inside the composer surface. Default `true`.
    #[prop(optional)]
    chrome: Option<bool>,
    /// A room's members, in room order. When set, `@` lists only them and
    /// picks a bruh chip; `#`, `$` and `/` don't open.
    #[prop(optional, into)]
    members: Option<Signal<Vec<BruhState>>>,
    /// The room's lead, marked in the `@` list.
    #[prop(optional, into)]
    lead: Signal<Option<String>>,
) -> impl IntoView {
    let members_mode = members.is_some();
    let textarea_ref: NodeRef<leptos::html::Textarea> = NodeRef::new();
    let placeholder = if placeholder.is_empty() {
        "Message bruhs...".to_string()
    } else {
        placeholder
    };
    let chrome = chrome.unwrap_or(true);
    let rows = if chrome { rows.unwrap_or(3) } else { 1 };
    let textarea_class = move || {
        if chrome {
            "resize-none block w-full rounded-grape bg-white/5 px-3 py-2 text-label leading-[1.45] text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400 disabled:opacity-60 disabled:cursor-not-allowed".to_string()
        } else if value.get().is_empty() {
            "block w-full resize-none bg-transparent px-3 pt-2.5 pb-1 text-body leading-relaxed text-rambutan-100 placeholder:text-rambutan-500 focus:outline-none disabled:opacity-60 disabled:cursor-not-allowed".to_string()
        } else {
            "block w-full resize-none bg-transparent px-3 pt-2.5 pb-1 text-body leading-relaxed text-transparent caret-rambutan-100 placeholder:text-rambutan-500 focus:outline-none disabled:opacity-60 disabled:cursor-not-allowed".to_string()
        }
    };

    autosize::bind_value(textarea_ref, value, !chrome);
    let ComposerChips(context_chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(chip_guard) = expect_context::<ChipRetargetGuard>();
    let threads = expect_context::<ThreadsSignal>().0;
    let active_thread_id = expect_context::<ActiveThreadId>().0;
    let force_inline = StoredValue::new(false);
    let caret_byte = RwSignal::new(0usize);
    let scroll_top = RwSignal::new(0i32);
    let overlay_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    let previous_draft = StoredValue::new(String::new());
    Effect::new(move |_| {
        let new = value.get();
        if chip_guard.get_value() {
            chip_guard.set_value(false);
            previous_draft.set_value(new);
            return;
        }
        let old = previous_draft.get_value();
        if old != new {
            context_chips.update(|chips| *chips = retarget(chips, &old, &new));
        }
        previous_draft.set_value(new);
    });
    let styled = Memo::new(move |_| {
        let text = value.get();
        if chrome || text.is_empty() {
            String::new()
        } else {
            let chips = context_chips.get();
            overlay_html_with_chips(&text, caret_byte.get(), &chips)
        }
    });
    Effect::new(move |_| {
        value.track();
        let Some(textarea) = textarea_ref.get() else {
            return;
        };
        remember_caret(&textarea, caret_byte);
    });

    // Popover state
    let (popover_kind, set_popover_kind) = signal::<Option<PopoverKind>>(None);
    let (popover_query, set_popover_query) = signal::<String>(String::new());
    let (file_results, set_file_results) = signal::<Vec<FileEntry>>(vec![]);
    let (pull_results, set_pull_results) = signal::<Vec<PullHit>>(vec![]);
    let (pull_error, set_pull_error) = signal::<Option<String>>(None);
    let (skill_results, set_skill_results) = signal::<Vec<SkillHit>>(vec![]);
    let (skill_error, set_skill_error) = signal::<Option<String>>(None);
    let (selected_idx, set_selected_idx) = signal::<usize>(0);
    // Tracks the textarea value at the moment the popover was dismissed via
    // Escape, so `recompute` doesn't immediately reopen it on the next key
    // event while the trigger token (`/`, `@`) is still present. Cleared when
    // the value changes via user input.
    let (suppressed_value, set_suppressed_value) = signal::<Option<String>>(None);
    let suggestion = suggestion::bind(
        textarea_ref,
        value,
        Signal::derive(move || {
            chrome
                || members_mode
                || disabled.get()
                || suggestion_blocked.get()
                || popover_kind.get().is_some()
        }),
    );

    // Fetch files when query / kind changes.
    Effect::new(move |_| {
        let kind = popover_kind.get();
        let query = popover_query.get();
        if kind != Some(PopoverKind::Files) {
            return;
        }
        let project = project_path.get_untracked();
        spawn_local(async move {
            // A failed search shows "No matches" rather than stale results.
            let entries = list_files(&query, project.as_deref())
                .await
                .unwrap_or_else(|error| {
                    log::warn!("file mention search failed: {error}");
                    Vec::new()
                });
            set_file_results.set(entries);
            set_selected_idx.set(0);
        });
    });

    Effect::new(move |_| {
        let kind = popover_kind.get();
        let query = popover_query.get();
        if kind != Some(PopoverKind::Pulls) {
            return;
        }
        let project = project_path.get_untracked();
        spawn_local(async move {
            let (entries, error) = list_pulls(&query, project.as_deref())
                .await
                .unwrap_or_else(|error| (Vec::new(), Some(error.to_string())));
            if popover_query.get_untracked() != query {
                return;
            }
            set_pull_results.set(entries);
            set_pull_error.set(error);
            set_selected_idx.set(0);
        });
    });

    let current_model = expect_context::<crate::state::CurrentModel>().0;
    Effect::new(move |_| {
        let kind = popover_kind.get();
        let query = popover_query.get();
        let model = current_model.get();
        if kind != Some(PopoverKind::Skills) {
            return;
        }
        let project = project_path.get_untracked();
        let provider = AgentProvider::for_model(&model).as_str().to_string();
        spawn_local(async move {
            let (entries, error) = list_skills(&query, project.as_deref(), &provider)
                .await
                .unwrap_or_else(|error| (Vec::new(), Some(error.to_string())));
            if popover_query.get_untracked() != query {
                return;
            }
            set_skill_results.set(entries);
            set_skill_error.set(error);
            set_selected_idx.set(0);
        });
    });

    // Memo: filtered slash commands matching current query.
    let filtered_slash = Memo::new(move |_| {
        let q = popover_query.get().to_lowercase();
        slash_commands()
            .into_iter()
            .filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q))
            .collect::<Vec<_>>()
    });

    // Number of items currently visible in the popover.
    let picker_rows = Memo::new(move |_| match popover_kind.get() {
        Some(PopoverKind::Files) => {
            let query = popover_query.get();
            let current = active_thread_id.get();
            let mut rows = matching_threads(&threads.get(), &query, current.as_deref())
                .into_iter()
                .map(|(id, title)| PickRow {
                    label: title.clone(),
                    sub: "Thread".into(),
                    icon: "@",
                    face: None,
                    pick: Pick::Thread { id, title },
                })
                .collect::<Vec<_>>();
            rows.extend(file_results.get().into_iter().map(|file| {
                let icon = if file.is_dir { "folder" } else { "file" };
                PickRow {
                    label: file.name,
                    sub: file.path.clone(),
                    icon,
                    face: None,
                    pick: Pick::File(file.path),
                }
            }));
            rows
        }
        Some(PopoverKind::Pulls) => pull_results
            .get()
            .into_iter()
            .map(|pull| {
                let label = format!("#{}", pull.number);
                let sub = format!("{}: {}", pull.state, pull.title);
                PickRow {
                    label,
                    sub,
                    icon: "#",
                    face: None,
                    pick: Pick::Pull(pull),
                }
            })
            .collect(),
        Some(PopoverKind::Skills) => skill_results
            .get()
            .into_iter()
            .map(|skill| PickRow {
                label: format!("${}", skill.name),
                sub: skill.description,
                icon: "$",
                face: None,
                pick: Pick::Skill(skill.name),
            })
            .collect(),
        Some(PopoverKind::Slash) => filtered_slash
            .get()
            .into_iter()
            .map(|command| PickRow {
                label: command.name.to_string(),
                sub: command.desc.to_string(),
                icon: "",
                face: None,
                pick: Pick::Slash(command.name),
            })
            .collect(),
        Some(PopoverKind::Members) => {
            let query = popover_query.get();
            let lead = lead.get();
            members.map_or_else(Vec::new, |members| {
                members.with(|members| member_rows(members, lead.as_deref(), &query))
            })
        }
        None => Vec::new(),
    });

    let item_count = move || picker_rows.with(Vec::len);

    // Close the popover and remember the textarea value at the moment of
    // dismissal, so subsequent `recompute()` calls (from keyup, click, etc.)
    // don't immediately reopen it while the trigger char is still present.
    let close_popover = move || {
        let current_value = textarea_ref
            .get_untracked()
            .map(|ta| ta.value())
            .unwrap_or_default();
        set_popover_kind.set(None);
        set_popover_query.set(String::new());
        set_selected_idx.set(0);
        set_suppressed_value.set(Some(current_value));
    };

    // Recompute popover state from the textarea after every keystroke.
    // If the value hasn't changed since an Escape-dismissal, skip re-opening
    // — the user explicitly closed it and hasn't edited the trigger away yet.
    let recompute = move || {
        let Some(ta) = textarea_ref.get() else { return };
        let val = ta.value();
        if suppressed_value.get_untracked().as_deref() == Some(val.as_str()) {
            return;
        }
        let caret = caret::selection_byte_offset(&ta, &val);
        match detect_trigger_in(members_mode, &val, caret) {
            Some((kind, query)) => {
                let kind_changed = popover_kind.get_untracked().as_ref() != Some(&kind);
                if kind_changed {
                    set_popover_kind.set(Some(kind));
                    set_selected_idx.set(0);
                }
                set_popover_query.set(query);
            }
            None => {
                if popover_kind.get_untracked().is_some() {
                    set_popover_kind.set(None);
                    set_popover_query.set(String::new());
                    set_selected_idx.set(0);
                }
            }
        }
    };

    // Context handles for slash command side effects. Pulled here so the
    // apply_selection callback can read them without re-entering context
    // lookups inside an event handler.
    let set_messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let SetModelSelectOpen(set_model_select_open) = expect_context::<SetModelSelectOpen>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();

    // Apply a selection from the popover. Wrapped as a Callback so the
    // popover-list closures (one per item) can each invoke it without
    // taking ownership of a non-Copy FnMut.
    let apply_selection: Callback<usize> = Callback::new(move |idx: usize| {
        let Some(ta) = textarea_ref.get() else { return };
        let val = ta.value();
        let caret = caret::selection_byte_offset(&ta, &val);

        let rows = picker_rows.get_untracked();
        let Some(row) = rows.get(idx) else { return };

        // Slash commands execute as actions rather than inserting text.
        if let Pick::Slash(name) = row.pick {
            match name {
                "/clear" => {
                    set_messages.set(Vec::new());
                }
                "/help" => {
                    // Insert a stub help message so the user sees something.
                    set_messages.update(|msgs| {
                        let now = js_sys::Date::now();
                        msgs.push(AgentMessage::assistant_note(
                            format!("help-{}", bruhs_models::EpochMs::saturating_from_f64(now)),
                            COMMAND_HELP.into(),
                            now,
                        ));
                    });
                }
                "/goal" | "/compact" | "/handoff" | "/second-opinion" | "/role" => {
                    if let Some(on_command) = on_command.as_ref() {
                        on_command.run(name);
                    }
                }
                "/model" => {
                    set_model_select_open.set(true);
                }
                "/settings" => {
                    set_active_view.set(AppView::Settings);
                }
                _ => {}
            }
            // Clear the textarea (drop the leading `/cmd` token entirely).
            let (cleared, _) = replace_trigger(&val, caret, "");
            let trimmed = cleared.trim_end().to_string();
            ta.set_value(&trimmed);
            let end = u32::try_from(trimmed.encode_utf16().count()).unwrap_or(u32::MAX);
            let _ = ta.set_selection_start(Some(end));
            let _ = ta.set_selection_end(Some(end));
            set_value.set(trimmed);
            set_popover_kind.set(None);
            set_popover_query.set(String::new());
            set_selected_idx.set(0);
            return;
        }

        let kind = match &row.pick {
            Pick::File(path) => ChipKind::File { path: path.clone() },
            Pick::Thread { id, title } => ChipKind::Thread {
                thread_id: id.clone(),
                title: title.clone(),
            },
            Pick::Pull(pull) => ChipKind::PullRequest {
                number: pull.number,
                title: pull.title.clone(),
                url: pull.url.clone(),
                state: pull.state.clone(),
            },
            Pick::Skill(name) => {
                let (next, new_caret) = insert_skill_mention(&val, caret, name);
                write_composer(&ta, &next, new_caret, set_value, caret_byte);
                set_popover_kind.set(None);
                set_popover_query.set(String::new());
                set_selected_idx.set(0);
                return;
            }
            Pick::Member { agent_id, name } => ChipKind::Bruh {
                agent_id: agent_id.clone(),
                name: name.clone(),
            },
            Pick::Slash(_) => return,
        };
        let bounds = if members_mode {
            mention_query(&val, caret).map(|(at, _)| (at, caret))
        } else {
            trigger_bounds(&val, caret)
        };
        let Some((trigger_start, trigger_end)) = bounds else {
            return;
        };
        let existing = context_chips.get_untracked();
        let (cleared, kept) = replace_range(&val, &existing, trigger_start, trigger_end, "");
        let id = uuid::Uuid::new_v4().to_string();
        let (next, new_caret, kept) = insert_chip(&cleared, &kept, trigger_start, id, kind);
        chip_guard.set_value(true);
        context_chips.set(kept);
        write_composer(&ta, &next, new_caret, set_value, caret_byte);
        set_popover_kind.set(None);
        set_popover_query.set(String::new());
        set_selected_idx.set(0);
    });

    let accept_suggestion = Callback::new(move |_: ()| {
        let Some(ta) = textarea_ref.get_untracked() else {
            return;
        };
        suggestion::remember_selection(&ta, suggestion.selection);
        let suffix = suggestion.suffix.get_untracked();
        if suffix.is_empty() {
            return;
        }
        let original = ta.value();
        let expected = format!("{original}{suffix}");
        // Native insertion preserves the browser's undo stack.
        if let Some(document) = web_sys::window().and_then(|window| window.document()) {
            if let Ok(command) = js_sys::Reflect::get(&document, &"execCommand".into()) {
                if let Some(command) = command.dyn_ref::<js_sys::Function>() {
                    let _ = command.call3(
                        &document,
                        &"insertText".into(),
                        &false.into(),
                        &suffix.into(),
                    );
                }
            }
        }
        // Browsers without execCommand still get acceptance.
        if ta.value() != expected {
            write_composer(&ta, &expected, expected.len(), set_value, caret_byte);
        } else {
            set_value.set(expected);
        }
        suggestion.dismiss.run(());
    });

    let on_input = move |ev: web_sys::Event| {
        let Some(ta) = ev
            .target()
            .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
        else {
            return;
        };
        let new_val = ta.value();
        suggestion::remember_selection(&ta, suggestion.selection);
        // Any edit invalidates an Escape-dismissal: the user typed something
        // new, so the next trigger should open the popover again.
        if suppressed_value.get_untracked().as_deref() != Some(new_val.as_str()) {
            set_suppressed_value.set(None);
        }
        set_value.set(new_val);
        remember_caret(&ta, caret_byte);
        recompute();
    };

    let on_keydown = move |ev: KeyboardEvent| {
        // IME owns all keys while composing, including Tab and Enter.
        if ev.is_composing() || ev.key_code() == 229 {
            return;
        }
        let key = ev.key();
        let popover_open = popover_kind.get_untracked().is_some();
        let meta = ev.meta_key() || ev.ctrl_key();
        if !popover_open && !meta && !ev.alt_key() && !ev.shift_key() {
            if key == "Escape" {
                suggestion.dismiss.run(());
            } else if key == "Tab" && !ev.repeat() {
                if let Some(ta) = textarea_ref.get_untracked() {
                    suggestion::remember_selection(&ta, suggestion.selection);
                    let suffix = suggestion.suffix.get_untracked();
                    if !suffix.is_empty() {
                        ev.prevent_default();
                        ev.stop_propagation();
                        accept_suggestion.run(());
                        return;
                    }
                }
            }
        }
        if key == "v" && meta && ev.shift_key() && !ev.alt_key() {
            force_inline.set_value(true);
        }

        // Opposite follow-up, before the popover can treat Enter as a selection.
        if key == "Enter" && !ev.shift_key() && meta {
            ev.prevent_default();
            if let Some(on_opposite) = on_opposite.as_ref() {
                on_opposite.run(());
            }
            return;
        }
        // Alt+Up at the start of the draft edits the latest queued message.
        // Anywhere else it does not move the caret.
        if key == "ArrowUp" && ev.alt_key() && !meta && !ev.shift_key() && !popover_open {
            ev.prevent_default();
            let at_start = ev
                .target()
                .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
                .is_some_and(|ta| {
                    ta.selection_start().ok().flatten() == Some(0)
                        && ta.selection_end().ok().flatten() == Some(0)
                });
            if at_start {
                if let Some(on_edit_queued) = on_edit_queued.as_ref() {
                    on_edit_queued.run(());
                }
            }
            return;
        }
        // Recall a sent prompt. Anywhere the caret is not on the matching
        // visual edge, the arrow moves as usual.
        if matches!(key.as_str(), "ArrowUp" | "ArrowDown")
            && !ev.alt_key()
            && !meta
            && !ev.shift_key()
            && !popover_open
        {
            if let Some(on_recall) = on_recall.as_ref() {
                let edges = ev
                    .target()
                    .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
                    .and_then(|ta| caret::visual_line_edges(&ta));
                if let Some((on_first, on_last)) = edges {
                    let older = key == "ArrowUp";
                    let on_edge = if older { on_first } else { on_last };
                    if on_recall.run((older, on_edge)) {
                        ev.prevent_default();
                        return;
                    }
                }
            }
        }

        if !meta && !ev.alt_key() && !ev.shift_key() {
            if let Some(removed) = chip_key_removal(&key, &ev, &context_chips) {
                ev.prevent_default();
                let (next, caret, kept) = removed;
                chip_guard.set_value(true);
                context_chips.set(kept);
                if let Some(ta) = ev
                    .target()
                    .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
                {
                    write_composer(&ta, &next, caret, set_value, caret_byte);
                }
                set_popover_kind.set(None);
                return;
            }
        }

        if popover_open {
            match key.as_str() {
                "Escape" => {
                    ev.prevent_default();
                    close_popover();
                    return;
                }
                "ArrowDown" => {
                    ev.prevent_default();
                    let n = item_count();
                    if n > 0 {
                        set_selected_idx.update(|i| *i = (*i + 1) % n);
                    }
                    return;
                }
                "ArrowUp" => {
                    ev.prevent_default();
                    let n = item_count();
                    if n > 0 {
                        set_selected_idx.update(|i| *i = if *i == 0 { n - 1 } else { *i - 1 });
                    }
                    return;
                }
                "Enter" if !ev.shift_key() && item_count() > 0 => {
                    ev.prevent_default();
                    let idx = selected_idx.get_untracked();
                    apply_selection.run(idx);
                    return;
                }
                _ => {}
            }
        }

        // Lists, quotes, and code blocks. Enter outside them still sends.
        if !popover_open {
            if let Some(composer_key) = composer_key_of(&key, ev.shift_key(), meta, ev.alt_key()) {
                if let Some(ta) = ev
                    .target()
                    .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
                {
                    let current = ta.value();
                    let start = caret::byte_offset(
                        &current,
                        ta.selection_start().ok().flatten().unwrap_or(0) as usize,
                    );
                    let end = caret::byte_offset(
                        &current,
                        ta.selection_end().ok().flatten().unwrap_or(0) as usize,
                    );
                    if let ComposerEdit::Apply {
                        text,
                        start,
                        end,
                        caret,
                    } = composer_edit(&current, start, end, composer_key)
                    {
                        ev.prevent_default();
                        let next = splice(&current, start, end, &text);
                        ta.set_value(&next);
                        let unit = caret::utf16_offset(&next, caret);
                        let _ = ta.set_selection_start(Some(unit));
                        let _ = ta.set_selection_end(Some(unit));
                        caret_byte.set(caret);
                        set_value.set(next);
                        return;
                    }
                }
            }
        }

        // Default Enter (no popover) → submit.
        if key == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            on_submit.run(());
        }
    };

    // Slight delay needed for selectionStart to update on key events that
    // change caret (e.g. ArrowLeft). Recompute on keyup as well.
    let on_keyup = move |ev: KeyboardEvent| {
        if ev.key() == "v" || !(ev.meta_key() || ev.ctrl_key()) || !ev.shift_key() {
            force_inline.set_value(false);
        }
        if let Some(ta) = ev
            .target()
            .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
        {
            remember_caret(&ta, caret_byte);
            suggestion::remember_selection(&ta, suggestion.selection);
        }
        recompute();
    };

    let on_click = move |ev: web_sys::MouseEvent| {
        if let Some(ta) = ev
            .target()
            .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
        {
            remember_caret(&ta, caret_byte);
            suggestion::remember_selection(&ta, suggestion.selection);
        }
        recompute();
    };

    let on_copy = move |ev: web_sys::ClipboardEvent| {
        let Some(ta) = ev
            .target()
            .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
        else {
            return;
        };
        let text = ta.value();
        let start = caret::byte_offset(
            &text,
            ta.selection_start().ok().flatten().unwrap_or(0) as usize,
        );
        let end = caret::byte_offset(
            &text,
            ta.selection_end().ok().flatten().unwrap_or(0) as usize,
        );
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        if start == end {
            return;
        }
        let chips = context_chips.get_untracked();
        let contained: Vec<_> = chips
            .iter()
            .filter(|chip| chip.start >= start && chip.end <= end)
            .cloned()
            .map(|mut chip| {
                chip.start -= start;
                chip.end -= start;
                chip
            })
            .collect::<Vec<_>>();
        if contained.is_empty() {
            return;
        }
        ev.prevent_default();
        let slice = text[start..end].to_string();
        let markdown = markdown_for_copy(&slice, &contained);
        let clip = ChipClipboard {
            text: slice,
            chips: contained,
        };
        if let Some(data) = ev.clipboard_data() {
            let _ = data.set_data("text/plain", &markdown);
            if let Ok(json) = serde_json::to_string(&clip) {
                let _ = data.set_data(CHIP_CLIPBOARD_MIME, &json);
            }
        }
    };

    let on_paste = move |ev: web_sys::ClipboardEvent| {
        let force = force_inline.get_value();
        force_inline.set_value(false);
        if let Some(data) = ev.clipboard_data() {
            if let Ok(json) = data.get_data(CHIP_CLIPBOARD_MIME) {
                if let Ok(clip) = serde_json::from_str::<ChipClipboard>(&json) {
                    if !clip.text.is_empty() {
                        ev.prevent_default();
                        let Some(ta) = ev
                            .target()
                            .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
                        else {
                            return;
                        };
                        let current = ta.value();
                        let start = caret::byte_offset(
                            &current,
                            ta.selection_start().ok().flatten().unwrap_or(0) as usize,
                        );
                        let end = caret::byte_offset(
                            &current,
                            ta.selection_end().ok().flatten().unwrap_or(0) as usize,
                        );
                        let (start, end) = if start <= end {
                            (start, end)
                        } else {
                            (end, start)
                        };
                        let existing = context_chips.get_untracked();
                        let (next, mut kept) =
                            replace_range(&current, &existing, start, end, &clip.text);
                        for chip in clip.chips {
                            kept.push(bruhs_models::logic::context_chip::ContextChip {
                                start: start + chip.start,
                                end: start + chip.end,
                                ..chip
                            });
                        }
                        kept.sort_by_key(|chip| chip.start);
                        let caret = start + clip.text.len();
                        chip_guard.set_value(true);
                        context_chips.set(kept);
                        write_composer(&ta, &next, caret, set_value, caret_byte);
                        return;
                    }
                }
            }
        }
        if let Some(files) = ev.clipboard_data().and_then(|data| data.files()) {
            if files.length() > 0 {
                ev.prevent_default();
                if let Some(on_attach_file) = on_attach_file.as_ref() {
                    for index in 0..files.length() {
                        if let Some(file) = files.get(index) {
                            on_attach_file.run(file);
                        }
                    }
                }
                return;
            }
        }
        let Some(text) = ev
            .clipboard_data()
            .and_then(|data| data.get_data("text/plain").ok())
        else {
            return;
        };
        // Without a host for attachments, such as a room, a long paste stays text.
        if paste_outcome(&value.get_untracked(), &text, force) == PasteOutcome::Attach {
            if let Some(on_attach_paste) = on_attach_paste.as_ref() {
                ev.prevent_default();
                on_attach_paste.run(text);
            }
        }
    };

    let on_scroll = move |ev: web_sys::Event| {
        let Some(ta) = ev
            .target()
            .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
        else {
            return;
        };
        let top = ta.scroll_top();
        scroll_top.set(top);
        if let Some(overlay) = overlay_ref.get() {
            overlay.set_scroll_top(top);
        }
    };

    // Keyboard moves through a long list keep the selected row in view.
    let list_ref: NodeRef<leptos::html::Ul> = NodeRef::new();
    Effect::new(move |_| {
        let index = selected_idx.get();
        if popover_kind.get().is_none() {
            return;
        }
        request_animation_frame(move || {
            let Some(Some(list)) = list_ref.try_get_untracked() else {
                return;
            };
            if let Ok(Some(row)) = list.query_selector(&format!("[data-mention-row=\"{index}\"]")) {
                let options = web_sys::ScrollIntoViewOptions::new();
                options.set_block(web_sys::ScrollLogicalPosition::Nearest);
                row.scroll_into_view_with_scroll_into_view_options(&options);
            }
        });
    });
    let room_label = members_mode.then(|| placeholder.clone());

    // ── Render ───────────────────────────────────────────────────────────────
    view! {
        <div class="relative grid">
            {move || {
                let suffix = suggestion.suffix.get();
                (!suffix.is_empty()).then(|| view! {
                    <div aria-hidden="true" data-prompt-suggestion="" class="pointer-events-none col-start-1 row-start-1 max-h-[200px] overflow-hidden whitespace-pre-wrap break-words px-3 pt-2.5 pb-1 text-body leading-relaxed">
                        <span class="text-transparent">{move || value.get()}</span><span class="text-rambutan-500">{suffix}</span>
                    </div>
                })
            }}
            {(!chrome).then(|| view! {
                <div
                    node_ref=overlay_ref
                    aria-hidden="true"
                    data-composer-overlay=""
                    class="pointer-events-none absolute inset-0 z-20 overflow-hidden whitespace-pre-wrap px-3 pt-2.5 pb-1 text-body leading-relaxed text-rambutan-100"
                    inner_html=move || styled.get()
                    on:click=move |ev: web_sys::MouseEvent| {
                        let Some(target) = ev
                            .target()
                            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                        else {
                            return;
                        };
                        let Some(host) = target.closest("[data-chip-start]").ok().flatten() else {
                            return;
                        };
                        let Some(start) = host
                            .get_attribute("data-chip-start")
                            .and_then(|value| value.parse::<usize>().ok())
                        else {
                            return;
                        };
                        let id = context_chips.with_untracked(|chips| {
                            chips
                                .iter()
                                .find(|chip| chip.start == start)
                                .map(|chip| chip.id.clone())
                        });
                        if let Some(id) = id {
                            ev.prevent_default();
                            if let Some(on_chip) = on_chip.as_ref() {
                                on_chip.run(id);
                            }
                        }
                    }
                />
            })}
            <textarea
                node_ref=textarea_ref
                // A room's composer is not the chat's: chat shortcuts look for `chat-input`.
                id=if members_mode { "room-input" } else { "chat-input" }
                aria-label=room_label
                class=move || format!("col-start-1 row-start-1 {}", textarea_class())
                placeholder=move || if suggestion.suffix.get().is_empty() { placeholder.clone() } else { String::new() }
                // A room has no prompt suggestions, and the id stays the chat's.
                aria-describedby=(!members_mode).then_some("prompt-suggestion-help")
                rows=rows
                disabled=move || disabled.get()
                on:input=on_input
                on:keydown=on_keydown
                on:keyup=on_keyup
                on:click=on_click
                on:paste=on_paste
                on:copy=on_copy
                on:scroll=on_scroll
                on:focus=move |_| suggestion.focused.set(true)
                on:blur=move |_| suggestion.focused.set(false)
                on:compositionstart=move |_| suggestion.composing.set(true)
                on:compositionend=move |_| suggestion.composing.set(false)
                on:select=move |_| { if let Some(ta) = textarea_ref.get_untracked() { suggestion::remember_selection(&ta, suggestion.selection); } }
            />
            {(!members_mode).then(|| view! { <span id="prompt-suggestion-help" class="sr-only">{move || if suggestion.suffix.get().is_empty() { "" } else { "Next prompt suggested. Press Tab to accept or Escape to dismiss." }}</span> })}
            {move || (!suggestion.suffix.get().is_empty()).then(|| view! {
                <button type="button" class="col-start-1 px-3 py-1 text-left text-label-sm text-rambutan-500 hover:text-rambutan-200 focus-visible:outline-persimmon-400 cursor-pointer" aria-label="Accept suggested prompt"
                    tabindex="-1"
                    on:pointerdown=move |event| event.prevent_default()
                    on:click=move |_| accept_suggestion.run(())>"Tab to accept suggestion"</button>
            })}
            {move || fence_languages(chrome, value, set_value, textarea_ref, scroll_top)}

            // Floating popover (anchored above the textarea, full width).
            {move || {
                let kind = popover_kind.get();
                if kind.is_none() {
                    return ().into_any();
                }

                // Build uniform rows from whichever source is active, then
                // render. This keeps the JSX simple and avoids
                // For-macro pitfalls.
                let items: Vec<PickRow> = picker_rows.get();

                if items.is_empty() {
                    let message = if matches!(kind, Some(PopoverKind::Pulls)) {
                        pull_error.get().unwrap_or_else(|| "No matches".into())
                    } else if matches!(kind, Some(PopoverKind::Skills)) {
                        skill_error.get().unwrap_or_else(|| "No matches".into())
                    } else {
                        "No matches".into()
                    };
                    let muted = message == "No matches";
                    return view! {
                        <div class="absolute bottom-full left-0 right-0 mb-1 z-40 rounded-grape bg-longan-700 inset-ring inset-ring-white/15" data-mention-popover="">
                            <div class=if muted { "px-3 py-2 text-label-sm text-rambutan-500" } else { "px-3 py-2 text-label-sm text-rambutan-400" }>
                                {message}
                            </div>
                        </div>
                    }.into_any();
                }

                let rendered: Vec<_> = items
                    .into_iter()
                    .enumerate()
                    .map(|(i, PickRow { label, sub, icon, face, .. })| {
                        view! {
                            <li
                                data-mention-row=i
                                // A room's list is a listbox of bruhs, each a touch-sized row.
                                role=members_mode.then_some("option")
                                aria-selected=move || members_mode.then(|| (selected_idx.get() == i).to_string())
                                on:mouseenter=move |_| set_selected_idx.set(i)
                                on:mousedown=move |ev: web_sys::MouseEvent| {
                                    // mousedown so the textarea doesn't lose focus first
                                    ev.prevent_default();
                                    apply_selection.run(i);
                                }
                                class=move || format!(
                                    "flex items-center gap-2 px-3 py-1.5 cursor-pointer text-body-sm {} {}",
                                    if members_mode { "bruhs-control" } else { "" },
                                    if selected_idx.get() == i {
                                        "bg-longan-800 text-rambutan-100"
                                    } else {
                                        "text-rambutan-300"
                                    }
                                )
                            >
                                {face.map(|(look, status)| {
                                    let still = face_view(&look, status, 0, true);
                                    view! { <BrailleBruh size=f64::from(RAIL_FACE_PX) view=Signal::derive(move || still) /> }
                                })}
                                {(!icon.is_empty()).then(|| {
                                    if matches!(icon, "file" | "folder") {
                                        view! { <Icon name=icon size=12 class="text-rambutan-500" /> }.into_any()
                                    } else {
                                        view! { <span class="w-3 text-center text-rambutan-500">{icon}</span> }.into_any()
                                    }
                                })}
                                <span class="shrink-0 truncate font-medium">{label}</span>
                                <span class="min-w-0 flex-1 truncate text-label-sm text-rambutan-500">{sub}</span>
                            </li>
                        }
                    })
                    .collect();

                view! {
                    <div class="absolute bottom-full left-0 right-0 mb-1 z-40 max-h-72 overflow-y-auto rounded-grape bg-longan-700 inset-ring inset-ring-white/15" data-mention-popover="">
                        <ul
                            node_ref=list_ref
                            class="py-1"
                            role=members_mode.then_some("listbox")
                            aria-label=members_mode.then_some("Members")
                        >{rendered}</ul>
                    </div>
                }.into_any()
            }}
        </div>
    }
}

fn write_composer(
    textarea: &HtmlTextAreaElement,
    next: &str,
    caret: usize,
    set_value: WriteSignal<String>,
    caret_byte: RwSignal<usize>,
) {
    textarea.set_value(next);
    let unit = caret::utf16_offset(next, caret);
    let _ = textarea.set_selection_start(Some(unit));
    let _ = textarea.set_selection_end(Some(unit));
    caret_byte.set(caret);
    set_value.set(next.to_string());
}

fn chip_key_removal(
    key: &str,
    ev: &KeyboardEvent,
    chips: &RwSignal<Vec<bruhs_models::logic::context_chip::ContextChip>>,
) -> Option<(
    String,
    usize,
    Vec<bruhs_models::logic::context_chip::ContextChip>,
)> {
    if key != "Backspace" && key != "Delete" {
        return None;
    }
    let textarea = ev
        .target()
        .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())?;
    let current = textarea.value();
    let start = caret::byte_offset(
        &current,
        textarea.selection_start().ok().flatten().unwrap_or(0) as usize,
    );
    let end = caret::byte_offset(
        &current,
        textarea.selection_end().ok().flatten().unwrap_or(0) as usize,
    );
    if start != end {
        return None;
    }
    let chips = chips.get_untracked();
    if key == "Backspace" {
        backspace_chip(&current, &chips, start)
    } else {
        delete_forward_chip(&current, &chips, start)
    }
}

/// Insert `kind` at the composer caret. The draft signal updates even when
/// the textarea is not mounted.
pub fn insert_chip_at_caret(kind: ChipKind) {
    let ComposerDraft(draft) = expect_context::<ComposerDraft>();
    let ComposerChips(chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(guard) = expect_context::<ChipRetargetGuard>();
    let text = draft.get_untracked();
    let existing = chips.get_untracked();
    let caret = chat_input()
        .filter(|textarea| textarea.value() == text)
        .map_or(text.len(), |textarea| {
            caret::selection_byte_offset(&textarea, &text)
        });
    let id = uuid::Uuid::new_v4().to_string();
    let (next, new_caret, kept) = insert_chip(&text, &existing, caret, id, kind);
    if let Some(textarea) = chat_input() {
        textarea.set_value(&next);
        let unit = caret::utf16_offset(&next, new_caret);
        let _ = textarea.set_selection_start(Some(unit));
        let _ = textarea.set_selection_end(Some(unit));
        let _ = textarea.focus();
    }
    guard.set_value(true);
    chips.set(kept);
    draft.set(next);
}

fn chat_input() -> Option<HtmlTextAreaElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id("chat-input")?
        .dyn_into::<HtmlTextAreaElement>()
        .ok()
}

fn remember_caret(textarea: &HtmlTextAreaElement, caret_byte: RwSignal<usize>) {
    let text = textarea.value();
    let offset = textarea.selection_start().ok().flatten().unwrap_or(0);
    caret_byte.set(caret::byte_offset(&text, offset as usize));
}

fn composer_key_of(key: &str, shift: bool, meta: bool, alt: bool) -> Option<ComposerKey> {
    if meta || alt {
        return None;
    }
    match (key, shift) {
        ("Enter", true) => Some(ComposerKey::ShiftEnter),
        ("Enter", false) => Some(ComposerKey::Enter),
        ("Tab", true) => Some(ComposerKey::ShiftTab),
        ("Tab", false) => Some(ComposerKey::Tab),
        ("Backspace", false) => Some(ComposerKey::Backspace),
        _ => None,
    }
}

fn splice(text: &str, start: usize, end: usize, insert: &str) -> String {
    let start = start.min(text.len());
    let end = end.clamp(start, text.len());
    let mut out = String::with_capacity(text.len() - (end - start) + insert.len());
    out.push_str(&text[..start]);
    out.push_str(insert);
    out.push_str(&text[end..]);
    out
}

fn fence_languages(
    chrome: bool,
    value: ReadSignal<String>,
    set_value: WriteSignal<String>,
    textarea_ref: NodeRef<leptos::html::Textarea>,
    scroll_top: RwSignal<i32>,
) -> impl IntoView {
    if chrome {
        return ().into_any();
    }
    let text = value.get();
    let listings = fences(&text);
    if listings.is_empty() {
        return ().into_any();
    }
    let line_px = composer_line_px(textarea_ref);
    let scrolled = f64::from(scroll_top.get());
    listings
        .into_iter()
        .map(|fence| {
            let open = fence.open_line_byte;
            let info = fence.info;
            let top = 16.0 + fence.line_index.to_f64_lossy() * line_px - scrolled;
            let current = if info.is_empty() {
                "text".to_string()
            } else {
                info
            };
            let shown = current.clone();
            view! {
                <select
                    class="absolute right-3 z-10 h-6 max-w-32 rounded-seed bg-longan-800 px-1 text-label-sm text-rambutan-300"
                    style=format!("top: {top}px")
                    aria-label="Code block language"
                    data-fence-language=shown
                    on:change=move |ev: web_sys::Event| {
                        let Some(select) = ev
                            .target()
                            .and_then(|target| target.dyn_into::<web_sys::HtmlSelectElement>().ok())
                        else {
                            return;
                        };
                        let Some(next) = set_fence_language(&value.get_untracked(), open, &select.value()) else {
                            return;
                        };
                        if let Some(textarea) = textarea_ref.get_untracked() {
                            textarea.set_value(&next);
                        }
                        set_value.set(next);
                    }
                >
                    {language_options(current)}
                </select>
            }
        })
        .collect_view()
        .into_any()
}

fn language_options(current: String) -> impl IntoView {
    let mut languages: Vec<String> = FENCE_LANGUAGES
        .iter()
        .map(|lang| (*lang).to_string())
        .collect();
    if !languages.iter().any(|lang| lang == &current) {
        languages.insert(0, current.clone());
    }
    languages
        .into_iter()
        .map(move |lang| {
            let selected = lang == current;
            let value = lang.clone();
            view! { <option value=value selected=selected>{lang}</option> }
        })
        .collect_view()
}

fn composer_line_px(textarea_ref: NodeRef<leptos::html::Textarea>) -> f64 {
    textarea_ref
        .get()
        .and_then(|textarea| {
            let style = web_sys::window()?
                .get_computed_style(&textarea)
                .ok()
                .flatten()?;
            let raw = style.get_property_value("line-height").ok()?;
            raw.trim().trim_end_matches("px").parse::<f64>().ok()
        })
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(22.75)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grow_height_clamps() {
        assert_eq!(grow_height(40, 70, 200), (70, false));
        assert_eq!(grow_height(120, 70, 200), (120, false));
        assert_eq!(grow_height(500, 70, 200), (200, true));
    }

    #[test]
    fn trigger_detection_for_files_and_slash() {
        assert!(
            matches!(detect_trigger("hi @src", 7), Some((PopoverKind::Files, q)) if q == "src")
        );
        assert!(matches!(detect_trigger("/mod", 4), Some((PopoverKind::Slash, q)) if q == "mod"));
        assert!(detect_trigger("email me@x", 10).is_none());
        assert!(matches!(detect_trigger("see #12", 7), Some((PopoverKind::Pulls, q)) if q == "12"));
        assert!(detect_trigger("# title", 3).is_none());
        assert!(
            matches!(detect_trigger("use $re", 7), Some((PopoverKind::Skills, q)) if q == "re")
        );
        assert!(detect_trigger("cost $12", 8).is_none());
    }

    #[test]
    fn room_mode_lists_members_and_opens_no_other_popover() {
        assert_eq!(
            detect_trigger_in(true, "hi @b", 5),
            Some((PopoverKind::Members, "b".into()))
        );
        assert_eq!(detect_trigger_in(true, "#12", 3), None);
        assert_eq!(detect_trigger_in(true, "/goal", 5), None);
        assert_eq!(detect_trigger_in(true, "use $re", 7), None);
        assert_eq!(
            detect_trigger_in(false, "hi @b", 5),
            Some((PopoverKind::Files, "b".into()))
        );
        // The room's own rule: `(@` opens, `mail@` does not.
        assert_eq!(
            detect_trigger_in(true, "(@a", 3),
            Some((PopoverKind::Members, "a".into()))
        );
        assert_eq!(detect_trigger_in(true, "mail@bo", 7), None);
        // A caret past the end reads as the end.
        assert_eq!(
            detect_trigger_in(true, "@", 9),
            Some((PopoverKind::Members, String::new()))
        );
    }

    #[test]
    fn room_rows_are_members_with_their_faces_and_the_lead_marked() {
        let bruh = |id: &str, name: &str| BruhState {
            agent_id: id.into(),
            name: name.into(),
            look: BruhLook::default(),
            access: Default::default(),
            effort: None,
            status: BruhStatus::Working,
            next_wake_ms: None,
            unread: 0,
            computer: false,
            vault: Vec::new(),
            apps: Vec::new(),
            mail: Vec::new(),
        };
        let members = [bruh("c", "Cy"), bruh("b", "Bobby"), bruh("a", "Abo")];
        let rows = member_rows(&members, Some("a"), "bo");
        let shown: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| (row.label.as_str(), row.sub.as_str()))
            .collect();
        assert_eq!(shown, [("Bobby", ""), ("Abo", "Lead")]);
        assert_eq!(
            rows[1].face,
            Some((BruhLook::default(), BruhStatus::Working))
        );
        assert!(matches!(
            &rows[1].pick,
            Pick::Member { agent_id, name } if agent_id == "a" && name == "Abo"
        ));
        assert_eq!(member_rows(&members, None, "").len(), 3);
    }

    #[test]
    fn unicode_prefix_keeps_file_trigger_and_suffix_intact() {
        let text = "🙂 é @界 suffix";
        let caret = caret::byte_offset(text, "🙂 é @界".encode_utf16().count());
        assert_eq!(
            detect_trigger(text, caret),
            Some((PopoverKind::Files, "界".into()))
        );
        let (replaced, end) = replace_trigger(text, caret, "@文/🙂.rs");
        assert_eq!(replaced, "🙂 é @文/🙂.rs suffix");
        assert_eq!(&replaced[..end], "🙂 é @文/🙂.rs");
    }

    #[test]
    fn unicode_prefix_survives_removing_slash_command() {
        let text = "é界🙂\n/model";
        let caret = caret::byte_offset(text, text.encode_utf16().count());
        assert_eq!(
            detect_trigger(text, caret),
            Some((PopoverKind::Slash, "model".into()))
        );
        assert_eq!(replace_trigger(text, caret, "").0.trim_end(), "é界🙂");
    }
}
