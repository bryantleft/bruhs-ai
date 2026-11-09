//! Composer: the prompt surface with its footer chips (model, access mode,
//! Plan/Build), the round send/stop action, the approval panel, the context
//! meter, the project/branch context strip, and the empty-thread hero.
//! `rounded-mango`, tone + ring elevation, persimmon only on the primary
//! action.

use bruhs_models::agent::{provider_for_model_id, AgentProvider};
use bruhs_models::num::ToIntSaturating;
use bruhs_models::PermissionMode;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::{
    approve_permission, checkout_git_branch, git_status, list_git_branches, ApiError, Decision,
};
use crate::components::file_attachments::{
    enqueue_browser_file, offers_block_send, FileGeneration, FileOffers, FileTiles,
};
use crate::components::find_bar::reveal_message;
use crate::components::mention_editor::MentionEditor;
use crate::components::model_picker::ModelPicker;
use crate::components::provider::{provider_logo_src_for_model, PROVIDER_LOGO_CLASS};
use crate::format::relative_time;
use crate::format::{context_percentage, context_window_for, format_tokens};
use crate::stash::stash_preview;
use crate::state::{
    activate_thread, ActiveThreadId, ActiveView, AppView, BaseBranchSignal, BranchMenuOpen,
    ChipRetargetGuard, ClaudeEffortSignal, CompactFullSignal, CompactIdleSignal, ComposerChips,
    EffortMenuOpen, FollowUpSteerSignal, GitStatusSignal, ModelSelectOpen, PendingPermissions,
    PendingPermissionsSignal, PermissionModeSignal, PermissionRequest, PreviousWorktreeSignal,
    PromptQueueSignal, PromptStashSignal, QueueHeldSignal, RunChoiceSignal, SetActiveThreadId,
    SetActiveView, SetMessagesSignal, SetModelSelectOpen, SetPermissionModeSignal,
    SetProjectPickerOpen, StashMenuOpen, StreamBufferSignal, ThreadNotice, ThreadsSignal,
    UnreadThreads, UsageInfo, WorkspaceMenuOpen,
};
use crate::ui::{Icon, Kbd, Menu, MenuAlign, MenuItem, MenuSide, Tooltip};
use bruhs_models::agent::Role;
use bruhs_models::logic::attachments::SEND_WAITS;
use bruhs_models::logic::commands::{COMPACT_UNAVAILABLE, COMPOSER_PLACEHOLDER};
use bruhs_models::logic::compact::{show_return_chip, CACHE_OAUTH_MS};
use bruhs_models::logic::composer_text::{
    exceeds_message_limit, exceeds_message_limit_with, paste_name, TextAttachment,
    MESSAGE_LIMIT_HINT,
};
use bruhs_models::logic::context_chip::{
    extra_limit_chars, join_drafts, replace_kind, ChipKind, ContextChip,
};
use bruhs_models::logic::follow_up::{
    action_enabled_for, action_label, action_title, composer_action, composer_after_lost_edit,
    provider_can_steer, ComposerAction, FollowUp,
};
use bruhs_models::logic::queue::PromptQueue;
use bruhs_models::logic::recall::{recall_step, RecallAction, RecallCursor, RecallDirection};
use bruhs_models::logic::run_place::{WorkspaceChoice, CLAUDE_EFFORTS};

/// The mounted composer publishes the draft Edit from here should keep.
/// A queued edit cancels first, so the draft from before that edit is the
/// one that stays above the restored prompt.
#[derive(Copy, Clone)]
pub struct ComposerDraftSource(pub StoredValue<Option<Callback<(), String>>>);

/// Pasted text kept beside the draft until a real send. In memory only.
#[derive(Copy, Clone)]
pub struct TextAttachments(pub RwSignal<Vec<TextAttachment>>);

/// Why the send button is disabled, if it is. A running turn no longer
/// blocks it: the prompt queues instead (see `crate::queue`).
pub fn send_disabled_reason(draft: &str, _streaming: bool) -> Option<&'static str> {
    if exceeds_message_limit(draft) {
        Some(MESSAGE_LIMIT_HINT)
    } else if draft.trim().is_empty() {
        Some("Type a message to send")
    } else {
        None
    }
}

/// Plan/Build toggle state: entering Plan remembers the access mode to
/// restore when leaving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanToggle {
    previous: PermissionMode,
}

impl PlanToggle {
    pub fn new(current: PermissionMode) -> Self {
        let previous = if current == PermissionMode::Plan {
            PermissionMode::Default
        } else {
            current
        };
        Self { previous }
    }

    /// Returns the mode to switch to.
    pub fn toggle(&mut self, current: PermissionMode) -> PermissionMode {
        if current == PermissionMode::Plan {
            self.previous
        } else {
            self.previous = current;
            PermissionMode::Plan
        }
    }
}

/// One-line summary of a tool input for the approval panel.
pub fn permission_summary(tool: &str, input: &serde_json::Value) -> String {
    let pick = |key: &str| -> Option<String> {
        input
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    match tool {
        "Bash" => pick("command").unwrap_or_default(),
        "Edit" | "Write" | "Read" | "MultiEdit" => pick("file_path").unwrap_or_default(),
        "Glob" | "Grep" => pick("pattern").unwrap_or_default(),
        "WebFetch" => pick("url").unwrap_or_default(),
        "WebSearch" => pick("query").unwrap_or_default(),
        _ => serde_json::to_string(input).unwrap_or_default(),
    }
}

/// Display label for a model id (see `bruhs_models::agent::model_label`).
pub fn model_label(model_id: &str) -> String {
    bruhs_models::agent::model_label(model_id)
}

/// Tokens that occupy the context window after a turn: the reported window,
/// else fresh input plus everything served from or written to the cache.
pub fn context_usage(usage: &UsageInfo) -> u64 {
    usage.window()
}

/// Tallest the chips row is on one line (h-7 chips plus p-1); taller means
/// it wrapped.
const CHIPS_ONE_ROW_MAX_PX: i32 = 48;
/// How much wider than where it wrapped the row must get before the
/// Attach and Stash labels come back.
const CHIPS_EXPAND_MARGIN_PX: i32 = 8;

const CHIP: &str = "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-grape px-2.5 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 cursor-pointer";

const QUEUE_HIT: &str = "inline-flex size-6 shrink-0 items-center justify-center rounded-seed text-rambutan-500 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100 cursor-pointer";

/// Signals a queued-message edit reads and writes. One value keeps each
/// helper under the argument limit.
#[derive(Copy, Clone)]
struct QueuedEditCtx {
    editing_id: RwSignal<Option<String>>,
    prior_draft: RwSignal<String>,
    prior_chips: RwSignal<Vec<ContextChip>>,
    value: ReadSignal<String>,
    set_value: WriteSignal<String>,
    chips: RwSignal<Vec<ContextChip>>,
    guard: StoredValue<bool>,
    queue: RwSignal<PromptQueue>,
}

fn open_queued_edit(id: String, text: String, queued_chips: Vec<ContextChip>, ctx: QueuedEditCtx) {
    if ctx.editing_id.get_untracked().as_deref() == Some(id.as_str()) {
        return;
    }
    if ctx.editing_id.get_untracked().is_some() {
        ctx.guard.set_value(true);
        ctx.chips.set(ctx.prior_chips.get_untracked());
        ctx.set_value.set(ctx.prior_draft.get_untracked());
    }
    ctx.prior_draft.set(ctx.value.get_untracked());
    ctx.prior_chips.set(ctx.chips.get_untracked());
    ctx.editing_id.set(Some(id));
    ctx.guard.set_value(true);
    ctx.chips.set(queued_chips);
    ctx.set_value.set(text);
}

fn commit_queued_edit(ctx: QueuedEditCtx) {
    let Some(id) = ctx.editing_id.get_untracked() else {
        return;
    };
    let text = ctx.value.get_untracked();
    let body = ctx.chips.get_untracked();
    let saved = ctx
        .queue
        .try_update(|items| items.set_body(&id, &text, body))
        .unwrap_or(false);
    if !saved {
        return;
    }
    restore_queued_draft(ctx);
}

fn restore_queued_draft(ctx: QueuedEditCtx) {
    let prior = ctx.prior_draft.get_untracked();
    let prior_body = ctx.prior_chips.get_untracked();
    ctx.editing_id.set(None);
    ctx.prior_draft.set(String::new());
    ctx.prior_chips.set(Vec::new());
    ctx.guard.set_value(true);
    ctx.chips.set(prior_body);
    ctx.set_value.set(prior);
}

fn cancel_queued_edit(ctx: QueuedEditCtx) {
    if ctx.editing_id.get_untracked().is_none() {
        return;
    }
    restore_queued_draft(ctx);
}

#[component]
pub fn Composer(
    value: ReadSignal<String>,
    set_value: WriteSignal<String>,
    on_submit: Callback<()>,
    on_opposite: Callback<()>,
    on_stop: Callback<()>,
    /// `/goal` or `/compact` chosen from the slash menu.
    on_slash: Callback<&'static str>,
    /// Working directory for @-mentions and the branch strip.
    #[prop(into)]
    project_path: Signal<Option<String>>,
    /// Display name of the thread's project; `None` for general threads.
    #[prop(into)]
    project_name: Signal<Option<String>>,
    /// Usage of the latest assistant message plus its model, for the meter.
    #[prop(into)]
    last_usage: Signal<Option<(UsageInfo, String)>>,
    /// Centered hero layout for an empty thread.
    #[prop(optional)]
    hero: bool,
    /// Unedited recalled prompt. The chat page clears it when a thread
    /// changes and when Edit from here rewinds.
    recall: RwSignal<Option<RecallCursor>>,
) -> impl IntoView {
    let is_streaming = expect_context::<crate::state::IsStreaming>().0;
    let current_model = expect_context::<crate::state::CurrentModel>().0;
    let PermissionModeSignal(permission_mode) = expect_context::<PermissionModeSignal>();
    let SetPermissionModeSignal(set_permission_mode) = expect_context::<SetPermissionModeSignal>();
    let PendingPermissionsSignal(pending_permissions) =
        expect_context::<PendingPermissionsSignal>();
    // Re-render the panel only when the request being asked about changes,
    // so a second request arriving keeps the first one's error and state.
    let asking = Memo::new(move |_| pending_permissions.with(|p| p.first().cloned()));
    let SetModelSelectOpen(set_model_select_open) = expect_context::<SetModelSelectOpen>();
    let ModelSelectOpen(model_select_open) = expect_context::<ModelSelectOpen>();
    let SetProjectPickerOpen(set_project_picker_open) = expect_context::<SetProjectPickerOpen>();
    let PromptQueueSignal(queue) = expect_context::<PromptQueueSignal>();
    let QueueHeldSignal(queue_held) = expect_context::<QueueHeldSignal>();
    let active_thread_id = expect_context::<crate::state::ActiveThreadId>().0;
    let editing_id = RwSignal::new(None::<String>);
    let prior_draft = RwSignal::new(String::new());
    let prior_chips = RwSignal::new(Vec::<ContextChip>::new());
    let ComposerChips(context_chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(chip_guard) = expect_context::<ChipRetargetGuard>();
    let edit_ctx = QueuedEditCtx {
        editing_id,
        prior_draft,
        prior_chips,
        value,
        set_value,
        chips: context_chips,
        guard: chip_guard,
        queue,
    };
    let comment_for = RwSignal::new(None::<String>);
    let chip_history = expect_context::<crate::thread_history::ThreadHistory>();
    let SetActiveThreadId(set_active_thread) = expect_context::<SetActiveThreadId>();
    let active_for_chip = expect_context::<ActiveThreadId>().0;
    let set_messages_chip = expect_context::<SetMessagesSignal>().0;
    let StreamBufferSignal(stream_buffer_chip) = expect_context::<StreamBufferSignal>();
    let UnreadThreads(unread_chip) = expect_context::<UnreadThreads>();
    let SetActiveView(set_view_chip) = expect_context::<SetActiveView>();
    let active_view_chip = expect_context::<ActiveView>().0;
    let on_chip = Callback::new(move |id: String| {
        let chip =
            context_chips.with_untracked(|chips| chips.iter().find(|chip| chip.id == id).cloned());
        let Some(chip) = chip else { return };
        match chip.kind {
            ChipKind::Citation { message_id, .. } => {
                comment_for.set(Some(id));
                reveal_message(&message_id);
            }
            ChipKind::PullRequest { url, .. } => {
                if let Some(window) = web_sys::window() {
                    let _ = window.open_with_url_and_target(&url, "_blank");
                }
            }
            ChipKind::Thread { thread_id, .. } => {
                activate_thread(
                    chip_history,
                    active_for_chip,
                    thread_id,
                    set_active_thread,
                    set_messages_chip,
                    stream_buffer_chip,
                    unread_chip,
                );
                if active_view_chip.get_untracked() != AppView::Chat {
                    set_view_chip.set(AppView::Chat);
                }
            }
            _ => {}
        }
    });
    let opposite_held = RwSignal::new(false);
    let keydown = window_event_listener(leptos::ev::keydown, move |ev: web_sys::KeyboardEvent| {
        if ev.meta_key() || ev.ctrl_key() || ev.key() == "Meta" || ev.key() == "Control" {
            opposite_held.set(true);
        }
    });
    let keyup = window_event_listener(leptos::ev::keyup, move |ev: web_sys::KeyboardEvent| {
        if !ev.meta_key() && !ev.ctrl_key() {
            opposite_held.set(false);
        }
    });
    let blur = window_event_listener(leptos::ev::blur, move |_| {
        opposite_held.set(false);
    });
    on_cleanup(move || {
        keydown.remove();
        keyup.remove();
        blur.remove();
    });
    Effect::new(move |_| {
        let Some(id) = editing_id.get() else {
            return;
        };
        if queue.with(|items| items.contains(&id)) {
            return;
        }
        let next = composer_after_lost_edit(&prior_draft.get_untracked(), &value.get_untracked());
        let prior_body = prior_chips.get_untracked();
        editing_id.set(None);
        prior_draft.set(String::new());
        prior_chips.set(Vec::new());
        chip_guard.set_value(true);
        context_chips.set(prior_body);
        set_value.set(next);
    });
    let on_editor_submit = Callback::new(move |_: ()| {
        if editing_id.get_untracked().is_some() {
            commit_queued_edit(edit_ctx);
            return;
        }
        on_submit.run(());
    });
    let on_editor_opposite = Callback::new(move |_: ()| {
        if editing_id.get_untracked().is_some() {
            return;
        }
        on_opposite.run(());
    });
    let on_edit_latest = Callback::new(move |_: ()| {
        let Some(tid) = active_thread_id.get_untracked() else {
            return;
        };
        let Some(item) = queue.with_untracked(|items| items.latest_for(&tid)) else {
            return;
        };
        recall.set(None);
        open_queued_edit(item.id, item.text, item.chips, edit_ctx);
    });
    let messages = expect_context::<crate::state::MessagesSignal>().0;
    let TextAttachments(attachments) = expect_context::<TextAttachments>();
    let FileOffers(file_offers) = expect_context::<FileOffers>();
    let FileGeneration(file_generation) = expect_context::<FileGeneration>();
    let thread_notice = expect_context::<ThreadNotice>();
    let on_attach_paste = Callback::new(move |text: String| {
        attachments.update(|items| {
            let name = paste_name(items.iter().map(|item| item.name.as_str()));
            items.push(TextAttachment { name, text });
        });
    });
    let on_attach_file = Callback::new(move |file: web_sys::File| {
        if let Some(message) = enqueue_browser_file(file_offers, file_generation, file) {
            thread_notice.flash(message);
        }
    });
    let on_file_input = move |event: web_sys::Event| {
        let Some(input) = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        if let Some(files) = input.files() {
            for index in 0..files.length() {
                if let Some(file) = files.get(index) {
                    on_attach_file.run(file);
                }
            }
        }
        input.set_value("");
    };
    let file_input: NodeRef<leptos::html::Input> = NodeRef::new();
    // When the chips row would wrap, it drops the Attach and Stash text
    // (icons and accessible names stay) to stay on one row, and brings it
    // back once the row is wider than where it wrapped. A ResizeObserver
    // sees the wrap as a height change; a CSS container query would contain
    // the menus' fixed click-away layers inside the row.
    let chips_row: NodeRef<leptos::html::Div> = NodeRef::new();
    let chips_compact = RwSignal::new(false);
    let chips_observer = StoredValue::new_local(
        None::<(
            web_sys::ResizeObserver,
            wasm_bindgen::closure::Closure<dyn FnMut()>,
        )>,
    );
    Effect::new(move |_| {
        let Some(row) = chips_row.get() else {
            return;
        };
        let measured = row.clone();
        let wrapped_at = std::cell::Cell::new(0);
        let callback = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
            let width = measured.client_width();
            let wraps = measured.client_height() > CHIPS_ONE_ROW_MAX_PX;
            let compact = chips_compact.get_untracked();
            if !compact && wraps {
                wrapped_at.set(width);
                chips_compact.set(true);
            } else if compact && width > wrapped_at.get() + CHIPS_EXPAND_MARGIN_PX {
                // Try the labels again; if they wrap, this fires once more.
                chips_compact.set(false);
            }
        }) as Box<dyn FnMut()>);
        match web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            Ok(observer) => {
                observer.observe(&row);
                chips_observer.update_value(|value| {
                    if let Some((old, _)) = value.replace((observer, callback)) {
                        old.disconnect();
                    }
                });
            }
            Err(error) => log::warn!("Composer chips resize observation unavailable: {error:?}"),
        }
    });
    on_cleanup(move || {
        chips_observer.update_value(|value| {
            if let Some((observer, _callback)) = value.take() {
                observer.disconnect();
            }
        });
    });
    let pick_files = move |_| {
        if let Some(input) = file_input.get() {
            input.click();
        }
    };
    let on_drag_over = move |event: web_sys::DragEvent| {
        if transfer_has_files(event.data_transfer().as_ref()) {
            event.prevent_default();
        }
    };
    let on_drop = move |event: web_sys::DragEvent| {
        let Some(files) = event.data_transfer().and_then(|transfer| transfer.files()) else {
            return;
        };
        if files.length() == 0 {
            return;
        }
        event.prevent_default();
        for index in 0..files.length() {
            if let Some(file) = files.get(index) {
                on_attach_file.run(file);
            }
        }
    };
    let on_recall = Callback::new(move |(older, on_edge): (bool, bool)| {
        if editing_id.get_untracked().is_some() {
            return false;
        }
        let composer = value.get_untracked();
        let prompts = messages.with_untracked(|items| {
            bruhs_models::logic::recall::sent_prompts(items.iter().map(|message| {
                (
                    message.id.clone(),
                    message.role == Role::User,
                    message.text(),
                )
            }))
        });
        let cursor = recall.get_untracked();
        let direction = if older {
            RecallDirection::Older
        } else {
            RecallDirection::Newer
        };
        let outcome = recall_step(
            &prompts,
            cursor.as_ref(),
            &composer,
            !attachments.with_untracked(|items| items.is_empty())
                || !file_offers.with_untracked(|items| items.is_empty()),
            direction,
            on_edge,
        );
        recall.set(outcome.cursor);
        match outcome.action {
            RecallAction::Place(text) => {
                set_value.set(text);
                true
            }
            RecallAction::Clear => {
                set_value.set(String::new());
                true
            }
            RecallAction::Pass => false,
        }
    });
    if let Some(ComposerDraftSource(slot)) = use_context::<ComposerDraftSource>() {
        slot.set_value(Some(Callback::new(move |_| {
            if editing_id.get_untracked().is_some() {
                let prior = prior_draft.get_untracked();
                cancel_queued_edit(edit_ctx);
                prior
            } else {
                value.get_untracked()
            }
        })));
    }
    let PromptStashSignal(stash) = expect_context::<PromptStashSignal>();
    let StashMenuOpen(stash_open) = expect_context::<StashMenuOpen>();

    let restore_stash = move |id: String| {
        let entry = stash.try_update(|s| s.take(&id)).flatten();
        if let Some(e) = entry {
            let (next, kept) = join_drafts(
                &value.get_untracked(),
                &context_chips.get_untracked(),
                &e.text,
                &e.chips,
                true,
            );
            chip_guard.set_value(true);
            context_chips.set(kept);
            set_value.set(next);
            stash_open.set(false);
            if let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("chat-input"))
            {
                if let Some(h) = el.dyn_ref::<web_sys::HtmlElement>() {
                    let _ = h.focus();
                }
            }
        }
    };

    let mode_open = RwSignal::new(false);
    let EffortMenuOpen(effort_open) = expect_context::<EffortMenuOpen>();
    let ClaudeEffortSignal(claude_effort_level) = expect_context::<ClaudeEffortSignal>();
    let plan_toggle = StoredValue::new(PlanToggle::new(permission_mode.get_untracked()));
    let is_plan = Signal::derive(move || permission_mode.get() == PermissionMode::Plan);
    let toggle_plan = move || {
        let current = permission_mode.get_untracked();
        let next = plan_toggle
            .try_update_value(|t| t.toggle(current))
            .unwrap_or(current);
        set_permission_mode.set(next);
    };

    let history = expect_context::<crate::thread_history::ThreadHistory>();
    let threads = expect_context::<ThreadsSignal>().0;
    let FollowUpSteerSignal(follow_up_steer) = expect_context::<FollowUpSteerSignal>();
    let CompactIdleSignal(compact_idle) = expect_context::<CompactIdleSignal>();
    let CompactFullSignal(compact_full) = expect_context::<CompactFullSignal>();
    let now_ms = RwSignal::new(js_sys::Date::now());
    let return_clock = StoredValue::new_local(None::<Interval>);
    return_clock.set_value(Some(Interval::new(30_000, move || {
        now_ms.set(js_sys::Date::now());
    })));
    on_cleanup(move || {
        return_clock.set_value(None);
    });

    let placeholder = COMPOSER_PLACEHOLDER;

    view! {
        <div class="mx-auto w-full max-w-3xl">
            {hero.then(|| view! {
                <p class="mb-8 text-center text-h2 text-rambutan-100">
                    {move || match project_name.get() {
                        Some(name) => view! {
                            "What should we build in "
                            <button
                                type="button"
                                class="border-b border-dotted border-rambutan-500/60 text-rambutan-100 transition-colors hover:border-rambutan-300 cursor-pointer"
                                title="Change project"
                                on:click=move |_| set_project_picker_open.set(true)
                            >
                                {name}
                            </button>
                            "?"
                        }.into_any(),
                        None => view! { "What's on your mind?" }.into_any(),
                    }}
                </p>
            })}

            {move || {
                let id = active_thread_id.get();
                let goal = threads.with(|items| {
                    id.as_ref().and_then(|id| {
                        items
                            .iter()
                            .find(|thread| &thread.id == id)
                            .and_then(|thread| thread.goal.clone())
                    })
                })?;
                let paused = goal.paused;
                let text = goal.text;
                Some(view! {
                    <div data-thread-goal="" class="mb-2 flex items-center gap-2 px-1 text-label-sm text-rambutan-400">
                        <span class="shrink-0">"Goal"</span>
                        <span class="min-w-0 flex-1 truncate">{text}</span>
                        {paused.then(|| view! { <span class="shrink-0 text-rambutan-500">"Paused"</span> })}
                    </div>
                })
            }}

            <div
                data-composer-surface=""
                class="rounded-mango bg-longan-900 inset-ring inset-ring-white/10 transition-[box-shadow] focus-within:inset-ring-persimmon-400/40"
                on:dragover=on_drag_over
                on:drop=on_drop
            >
                // Approval panel — only while the agent waits on a decision.
                {move || asking.get().map(|req| view! {
                    <ApprovalPanel request=req pending=pending_permissions />
                })}

                // Prompts waiting for the running turn to finish.
                {move || {
                    let _editing = editing_id.get();
                    let tid = active_thread_id.get()?;
                    let queued = queue.with(|q| q.for_thread(&tid));
                    (!queued.is_empty()).then(|| view! {
                        <div data-queued-prompts="" class="flex flex-col border-b border-longan-800/60">
                            {queued.into_iter().map(|p| {
                                let id = p.id.clone();
                                let nudge_id = id.clone();
                                let drop_id = id.clone();
                                let drag_id = id.clone();
                                let edit_id = id.clone();
                                let remove_id = id.clone();
                                let text = p.text.clone();
                                let edit_text = p.text.clone();
                                let edit_chips = p.chips.clone();
                                let title = p.text;
                                let editing = editing_id.with_untracked(|current| current.as_deref() == Some(id.as_str()));
                                let row_class = if editing {
                                    "flex items-center gap-1 bg-longan-800 px-2 py-1 text-label-sm"
                                } else {
                                    "flex items-center gap-1 px-2 py-1 text-label-sm"
                                };
                                view! {
                                    <div
                                        class=row_class
                                        data-queued-id=id
                                        on:dragover=move |ev: web_sys::DragEvent| { ev.prevent_default(); }
                                        on:drop=move |ev: web_sys::DragEvent| {
                                            ev.prevent_default();
                                            let Some(from) = ev.data_transfer().and_then(|dt| dt.get_data("text/plain").ok()) else {
                                                return;
                                            };
                                            let before = drop_id.clone();
                                            queue.update(|items| { items.move_before(&from, &before); });
                                        }
                                    >
                                        <button
                                            type="button"
                                            draggable="true"
                                            aria-label="Reorder queued message"
                                            class=QUEUE_HIT
                                            on:dragstart=move |ev: web_sys::DragEvent| {
                                                if let Some(dt) = ev.data_transfer() {
                                                    let _ = dt.set_data("text/plain", &drag_id);
                                                    dt.set_effect_allowed("move");
                                                }
                                            }
                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                let earlier = match ev.key().as_str() {
                                                    "ArrowUp" => Some(true),
                                                    "ArrowDown" => Some(false),
                                                    _ => None,
                                                };
                                                let Some(earlier) = earlier else { return };
                                                ev.prevent_default();
                                                let id = nudge_id.clone();
                                                queue.update(|items| { items.nudge(&id, earlier); });
                                            }
                                        >
                                            "⋮"
                                        </button>
                                        <span class="shrink-0 text-blueberry-300">"Queued"</span>
                                        <span class="min-w-0 flex-1 truncate text-rambutan-300" title=title>{text}</span>
                                        {editing.then(|| view! {
                                            <button
                                                type="button"
                                                aria-label="Save queued message"
                                                class=QUEUE_HIT
                                                on:click=move |_| commit_queued_edit(edit_ctx)
                                            >
                                                <Icon name="check" size=14 />
                                            </button>
                                            <button
                                                type="button"
                                                aria-label="Cancel editing queued message"
                                                class=QUEUE_HIT
                                                on:click=move |_| cancel_queued_edit(edit_ctx)
                                            >
                                                <Icon name="x" size=14 />
                                            </button>
                                        })}
                                        {(!editing).then(|| view! {
                                            <button
                                                type="button"
                                                aria-label="Edit queued message"
                                                class=QUEUE_HIT
                                                on:click=move |_| open_queued_edit(edit_id.clone(), edit_text.clone(), edit_chips.clone(), edit_ctx)
                                            >
                                                <Icon name="pencil" size=14 />
                                            </button>
                                            <button
                                                type="button"
                                                aria-label="Remove queued message"
                                                class=QUEUE_HIT
                                                on:click=move |_| {
                                                    let id = remove_id.clone();
                                                    queue.update(|items| { items.remove(&id); });
                                                }
                                            >
                                                <Icon name="x" size=14 />
                                            </button>
                                        })}
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })
                }}

                {move || {
                    let items = attachments.get();
                    (!items.is_empty()).then(|| view! {
                        <div data-text-attachments="" class="flex flex-col border-b border-longan-800/60">
                            {items.into_iter().enumerate().map(|(index, item)| {
                                let name = item.name;
                                let label = name.clone();
                                view! {
                                    <div class="flex h-6 items-center gap-1 px-2 text-label-sm" data-paste-name=name>
                                        <span class="min-w-0 flex-1 truncate text-rambutan-300">{label}</span>
                                        <button
                                            type="button"
                                            aria-label="Remove text attachment"
                                            class=QUEUE_HIT
                                            on:click=move |_| {
                                                attachments.update(|items| {
                                                    if index < items.len() {
                                                        items.remove(index);
                                                    }
                                                });
                                            }
                                        >
                                            <Icon name="x" size=14 />
                                        </button>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })
                }}

                <FileTiles />

                <MentionEditor
                    value=value
                    set_value=set_value
                    project_path=project_path
                    on_submit=on_editor_submit
                    on_opposite=on_editor_opposite
                    on_edit_queued=on_edit_latest
                    on_recall=on_recall
                    on_attach_paste=on_attach_paste
                    on_attach_file=on_attach_file
                    on_chip=on_chip
                    on_command=on_slash
                    placeholder=placeholder.to_string()
                    chrome=false
                />

                {move || {
                    let id = comment_for.get()?;
                    let comment = context_chips.with(|chips| {
                        chips.iter().find(|chip| chip.id == id).and_then(|chip| match &chip.kind {
                            ChipKind::Citation { comment, .. } => Some(comment.clone()),
                            _ => None,
                        })
                    })?;
                    Some(view! {
                        <label class="flex items-center gap-2 px-4 pb-1 text-label-sm text-rambutan-400">
                            "Comment"
                            <input
                                class="h-6 min-w-0 flex-1 rounded-seed bg-longan-800 px-2 text-label-sm text-rambutan-100"
                                data-chip-comment=""
                                aria-label="Citation comment"
                                prop:value=comment
                                on:change=move |ev: web_sys::Event| {
                                    let Some(input) = ev.target().and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok()) else {
                                        return;
                                    };
                                    let next_comment = input.value();
                                    let current = context_chips.get_untracked();
                                    let Some(existing) = current.iter().find(|chip| chip.id == id).cloned() else {
                                        return;
                                    };
                                    let ChipKind::Citation { message_id, quote, .. } = existing.kind else {
                                        return;
                                    };
                                    let kind = ChipKind::Citation {
                                        message_id,
                                        quote,
                                        comment: next_comment,
                                    };
                                    let Some((next, kept)) = replace_kind(&value.get_untracked(), &current, &id, kind) else {
                                        return;
                                    };
                                    chip_guard.set_value(true);
                                    context_chips.set(kept);
                                    if let Some(textarea) = web_sys::window()
                                        .and_then(|window| window.document())
                                        .and_then(|document| document.get_element_by_id("chat-input"))
                                        .and_then(|element| element.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
                                    {
                                        textarea.set_value(&next);
                                    }
                                    set_value.set(next);
                                }
                            />
                        </label>
                    })
                }}

                {move || {
                    let extra = context_chips.with(|chips| extra_limit_chars(chips));
                    exceeds_message_limit_with(&value.get(), extra)
                }.then(|| view! {
                    <p data-message-limit="" class="px-4 pb-1 text-label-sm text-durian-300">{MESSAGE_LIMIT_HINT}</p>
                })}

                // Footer: chips left, meter + primary action right.
                <div class="flex items-center justify-between gap-2 px-3 pb-3 sm:px-4 sm:pb-4">
                    <div node_ref=chips_row data-composer-chips="" data-compact=move || chips_compact.get().then_some("") class="-m-1 flex min-w-0 flex-1 flex-wrap items-center gap-1 p-1">
                        <div class="relative shrink-0">
                            <button
                                type="button"
                                class=CHIP
                                title="Change model (⌘⇧M)"
                                aria-label="Change model (⌘⇧M)"
                                aria-haspopup="dialog"
                                aria-expanded=move || model_select_open.get().to_string()
                                on:click=move |_| set_model_select_open.update(|v| *v = !*v)
                            >
                                <img
                                    src=move || provider_logo_src_for_model(&current_model.get())
                                    alt=""
                                    aria-hidden="true"
                                    class=PROVIDER_LOGO_CLASS
                                />
                                <span class="max-w-[12rem] truncate">{move || model_label(&current_model.get())}</span>
                                <Icon name="chevron-down" size=12 class="text-rambutan-500" />
                            </button>
                            <ModelPicker />
                        </div>

                        <Menu open=mode_open align=MenuAlign::Start side=MenuSide::Top label="Access mode" class="min-w-[18rem]"
                            trigger=move || view! {
                                <button
                                    type="button"
                                    class=CHIP
                                    aria-haspopup="menu"
                                    aria-expanded=move || mode_open.get().to_string()
                                    title=move || permission_mode.get().chip_title(provider_for_model_id(&current_model.get()))
                                    on:click=move |_| mode_open.update(|v| *v = !*v)
                                >
                                    {move || view! { <Icon name=permission_mode.get().icon() size=14 /> }}
                                    <span>{move || permission_mode.get().label()}</span>
                                    <Icon name="chevron-down" size=12 class="text-rambutan-500" />
                                </button>
                            }
                        >
                            {move || {
                                let provider = provider_for_model_id(&current_model.get());
                                [
                                    PermissionMode::Default,
                                    PermissionMode::AcceptEdits,
                                    PermissionMode::BypassPermissions,
                                ]
                                .into_iter()
                                .map(|mode| view! {
                                <MenuItem
                                    selected=Signal::derive(move || permission_mode.get() == mode)
                                    description=mode.description(Some(provider))
                                    on_select=Callback::new(move |_| {
                                        set_permission_mode.set(mode);
                                        plan_toggle.set_value(PlanToggle::new(mode));
                                        mode_open.set(false);
                                    })
                                >
                                    <Icon name=mode.icon() size=14 class="text-rambutan-400" />
                                    <span>{mode.label()}</span>
                                </MenuItem>
                                })
                                .collect_view()
                            }}
                        </Menu>

                        <button
                            type="button"
                            class=move || if is_plan.get() {
                                format!("{CHIP} bg-longan-800 text-rambutan-100 hover:bg-longan-800")
                            } else {
                                CHIP.to_string()
                            }
                            aria-pressed=move || is_plan.get().to_string()
                            title=move || if is_plan.get() {
                                "Plan mode — click to return to build mode"
                            } else {
                                "Build mode — click to enter plan mode"
                            }
                            on:click=move |_| toggle_plan()
                        >
                            {move || view! { <Icon name=if is_plan.get() { "clipboard-list" } else { "bot" } size=14 /> }}
                            <span>{move || if is_plan.get() { "Plan" } else { "Build" }}</span>
                        </button>

                        {move || {
                            (provider_for_model_id(&current_model.get()) == AgentProvider::Claude).then(|| {
                                let effort = claude_effort_level;
                                view! {
                                    <Menu open=effort_open align=MenuAlign::Start side=MenuSide::Top label="Effort" class="min-w-[12rem]"
                                        trigger=move || view! {
                                            <button
                                                type="button"
                                                class=CHIP
                                                data-effort-chip=""
                                                data-effort=move || effort.get().unwrap_or_else(|| "default".into())
                                                aria-haspopup="menu"
                                                aria-expanded=move || effort_open.get().to_string()
                                                title="Effort (⌘⇧E)"
                                                on:click=move |_| effort_open.update(|open| *open = !*open)
                                            >
                                                <Icon name="zap" size=14 />
                                                <span>{move || effort.get().unwrap_or_else(|| "Effort".into())}</span>
                                                <Icon name="chevron-down" size=12 class="text-rambutan-500" />
                                            </button>
                                        }
                                    >
                                        <MenuItem
                                            selected=Signal::derive(move || effort.get().is_none())
                                            on_select=Callback::new(move |_| {
                                                effort.set(None);
                                                effort_open.set(false);
                                            })
                                        >
                                            <span>"Default"</span>
                                        </MenuItem>
                                        {CLAUDE_EFFORTS.into_iter().map(|level| {
                                            let label = level.to_string();
                                            let pick = label.clone();
                                            view! {
                                                <MenuItem
                                                    selected=Signal::derive(move || effort.get().as_deref() == Some(level))
                                                    on_select=Callback::new(move |_| {
                                                        effort.set(Some(pick.clone()));
                                                        effort_open.set(false);
                                                    })
                                                >
                                                    <span>{label}</span>
                                                </MenuItem>
                                            }
                                        }).collect_view()}
                                    </Menu>
                                }
                            })
                        }}

                        <button
                            type="button"
                            class=CHIP
                            data-attach-file=""
                            aria-label="Attach file"
                            title="Attach a file"
                            on:click=pick_files
                        >
                            <Icon name="paperclip" size=14 />
                            <span data-chip-label="secondary">"Attach"</span>
                        </button>
                        <input
                            node_ref=file_input
                            type="file"
                            multiple=true
                            class="sr-only"
                            tabindex="-1"
                            aria-hidden="true"
                            on:change=on_file_input
                        />

                        // Prompt stash (⌘S): parked drafts.
                        <Menu open=stash_open align=MenuAlign::Start side=MenuSide::Top label="Prompt stash" class="min-w-[20rem] max-h-80 overflow-y-auto"
                            trigger=move || view! {
                                <button
                                    type="button"
                                    class=CHIP
                                    data-stash-chip=""
                                    aria-label=move || match stash.with(|s| s.len()) {
                                        0 => "Prompt stash".to_string(),
                                        n => format!("Prompt stash, {n} saved"),
                                    }
                                    aria-haspopup="menu"
                                    aria-expanded=move || stash_open.get().to_string()
                                    title="Prompt stash (⌘S stashes the draft)"
                                    on:click=move |_| stash_open.update(|v| *v = !*v)
                                >
                                    <Icon name="archive" size=14 />
                                    <span data-chip-label="secondary">"Stash"</span>
                                    {move || {
                                        let n = stash.with(|s| s.len());
                                        (n > 0).then(|| view! {
                                            <span data-stash-count=n class="rounded-orb bg-blueberry-500/20 px-1.5 text-label-sm tabular-nums text-blueberry-300">{n}</span>
                                        })
                                    }}
                                </button>
                            }
                        >
                            {move || {
                                let entries = stash.with(|s| s.entries.clone());
                                if entries.is_empty() {
                                    return view! {
                                        <p class="px-3 py-2 text-label-sm text-rambutan-500">"No stashed prompts. ⌘S stashes the current draft."</p>
                                    }.into_any();
                                }
                                let now = js_sys::Date::now();
                                view! {
                                    <div class="flex flex-col gap-px p-1" role="list">
                                        {entries.into_iter().map(|e| {
                                            let id_restore = e.id.clone();
                                            let id_delete = e.id.clone();
                                            let preview = stash_preview(&e.text, 80);
                                            let when = relative_time(e.at_ms, now);
                                            let full = e.text;
                                            view! {
                                                <div role="listitem" class="flex items-center gap-1">
                                                    <button
                                                        type="button"
                                                        data-stash-entry=""
                                                        class="min-w-0 flex-1 rounded-grape px-2 py-1.5 text-left transition-colors hover:bg-longan-800/60 cursor-pointer"
                                                        title=full
                                                        on:click=move |_| restore_stash(id_restore.clone())
                                                    >
                                                        <span class="block truncate text-label-sm text-rambutan-200">{preview}</span>
                                                        <span class="block text-label-sm text-rambutan-500">{when}</span>
                                                    </button>
                                                    <button
                                                        type="button"
                                                        aria-label="Delete stashed prompt"
                                                        class="shrink-0 rounded-seed p-1 text-rambutan-500 transition-colors hover:text-lychee-300 cursor-pointer"
                                                        on:click=move |ev| {
                                                            ev.stop_propagation();
                                                            stash.update(|s| { s.remove(&id_delete); });
                                                        }
                                                    >
                                                        <Icon name="x" size=12 />
                                                    </button>
                                                </div>
                                            }
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            }}
                        </Menu>
                    </div>

                    <div class="flex shrink-0 items-center gap-2">
                        {move || last_usage.get().map(|(usage, model)| {
                            let claude = provider_can_steer(provider_for_model_id(&current_model.get()));
                            let title = if claude {
                                "Summarize this Claude thread"
                            } else {
                                COMPACT_UNAVAILABLE
                            };
                            let slash = on_slash;
                            view! {
                            <div class="flex items-center gap-1">
                                <ContextMeter usage=usage model=model />
                                <button
                                    type="button"
                                    data-compact-context=""
                                    title=title
                                    aria-label="Compact"
                                    class="inline-flex h-6 items-center rounded-seed px-2 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-800/60 hover:text-rambutan-200 cursor-pointer"
                                    on:click=move |_| slash.run("/compact")
                                >
                                    "Compact"
                                </button>
                            </div>
                            }
                        })}
                        {move || {
                            let streaming = is_streaming.get();
                            let draft = value.get();
                            let typed_empty = draft.trim().is_empty();
                            let extra = context_chips.with(|chips| extra_limit_chars(chips));
                            let over_limit = exceeds_message_limit_with(&draft, extra);
                            let ready_files = file_offers.with(|items| {
                                items.iter().any(|item| item.ready_path().is_some())
                            });
                            let uploads_block = !streaming && file_offers.with(|items| offers_block_send(items));
                            let draft_empty = if streaming {
                                typed_empty
                            } else {
                                typed_empty
                                    && attachments.with(|items| items.is_empty())
                                    && !ready_files
                            };
                            let held = queue_held.get();
                            let has_queued = active_thread_id
                                .get()
                                .is_some_and(|tid| queue.with(|items| items.has_for(&tid)));
                            let follow = FollowUp {
                                wants_steer: follow_up_steer.get(),
                                can_steer: provider_can_steer(provider_for_model_id(
                                    &current_model.get(),
                                )),
                            };
                            let action = composer_action(
                                streaming,
                                draft_empty,
                                opposite_held.get(),
                                held,
                                has_queued,
                                follow,
                            );
                            let return_chip = !streaming
                                && show_return_chip(
                                    compact_idle.get(),
                                    messages.with(|rows| {
                                        rows.iter().rev().find_map(|message| {
                                            message.usage.as_ref().map(context_usage)
                                        })
                                    }).unwrap_or(0),
                                    active_thread_id
                                        .get()
                                        .and_then(|tid| {
                                            threads.with(|rows| {
                                                rows.iter().find(|thread| thread.id == tid).map(
                                                    |thread| {
                                                        (now_ms.get() - thread.updated_at.as_f64())
                                                            .max(0.0)
                                                    },
                                                )
                                            })
                                        })
                                        .unwrap_or(0.0),
                                    CACHE_OAUTH_MS,
                                )
                                && provider_can_steer(provider_for_model_id(&current_model.get()));
                            let history_reason =
                                active_thread_id.with(|id| history.reason(id.as_deref()));
                            let history_blocks = history_reason.is_some()
                                && matches!(action, ComposerAction::Send | ComposerAction::Queue);
                            let enabled = action_enabled_for(action, draft_empty || over_limit)
                                && !history_blocks
                                && !uploads_block;
                            let label = action_label(action);
                            let title = if history_blocks {
                                history_reason.unwrap_or(action_title(action)).to_string()
                            } else if enabled || matches!(action, ComposerAction::SteerUnavailable)
                            {
                                action_title(action).to_string()
                            } else if uploads_block {
                                SEND_WAITS.to_string()
                            } else if over_limit {
                                MESSAGE_LIMIT_HINT.to_string()
                            } else {
                                send_disabled_reason(&draft, streaming)
                                    .unwrap_or(action_title(action))
                                    .to_string()
                            };
                            let show_stop =
                                streaming && !matches!(action, ComposerAction::Interrupt);
                            let class = match action {
                                ComposerAction::Send => "inline-flex size-9 items-center justify-center rounded-orb bg-persimmon-500 text-longan-950 transition-[background-color,opacity] hover:bg-persimmon-400 disabled:opacity-30 disabled:hover:bg-persimmon-500 disabled:cursor-not-allowed sm:size-8 cursor-pointer",
                                ComposerAction::Interrupt => "inline-flex h-8 items-center gap-1.5 rounded-orb bg-lychee-500 px-3 text-label-sm text-lychee-50 transition-colors hover:bg-lychee-400 cursor-pointer",
                                ComposerAction::SteerUnavailable => "inline-flex h-8 items-center rounded-orb bg-longan-800 px-3 text-label-sm text-rambutan-400 opacity-40 cursor-not-allowed",
                                _ => "inline-flex h-8 items-center rounded-orb bg-persimmon-500 px-3 text-label-sm text-longan-950 transition-[background-color,opacity] hover:bg-persimmon-400 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer",
                            };
                            let aria = if matches!(action, ComposerAction::Send | ComposerAction::Queue) {
                                format!("{label} message")
                            } else {
                                title.clone()
                            };
                            let submit = on_editor_submit;
                            let opposite = on_editor_opposite;
                            let stop_square = on_stop;
                            let stop_primary = on_stop;
                            let label_attr = label.to_string();
                            let full = compact_full.get();
                            let chip_label = if full { "Full" } else { "Compact" };
                            let chip_title = if full {
                                "Send the full draft"
                            } else {
                                "Summarize this thread, then send the draft"
                            };
                            view! {
                                {return_chip.then(|| view! {
                                    <button
                                        type="button"
                                        data-return-compact=chip_label
                                        title=chip_title
                                        aria-label=chip_title
                                        class="inline-flex h-6 items-center rounded-seed px-2 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-800/60 hover:text-rambutan-200 cursor-pointer"
                                        on:click=move |ev: web_sys::MouseEvent| {
                                            ev.stop_propagation();
                                            compact_full.update(|full| *full = !*full);
                                        }
                                    >
                                        {chip_label}
                                    </button>
                                })}
                                {show_stop.then(|| view! {
                                    <button
                                        type="button"
                                        aria-label="Stop generation"
                                        title="Stop generation"
                                        class="inline-flex size-9 items-center justify-center rounded-orb bg-lychee-500 text-lychee-50 transition-colors hover:bg-lychee-400 sm:size-8 cursor-pointer"
                                        on:click=move |_| stop_square.run(())
                                    >
                                        <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                                            <rect x="2" y="2" width="8" height="8" rx="1.5" fill="currentColor" />
                                        </svg>
                                    </button>
                                })}
                                <button
                                    type="button"
                                    data-send-label=label_attr
                                    aria-label=aria
                                    title=title
                                    disabled=!enabled
                                    class=class
                                    on:click=move |ev: web_sys::MouseEvent| {
                                        if matches!(action, ComposerAction::Interrupt) {
                                            stop_primary.run(());
                                            return;
                                        }
                                        if ev.meta_key() || ev.ctrl_key() {
                                            opposite.run(());
                                        } else {
                                            submit.run(());
                                        }
                                    }
                                >
                                    {match action {
                                        ComposerAction::Send => view! { <Icon name="arrow-up" size=14 /> }.into_any(),
                                        ComposerAction::Interrupt => view! {
                                            <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                                                <rect x="2" y="2" width="8" height="8" rx="1.5" fill="currentColor" />
                                            </svg>
                                            <span>{label}</span>
                                        }.into_any(),
                                        _ => view! { <span>{label}</span> }.into_any(),
                                    }}
                                </button>
                            }
                        }}
                    </div>
                </div>
            </div>

            // Context strip: project + branch.
            {move || project_name.get().map(|name| view! {
                <ContextStrip project_name=name project_path=project_path />
            })}

            {hero.then(|| view! {
                <p class="mt-4 flex items-center justify-center gap-1.5 text-label-sm text-rambutan-500">
                    <Kbd>"/"</Kbd><span>"commands"</span>
                    <span aria-hidden="true">"·"</span>
                    <Kbd>"@"</Kbd><span>"files"</span>
                    <span aria-hidden="true">"·"</span>
                    <Kbd>"⌘K"</Kbd><span>"palette"</span>
                </p>
            })}
        </div>
    }
}

/// What an answer to a permission request did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AnswerOutcome {
    /// The server passed the decision on.
    Answered,
    /// The server no longer has the request (the turn ended or the provider
    /// withdrew it): nothing is waiting on an answer.
    Gone,
    /// The answer may not have arrived; the request is still waiting.
    Failed(String),
}

fn answer_outcome(result: &Result<(), ApiError>) -> AnswerOutcome {
    match result {
        Ok(()) => AnswerOutcome::Answered,
        Err(error) if error.is_status(404) => AnswerOutcome::Gone,
        Err(error) => AnswerOutcome::Failed(format!("Your answer was not sent: {error}")),
    }
}

#[component]
fn ApprovalPanel(
    request: PermissionRequest,
    pending: RwSignal<PendingPermissions>,
) -> impl IntoView {
    let (in_flight, set_in_flight) = signal(false);
    let error = RwSignal::new(None::<String>);
    let summary = permission_summary(&request.tool, &request.input);
    let tool = request.tool.clone();
    let request_id = StoredValue::new(request.id);
    let respond = move |decision: Decision| {
        if in_flight.get_untracked() {
            return;
        }
        set_in_flight.set(true);
        error.set(None);
        let id = request_id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = approve_permission(&id, decision).await;
            match answer_outcome(&result) {
                // Only this request is settled; others keep waiting.
                AnswerOutcome::Answered | AnswerOutcome::Gone => {
                    pending.try_update(|p| p.remove(&id));
                }
                // Keep the panel so the same buttons retry.
                AnswerOutcome::Failed(message) => {
                    error.try_set(Some(message));
                }
            }
            // The panel may already be gone (turn ended, request withdrawn).
            set_in_flight.try_set(false);
        });
    };
    let deny = move || Decision::Deny {
        reason: "Denied by user".into(),
    };
    view! {
        <div role="group" aria-label="Permission request" class="border-b border-longan-800/60">
            <div class="flex items-center gap-2 px-4 py-2">
                <Icon name="shield" size=14 class="shrink-0 text-durian-300" />
                <span class="shrink-0 text-label-sm text-rambutan-100">{tool}</span>
                <code class="min-w-0 flex-1 max-h-20 overflow-auto whitespace-pre text-code-sm text-rambutan-200 [scrollbar-width:thin]">{summary}</code>
                <button
                    type="button"
                    class="h-6 shrink-0 rounded-seed px-2 text-label-sm text-lychee-300 transition-colors hover:bg-longan-800/60 disabled:opacity-50 cursor-pointer"
                    disabled=move || in_flight.get()
                    on:click=move |_| respond(deny())
                >
                    "Decline"
                </button>
                <button
                    type="button"
                    class="h-6 shrink-0 rounded-seed px-2 text-label-sm text-rambutan-100 transition-colors hover:bg-longan-800/60 disabled:opacity-50 cursor-pointer"
                    disabled=move || in_flight.get()
                    on:click=move |_| respond(Decision::Allow)
                >
                    "Approve"
                </button>
            </div>
            {move || error.get().map(|message| view! {
                <p role="alert" class="px-4 pb-2 text-label-sm text-lychee-300">
                    {message}" Choose again to retry."
                </p>
            })}
        </div>
    }
}

#[component]
fn ContextMeter(usage: UsageInfo, model: String) -> impl IntoView {
    let used = context_usage(&usage);
    let window = context_window_for(&model);
    let pct = context_percentage(used, window);
    let radius = 9.75_f64;
    let circumference = 2.0 * std::f64::consts::PI * radius;
    let offset = circumference * (1.0 - pct / 100.0);
    let color = if pct > 90.0 { "#f06b65" } else { "#c3afa1" };
    let tip = format!(
        "Context window · {}% · {} / {}",
        pct.round().to_u64_saturating(),
        format_tokens(used),
        format_tokens(window)
    );
    view! {
        <Tooltip tip=tip.clone()>
            <span
                role="img"
                aria-label=tip
                class="inline-flex size-7 items-center justify-center rounded-orb text-rambutan-400"
            >
                <svg viewBox="0 0 24 24" width="20" height="20" class="-rotate-90" aria-hidden="true">
                    <circle cx="12" cy="12" r=radius fill="none" stroke="rgba(171,153,143,0.25)" stroke-width="3" />
                    <circle
                        cx="12" cy="12" r=radius fill="none"
                        stroke=color stroke-width="3" stroke-linecap="round"
                        stroke-dasharray=format!("{circumference}")
                        stroke-dashoffset=format!("{offset}")
                    />
                </svg>
            </span>
        </Tooltip>
    }
}

#[component]
fn ContextStrip(
    project_name: String,
    #[prop(into)] project_path: Signal<Option<String>>,
) -> impl IntoView {
    let SetProjectPickerOpen(set_project_picker_open) = expect_context::<SetProjectPickerOpen>();
    let GitStatusSignal(git) = expect_context::<GitStatusSignal>();
    let is_streaming = expect_context::<crate::state::IsStreaming>().0;

    let branches = RwSignal::new(Vec::<String>::new());
    let current = RwSignal::new(Option::<String>::None);
    let BranchMenuOpen(branch_open) = expect_context::<BranchMenuOpen>();
    let WorkspaceMenuOpen(workspace_open) = expect_context::<WorkspaceMenuOpen>();
    let RunChoiceSignal(run_choice) = expect_context::<RunChoiceSignal>();
    let BaseBranchSignal(base_branch) = expect_context::<BaseBranchSignal>();
    let PreviousWorktreeSignal(previous_worktree) = expect_context::<PreviousWorktreeSignal>();
    let ActiveThreadId(active_thread_id) = expect_context::<ActiveThreadId>();
    let ThreadsSignal(threads) = expect_context::<ThreadsSignal>();
    let started = Signal::derive(move || {
        active_thread_id.get().is_some_and(|id| {
            threads.with(|rows| {
                rows.iter()
                    .find(|thread| thread.id == id)
                    .is_some_and(|thread| thread.run_locked || thread.worktree_path.is_some())
            })
        })
    });

    let refresh = move || {
        let Some(path) = project_path.get_untracked() else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            // Not a repository (or unreachable): show no branches.
            let resp = list_git_branches(&path).await.unwrap_or_else(|error| {
                log::debug!("branch strip: no branches for {path}: {error}");
                Default::default()
            });
            // The strip unmounts when the thread changes mid-fetch.
            let _ = branches.try_set(resp.branches);
            let _ = current.try_set(resp.current);
            match git_status(&path).await {
                Ok(status) => git.set(status),
                // The last known status stays until a refresh succeeds.
                Err(error) => log::debug!("branch strip: git status failed for {path}: {error}"),
            }
        });
    };
    // Fetch on mount / project change, and again when a turn finishes.
    Effect::new(move |_| {
        project_path.track();
        refresh();
    });
    Effect::new(move |prev: Option<bool>| {
        let streaming = is_streaming.get();
        if prev == Some(true) && !streaming {
            refresh();
        }
        streaming
    });

    let pick_branch = move |branch: String| {
        branch_open.set(false);
        let choice = run_choice.get_untracked();
        // A new thread's branch is the base of the worktree. It does not
        // switch the checkout. Once the thread has a place, the menu checks
        // that place out, which is what the branch strip did before.
        if !started.get_untracked() && choice == WorkspaceChoice::NewWorktree {
            base_branch.set(branch);
            return;
        }
        if !started.get_untracked() && choice == WorkspaceChoice::PreviousWorktree {
            return;
        }
        let Some(path) = project_path.get_untracked() else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            match checkout_git_branch(&path, &branch).await {
                Ok(()) => refresh(),
                Err(e) => log::warn!("checkout failed: {e}"),
            }
        });
    };

    let branch_label = Signal::derive(move || {
        if !started.get() && run_choice.get() == WorkspaceChoice::NewWorktree {
            let base = base_branch.get();
            if !base.is_empty() {
                return Some(base);
            }
        }
        if !started.get() && run_choice.get() == WorkspaceChoice::PreviousWorktree {
            if let Some(item) = previous_worktree.get() {
                return Some(item.branch);
            }
        }
        current.get()
    });
    let branch_title = Signal::derive(move || {
        if !started.get() && run_choice.get() == WorkspaceChoice::NewWorktree {
            "Base branch (⌘⇧G)".to_string()
        } else if !started.get() && run_choice.get() == WorkspaceChoice::PreviousWorktree {
            "Previous worktree (⌘⇧G)".to_string()
        } else {
            "Switch branch (⌘⇧G)".to_string()
        }
    });
    let workspace_title = Signal::derive(move || {
        if started.get() {
            "Workspace for the next thread (⌘⇧X)".to_string()
        } else {
            "Where this thread runs (⌘⇧X)".to_string()
        }
    });

    let dirty_count = Signal::derive(move || git.with(|g| g.dirty.len()));

    view! {
        <div
            class="mx-auto -mt-3 flex w-[calc(100%-2.75rem)] items-center gap-1 rounded-b-lychee bg-longan-900/70 px-2 pb-1 pt-4 text-label-sm text-rambutan-500 inset-ring inset-ring-white/5"
            data-git-branch=move || current.get().unwrap_or_default()
        >
            <button
                type="button"
                class="inline-flex h-6 min-w-0 items-center gap-1.5 rounded-seed px-1.5 transition-colors hover:bg-longan-800/50 hover:text-rambutan-200 cursor-pointer"
                title="Change project"
                on:click=move |_| set_project_picker_open.set(true)
            >
                <Icon name="folder" size=12 class="shrink-0" />
                <span class="truncate">{project_name}</span>
            </button>
            <span class="flex-1"></span>
            <Menu open=workspace_open align=MenuAlign::End side=MenuSide::Top label="Workspace" class="min-w-[16rem]"
                trigger=move || view! {
                    <button
                        type="button"
                        class="inline-flex h-6 max-w-[14rem] items-center gap-1.5 rounded-seed px-1.5 transition-colors hover:bg-longan-800/50 hover:text-rambutan-200 cursor-pointer"
                        data-run-choice=move || run_choice.get().as_str()
                        aria-haspopup="menu"
                        aria-expanded=move || workspace_open.get().to_string()
                        title=move || workspace_title.get()
                        on:click=move |_| {
                            branch_open.set(false);
                            workspace_open.update(|open| *open = !*open)
                        }
                    >
                        <Icon name="folder" size=12 class="shrink-0" />
                        <span class="truncate">{move || run_choice.get().label()}</span>
                        <Icon name="chevron-down" size=12 class="shrink-0" />
                    </button>
                }
            >
                {move || {
                    let mut choices = vec![
                        WorkspaceChoice::Checkout,
                        WorkspaceChoice::NewWorktree,
                    ];
                    if previous_worktree.get().is_some() {
                        choices.push(WorkspaceChoice::PreviousWorktree);
                    }
                    choices.into_iter().map(|choice| {
                        let detail = (choice == WorkspaceChoice::PreviousWorktree)
                            .then(|| previous_worktree.get().map(|item| item.label))
                            .flatten()
                            .unwrap_or_default();
                        view! {
                            <MenuItem
                                selected=Signal::derive(move || run_choice.get() == choice)
                                description=detail
                                on_select=Callback::new(move |_| {
                                    run_choice.set(choice);
                                    workspace_open.set(false);
                                })
                            >
                                <span data-workspace-option=choice.as_str()>{choice.label()}</span>
                            </MenuItem>
                        }
                    }).collect_view()
                }}
            </Menu>
            {move || branch_label.get().map(|branch| {
                let show_dirty = started.get() || run_choice.get() == WorkspaceChoice::Checkout;
                view! {
                <Menu open=branch_open align=MenuAlign::End side=MenuSide::Top label="Switch branch" class="max-h-72 overflow-y-auto"
                    trigger=move || {
                        let b = branch.clone();
                        view! {
                            <button
                                type="button"
                                class="inline-flex h-6 max-w-[16rem] items-center gap-1.5 rounded-seed px-1.5 transition-colors hover:bg-longan-800/50 hover:text-rambutan-200 cursor-pointer"
                                data-branch-chip=""
                                data-base-branch=move || base_branch.get()
                                aria-haspopup="menu"
                                aria-expanded=move || branch_open.get().to_string()
                                title=move || branch_title.get()
                                on:click=move |_| {
                                    workspace_open.set(false);
                                    branch_open.update(|v| *v = !*v)
                                }
                            >
                                <Icon name="git-branch" size=12 class="shrink-0" />
                                <span class="truncate">{b}</span>
                                {move || (show_dirty && dirty_count.get() > 0).then(|| view! {
                                    <span class="rounded-seed bg-durian-400/15 px-1 tabular-nums text-durian-300" title="Uncommitted changes">
                                        "•"{dirty_count.get()}
                                    </span>
                                })}
                                <Icon name="chevron-down" size=12 class="shrink-0" />
                            </button>
                        }
                    }
                >
                    <For
                        each=move || branches.get()
                        key=|b| b.clone()
                        children=move |b| {
                            let b_sel = b.clone();
                            let b_pick = b.clone();
                            view! {
                                <MenuItem
                                    selected=Signal::derive(move || {
                                        if !started.get() && run_choice.get() == WorkspaceChoice::NewWorktree {
                                            let base = base_branch.get();
                                            if !base.is_empty() {
                                                return base == b_sel;
                                            }
                                        }
                                        current.get().as_deref() == Some(b_sel.as_str())
                                    })
                                    on_select=Callback::new(move |_| pick_branch(b_pick.clone()))
                                >
                                    <Icon name="git-branch" size=12 class="text-rambutan-500" />
                                    <span class="truncate">{b.clone()}</span>
                                </MenuItem>
                            }
                        }
                    />
                </Menu>
            }})}
        </div>
    }
}

fn transfer_has_files(transfer: Option<&web_sys::DataTransfer>) -> bool {
    let Some(transfer) = transfer else {
        return false;
    };
    transfer
        .types()
        .iter()
        .any(|kind| kind.as_string().as_deref() == Some("Files"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed answer keeps the request (the panel stays to retry); an
    /// answer the server no longer needs clears it like a success.
    #[test]
    fn answers_settle_only_when_delivered_or_moot() {
        assert_eq!(answer_outcome(&Ok(())), AnswerOutcome::Answered);
        assert_eq!(
            answer_outcome(&Err(ApiError::http(404, r#"{"error":"unknown request"}"#))),
            AnswerOutcome::Gone
        );
        for error in [
            ApiError::Network("offline".into()),
            ApiError::http(500, ""),
            ApiError::Decode("x".into()),
        ] {
            assert!(matches!(
                answer_outcome(&Err(error)),
                AnswerOutcome::Failed(message) if message.starts_with("Your answer was not sent")
            ));
        }
    }

    #[test]
    fn send_disabled_reasons() {
        assert_eq!(
            send_disabled_reason("   ", false),
            Some("Type a message to send")
        );
        assert_eq!(send_disabled_reason("hi", true), None);
        assert_eq!(send_disabled_reason("hi", false), None);
        let over = "a".repeat(bruhs_models::logic::composer_text::MESSAGE_CHAR_LIMIT + 1);
        assert_eq!(send_disabled_reason(&over, false), Some(MESSAGE_LIMIT_HINT));
    }

    #[test]
    fn send_button_label_follows_the_follow_up_action() {
        assert_eq!(
            action_label(composer_action(
                false,
                false,
                false,
                false,
                false,
                FollowUp::default(),
            )),
            "Send"
        );
        assert_eq!(
            action_label(composer_action(
                true,
                false,
                false,
                false,
                true,
                FollowUp::default(),
            )),
            "Queue"
        );
        assert_eq!(
            action_label(composer_action(
                true,
                true,
                false,
                false,
                true,
                FollowUp::default(),
            )),
            "Interrupt"
        );
        assert_eq!(
            action_label(composer_action(
                false,
                true,
                false,
                true,
                true,
                FollowUp::default(),
            )),
            "Resume"
        );
    }

    #[test]
    fn plan_toggle_restores_previous_mode() {
        let mut s = PlanToggle::new(PermissionMode::AcceptEdits);
        assert_eq!(s.toggle(PermissionMode::AcceptEdits), PermissionMode::Plan);
        assert_eq!(s.toggle(PermissionMode::Plan), PermissionMode::AcceptEdits);
        assert_eq!(s.toggle(PermissionMode::Default), PermissionMode::Plan);
        let mut started_in_plan = PlanToggle::new(PermissionMode::Plan);
        assert_eq!(
            started_in_plan.toggle(PermissionMode::Plan),
            PermissionMode::Default
        );
    }

    #[test]
    fn permission_summary_picks_the_salient_field() {
        assert_eq!(
            permission_summary("Bash", &serde_json::json!({"command":"rm -rf dist"})),
            "rm -rf dist"
        );
        assert_eq!(
            permission_summary("Edit", &serde_json::json!({"file_path":"src/a.rs"})),
            "src/a.rs"
        );
        assert_eq!(
            permission_summary("Other", &serde_json::json!({"k":1})),
            "{\"k\":1}"
        );
    }

    #[test]
    fn context_usage_counts_cache_tokens() {
        let u = UsageInfo {
            input_tokens: 10,
            output_tokens: 99,
            cache_read_tokens: 20,
            cache_creation_tokens: 5,
            context_tokens: None,
        };
        assert_eq!(context_usage(&u), 35, "older rows: estimate");
        let reported = UsageInfo {
            context_tokens: Some(30),
            ..u
        };
        assert_eq!(context_usage(&reported), 30, "the reported window wins");
    }

    #[test]
    fn model_label_falls_back_to_id() {
        assert_eq!(model_label("claude-sonnet-4-6"), "Claude Sonnet 4.6");
        assert_eq!(model_label("mystery-model"), "mystery-model");
    }
}
