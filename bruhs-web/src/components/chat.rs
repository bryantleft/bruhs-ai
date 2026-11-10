//! Chat thread page: owns the turn lifecycle (submit → stream → persist) and
//! hosts the transcript and composer.
//!
//! A turn's prompt goes into its thread's running turn (`Turns`) and, when
//! that thread is on screen, the visible `messages`; the reply streams into
//! the turn's own signal, patched in
//! place per token, and joins both lists when the turn ends. So a running
//! turn survives switching threads and finishes in the background, and a
//! token re-renders only the running turn.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use bruhs_models::agent::{AgentProvider, Role};
use bruhs_models::api::AgentRequest;
use bruhs_models::ids::ThreadId;
use gloo_timers::callback::Interval;
use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::api::{
    create_worktree, restore_worktree, send_agent_message_streaming, snapshot_worktree, steer_turn,
    StreamError,
};
use crate::components::auth_banner::AuthBanner;
use crate::components::chat_header::ChatHeader;
use crate::components::composer::{Composer, ComposerDraftSource, TextAttachments};
use crate::components::file_attachments::{
    offers_block_send, show_open_file, FileGeneration, FileOffer, FileOffers,
};
use crate::components::find_bar::{clear_message_mark, reveal_message, FindBar, FindCommands};
use crate::components::import_thread_dialog;
use crate::components::render_pass::install_render_pass;
use crate::components::terminal_panel::{TerminalPanel, TerminalPanelMode};
use crate::components::timeline::{build_turns, latest_assistant_id, RowBuilder};
use crate::components::timeline_view::Timeline;
use crate::state::{
    auto_title, ActiveProjectSignal, AgentMessage, BaseBranchSignal, ChipRetargetGuard,
    ClaudeAutoCompactSignal, ClaudeEffortSignal, CompactFullSignal, CompactIdleSignal,
    ComposerChips, ComposerDraft, DiffFocusPath, DiffScope, DiffScopeSignal, FollowUpSteerSignal,
    GitStatusSignal, MessageStatus, OpenFileSignal, PanelSurface, PendingPermissionsSignal,
    PermissionModeSignal, PreviousWorktreeSignal, PromptQueueSignal, QueueHeldSignal,
    RightPanelSignal, RunChoiceSignal, ThreadNotice, Turns, UnreadThreads,
};
use crate::stream::{apply_message, StreamEffect, UnknownEvents};
use crate::ui::Icon;
use bruhs_models::logic::attachments::{
    append_uploaded_files, upload_label, viewer_target, UploadedFile, SEND_WAITS,
};
use bruhs_models::logic::commands::{
    append_goal, classify_command, prepare_command, CommandKind, GoalWrite,
};
use bruhs_models::logic::compact::{
    autocompact_flag, before_expiry_due, show_return_chip, IdleCompact, CACHE_OAUTH_MS,
};
use bruhs_models::logic::composer_text::{
    exceeds_message_limit_with, visible_prompt, TextAttachment,
};
use bruhs_models::logic::context_chip::{expand_prompt, extra_limit_chars, ContextChip};
use bruhs_models::logic::find::search_texts;
use bruhs_models::logic::follow_up::{
    composer_action, provider_can_steer, ComposerAction, FollowUp, STEER_UNAVAILABLE,
};
use bruhs_models::logic::handoff::{
    handoff_prompt, provider_name, review_holds_up, reviewer_for, second_opinion_agrees,
    second_opinion_followup, SECOND_OPINION_NOTHING, SECOND_OPINION_WAIT,
};
use bruhs_models::logic::portable::{thread_context, with_import_context};
use bruhs_models::logic::recall::{compose_after_rewind, rewind_keep_end, RecallCursor};
use bruhs_models::logic::rewind::file_revert_block;
use bruhs_models::logic::run_place::{claude_effort, first_send, FirstSend, RememberedWorktree};

/// Distance from the bottom (px) within which the transcript still follows
/// new content.
const FOLLOW_THRESHOLD_PX: i32 = 48;

/// After a scroll event: whether the view follows new rows, and whether to
/// scroll back to the bottom. Only the user scrolling up stops following;
/// a following view that is short of the bottom because rows grew taller
/// is put back there.
fn follow_after_scroll(following: bool, bottom: bool, moved_up: bool) -> (bool, bool) {
    match (bottom, moved_up, following) {
        (true, _, _) => (true, false),
        (false, true, _) => (false, false),
        (false, false, following) => (following, following),
    }
}

/// How often a turn sent to a thread that is still finishing its previous
/// turn (409) is retried, and the base wait between tries.
const BUSY_RETRIES: u8 = 3;
const BUSY_RETRY_MS: u32 = 1_500;

/// Shown when a send has to wait for one of the running turns to end.
const LANES_FULL: &str =
    "Three turns are already running. This one is queued and starts when one of them ends.";

/// Shown when the thread stayed busy through every retry.
const STILL_BUSY: &str = "This thread is still finishing its previous turn. Try again in a moment.";

/// What a turn does after one attempt to stream it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NextStep {
    /// Keep the outcome.
    Finish,
    /// The provider rejected the resumed session before it started: forget
    /// it and try once more from scratch.
    RetryCold,
    /// The thread is still running a previous turn: keep the session and
    /// try again after `delay_ms`.
    RetryBusy { delay_ms: u32 },
    /// Still busy after every retry: fail the turn with [`STILL_BUSY`].
    GiveUpBusy,
}

/// One attempt's outcome, as [`next_step`] reads it.
#[derive(Debug, Clone, Copy)]
struct Attempt<'a> {
    result: &'a Result<(), StreamError>,
    /// The assistant message carries an error (from the stream itself).
    message_errored: bool,
    /// The provider announced a session, so the turn did start.
    saw_init: bool,
    /// The attempt resumed a saved session.
    resumed: bool,
    /// A cold retry already happened.
    cold_retried: bool,
    /// Busy retries so far.
    busy_retries: u8,
}

/// The last assistant answer with text, and the user message before it.
fn last_exchange(rows: &[AgentMessage]) -> (String, Option<String>) {
    let Some(at) = rows
        .iter()
        .rposition(|row| row.role == Role::Assistant && !row.text().trim().is_empty())
    else {
        return (String::new(), None);
    };
    let task = rows[..at]
        .iter()
        .rev()
        .find(|row| row.role == Role::User)
        .map(AgentMessage::text)
        .unwrap_or_default();
    (task, Some(rows[at].text()))
}

fn latest_context_tokens(messages: &[AgentMessage]) -> Option<u64> {
    messages
        .iter()
        .rev()
        .find_map(|message| Some(message.usage.as_ref()?.window()))
}

fn ready_uploads(offers: &[FileOffer]) -> Vec<UploadedFile> {
    offers
        .iter()
        .filter_map(|offer| {
            let path = offer.ready_path()?.to_string();
            Some(UploadedFile {
                name: offer.name.clone(),
                path,
            })
        })
        .collect()
}

fn event_anchor(ev: &web_sys::MouseEvent) -> Option<web_sys::Element> {
    let target = ev.target()?;
    let node = target.dyn_into::<web_sys::Node>().ok()?;
    let element = if node.node_type() == web_sys::Node::ELEMENT_NODE {
        node.dyn_into::<web_sys::Element>().ok()?
    } else {
        node.parent_element()?
    };
    element.closest("a").ok().flatten()
}

/// Text `Cmd+F` searches: the message, its thinking, its error, and the
/// string values of each tool call (command, path, output).
fn searchable_text(message: &AgentMessage) -> String {
    let mut out = String::new();
    push_block(&mut out, &message.text());
    push_block(&mut out, &message.thinking());
    if let Some(error) = &message.error {
        push_block(&mut out, error);
    }
    for call in message.tool_calls_iter() {
        push_block(&mut out, &call.tool);
        push_json_strings(&mut out, &call.input);
        if let Some(output) = &call.output {
            push_block(&mut out, output);
        }
    }
    out
}

fn push_block(out: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(text);
}

fn push_json_strings(out: &mut String, value: &serde_json::Value) {
    match value {
        serde_json::Value::String(text) => push_block(out, text),
        serde_json::Value::Array(items) => {
            for item in items {
                push_json_strings(out, item);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                push_json_strings(out, item);
            }
        }
        _ => {}
    }
}

fn next_step(attempt: Attempt<'_>) -> NextStep {
    match attempt.result {
        Err(StreamError::Stopped) => NextStep::Finish,
        Err(error) if error.is_busy() => {
            if attempt.busy_retries < BUSY_RETRIES {
                NextStep::RetryBusy {
                    delay_ms: BUSY_RETRY_MS * u32::from(attempt.busy_retries + 1),
                }
            } else {
                NextStep::GiveUpBusy
            }
        }
        result => {
            let errored = result.is_err() || attempt.message_errored;
            if errored && attempt.resumed && !attempt.saw_init && !attempt.cold_retried {
                NextStep::RetryCold
            } else {
                NextStep::Finish
            }
        }
    }
}

#[component]
pub fn ChatPage() -> impl IntoView {
    // ── Context signals ──────────────────────────────────────────────────────
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let active_thread_id = expect_context::<crate::state::ActiveThreadId>().0;
    let messages = expect_context::<crate::state::MessagesSignal>().0;
    let set_messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let set_threads = expect_context::<crate::state::SetThreadsSignal>().0;
    let set_active_thread_id = expect_context::<crate::state::SetActiveThreadId>().0;
    let is_streaming = expect_context::<crate::state::IsStreaming>().0;
    let PendingPermissionsSignal(pending_permissions) =
        expect_context::<PendingPermissionsSignal>();
    let crate::state::PendingSecretsSignal(pending_secrets) =
        expect_context::<crate::state::PendingSecretsSignal>();
    let current_model = expect_context::<crate::state::CurrentModel>().0;
    let PermissionModeSignal(permission_mode) = expect_context::<PermissionModeSignal>();
    let ActiveProjectSignal(active_project) = expect_context::<ActiveProjectSignal>();
    let turns = expect_context::<Turns>();
    let crate::state::DefaultModel(default_model) = expect_context::<crate::state::DefaultModel>();
    let UnreadThreads(unread_threads) = expect_context::<UnreadThreads>();
    let history = expect_context::<crate::thread_history::ThreadHistory>();
    let catalog = expect_context::<crate::thread_catalog::ThreadCatalog>();
    let thread_link = expect_context::<crate::thread_link::ThreadLink>();
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let thread_notice = expect_context::<ThreadNotice>();
    let PromptQueueSignal(prompt_queue) = expect_context::<PromptQueueSignal>();
    let QueueHeldSignal(queue_held) = expect_context::<QueueHeldSignal>();
    let DiffScopeSignal(diff_scope) = expect_context::<DiffScopeSignal>();
    let DiffFocusPath(diff_focus) = expect_context::<DiffFocusPath>();
    let GitStatusSignal(git) = expect_context::<GitStatusSignal>();
    let RunChoiceSignal(run_choice) = expect_context::<RunChoiceSignal>();
    let ClaudeEffortSignal(claude_effort_pref) = expect_context::<ClaudeEffortSignal>();
    let ClaudeAutoCompactSignal(claude_autocompact) = expect_context::<ClaudeAutoCompactSignal>();
    let CompactIdleSignal(compact_idle) = expect_context::<CompactIdleSignal>();
    let CompactFullSignal(compact_full) = expect_context::<CompactFullSignal>();
    let FollowUpSteerSignal(follow_up_steer) = expect_context::<FollowUpSteerSignal>();
    let crate::state::ThreadAccessSignal(thread_access) =
        expect_context::<crate::state::ThreadAccessSignal>();
    let crate::state::RolesSignal(roles) = expect_context::<crate::state::RolesSignal>();
    let crate::state::TaskModelsSignal(task_models) =
        expect_context::<crate::state::TaskModelsSignal>();
    let BaseBranchSignal(base_branch) = expect_context::<BaseBranchSignal>();
    let PreviousWorktreeSignal(previous_worktree) = expect_context::<PreviousWorktreeSignal>();
    let RightPanelSignal(panel) = expect_context::<RightPanelSignal>();
    let find_cmds = expect_context::<FindCommands>();
    let title_instructions = expect_context::<crate::state::TitleInstructionsSignal>().0;

    // ── Local signals ────────────────────────────────────────────────────────
    let find_query = RwSignal::new(String::new());
    let find_index = RwSignal::new(0usize);
    let ComposerDraft(draft) = expect_context::<ComposerDraft>();
    let (input_value, set_input_value) = draft.split();
    let recall = RwSignal::new(None::<RecallCursor>);
    let text_attachments = RwSignal::new(Vec::<TextAttachment>::new());
    let file_offers = RwSignal::new_local(Vec::<FileOffer>::new());
    let file_generation = StoredValue::new(0u32);
    let ComposerChips(context_chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(chip_guard) = expect_context::<ChipRetargetGuard>();
    let draft_source: StoredValue<Option<Callback<(), String>>> = StoredValue::new(None);
    provide_context(ComposerDraftSource(draft_source));
    provide_context(TextAttachments(text_attachments));
    provide_context(FileOffers(file_offers));
    provide_context(FileGeneration(file_generation));
    Effect::new(move |_| {
        active_thread_id.track();
        recall.set(None);
        compact_full.set(false);
        text_attachments.set(Vec::new());
        file_generation.update_value(|generation| *generation = generation.wrapping_add(1));
        file_offers.set(Vec::new());
        context_chips.set(Vec::new());
    });
    let OpenFileSignal(open_file) = expect_context::<OpenFileSignal>();
    let link_listener = window_event_listener(leptos::ev::click, move |ev| {
        let Some(anchor) = event_anchor(&ev) else {
            return;
        };
        if anchor.closest(".prose-msg").ok().flatten().is_none() {
            return;
        }
        let Some(href) = anchor.get_attribute("href") else {
            return;
        };
        let Some(link) = viewer_target(&href) else {
            return;
        };
        ev.prevent_default();
        let project = active_project.get_untracked().map(|project| project.path);
        show_open_file(
            open_file,
            panel,
            link.path.clone(),
            upload_label(&link.path),
            project,
            link.source,
        );
    });
    on_cleanup(move || {
        file_generation.update_value(|generation| *generation = generation.wrapping_add(1));
        link_listener.remove();
    });
    // Debounced IndexedDB flush while a turn streams.
    let persist_timer = StoredValue::new_local(None::<gloo_timers::callback::Timeout>);

    let messages_container = NodeRef::<Div>::new();

    // ── Derived values ───────────────────────────────────────────────────────
    let active_thread = move || {
        let id = active_thread_id.get()?;
        threads.with(|ts| ts.iter().find(|t| t.id == id).cloned())
    };
    let project_root = Memo::new(move |_| {
        active_thread()
            .and_then(|t| t.project_path)
            .or_else(|| active_project.get().map(|p| p.path))
    });
    // Whether the thread on screen has a running turn. Other threads may be
    // running too; each has its own turn in `turns`.
    let live_here = turns.here(active_thread_id);
    // The reply that turn is writing.
    let live_reply = move || {
        if !live_here.get() {
            return None;
        }
        active_thread_id.with(|id| turns.reply(id.as_deref()?))
    };
    // The same answers for handlers, read now: `is_streaming` and the memo
    // above settle a tick after a turn starts or ends.
    let running_here = move || {
        active_thread_id.with_untracked(|id| {
            id.as_deref()
                .is_some_and(|id| turns.is_running_untracked(id))
        })
    };
    let stop_here = move || {
        if let Some(id) = active_thread_id.get_untracked() {
            turns.abort(&id);
        }
    };
    // Its message id; changes when a turn starts or ends, not per token.
    let live_id = Memo::new(move |_| live_reply()?.with(|m| m.as_ref().map(|m| m.id.clone())));
    // The settled transcript, built when it changes (a turn starts or ends,
    // a thread loads), never per token.
    let settled_turns = Memo::new(move |_| {
        let root = project_root.get();
        let live_id = live_id.get();
        messages.with(|ms| {
            let latest = live_id.as_deref().or_else(|| latest_assistant_id(ms));
            build_turns(ms, root.as_deref(), latest)
        })
    });
    // Cmd+F over the settled transcript plus the running turn.
    let find_hits = Memo::new(move |_| {
        let query = find_query.get();
        let mut owned: Vec<(String, String)> = Vec::new();
        messages.with(|ms| {
            for message in ms {
                owned.push((message.id.clone(), searchable_text(message)));
            }
        });
        if let Some(live) = live_reply() {
            live.with(|current| {
                if let Some(message) = current {
                    if !owned.iter().any(|(id, _)| id == &message.id) {
                        owned.push((message.id.clone(), searchable_text(message)));
                    }
                }
            });
        }
        let pairs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(id, text)| (id.as_str(), text.as_str()))
            .collect();
        search_texts(&query, &pairs)
    });
    let find_count = Signal::derive(move || find_hits.with(|hits| hits.len()));
    let find_focus = Signal::derive(move || find_cmds.thread.get());
    Effect::new(move |_| {
        if find_cmds.thread.get() > 0 {
            find_cmds.thread_open.set(true);
        }
    });
    Effect::new(move |_| {
        active_thread_id.track();
        find_index.set(0);
    });

    // The running turn, rebuilt per token from its message alone.
    let live_turn = Memo::new(move |_| {
        let live = live_reply()?;
        let root = project_root.get();
        live.with(|m| {
            let m = m.as_ref()?;
            let mut builder =
                messages.with(|ms| RowBuilder::after(ms, root.as_deref(), Some(&m.id)));
            Some(builder.turn(m))
        })
    });

    // ── Scroll follow ────────────────────────────────────────────────────────
    // Follow new rows while the user is at (or near) the bottom; a manual
    // scroll up parks the view and shows the "Scroll to end" pill.
    let at_bottom = RwSignal::new(true);
    // Deferred callers may outlive this page (view switch, thread change), so
    // never unwrap the node ref.
    let scroll_to_bottom = move || {
        if let Some(Some(el)) = messages_container.try_get_untracked() {
            let html_el: web_sys::HtmlElement = (*el).clone();
            html_el.set_scroll_top(html_el.scroll_height());
        }
    };
    let scroll_soon = move || {
        gloo_timers::callback::Timeout::new(0, scroll_to_bottom).forget();
    };
    // Where the last scroll event left the view, to tell the user scrolling
    // up from rows rendering taller under a view that follows them.
    let last_scroll_top = StoredValue::new(0i32);
    // Rows that grow after they render (highlighted code, diagrams, fonts)
    // fire no scroll event: keep a following view at the bottom as they do.
    let content_observer =
        StoredValue::new_local(None::<(web_sys::ResizeObserver, Closure<dyn FnMut()>)>);
    Effect::new(move |_| {
        let Some(container) = messages_container.get() else {
            return;
        };
        let Some(content) = container.first_element_child() else {
            return;
        };
        let callback = Closure::wrap(Box::new(move || {
            if at_bottom.get_untracked() {
                scroll_to_bottom();
            }
        }) as Box<dyn FnMut()>);
        match web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            Ok(resize) => {
                resize.observe(&content);
                content_observer.update_value(|value| {
                    if let Some((old, _)) = value.replace((resize, callback)) {
                        old.disconnect();
                    }
                });
            }
            Err(error) => log::warn!("Transcript resize observation unavailable: {error:?}"),
        }
    });
    on_cleanup(move || {
        content_observer.update_value(|value| {
            if let Some((resize, _callback)) = value.take() {
                resize.disconnect();
            }
        });
    });
    Effect::new(move |_| {
        settled_turns.track();
        live_turn.track();
        if at_bottom.get_untracked() {
            scroll_soon();
        }
    });
    Effect::new(move |_| {
        active_thread_id.track();
        at_bottom.set(true);
        if !find_cmds.thread_open.get_untracked() {
            scroll_soon();
        }
    });
    // After the follow effect, so an open find keeps the current match on screen.
    Effect::new(move |_| {
        if !find_cmds.thread_open.get() {
            clear_message_mark();
            return;
        }
        let hits = find_hits.get();
        if hits.is_empty() {
            clear_message_mark();
            return;
        }
        let index = find_index.get() % hits.len();
        let id = hits[index].source_id.clone();
        at_bottom.set(false);
        gloo_timers::callback::Timeout::new(0, move || reveal_message(&id)).forget();
    });

    install_render_pass(messages, is_streaming);

    // ── Running turn helpers ─────────────────────────────────────────────────
    // Save every running turn as it stands: its settled messages plus the
    // reply so far.
    let persist_running_now = move || {
        for tid in turns.running_untracked() {
            if let Some(messages) = turns.snapshot(&tid) {
                saves.edit_conversation(tid, messages);
            }
        }
    };
    let flush_persist = move || {
        persist_timer.update_value(|slot| {
            if let Some(t) = slot.take() {
                t.cancel();
            }
        });
        persist_running_now();
    };
    let schedule_persist = move || {
        persist_timer.update_value(|slot| {
            if let Some(t) = slot.take() {
                t.cancel();
            }
        });
        let timeout = gloo_timers::callback::Timeout::new(1000, move || {
            persist_running_now();
            persist_timer.set_value(None);
        });
        persist_timer.set_value(Some(timeout));
    };

    // ── Turn lifecycle ───────────────────────────────────────────────────────
    let apply_goal_to = move |tid: &str, write: GoalWrite| {
        let next = match write {
            GoalWrite::Keep => return,
            GoalWrite::Set(goal) => Some(goal),
            GoalWrite::Remove => None,
        };
        set_threads.update(|threads| {
            let Some(thread) = threads.iter_mut().find(|thread| thread.id == tid) else {
                return;
            };
            if thread.goal == next {
                return;
            }
            let original = thread.clone();
            thread.goal = next;
            catalog.persist(Some(original), thread.clone());
        });
    };
    let apply_goal = move |write: GoalWrite| {
        if let Some(tid) = active_thread_id.get_untracked() {
            apply_goal_to(&tid, write);
        }
    };
    // Start a turn in `thread_id`. `prior` is that thread's settled
    // transcript when it is not the one on screen; `None` reads the visible
    // one. Returns whether a turn started (or, with no lane free, was queued).
    let begin_turn = move |thread_id: String,
                           prior: Option<Vec<AgentMessage>>,
                           raw: String,
                           include_attachments: bool,
                           queued_chips: Option<Vec<ContextChip>>,
                           keep_composer: bool|
          -> bool {
        let here = active_thread_id.get_untracked().as_deref() == Some(thread_id.as_str());
        let pending = if let Some(chips) = &queued_chips {
            chips.clone()
        } else if include_attachments {
            context_chips.with_untracked(|chips| chips.clone())
        } else {
            Vec::new()
        };
        if exceeds_message_limit_with(&raw, extra_limit_chars(&pending)) {
            return false;
        }
        let typed = raw.trim().to_string();
        let attachment_count = if include_attachments {
            text_attachments.with_untracked(Vec::len)
        } else {
            0
        };
        let file_count = if include_attachments {
            file_offers.with_untracked(Vec::len)
        } else {
            0
        };
        if include_attachments && file_offers.with_untracked(|items| offers_block_send(items)) {
            thread_notice.flash(SEND_WAITS);
            return false;
        }
        if typed.is_empty() && attachment_count == 0 && file_count == 0 {
            return false;
        }
        if turns.is_running_untracked(&thread_id) {
            // `submit` queues instead; a stray call while streaming is a no-op.
            return false;
        }
        if here && prior.is_none() && history.blocked(&thread_id) {
            return false;
        }
        let Some(thread) =
            threads.with_untracked(|ts| ts.iter().find(|t| t.id == thread_id).cloned())
        else {
            return false;
        };
        // The composer shows the model of the thread on screen; any other
        // thread runs on the model it last used.
        let model = if here {
            current_model.get_untracked()
        } else {
            thread
                .model
                .clone()
                .filter(|model| !model.is_empty())
                .unwrap_or_else(|| default_model.get_untracked())
        };
        let provider = AgentProvider::for_model(&model);
        // A handoff turn asks for a brief; when it lands, a fresh thread in
        // the same project opens with the brief as its draft.
        let handoff = match classify_command(&typed, provider) {
            CommandKind::Handoff(next_goal) => Some(next_goal),
            _ => None,
        };
        let handoff_place = handoff
            .is_some()
            .then(|| (thread.project_name.clone(), thread.project_path.clone()));
        let prepared = prepare_command(&typed, provider, thread.goal.as_ref(), false);
        apply_goal_to(&thread_id, prepared.write);
        if let Some(notice) = prepared.notice {
            thread_notice.flash(notice);
        }
        if !prepared.send {
            return false;
        }
        // Every lane is taken: the prompt waits in this thread's queue and
        // starts when a turn ends. A turn prepared in the background already
        // holds its lane.
        if !turns.is_busy_untracked(&thread_id) && !turns.has_room_untracked() {
            prompt_queue.update(|queue| {
                queue.push_with_chips(&thread_id, &raw, &pending, js_sys::Date::now());
            });
            if here {
                thread_notice.flash(LANES_FULL);
                if !keep_composer {
                    set_input_value.set(String::new());
                    context_chips.set(Vec::new());
                }
            }
            return true;
        }
        if here {
            thread_notice.set(None);
        }
        // This thread's settled messages before the turn.
        let transcript = prior.unwrap_or_else(|| messages.get_untracked());
        let attachments = if include_attachments {
            text_attachments
                .try_update(std::mem::take)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let uploaded = if include_attachments {
            file_offers.try_update(std::mem::take).unwrap_or_default()
        } else {
            Vec::new()
        };
        let files = ready_uploads(&uploaded);
        let prompt_chips = if queued_chips.is_some() {
            pending
        } else if include_attachments {
            context_chips.try_update(std::mem::take).unwrap_or_default()
        } else {
            Vec::new()
        };
        let expanded = expand_prompt(&raw, &prompt_chips);
        let mut named = attachments.clone();
        for file in &files {
            named.push(TextAttachment {
                name: file.name.clone(),
                text: String::new(),
            });
        }
        let text = if named.is_empty() {
            expanded.trim().to_string()
        } else {
            visible_prompt(&expanded, &named)
        };
        let goal = threads.with_untracked(|ts| {
            ts.iter()
                .find(|t| t.id == thread_id)
                .and_then(|t| t.goal.clone())
        });
        let model_prompt = match &handoff {
            Some(next_goal) => handoff_prompt(next_goal),
            None => append_goal(&text, goal.as_ref()),
        };
        // An imported chat with no session yet sends its earlier
        // conversation along, until a turn gives it a session. So does a
        // thread whose session belongs to the other provider (its model
        // changed): that turn starts a new session.
        let other_provider = thread.session_id.is_some()
            && thread
                .model
                .as_deref()
                .map(AgentProvider::for_model)
                .is_some_and(|owner| owner != provider);
        let carried = if handoff.is_some() {
            None
        } else if other_provider {
            thread_context(
                &thread.title,
                &import_thread_dialog::portable_messages(&transcript),
            )
        } else if thread.session_id.is_none() {
            thread.import_context.clone()
        } else {
            None
        };
        // A resumed session the provider rejects is retried without it,
        // carrying the conversation instead.
        let carry_on_retry = handoff.is_none() && carried.is_none();
        let context_title = thread.title.clone();
        let base_prompt = model_prompt;
        let mut model_prompt = carried.as_deref().map_or_else(
            || base_prompt.clone(),
            |context| with_import_context(context, &base_prompt),
        );

        let now_ms = js_sys::Date::now();
        let assistant = AgentMessage::assistant_placeholder(model.clone(), now_ms);
        let assistant_id = assistant.id.clone();

        // The turn holds the thread's transcript plus the prompt; the reply
        // streams into the turn's own signal, `live`.
        let prompt = AgentMessage::user_with_files(text.clone(), now_ms, files.clone());
        let prompt_id = prompt.id.clone();
        let existing_worktree = thread.worktree_path.clone();
        let mut settled = transcript;
        settled.push(prompt);
        if here {
            set_messages.set(settled.clone());
        }
        let controller = web_sys::AbortController::new().ok();
        let signal = controller.as_ref().map(|c| c.signal());
        let live = turns.begin(&thread_id, settled, assistant, controller);

        // Stamp activity, remember model + branch for the sidebar, and name
        // the thread after its first prompt.
        let is_default = thread.is_default_title();
        {
            let tid = thread_id.clone();
            let instructions = title_instructions.with_untracked(|store| {
                crate::state::instructions_text(
                    &store.global,
                    &store.by_project,
                    &thread.project_name,
                )
            });
            let title = crate::state::titled(&auto_title(&text), Some(&text), &instructions);
            let model_now = model.clone();
            // Git status is the project on screen's; another thread keeps
            // the branch it last ran on.
            let branch_now = if !here {
                thread.branch.clone()
            } else if thread.project_path.is_some() {
                git.get_untracked().branch
            } else {
                None
            };
            set_threads.update(move |ts| {
                if let Some(t) = ts.iter_mut().find(|t| t.id == tid) {
                    let original = t.clone();
                    t.updated_at =
                        bruhs_models::EpochMs::from_f64_floor(now_ms).unwrap_or(t.updated_at);
                    t.model = Some(model_now);
                    t.branch = branch_now;
                    // New activity unsettles a manually settled thread and ends a snooze.
                    t.settled = false;
                    crate::state::clear_snooze(t);
                    if is_default && !title.is_empty() {
                        t.title = title;
                    }
                    catalog.persist(Some(original), t.clone());
                }
            });
        }

        if here && !keep_composer {
            set_input_value.set(String::new());
            compact_full.set(false);
        }
        if here {
            at_bottom.set(true);
        }
        flush_persist();

        // The thread's role, as it runs in this project. It sets the effort
        // and instructions, and can narrow the access mode, never widen it.
        let role = thread
            .role
            .as_deref()
            .and_then(|id| roles.with_untracked(|book| book.resolve(id, &thread.project_name)));
        let perm_mode = bruhs_models::roles::narrow_access(
            permission_mode.get_untracked(),
            role.as_ref().and_then(|role| role.access),
        );
        let role_effort = role.as_ref().and_then(|role| role.effort.clone());
        let role_instructions = role
            .map(|role| role.instructions)
            .filter(|text| !text.trim().is_empty());
        // A chat without a project sends no path; the server runs it in the
        // home directory.
        let project_path = thread.project_path.clone().or_else(|| {
            here.then(|| active_project.get_untracked().map(|p| p.path))
                .flatten()
        });
        let session_id = thread.session_id.filter(|_| !other_provider);
        let fresh_session = thread.fresh_session || other_provider;
        let choice = run_choice.get_untracked();
        let base = base_branch.get_untracked();
        let previous = previous_worktree.get_untracked();
        let git_branch = if here {
            git.get_untracked().branch
        } else {
            thread.branch.clone()
        };
        let project_label = thread.project_name.clone();
        let effort = (provider == AgentProvider::Claude)
            .then(|| role_effort.or_else(|| claude_effort_pref.get_untracked()))
            .flatten()
            .as_deref()
            .and_then(claude_effort)
            .map(str::to_string);

        wasm_bindgen_futures::spawn_local(async move {
            let finished_id = assistant_id.clone();
            let finishing = live.clone();
            let finish_message = move |error: Option<String>| {
                let now = js_sys::Date::now();
                finishing.update(|m| {
                    if let Some(m) = m.as_mut().filter(|m| m.id == finished_id) {
                        m.status = MessageStatus::Finished { at_ms: Some(now) };
                        if m.error.is_none() {
                            m.error = error;
                        }
                    }
                });
            };
            let request_thread = match ThreadId::try_from(thread_id.as_str()) {
                Ok(id) => Some(id),
                Err(error) => {
                    finish_message(Some(format!("This thread cannot be sent: {error}.")));
                    None
                }
            };
            let mut fresh = fresh_session;
            let mut resume = if fresh { None } else { session_id.clone() };
            let mut cold_retried = false;
            let mut busy_retries = 0u8;
            // Only a turn that ran to its end yields a handoff brief; a
            // stopped one has a cut-off reply.
            let mut completed = false;
            // The user stopped it: another thread waiting on this reply is
            // told so, not handed a cut-off answer.
            let mut stopped = false;
            // One per turn, so a new event kind is logged once, not per token.
            let unknown = Rc::new(RefCell::new(UnknownEvents::default()));
            let mut project_path = project_path;
            let mut blocked: Option<String> = None;
            let mut snapshot_path: Option<String> = None;
            let place_thread = |worktree: Option<String>, branch: Option<String>| {
                let tid = thread_id.clone();
                let stamped = effort.clone();
                set_threads.update(move |rows| {
                    if let Some(row) = rows.iter_mut().find(|row| row.id == tid) {
                        let original = row.clone();
                        row.run_locked = true;
                        if let Some(path) = worktree {
                            row.worktree_path = Some(path.clone());
                            row.project_path = Some(path);
                        }
                        if let Some(branch) = branch {
                            row.branch = Some(branch);
                        }
                        row.effort = stamped;
                        catalog.persist(Some(original), row.clone());
                    }
                });
            };
            if request_thread.is_some() {
                match first_send(
                    thread.run_locked,
                    choice,
                    project_path.as_deref(),
                    &base,
                    git_branch.as_deref(),
                    previous.as_ref(),
                    &thread_id,
                ) {
                    Err(message) => blocked = Some(message.to_string()),
                    Ok(FirstSend::Stay) => {
                        place_thread(None, None);
                        snapshot_path = existing_worktree.clone();
                    }
                    Ok(FirstSend::UsePath { path, branch }) => {
                        snapshot_path = Some(path.clone());
                        project_path = Some(path.clone());
                        place_thread(Some(path), Some(branch));
                    }
                    Ok(FirstSend::Create { base, branch }) => match project_path.clone() {
                        Some(repo) => match create_worktree(&repo, &base, &branch).await {
                            Ok(created) => {
                                snapshot_path = Some(created.path.clone());
                                project_path = Some(created.path.clone());
                                previous_worktree.set(Some(RememberedWorktree {
                                    path: created.path.clone(),
                                    branch: created.branch.clone(),
                                    label: project_label.clone(),
                                }));
                                place_thread(Some(created.path), Some(created.branch));
                            }
                            Err(error) => blocked = Some(error.to_string()),
                        },
                        None => {
                            blocked = Some("Choose a project before creating a worktree.".into());
                        }
                    },
                }
            }
            if blocked.is_none() {
                if let Some(dir) = snapshot_path.filter(|path| !path.trim().is_empty()) {
                    if let Ok(tree) = snapshot_worktree(&dir).await {
                        let stamped = tree.clone();
                        set_messages.update(|ms| {
                            if let Some(message) =
                                ms.iter_mut().find(|message| message.id == prompt_id)
                            {
                                message.checkpoint = Some(stamped);
                            }
                        });
                        turns.update_messages(&thread_id, |messages| {
                            if let Some(message) =
                                messages.iter_mut().find(|message| message.id == prompt_id)
                            {
                                message.checkpoint = Some(tree);
                            }
                        });
                        schedule_persist();
                    }
                }
            }
            if let Some(message) = blocked {
                finish_message(Some(message));
            } else {
                while let Some(request_thread) = request_thread.as_ref() {
                    let resumed = resume.is_some() && !fresh;
                    let request = AgentRequest {
                        thread_id: request_thread.clone(),
                        prompt: model_prompt.clone(),
                        project_path: project_path.clone(),
                        model: Some(model.clone()),
                        permission_mode: Some(perm_mode),
                        session_id: resume.clone(),
                        fresh_session: fresh,
                        text_attachments: attachments.clone(),
                        file_attachments: files.clone(),
                        effort: effort.clone(),
                        autocompact: if provider == AgentProvider::Claude {
                            autocompact_flag(claude_autocompact.get_untracked().as_deref())
                        } else {
                            None
                        },
                        thread_access: Some(thread_access.get_untracked())
                            .filter(|access| *access != Default::default()),
                        instructions: role_instructions.clone(),
                    };
                    let saw_init = Rc::new(Cell::new(false));
                    let saw_init_cb = Rc::clone(&saw_init);
                    let aid = assistant_id.clone();
                    let tid = thread_id.clone();
                    let unknown_cb = Rc::clone(&unknown);
                    let streaming = live.clone();

                    let result =
                        send_agent_message_streaming(&request, signal.as_ref(), move |message| {
                            let now = js_sys::Date::now();
                            let mut effect = StreamEffect::None;
                            // Patched in place: no copy of the transcript per token.
                            streaming.update(|m| {
                                if let Some(m) = m.as_mut().filter(|m| m.id == aid) {
                                    effect = apply_message(
                                        m,
                                        &message,
                                        now,
                                        &mut unknown_cb.borrow_mut(),
                                    );
                                }
                            });
                            match effect {
                                StreamEffect::SecretRequest(request) => {
                                    if request.thread_id.as_str() == tid {
                                        pending_secrets.update(|items| {
                                            items.retain(|item| item.id != request.id);
                                            items.push(request);
                                        });
                                    }
                                }
                                StreamEffect::SecretResolved(id) => pending_secrets
                                    .update(|items| items.retain(|item| item.id != id)),
                                StreamEffect::ThreadRequest(request) => {
                                    if request.thread_id.as_str() == tid {
                                        thread_link.answer(request);
                                    }
                                }
                                StreamEffect::SessionId(sess) => {
                                    saw_init_cb.set(true);
                                    let tid2 = tid.clone();
                                    set_threads.update(move |ts| {
                                        if let Some(t) = ts.iter_mut().find(|t| t.id == tid2) {
                                            if t.session_id.as_deref() != Some(sess.as_str())
                                                || t.fresh_session
                                                || t.import_context.is_some()
                                            {
                                                let original = t.clone();
                                                t.session_id = Some(sess);
                                                t.fresh_session = false;
                                                // The session now holds the
                                                // imported conversation.
                                                t.import_context = None;
                                                catalog.persist(Some(original), t.clone());
                                            }
                                        }
                                    });
                                }
                                StreamEffect::PermissionRequest(mut req) => {
                                    // So only this thread asks, and only its
                                    // own requests go when its turn ends.
                                    req.thread_id.clone_from(&tid);
                                    pending_permissions.update(|p| p.insert(req));
                                }
                                StreamEffect::PermissionCancelled(id) => {
                                    pending_permissions.update(|p| {
                                        p.remove(&id);
                                    });
                                }
                                StreamEffect::Finished { .. } | StreamEffect::None => {}
                            }
                            schedule_persist();
                        })
                        .await;

                    let message_errored = live.with_untracked(|m| {
                        m.as_ref()
                            .is_some_and(|m| m.id == assistant_id && m.error.is_some())
                    });
                    match next_step(Attempt {
                        result: &result,
                        message_errored,
                        saw_init: saw_init.get(),
                        resumed,
                        cold_retried,
                        busy_retries,
                    }) {
                        NextStep::RetryCold => {
                            // The provider rejected the session before it even
                            // started: drop the stale id and retry once from scratch.
                            cold_retried = true;
                            fresh = true;
                            resume = None;
                            if carry_on_retry {
                                let context = turns.messages(&thread_id).and_then(|settled| {
                                    // All but the prompt this turn sends.
                                    let earlier = &settled[..settled.len().saturating_sub(1)];
                                    thread_context(
                                        &context_title,
                                        &import_thread_dialog::portable_messages(earlier),
                                    )
                                });
                                if let Some(context) = context {
                                    model_prompt = with_import_context(&context, &base_prompt);
                                }
                            }
                            live.update(|m| {
                                if let Some(m) = m.as_mut().filter(|m| m.id == assistant_id) {
                                    m.error = None;
                                }
                            });
                            let tid2 = thread_id.clone();
                            set_threads.update(move |ts| {
                                if let Some(t) = ts.iter_mut().find(|t| t.id == tid2) {
                                    let original = t.clone();
                                    t.session_id = None;
                                    t.fresh_session = true;
                                    catalog.persist(Some(original), t.clone());
                                }
                            });
                        }
                        NextStep::RetryBusy { delay_ms } => {
                            // The session is fine; the server is still closing
                            // the previous turn of this thread.
                            busy_retries += 1;
                            gloo_timers::future::TimeoutFuture::new(delay_ms).await;
                        }
                        NextStep::GiveUpBusy => {
                            thread_notice.set(Some(STILL_BUSY.to_string()));
                            finish_message(Some(STILL_BUSY.to_string()));
                            break;
                        }
                        NextStep::Finish => {
                            completed = result.is_ok();
                            stopped = matches!(result, Err(StreamError::Stopped));
                            finish_message(match result {
                                Err(StreamError::Stopped) | Ok(()) => None,
                                Err(error) => Some(error.to_string()),
                            });
                            break;
                        }
                    }
                }
            }

            // The reply joins the transcript, and the turn is over.
            let settled = turns.end(&thread_id).unwrap_or_default();
            let brief = settled
                .last()
                .filter(|reply| {
                    reply.id == assistant_id
                        && handoff_place.is_some()
                        && completed
                        && reply.error.is_none()
                })
                .map(AgentMessage::text)
                .filter(|text| !text.trim().is_empty());
            pending_permissions.update(|p| p.clear_thread(&thread_id));
            pending_secrets
                .update(|items| items.retain(|item| item.thread_id.as_str() != thread_id));
            if active_thread_id.get_untracked().as_deref() == Some(thread_id.as_str()) {
                set_messages.set(settled.clone());
            } else {
                // Finished while the user was elsewhere → mark as ready/unread.
                unread_threads.update(|set| {
                    set.insert(thread_id.clone());
                });
            }
            // Another thread's agent may be waiting on this reply.
            thread_link.turn_ended(&thread_id, &settled, stopped);
            saves.edit_conversation(thread_id.clone(), settled);
            flush_persist();

            if let (Some((project_name, project_path)), Some(brief)) = (handoff_place, brief) {
                // Never pull the user away from another thread or over a
                // draft they started; the brief stays as the last reply.
                let here = active_thread_id.get_untracked().as_deref() == Some(thread_id.as_str());
                if here && input_value.get_untracked().trim().is_empty() {
                    crate::state::create_thread(
                        catalog,
                        set_threads,
                        set_active_thread_id,
                        set_messages,
                        project_name,
                        project_path,
                    );
                    set_input_value.set(brief);
                    thread_notice.flash("Handoff ready: review the brief, then send.");
                } else {
                    thread_notice.flash(
                        "The handoff brief is the last reply in its thread; copy it into a new thread.",
                    );
                }
            }
        });
        true
    };
    let start_turn = move |raw: String,
                           include_attachments: bool,
                           queued_chips: Option<Vec<ContextChip>>,
                           keep_composer: bool| {
        if let Some(thread_id) = active_thread_id.get_untracked() {
            begin_turn(
                thread_id,
                None,
                raw,
                include_attachments,
                queued_chips,
                keep_composer,
            );
        }
    };

    // `/second-opinion`: the other provider reviews the last answer
    // read-only; its review becomes the draft of the next message.
    let run_second_opinion = move |focus: String| {
        let Some(thread_id) = active_thread_id.get_untracked() else {
            return;
        };
        let provider = AgentProvider::for_model(&current_model.get_untracked());
        let (task, answer) = messages.with_untracked(|rows| last_exchange(rows));
        let Some(answer) = answer else {
            thread_notice.flash(SECOND_OPINION_NOTHING);
            return;
        };
        let project_path = threads.with_untracked(|rows| {
            rows.iter()
                .find(|row| row.id == thread_id)
                .and_then(|row| row.project_path.clone())
        });
        let reviewer = reviewer_for(provider);
        thread_notice.set(Some(format!(
            "Asking {} for a second opinion…",
            provider_name(reviewer)
        )));
        let request = bruhs_models::api::SecondOpinionRequest {
            project_path,
            thread_provider: provider,
            task,
            answer,
            focus,
            // The user's model for this reviewer, if they chose one.
            reviewer_model: task_models
                .with_untracked(|models| models.review(reviewer).map(str::to_string))
                .and_then(|model| bruhs_models::agent::ModelId::try_from(model).ok()),
        };
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::second_opinion(&request).await {
                Ok(reply) if reply.error.is_none() && review_holds_up(&reply.review) => {
                    // Nothing to fix: a follow-up turn would cost tokens for nothing.
                    thread_notice.flash(second_opinion_agrees(reply.reviewer));
                }
                Ok(reply) if reply.error.is_none() && !reply.review.trim().is_empty() => {
                    let here =
                        active_thread_id.get_untracked().as_deref() == Some(thread_id.as_str());
                    if here && input_value.get_untracked().trim().is_empty() {
                        thread_notice.set(None);
                        set_input_value.set(second_opinion_followup(reply.reviewer, &reply.review));
                    } else {
                        thread_notice.flash(
                            "The second opinion came back while the composer was in use; run it again.",
                        );
                    }
                }
                Ok(reply) => thread_notice.flash(
                    reply
                        .error
                        .unwrap_or_else(|| "The reviewer returned no text.".to_string()),
                ),
                Err(error) => thread_notice.flash(error.to_string()),
            }
        });
    };

    let thread_tokens_and_idle = move |thread_id: &str| -> Option<(u64, f64)> {
        let tokens = messages.with_untracked(|rows| latest_context_tokens(rows))?;
        let updated = threads.with_untracked(|rows| {
            rows.iter()
                .find(|thread| thread.id == thread_id)
                .map(|thread| thread.updated_at)
        })?;
        Some((tokens, (js_sys::Date::now() - updated.as_f64()).max(0.0)))
    };
    // Writes the draft into the Claude turn that is already streaming here.
    let steer_draft = move |raw: String| {
        if file_offers.with_untracked(|items| offers_block_send(items)) {
            thread_notice.flash(SEND_WAITS);
            return;
        }
        let Some(thread_id) = active_thread_id.get_untracked() else {
            return;
        };
        if history.blocked(&thread_id) {
            return;
        }
        let chips = context_chips.try_update(std::mem::take).unwrap_or_default();
        let attachments = text_attachments
            .try_update(std::mem::take)
            .unwrap_or_default();
        let offers = file_offers.try_update(std::mem::take).unwrap_or_default();
        let files = ready_uploads(&offers);
        let expanded = expand_prompt(&raw, &chips);
        let mut named = attachments.clone();
        for file in &files {
            named.push(TextAttachment {
                name: file.name.clone(),
                text: String::new(),
            });
        }
        let visible = if named.is_empty() {
            expanded.trim().to_string()
        } else {
            visible_prompt(&expanded, &named)
        };
        if visible.trim().is_empty() {
            text_attachments.set(attachments);
            file_offers.set(offers);
            chip_guard.set_value(true);
            context_chips.set(chips);
            return;
        }
        let goal = threads.with_untracked(|rows| {
            rows.iter()
                .find(|thread| thread.id == thread_id)
                .and_then(|thread| thread.goal.clone())
        });
        let mut prompt = append_goal(&visible, goal.as_ref());
        for attachment in &attachments {
            if attachment.text.is_empty() {
                continue;
            }
            prompt.push_str("\n\n");
            prompt.push_str(&attachment.name);
            prompt.push_str(":\n");
            prompt.push_str(&attachment.text);
        }
        prompt = append_uploaded_files(&prompt, &files);
        let now_ms = js_sys::Date::now();
        let message = AgentMessage::user(visible, now_ms);
        let message_id = message.id.clone();
        set_messages.update(|rows| rows.push(message.clone()));
        turns.update_messages(&thread_id, |messages| messages.push(message));
        chip_guard.set_value(true);
        set_input_value.set(String::new());
        schedule_persist();
        let draft = raw;
        wasm_bindgen_futures::spawn_local(async move {
            let result = steer_turn(&thread_id, &prompt).await;
            if let Err(error) = result {
                thread_notice.flash(error.to_string());
                set_messages.update(|rows| rows.retain(|message| message.id != message_id));
                turns.update_messages(&thread_id, |messages| {
                    messages.retain(|message| message.id != message_id);
                });
                if input_value.get_untracked().is_empty() {
                    set_input_value.set(draft);
                    text_attachments.set(attachments);
                    file_offers.set(offers);
                    chip_guard.set_value(true);
                    context_chips.set(chips);
                }
                schedule_persist();
            }
        });
    };
    // Enter while a turn runs queues the prompt. On Claude it can steer
    // that turn instead. Cmd+Enter asks for the other one. An empty
    // composer interrupts a running turn, or resumes a held queue.
    let submit = move |raw: String, opposite: bool| {
        if !running_here() && file_offers.with_untracked(|items| offers_block_send(items)) {
            thread_notice.flash(SEND_WAITS);
            return;
        }
        let chip_extra = context_chips.with_untracked(|chips| extra_limit_chars(chips));
        if exceeds_message_limit_with(&raw, chip_extra) {
            return;
        }
        let text = raw.trim().to_string();
        // Before the queue and steer paths: a review is never a turn.
        if let CommandKind::SecondOpinion(focus) = classify_command(
            &text,
            AgentProvider::for_model(&current_model.get_untracked()),
        ) {
            if running_here() {
                thread_notice.flash(SECOND_OPINION_WAIT);
                return;
            }
            chip_guard.set_value(true);
            context_chips.set(Vec::new());
            set_input_value.set(String::new());
            run_second_opinion(focus);
            return;
        }
        let has_attachments = !text_attachments.with_untracked(|items| items.is_empty())
            || file_offers
                .with_untracked(|items| items.iter().any(|item| item.ready_path().is_some()));
        if text.is_empty() {
            if running_here() {
                queue_held.set(true);
                stop_here();
                return;
            }
            if has_attachments {
                queue_held.set(false);
                start_turn(raw, true, None, false);
                return;
            }
            if queue_held.get_untracked() {
                queue_held.set(false);
            }
            return;
        }
        if active_thread_id.with_untracked(|id| id.as_deref().is_some_and(|id| history.blocked(id)))
        {
            return;
        }
        let provider = AgentProvider::for_model(&current_model.get_untracked());
        let streaming_here = running_here();
        if streaming_here {
            let follow = FollowUp {
                wants_steer: follow_up_steer.get_untracked(),
                can_steer: provider_can_steer(provider),
            };
            match composer_action(true, false, opposite, false, false, follow) {
                ComposerAction::Steer => {
                    steer_draft(raw);
                    return;
                }
                ComposerAction::SteerUnavailable => {
                    thread_notice.flash(STEER_UNAVAILABLE);
                    return;
                }
                _ => {}
            }
        }
        let goal = active_thread_id.get_untracked().and_then(|id| {
            threads.with_untracked(|items| {
                items
                    .iter()
                    .find(|thread| thread.id == id)
                    .and_then(|thread| thread.goal.clone())
            })
        });
        let prepared = prepare_command(&text, provider, goal.as_ref(), running_here());
        if !prepared.send {
            apply_goal(prepared.write);
            if let Some(notice) = prepared.notice {
                thread_notice.flash(notice);
            }
            if prepared.stop {
                queue_held.set(true);
                stop_here();
            }
            if prepared.queue {
                if let Some(tid) = active_thread_id.get_untracked() {
                    let chips = context_chips.get_untracked();
                    prompt_queue.update(|q| {
                        q.push_with_chips(&tid, &raw, &chips, js_sys::Date::now());
                    });
                }
            }
            chip_guard.set_value(true);
            context_chips.set(Vec::new());
            set_input_value.set(String::new());
            return;
        }
        // A new send resumes the queue after this turn, not before it.
        let summarize = compact_idle.get_untracked() == IdleCompact::Return
            && !compact_full.get_untracked()
            && provider == AgentProvider::Claude
            // A handoff replaces compaction; compacting first would pay twice.
            && !matches!(
                classify_command(&text, provider),
                CommandKind::Compact | CommandKind::Handoff(_)
            )
            && text_attachments.with_untracked(|items| items.is_empty())
            && file_offers.with_untracked(|items| items.is_empty())
            && active_thread_id.get_untracked().is_some_and(|thread_id| {
                thread_tokens_and_idle(&thread_id).is_some_and(|(tokens, idle)| {
                    show_return_chip(IdleCompact::Return, tokens, idle, CACHE_OAUTH_MS)
                })
            });
        if summarize {
            queue_held.set(true);
            if let Some(tid) = active_thread_id.get_untracked() {
                let chips = context_chips.get_untracked();
                prompt_queue.update(|q| {
                    q.push_with_chips(&tid, &raw, &chips, js_sys::Date::now());
                });
            }
            chip_guard.set_value(true);
            context_chips.set(Vec::new());
            set_input_value.set(String::new());
            compact_full.set(false);
            start_turn("/compact".to_string(), false, None, true);
            queue_held.set(false);
            return;
        }
        queue_held.set(false);
        start_turn(raw, true, None, false);
    };
    let on_slash = Callback::new(move |name: &'static str| {
        let provider = AgentProvider::for_model(&current_model.get_untracked());
        if let CommandKind::SecondOpinion(focus) = classify_command(name, provider) {
            if running_here() {
                thread_notice.flash(SECOND_OPINION_WAIT);
            } else {
                run_second_opinion(focus);
            }
            return;
        }
        let goal = active_thread_id.get_untracked().and_then(|id| {
            threads.with_untracked(|items| {
                items
                    .iter()
                    .find(|thread| thread.id == id)
                    .and_then(|thread| thread.goal.clone())
            })
        });
        let prepared = prepare_command(name, provider, goal.as_ref(), running_here());
        apply_goal(prepared.write);
        if let Some(notice) = prepared.notice {
            thread_notice.flash(notice);
        }
        if prepared.stop {
            queue_held.set(true);
            stop_here();
        }
        if prepared.queue {
            if let Some(tid) = active_thread_id.get_untracked() {
                prompt_queue.update(|q| {
                    q.push(&tid, name, js_sys::Date::now());
                });
            }
        }
        if prepared.send {
            queue_held.set(false);
            start_turn(name.to_string(), false, None, true);
        }
    });
    // Drain when the thread is idle and the queue is not held. Interrupt and
    // a reload hold the rows until Resume or the next send.
    Effect::new(move |_| {
        // A turn ending anywhere frees a lane.
        turns.track();
        if live_here.get() || queue_held.get() {
            return;
        }
        let Some(tid) = active_thread_id.get() else {
            return;
        };
        if history.reason(Some(&tid)).is_some() {
            return;
        }
        // Being prepared in the background, or no lane is free.
        if turns.is_busy_untracked(&tid) || !turns.has_room_untracked() {
            return;
        }
        let has_queued = prompt_queue.with(|q| !q.for_thread(&tid).is_empty());
        if !has_queued {
            return;
        }
        let next = prompt_queue.try_update(|q| q.next_for(&tid)).flatten();
        if let Some(p) = next {
            start_turn(p.text, false, Some(p.chips), false);
        }
    });
    // Threads that are not on screen drain too: a prompt queued there (by the
    // user earlier, or by another thread's agent) starts as soon as its
    // thread is idle and a lane is free. Its saved messages load first.
    Effect::new(move |_| {
        turns.track();
        if queue_held.get() {
            return;
        }
        let active = active_thread_id.get();
        let waiting: Vec<String> = prompt_queue.with(|queue| {
            queue
                .thread_ids()
                .into_iter()
                .filter(|id| active.as_deref() != Some(*id))
                .map(str::to_string)
                .collect()
        });
        for thread_id in waiting {
            if !threads.with_untracked(|rows| rows.iter().any(|row| row.id == thread_id)) {
                continue;
            }
            if !turns.has_room_untracked() {
                break;
            }
            // False when it is running, or already being prepared.
            if !turns.reserve(&thread_id) {
                continue;
            }
            wasm_bindgen_futures::spawn_local(async move {
                let prior = match crate::storage::load_messages(&thread_id).await {
                    Ok(loaded) => saves.loaded_conversation(&thread_id, loaded),
                    Err(error) => {
                        // Hold the queue: retrying would fail the same way.
                        log::warn!("a queued prompt could not start: {error:?}");
                        queue_held.set(true);
                        thread_notice.flash(
                            "A queued prompt could not start: its thread's saved messages did not load.",
                        );
                        turns.release(&thread_id);
                        return;
                    }
                };
                // Held, removed, or sent from the thread itself meanwhile.
                let next = (!queue_held.get_untracked())
                    .then(|| {
                        prompt_queue
                            .try_update(|q| q.next_for(&thread_id))
                            .flatten()
                    })
                    .flatten();
                let started = next.is_some_and(|next| {
                    begin_turn(
                        thread_id.clone(),
                        Some(prior),
                        next.text,
                        false,
                        Some(next.chips),
                        true,
                    )
                });
                if !started {
                    turns.release(&thread_id);
                }
            });
        }
    });
    // A message between threads can start right away only while this page
    // is mounted: it owns the turn lifecycle.
    thread_link.set_chat_mounted(true);
    on_cleanup(move || thread_link.set_chat_mounted(false));
    let compact_idle_if_due = move || {
        if running_here() || queue_held.get_untracked() {
            return;
        }
        if compact_idle.get_untracked() != IdleCompact::Before {
            return;
        }
        if AgentProvider::for_model(&current_model.get_untracked()) != AgentProvider::Claude {
            return;
        }
        let Some(thread_id) = active_thread_id.get_untracked() else {
            return;
        };
        if history.blocked(&thread_id)
            || prompt_queue.with_untracked(|queue| queue.has_for(&thread_id))
        {
            return;
        }
        let Some((tokens, idle)) = thread_tokens_and_idle(&thread_id) else {
            return;
        };
        if before_expiry_due(IdleCompact::Before, tokens, idle, CACHE_OAUTH_MS) {
            start_turn("/compact".to_string(), false, None, true);
        }
    };
    let idle_clock = StoredValue::new_local(None::<Interval>);
    idle_clock.set_value(Some(Interval::new(30_000, compact_idle_if_due)));
    on_cleanup(move || {
        idle_clock.set_value(None);
    });

    let on_submit_cb = Callback::new(move |_: ()| submit(input_value.get_untracked(), false));
    let on_opposite_cb = Callback::new(move |_: ()| submit(input_value.get_untracked(), true));

    let on_stop_cb = Callback::new(move |_: ()| {
        queue_held.set(true);
        if let Some(thread) = active_thread_id.get_untracked() {
            pending_secrets.update(|items| items.retain(|item| item.thread_id.as_str() != thread));
        }
        stop_here();
    });

    // Edit from here: drop the selected prompt and everything after it.
    // The unsent draft stays above the restored prompt. The next turn
    // starts a new provider session. A worktree thread can also put files
    // back to the checkpoint from before that message.
    let reverting_files = StoredValue::new(false);
    let rewind_to = move |message_id: String| {
        if running_here() {
            return;
        }
        let Some(tid) = active_thread_id.get_untracked() else {
            return;
        };
        if history.blocked(&tid) {
            return;
        }
        let msgs = messages.get_untracked();
        let facts: Vec<(bool, &str)> = msgs
            .iter()
            .map(|message| (message.role == Role::User, message.id.as_str()))
            .collect();
        let Some(end) = rewind_keep_end(&facts, &message_id) else {
            return;
        };
        let prompt = msgs[end].text();
        diff_scope.update(|scope| {
            if let Some(DiffScope::Turn { user_message_id }) = scope {
                if !msgs[..end]
                    .iter()
                    .any(|message| &message.id == user_message_id)
                {
                    *scope = None;
                }
            }
        });
        let draft = draft_source.get_value().as_ref().map_or_else(
            || input_value.get_untracked(),
            |read_draft| read_draft.run(()),
        );
        let next = compose_after_rewind(&draft, &prompt);
        let kept = msgs[..end].to_vec();
        set_messages.set(kept.clone());
        saves.edit_conversation(tid.clone(), kept);
        set_input_value.set(next);
        recall.set(None);
        set_threads.update(|ts| {
            if let Some(thread) = ts.iter_mut().find(|thread| thread.id == tid) {
                let original = thread.clone();
                thread.session_id = None;
                thread.fresh_session = true;
                catalog.persist(Some(original), thread.clone());
            }
        });
        // The rewind click removes its own row, and the composer updates on
        // the following render. Focus after that paint or the caret is lost.
        request_animation_frame(move || {
            if let Some(input) = document()
                .get_element_by_id("chat-input")
                .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = input.focus();
            }
        });
    };
    let on_edit_from_here = Callback::new(move |(message_id, revert_files): (String, bool)| {
        if running_here() {
            return;
        }
        let Some(tid) = active_thread_id.get_untracked() else {
            return;
        };
        if history.blocked(&tid) {
            return;
        }
        if !revert_files {
            rewind_to(message_id);
            return;
        }
        if reverting_files.get_value() {
            return;
        }
        let worktree = threads.with_untracked(|rows| {
            rows.iter()
                .find(|row| row.id == tid)
                .and_then(|row| row.worktree_path.clone())
        });
        let Some(worktree) = worktree.filter(|path| !path.trim().is_empty()) else {
            rewind_to(message_id);
            return;
        };
        let checkpoint = messages.with_untracked(|rows| {
            rows.iter()
                .find(|row| row.id == message_id)
                .and_then(|row| row.checkpoint.clone())
        });
        let others = threads.with_untracked(|rows| {
            rows.iter()
                .filter(|row| row.id != tid)
                .filter_map(|row| row.worktree_path.clone())
                .collect::<Vec<_>>()
        });
        if let Some(reason) = file_revert_block(
            &worktree,
            checkpoint.as_deref(),
            others.iter().map(String::as_str),
        ) {
            thread_notice.flash(reason);
            return;
        }
        let Some(tree) = checkpoint else {
            return;
        };
        reverting_files.set_value(true);
        wasm_bindgen_futures::spawn_local(async move {
            let restored = restore_worktree(&worktree, &tree).await;
            reverting_files.set_value(false);
            match restored {
                Ok(()) => rewind_to(message_id),
                Err(error) => thread_notice.flash(error.to_string()),
            }
        });
    });
    let can_rewind = Signal::derive(move || !live_here.get());
    let in_worktree = Signal::derive(move || {
        active_thread().is_some_and(|thread| {
            thread
                .worktree_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
        })
    });

    // Retry a failed turn: drop the failed assistant message and the prompt
    // that produced it, then resend the prompt so the pair is re-added.
    let on_retry = Callback::new(move |(message_id, prompt): (String, String)| {
        if running_here() {
            return;
        }
        if active_thread_id.with_untracked(|id| id.as_deref().is_some_and(|id| history.blocked(id)))
        {
            return;
        }
        let mut msgs = messages.get_untracked();
        if let Some(pos) = msgs.iter().position(|m| m.id == message_id) {
            msgs.remove(pos);
            if pos > 0
                && msgs[pos - 1].role == Role::User
                && msgs[pos - 1].text().trim() == prompt.trim()
            {
                msgs.remove(pos - 1);
            }
        }
        set_messages.set(msgs.clone());
        if let Some(tid) = active_thread_id.get_untracked() {
            saves.edit_conversation(tid, msgs);
        }
        start_turn(prompt, false, None, false);
    });

    // "Open diff" on a changed-files card: scope the diff panel to that turn.
    let on_open_diff = Callback::new(move |(user_message_id, path): (String, Option<String>)| {
        diff_focus.set(path);
        diff_scope.set(Some(DiffScope::Turn { user_message_id }));
        panel.update(|p| p.show(PanelSurface::Diff));
    });

    // Composer inputs derived from the active thread.
    let composer_project_path = project_root;
    let composer_project_name = Signal::derive(move || {
        active_thread()
            .map(|t| t.project_name)
            .filter(|p| !p.is_empty())
    });
    let awaiting_approval = Signal::derive(move || {
        let Some(thread) = active_thread_id.get() else {
            return false;
        };
        pending_permissions.with(|p| p.has_for(&thread))
            || pending_secrets
                .with(|items| items.iter().any(|item| item.thread_id.as_str() == thread))
    });
    // `(name, path)` the terminal drawer offers first.
    let composer_context_project =
        Signal::derive(
            move || match (composer_project_name.get(), composer_project_path.get()) {
                (Some(name), Some(path)) => Some((name, path)),
                _ => None,
            },
        );
    let last_usage = Memo::new(move |_| {
        let usage_of = |m: &AgentMessage| {
            m.usage
                .clone()
                .map(|u| (u, m.model.clone().unwrap_or_default()))
        };
        let running = live_reply().and_then(|live| live.with(|m| m.as_ref().and_then(usage_of)));
        running.or_else(|| {
            messages.with(|ms| {
                ms.iter()
                    .rev()
                    .filter(|m| m.role == Role::Assistant)
                    .find_map(usage_of)
            })
        })
    });

    // ── View ─────────────────────────────────────────────────────────────────
    view! {
        {move || {
            match active_thread() {
                None => view! {
                    <div class="flex flex-1 flex-col items-center justify-center h-full text-body-sm text-rambutan-500">
                        <p>"Pick a conversation, or start a new one."</p>
                    </div>
                }.into_any(),

                Some(thread) => view! {
                    <div class="flex flex-col h-full">
                        <ChatHeader thread_id=thread.id />
                        {move || find_cmds.thread_open.get().then(|| view! {
                            <FindBar
                                label="Find in thread"
                                query=find_query
                                index=find_index
                                count=find_count
                                open=find_cmds.thread_open
                                focus_gen=find_focus
                            />
                        })}

                        <AuthBanner />
                        <crate::thread_history::ThreadHistoryStatus />

                        {move || {
                            let empty = messages.with(|m| m.is_empty()) && !live_here.get();
                            if empty {
                                // Empty thread: the draft hero — headline +
                                // composer centered in the column.
                                view! {
                                    <div class="flex flex-1 flex-col items-center justify-center gap-3 overflow-y-auto px-3 py-6 sm:px-5">
                                        <Composer
                                            value=input_value
                                            set_value=set_input_value
                                            on_submit=on_submit_cb
                                            on_opposite=on_opposite_cb
                                            on_stop=on_stop_cb
                                            on_slash=on_slash
                                            project_path=composer_project_path
                                            project_name=composer_project_name
                                            last_usage=last_usage
                                            hero=true
                                            recall=recall
                                        />
                                        {move || thread_notice.get().map(|n| view! {
                                            <p class="text-label-sm text-durian-300">{n}</p>
                                        })}
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div class="relative flex-1 overflow-hidden">
                                        <div
                                            node_ref=messages_container
                                            on:scroll=move |_| {
                                                if let Some(Some(el)) = messages_container.try_get_untracked() {
                                                    let html_el: web_sys::HtmlElement = (*el).clone();
                                                    let top = html_el.scroll_top();
                                                    let bottom = top + html_el.client_height()
                                                        >= html_el.scroll_height() - FOLLOW_THRESHOLD_PX;
                                                    let moved_up = top < last_scroll_top.get_value();
                                                    last_scroll_top.set_value(top);
                                                    let following = at_bottom.get_untracked();
                                                    let (follow, repin) =
                                                        follow_after_scroll(following, bottom, moved_up);
                                                    if follow != following {
                                                        at_bottom.set(follow);
                                                    }
                                                    if repin {
                                                        scroll_to_bottom();
                                                    }
                                                }
                                            }
                                            class="h-full overflow-y-auto"
                                        >
                                            <Timeline
                                                settled=settled_turns
                                                live=live_turn
                                                is_streaming=live_here
                                                awaiting_approval=awaiting_approval
                                                on_retry=on_retry
                                                on_open_diff=on_open_diff
                                                can_rewind=can_rewind
                                                in_worktree=in_worktree
                                                on_edit_from_here=on_edit_from_here
                                            />
                                        </div>
                                        {move || (!at_bottom.get()).then(|| view! {
                                            <div class="pointer-events-none absolute inset-x-0 bottom-3 z-30 flex justify-center">
                                                <button
                                                    type="button"
                                                    on:click=move |_| {
                                                        at_bottom.set(true);
                                                        scroll_to_bottom();
                                                    }
                                                    class="pointer-events-auto inline-flex h-7 items-center gap-1.5 rounded-orb bg-longan-700 px-3 text-label-sm text-rambutan-200 inset-ring inset-ring-white/15 transition-colors hover:text-rambutan-100"
                                                >
                                                    <Icon name="chevron-down" size=12 />
                                                    "Scroll to end"
                                                </button>
                                            </div>
                                        })}
                                    </div>

                                    {move || thread_notice.get().map(|n| view! {
                                        <p class="mx-auto w-full max-w-3xl px-3 pb-1 text-label-sm text-durian-300 sm:px-5">{n}</p>
                                    })}

                                    <div class="shrink-0 px-3 pb-4 pt-2 sm:px-5">
                                        <crate::components::secret_prompt::SecretPrompts />
                                        <Composer
                                            value=input_value
                                            set_value=set_input_value
                                            on_submit=on_submit_cb
                                            on_opposite=on_opposite_cb
                                            on_stop=on_stop_cb
                                            on_slash=on_slash
                                            project_path=composer_project_path
                                            project_name=composer_project_name
                                            last_usage=last_usage
                                            recall=recall
                                        />
                                    </div>
                                }.into_any()
                            }
                        }}
                        // Terminal drawer (⌘J) — bottom of the thread column;
                        // hidden rather than unmounted so the shell lives on.
                        <TerminalPanel mode=TerminalPanelMode::Drawer context_project=composer_context_project />
                    </div>
                }.into_any(),
            }
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(result: &Result<(), StreamError>) -> Attempt<'_> {
        Attempt {
            result,
            message_errored: false,
            saw_init: false,
            resumed: true,
            cold_retried: false,
            busy_retries: 0,
        }
    }

    /// A 409 means the thread is still finishing a turn, not that its
    /// session is stale: the session is kept and the send waits and retries,
    /// then gives up with a notice.
    #[test]
    fn a_busy_thread_is_retried_with_its_session_then_reported() {
        let busy = Err(StreamError::Http {
            status: 409,
            detail: Some("thread busy".into()),
        });
        let mut delays = Vec::new();
        for tries in 0..BUSY_RETRIES {
            match next_step(Attempt {
                busy_retries: tries,
                ..attempt(&busy)
            }) {
                NextStep::RetryBusy { delay_ms } => delays.push(delay_ms),
                other => panic!("try {tries}: {other:?}"),
            }
        }
        assert_eq!(delays, vec![1_500, 3_000, 4_500]);
        assert_eq!(
            next_step(Attempt {
                busy_retries: BUSY_RETRIES,
                ..attempt(&busy)
            }),
            NextStep::GiveUpBusy
        );
    }

    #[test]
    fn second_opinion_reviews_the_last_answer_with_text_and_its_question() {
        use crate::state::MessagePart;
        let answer = |text: &str| {
            let mut message = AgentMessage::assistant_placeholder("m".into(), 1.0);
            message.parts.push(MessagePart::Text(text.into()));
            message
        };
        assert_eq!(last_exchange(&[]), (String::new(), None));
        let rows = vec![
            AgentMessage::user("first".into(), 1.0),
            answer("old answer"),
            AgentMessage::user("is add right?".into(), 2.0),
            answer("yes"),
            AgentMessage::assistant_placeholder("m".into(), 3.0),
        ];
        assert_eq!(
            last_exchange(&rows),
            ("is add right?".to_string(), Some("yes".to_string())),
            "an empty trailing reply is skipped"
        );
    }

    #[test]
    fn find_text_includes_the_message_and_tool_strings() {
        use crate::state::{MessagePart, ToolCallInfo};
        let mut message = AgentMessage::user("hello rust".into(), 1.0);
        message.parts.push(MessagePart::Thinking("ponder".into()));
        message.error = Some("boom".into());
        message.parts.push(MessagePart::ToolCall(ToolCallInfo {
            id: "1".into(),
            tool: "Bash".into(),
            input: serde_json::json!({"command": "cargo test", "n": 1}),
            output: Some("ok".into()),
            is_error: false,
            started_at_ms: None,
            ended_at_ms: None,
        }));
        let text = searchable_text(&message);
        assert!(text.contains("hello rust"));
        assert!(text.contains("ponder"));
        assert!(text.contains("boom"));
        assert!(text.contains("cargo test"));
        assert!(text.contains("ok"));
        assert!(!text.contains("\"n\""));
    }

    /// Rows rendering taller under a following view keep it following;
    /// only scrolling up parks it.
    #[test]
    fn only_scrolling_up_stops_following_new_rows() {
        assert_eq!(follow_after_scroll(false, true, false), (true, false));
        assert_eq!(follow_after_scroll(true, false, true), (false, false));
        assert_eq!(follow_after_scroll(true, false, false), (true, true));
        assert_eq!(follow_after_scroll(false, false, false), (false, false));
    }

    #[test]
    fn a_resumed_turn_that_fails_before_init_retries_cold_once() {
        let refused = Err(StreamError::Http {
            status: 500,
            detail: None,
        });
        assert_eq!(next_step(attempt(&refused)), NextStep::RetryCold);
        let ok = Ok(());
        assert_eq!(
            next_step(Attempt {
                message_errored: true,
                ..attempt(&ok)
            }),
            NextStep::RetryCold
        );
        assert_eq!(
            next_step(Attempt {
                cold_retried: true,
                ..attempt(&refused)
            }),
            NextStep::Finish
        );
        assert_eq!(
            next_step(Attempt {
                saw_init: true,
                ..attempt(&refused)
            }),
            NextStep::Finish
        );
        assert_eq!(
            next_step(Attempt {
                resumed: false,
                ..attempt(&refused)
            }),
            NextStep::Finish
        );
    }

    #[test]
    fn stop_and_success_finish() {
        assert_eq!(
            next_step(attempt(&Err(StreamError::Stopped))),
            NextStep::Finish
        );
        assert_eq!(next_step(attempt(&Ok(()))), NextStep::Finish);
    }
}
