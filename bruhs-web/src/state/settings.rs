//! Model, permissions, sign-in, transcription and personality settings.

use bruhs_models::logic::compact::{autocompact_tokens, IdleCompact};
use bruhs_models::logic::keybindings::{stored_keybindings, Keybinding};
use bruhs_models::logic::run_place::{claude_effort, RememberedWorktree, WorkspaceChoice};
use bruhs_models::TranscriptionSettings;

use super::*;

/// Model id the next turn uses: the open thread's, else [`DefaultModel`].
#[derive(Copy, Clone)]
pub struct CurrentModel(pub ReadSignal<String>);
/// Model a thread with no turn yet starts on: Settings → Model, and the
/// last model picked in the composer. Saved; opening a thread on another
/// model does not change it.
#[derive(Copy, Clone)]
pub struct DefaultModel(pub RwSignal<String>);
#[derive(Copy, Clone)]
pub struct SetCurrentModel(pub WriteSignal<String>);

/// Claude Code CLI sign-in state ([`CodexAuthSignal`] is Codex's).
#[derive(Copy, Clone)]
pub struct ClaudeAuthSignal(pub ReadSignal<AuthStatus>);
#[derive(Copy, Clone)]
pub struct SetClaudeAuthSignal(pub WriteSignal<AuthStatus>);
#[derive(Copy, Clone)]
pub struct SidebarOpen(pub ReadSignal<bool>);
#[derive(Copy, Clone)]
pub struct SetSidebarOpen(pub WriteSignal<bool>);

#[derive(Copy, Clone)]
pub struct ModelSelectOpen(pub ReadSignal<bool>);
#[derive(Copy, Clone)]
pub struct SetModelSelectOpen(pub WriteSignal<bool>);

#[derive(Copy, Clone)]
pub struct PersonalitySettingsSignal(pub ReadSignal<PersonalitySettings>);
#[derive(Copy, Clone)]
pub struct SetPersonalitySettingsSignal(pub WriteSignal<PersonalitySettings>);

#[derive(Copy, Clone)]
pub struct PermissionModeSignal(pub ReadSignal<PermissionMode>);
#[derive(Copy, Clone)]
pub struct SetPermissionModeSignal(pub WriteSignal<PermissionMode>);

#[derive(Copy, Clone)]
pub struct WhisperModelSignal(pub ReadSignal<WhisperModel>);
#[derive(Copy, Clone)]
pub struct SetWhisperModelSignal(pub WriteSignal<WhisperModel>);

#[derive(Copy, Clone)]
pub struct LiveWhisperModelSignal(pub ReadSignal<WhisperModel>);
#[derive(Copy, Clone)]
pub struct SetLiveWhisperModelSignal(pub WriteSignal<WhisperModel>);

#[derive(Copy, Clone)]
pub struct TranscriptionSettingsSignal(pub RwSignal<TranscriptionSettings>);

#[derive(Copy, Clone)]
pub struct DiarizeSettingsSignal(pub ReadSignal<DiarizeSettings>);
#[derive(Copy, Clone)]
pub struct SetDiarizeSettingsSignal(pub WriteSignal<DiarizeSettings>);

/// Preferred editor id for ⌘O / "Open in editor" (`bruhs.editor`).
#[derive(Copy, Clone)]
pub struct EditorSignal(pub RwSignal<String>);

/// Where the next new thread runs (`bruhs.runChoice`).
#[derive(Copy, Clone)]
pub struct RunChoiceSignal(pub RwSignal<bruhs_models::logic::run_place::WorkspaceChoice>);

/// Claude effort for the next turn (`bruhs.effort`). `None` leaves the CLI default.
#[derive(Copy, Clone)]
pub struct ClaudeEffortSignal(pub RwSignal<Option<String>>);

/// Stored follow-up is Steer (`bruhs.followUpBehavior`). Codex still queues.
#[derive(Copy, Clone)]
pub struct FollowUpSteerSignal(pub RwSignal<bool>);

/// What agents may do with other threads. Stored as `bruhs.threadAccess`.
#[derive(Copy, Clone)]
pub struct ThreadAccessSignal(pub RwSignal<bruhs_models::thread_link::ThreadAccess>);

/// Agent-to-agent messages allowed in a row. Stored as `bruhs.threadHops`.
#[derive(Copy, Clone)]
pub struct ThreadHopsSignal(pub RwSignal<u32>);

/// Claude `--autocompact` (`bruhs.claudeAutoCompact`). Empty omits the flag.
#[derive(Copy, Clone)]
pub struct ClaudeAutoCompactSignal(pub RwSignal<Option<String>>);

/// Idle compact mode (`bruhs.compactIdle`).
#[derive(Copy, Clone)]
pub struct CompactIdleSignal(pub RwSignal<IdleCompact>);

/// This message sends in full. The return chip sets it. Not persisted.
#[derive(Copy, Clone)]
pub struct CompactFullSignal(pub RwSignal<bool>);

/// Base branch for a new worktree. Empty means the current git branch.
#[derive(Copy, Clone)]
pub struct BaseBranchSignal(pub RwSignal<String>);

/// Last worktree this browser created (`bruhs.previousWorktree`).
#[derive(Copy, Clone)]
pub struct PreviousWorktreeSignal(
    pub RwSignal<Option<bruhs_models::logic::run_place::RememberedWorktree>>,
);

/// ⌘⇧G branch menu.
#[derive(Copy, Clone)]
pub struct BranchMenuOpen(pub RwSignal<bool>);

/// ⌘⇧X workspace menu.
#[derive(Copy, Clone)]
pub struct WorkspaceMenuOpen(pub RwSignal<bool>);

/// ⌘⇧E effort menu.
#[derive(Copy, Clone)]
pub struct EffortMenuOpen(pub RwSignal<bool>);

/// Shortcut rules (`bruhs.keybindings`). The command id is the stable name.
#[derive(Copy, Clone)]
pub struct KeybindingsSignal(pub RwSignal<Vec<Keybinding>>);

pub use bruhs_models::logic::editor::DEFAULT_EDITOR_ID;

/// Codex CLI sign-in state ([`ClaudeAuthSignal`] is Claude's).
#[derive(Copy, Clone)]
pub struct CodexAuthSignal(pub ReadSignal<AuthStatus>);
#[derive(Copy, Clone)]
pub struct SetCodexAuthSignal(pub WriteSignal<AuthStatus>);

/// Model, permission mode, sign-in, editor, transcription and personality
/// settings; the persisted ones are saved to `localStorage` on change.
pub(super) fn provide() {
    let (model_select_open, set_model_select_open) = signal(false);
    let initial_model = load_pref("bruhs.model")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| bruhs_models::agent::DEFAULT_AGENT_MODEL.to_string());
    let default_model = RwSignal::new(initial_model.clone());
    let (current_model, set_current_model) = signal(initial_model);
    Effect::new(move |_| {
        save_pref("bruhs.model", &default_model.get());
    });

    let initial_permission_mode = load_pref("bruhs.permissionMode")
        .map(|s| PermissionMode::parse_lossy(&s))
        .unwrap_or_default();
    let (permission_mode, set_permission_mode) = signal(initial_permission_mode);
    Effect::new(move |_| {
        save_pref("bruhs.permissionMode", permission_mode.get().as_str());
    });

    let editor_id = RwSignal::new(
        load_pref("bruhs.editor")
            .filter(|id| bruhs_models::logic::editor::editor_by_id(id).is_some())
            .unwrap_or_else(|| DEFAULT_EDITOR_ID.to_string()),
    );
    Effect::new(move |_| save_pref("bruhs.editor", &editor_id.get()));

    let (auth_status, set_auth_status) = signal(AuthStatus::default());
    let (codex_auth, set_codex_auth) = signal(AuthStatus::default());

    let initial_whisper_model: WhisperModel =
        load_json_pref("bruhs.whisper_model").unwrap_or_default();
    let (whisper_model, set_whisper_model) = signal(initial_whisper_model);
    Effect::new(move |_| {
        let current = whisper_model.get();
        save_json_pref("bruhs.whisper_model", &current);
    });

    let initial_live_model: WhisperModel =
        load_json_pref("bruhs.whisper_model_live").unwrap_or_default();
    let (live_whisper_model, set_live_whisper_model) = signal(initial_live_model);
    Effect::new(move |_| {
        let current = live_whisper_model.get();
        save_json_pref("bruhs.whisper_model_live", &current);
    });

    let transcription = RwSignal::new(
        load_json_pref::<TranscriptionSettings>("bruhs.transcription").unwrap_or_default(),
    );
    Effect::new(move |_| save_json_pref("bruhs.transcription", &transcription.get()));
    provide_context(TranscriptionSettingsSignal(transcription));

    let initial_personality: PersonalitySettings =
        load_json_pref("bruhs.personality").unwrap_or_default();
    let (personality_settings, set_personality_settings) = signal(initial_personality);
    Effect::new(move |_| {
        let current = personality_settings.get();
        save_json_pref("bruhs.personality", &current);
    });

    let initial_diarize: DiarizeSettings = load_json_pref("bruhs.diarize").unwrap_or_default();
    let (diarize_settings, set_diarize_settings) = signal(initial_diarize);
    Effect::new(move |_| {
        let current = diarize_settings.get();
        save_json_pref("bruhs.diarize", &current);
    });

    provide_context(ModelSelectOpen(model_select_open));
    provide_context(SetModelSelectOpen(set_model_select_open));
    provide_context(CurrentModel(current_model));
    provide_context(SetCurrentModel(set_current_model));
    provide_context(DefaultModel(default_model));
    provide_context(PermissionModeSignal(permission_mode));
    provide_context(SetPermissionModeSignal(set_permission_mode));
    provide_context(EditorSignal(editor_id));
    provide_context(ClaudeAuthSignal(auth_status));
    provide_context(SetClaudeAuthSignal(set_auth_status));
    provide_context(CodexAuthSignal(codex_auth));
    provide_context(SetCodexAuthSignal(set_codex_auth));
    provide_context(WhisperModelSignal(whisper_model));
    provide_context(SetWhisperModelSignal(set_whisper_model));
    provide_context(LiveWhisperModelSignal(live_whisper_model));
    provide_context(SetLiveWhisperModelSignal(set_live_whisper_model));
    provide_context(DiarizeSettingsSignal(diarize_settings));
    provide_context(SetDiarizeSettingsSignal(set_diarize_settings));
    provide_context(PersonalitySettingsSignal(personality_settings));
    provide_context(SetPersonalitySettingsSignal(set_personality_settings));

    let run_choice = RwSignal::new(
        load_pref("bruhs.runChoice")
            .map(|value| WorkspaceChoice::parse(&value))
            .unwrap_or_default(),
    );
    Effect::new(move |_| save_pref("bruhs.runChoice", run_choice.get().as_str()));
    let claude_effort_pref = RwSignal::new(
        load_pref("bruhs.effort").and_then(|value| claude_effort(&value).map(str::to_string)),
    );
    Effect::new(move |_| {
        save_pref(
            "bruhs.effort",
            claude_effort_pref.get().as_deref().unwrap_or(""),
        );
    });
    let base_branch = RwSignal::new(String::new());
    let previous_worktree = RwSignal::new(
        load_pref("bruhs.previousWorktree")
            .filter(|value| !value.is_empty())
            .and_then(|value| serde_json::from_str::<RememberedWorktree>(&value).ok())
            .filter(|item| !item.path.is_empty()),
    );
    Effect::new(move |_| match previous_worktree.get() {
        Some(item) => {
            if let Ok(json) = serde_json::to_string(&item) {
                save_pref("bruhs.previousWorktree", &json);
            }
        }
        None => save_pref("bruhs.previousWorktree", ""),
    });

    provide_context(RunChoiceSignal(run_choice));
    provide_context(ClaudeEffortSignal(claude_effort_pref));
    let follow_up_steer =
        RwSignal::new(load_pref("bruhs.followUpBehavior").as_deref() == Some("steer"));
    Effect::new(move |_| {
        save_pref(
            "bruhs.followUpBehavior",
            if follow_up_steer.get() {
                "steer"
            } else {
                "queue"
            },
        );
    });
    let claude_autocompact = RwSignal::new(
        load_pref("bruhs.claudeAutoCompact")
            .and_then(|value| autocompact_tokens(&value).map(|tokens| tokens.to_string())),
    );
    Effect::new(move |_| {
        save_pref(
            "bruhs.claudeAutoCompact",
            claude_autocompact.get().as_deref().unwrap_or(""),
        );
    });
    let compact_idle = RwSignal::new(
        load_pref("bruhs.compactIdle")
            .map(|value| IdleCompact::parse(&value))
            .unwrap_or_default(),
    );
    Effect::new(move |_| save_pref("bruhs.compactIdle", compact_idle.get().as_str()));
    provide_context(FollowUpSteerSignal(follow_up_steer));
    let thread_access = RwSignal::new(
        load_pref("bruhs.threadAccess")
            .map(|value| bruhs_models::thread_link::ThreadAccess::parse(&value))
            .unwrap_or_default(),
    );
    Effect::new(move |_| save_pref("bruhs.threadAccess", thread_access.get().as_str()));
    let thread_hops = RwSignal::new(bruhs_models::thread_link::parse_hops(
        &load_pref("bruhs.threadHops").unwrap_or_default(),
    ));
    Effect::new(move |_| save_pref("bruhs.threadHops", &thread_hops.get().to_string()));
    provide_context(ThreadAccessSignal(thread_access));
    provide_context(ThreadHopsSignal(thread_hops));
    provide_context(ClaudeAutoCompactSignal(claude_autocompact));
    provide_context(CompactIdleSignal(compact_idle));
    provide_context(CompactFullSignal(RwSignal::new(false)));
    provide_context(BaseBranchSignal(base_branch));
    provide_context(PreviousWorktreeSignal(previous_worktree));
    provide_context(BranchMenuOpen(RwSignal::new(false)));
    provide_context(WorkspaceMenuOpen(RwSignal::new(false)));
    provide_context(EffortMenuOpen(RwSignal::new(false)));

    let keybindings = RwSignal::new(stored_keybindings(load_json_pref("bruhs.keybindings")));
    Effect::new(move |_| {
        let rules = keybindings.get();
        save_json_pref("bruhs.keybindings", &rules);
    });
    provide_context(KeybindingsSignal(keybindings));
}
