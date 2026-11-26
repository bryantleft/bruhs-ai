//! Settings → Keybindings. Each environment owns its shortcut file. The last
//! matching rule wins. `mod` is Command on macOS and Control elsewhere.
//!
//! GPUI owns dispatch for most commands. Sidebar undo stays out of the keymap
//! so a text field keeps ⌘Z. A rule whose `when` fails
//! swallows a modified chord, so a customized condition can turn a shortcut
//! off. A plain key such as `/` or `c` is left alone when its condition fails,
//! or the character would never reach the field.

mod condition;
mod draft;
pub(super) use draft::NewBinding;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BindingEditor {
    Existing,
    New,
}

use bruhs_ui::input::{Input, InputEvent, InputState};
use bruhs_ui::zoom::px;
use bruhs_ui::WindowExt;
use gpui::prelude::*;
use gpui::{
    div, Action, AnyElement, App, Context, Entity, Focusable, KeyBinding,
    KeyBindingContextPredicate, Keystroke, KeystrokeEvent, OperatingSystem, Role, SharedString,
    Subscription, Unbind, WeakEntity, Window,
};

use bruhs_models::logic::keybindings::{
    chord_claimed, chord_from_event, default_keybindings, gpui_shortcut, resolve, KeyContext,
    Keybinding, KeybindingEdit,
};

use crate::views::chat::{FindInThread, StashDraft};
use crate::views::diff_panel::FindInDiff;

use super::{
    CaptureWindow, CloseOverlays, ClosePanel, ContinueInWorkbench, CopyPullLink, CopyPullNumber,
    CopyResumeCommand, CopyThreadLink, CopyThreadMarkdown, FocusChatInput, FocusCommandBar,
    FocusCrew, ImportThread, JumpThread, MentionFile, NavigateBack, NavigateForward, NewThread,
    NextThread, OpenCapture, OpenInboxView, OpenKnowledgeView, OpenProjectInEditor, OpenSettings,
    OpenTaskCreateShortcut, OpenTodayView, PinThread, PrevThread, SaveKnowledgeDocument,
    SearchThreads, SettleThread, StopAll, ToggleBranchMenu, ToggleDiff, ToggleEffortMenu,
    ToggleModelPicker, TogglePanel, TogglePreview, ToggleSidebar, ToggleTasks, ToggleTerminal,
    ToggleWorkspaceMenu, UndoSidebar, UsePreviousWorktree, Workspace, SETTINGS_CODE_RADIUS_PX,
    SETTINGS_MODEL_LABEL_TEXT_SIZE_PX,
};

const KEY_INPUT_WIDTH_PX: f32 = 144.0;
const FIELD_HEIGHT_PX: f32 = crate::theme::CONTROL_PX;

#[derive(Clone)]
pub(super) struct InstalledBinding {
    shortcut: String,
    action: &'static str,
}

enum KeyDecision {
    Dictation,
    Dispatch(Box<dyn Action>),
    Usage(crate::views::usage::UsageShortcut),
    WorkspaceFocus(bool),
    ProjectAction(super::project_actions::ProjectAction, String),
    Undo,
    SidebarFocus(bool),
    /// Tab (true: Shift+Tab) in an open thread menu: close it and move on
    /// from its "…" button, before the window root's Tab binding runs.
    CloseThreadMenu(bool),
    /// Tab (true: Shift+Tab) in an open dialog: step within it, as the web
    /// dialog's focus trap does. The window root's Tab binding runs before
    /// the dialog's own key handler and would move focus to the page
    /// behind the dialog.
    DialogFocus(bool),
    Swallow,
    Pass,
}

pub(super) use bruhs_models::logic::keybindings::rules_from_saved;

/// Bindings `main` installs before the workspace exists. The first sync
/// unbinds them so a saved rule can move the key.
pub(super) fn startup_bindings() -> Vec<InstalledBinding> {
    const ROWS: &[(&str, &str)] = &[
        ("cmd-shift-t", "workspace::ToggleTasks"),
        ("cmd-b", "workspace::ToggleSidebar"),
        ("cmd-k", "workspace::FocusCommandBar"),
        ("/", "workspace::FocusChatInput"),
        ("c", "workspace::OpenTaskCreateShortcut"),
        ("escape", "workspace::CloseOverlays"),
        ("cmd-shift-p", "workspace::PinThread"),
        ("cmd-shift-s", "workspace::SettleThread"),
        ("cmd-shift-c", "workspace::CopyThreadLink"),
        ("cmd-d", "workspace::ToggleDiff"),
        ("cmd-alt-b", "workspace::TogglePanel"),
        ("cmd-w", "workspace::ClosePanel"),
        ("cmd-shift-m", "workspace::ToggleModelPicker"),
        ("cmd-n", "workspace::NewThread"),
        ("cmd-shift-o", "workspace::NewThread"),
        ("cmd-shift-n", "workspace::NewThread"),
        ("cmd-shift-[", "workspace::PrevThread"),
        ("cmd-shift-]", "workspace::NextThread"),
        ("cmd-[", "workspace::NavigateBack"),
        ("cmd-]", "workspace::NavigateForward"),
        ("cmd-/", "workspace::OpenSettings"),
        ("cmd-p", "workspace::MentionFile"),
        ("cmd-j", "workspace::ToggleTerminal"),
        ("cmd-shift-j", "workspace::TogglePreview"),
        ("cmd-shift-f", "workspace::SearchThreads"),
        ("cmd-shift-space", "workspace::OpenCapture"),
        ("cmd-shift-y", "workspace::OpenTodayView"),
        ("cmd-shift-k", "workspace::OpenKnowledgeView"),
        ("cmd-shift-i", "workspace::OpenInboxView"),
        ("cmd-shift-g", "workspace::ToggleBranchMenu"),
        ("cmd-shift-x", "workspace::ToggleWorkspaceMenu"),
        ("cmd-shift-e", "workspace::ToggleEffortMenu"),
        ("cmd-shift-l", "workspace::UsePreviousWorktree"),
        ("cmd-1", "workspace::JumpThread"),
        ("cmd-2", "workspace::JumpThread"),
        ("cmd-3", "workspace::JumpThread"),
        ("cmd-4", "workspace::JumpThread"),
        ("cmd-5", "workspace::JumpThread"),
        ("cmd-6", "workspace::JumpThread"),
        ("cmd-7", "workspace::JumpThread"),
        ("cmd-8", "workspace::JumpThread"),
        ("cmd-9", "workspace::JumpThread"),
        ("cmd-s", "chat::StashDraft"),
        ("cmd-f", "chat::FindInThread"),
        ("cmd-f", "diff_panel::FindInDiff"),
        ("cmd-s", "workspace::SaveKnowledgeDocument"),
    ];
    ROWS.iter()
        .map(|(shortcut, action)| InstalledBinding {
            shortcut: (*shortcut).to_string(),
            action,
        })
        .collect()
}

/// Action names a command installs. Sidebar undo is intentionally absent.
pub(super) fn binding_names(command: &str) -> &'static [&'static str] {
    match command {
        "command.palette" => &["workspace::FocusCommandBar"],
        "capture.open" => &["workspace::OpenCapture"],
        "view.today" => &["workspace::OpenTodayView"],
        "view.knowledge" => &["workspace::OpenKnowledgeView"],
        "view.inbox" => &["workspace::OpenInboxView"],
        "chat.new" => &["workspace::NewThread"],
        "sidebar.toggle" => &["workspace::ToggleSidebar"],
        "rightPanel.toggle" => &["workspace::TogglePanel"],
        "rightPanel.close" => &["workspace::ClosePanel"],
        "diff.toggle" => &["workspace::ToggleDiff"],
        "tasks.toggle" => &["workspace::ToggleTasks"],
        "model.picker" => &["workspace::ToggleModelPicker"],
        "workspace.menu" => &["workspace::ToggleWorkspaceMenu"],
        "branch.menu" => &["workspace::ToggleBranchMenu"],
        "effort.menu" => &["workspace::ToggleEffortMenu"],
        "worktree.previous" => &["workspace::UsePreviousWorktree"],
        "mention.file" => &["workspace::MentionFile"],
        "thread.previous" => &["workspace::PrevThread"],
        "thread.next" => &["workspace::NextThread"],
        "navigation.back" => &["workspace::NavigateBack"],
        "navigation.forward" => &["workspace::NavigateForward"],
        "thread.pin" => &["workspace::PinThread"],
        "thread.settle" => &["workspace::SettleThread"],
        "thread.copyLink" => &["workspace::CopyThreadLink"],
        "pull.copyLink" => &["workspace::CopyPullLink"],
        "pull.copyNumber" => &["workspace::CopyPullNumber"],
        "thread.import" => &["workspace::ImportThread"],
        "thread.copyResumeCommand" => &["workspace::CopyResumeCommand"],
        "thread.copyMarkdown" => &["workspace::CopyThreadMarkdown"],
        "thread.continueInWorkbench" => &["workspace::ContinueInWorkbench"],
        "settings.open" => &["workspace::OpenSettings"],
        "terminal.toggle" => &["workspace::ToggleTerminal"],
        "preview.toggle" => &["workspace::TogglePreview"],
        "draft.stash" => &["chat::StashDraft", "workspace::SaveKnowledgeDocument"],
        "snapshot.capture" => &["workspace::CaptureWindow"],
        "voice.dictate" => &["chat::ToggleDictation"],
        "search.open" => &["workspace::SearchThreads"],
        "chat.find" => &["chat::FindInThread", "diff_panel::FindInDiff"],
        "project.openInEditor" => &["workspace::OpenProjectInEditor"],
        "dialog.dismiss" => &["workspace::CloseOverlays"],
        "composer.focus" => &["workspace::FocusChatInput"],
        "crew.focus" => &["workspace::FocusCrew"],
        "agents.stopAll" => &["workspace::StopAll"],
        "task.new" => &["workspace::OpenTaskCreateShortcut"],
        "thread.undo" => &[],
        other
            if other.strip_prefix("thread.jump.").is_some_and(|digit| {
                digit.parse::<usize>().is_ok_and(|n| (1..=9).contains(&n))
            }) =>
        {
            &["workspace::JumpThread"]
        }
        _ => &[],
    }
}

pub(super) fn new_keybinding_inputs(
    rules: &[Keybinding],
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
    subscriptions: &mut Vec<Subscription>,
) -> (Entity<InputState>, Entity<InputState>) {
    let first = rules.first();
    let key = first.map(|rule| rule.key.clone()).unwrap_or_default();
    let when = first.and_then(|rule| rule.when.clone()).unwrap_or_default();
    let key_input = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(key)
            .placeholder("mod+k")
    });
    let when_input = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(when)
            .placeholder("when")
    });
    subscriptions.push(cx.subscribe_in(
        &key_input,
        window,
        |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            } else if commits_keybinding(event) {
                this.commit_keybinding_edit(window, cx);
            }
        },
    ));
    subscriptions.push(cx.subscribe_in(
        &when_input,
        window,
        |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            } else if commits_keybinding(event) {
                this.commit_keybinding_edit(window, cx);
            }
        },
    ));
    let watch = cx.weak_entity();
    subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
        intercept_keybinding(&watch, event, window, cx);
    }));
    (key_input, when_input)
}

fn commits_keybinding(event: &InputEvent) -> bool {
    matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
}

fn intercept_keybinding(
    workspace: &WeakEntity<Workspace>,
    event: &KeystrokeEvent,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(workspace) = workspace.upgrade() else {
        return;
    };
    let ws = workspace.read(cx);
    let focused_draft = ws
        .keybinding_draft
        .as_ref()
        .is_some_and(|draft| draft.focus.is_focused(window));
    let active = if focused_draft
        && (event.keystroke.key != "tab" || ws.keybinding_recording == Some(BindingEditor::New))
    {
        Some(BindingEditor::New)
    } else {
        ws.keybinding_recording.filter(|editor| {
            *editor == BindingEditor::Existing && ws.keybinding_record_focus.is_focused(window)
        })
    };
    if let Some(editor) = active {
        window.prevent_default();
        cx.stop_propagation();
        workspace.update(cx, |this, cx| {
            let stroke = &event.keystroke;

            this.keybinding_recording = None;
            if stroke.key != "escape" || stroke.modifiers.modified() {
                let modifiers = stroke.modifiers;
                let mut parts = Vec::new();
                if cfg!(target_os = "macos") {
                    if modifiers.platform {
                        parts.push("mod");
                    }
                    if modifiers.control {
                        parts.push("ctrl");
                    }
                } else {
                    if modifiers.control {
                        parts.push("mod");
                    }
                    if modifiers.platform {
                        parts.push("meta");
                    }
                }
                if modifiers.alt {
                    parts.push("alt");
                }
                if modifiers.shift {
                    parts.push("shift");
                }
                parts.push(&stroke.key);
                let shortcut = parts.join("+");
                match editor {
                    BindingEditor::Existing => {
                        this.keybinding_key_input
                            .update(cx, |f, cx| f.set_value(shortcut, window, cx));
                        this.commit_keybinding_edit(window, cx);
                    }
                    BindingEditor::New => {
                        if let Some(draft) = &mut this.keybinding_draft {
                            draft.key = shortcut;
                        }
                    }
                }
            } else if editor == BindingEditor::New {
                if let Some(draft) = &mut this.keybinding_draft {
                    draft.key.clear();
                }
            }
            cx.notify();
        });
        return;
    }
    if matches!(event.keystroke.key.as_str(), "enter" | "tab" | "escape")
        && workspace.update(cx, |ws, cx| {
            ws.handle_theme_css_key(&event.keystroke, window, cx)
        })
    {
        window.prevent_default();
        cx.stop_propagation();
        return;
    }
    let editable_input = window.has_focused_input(cx);
    if event
        .context_stack
        .iter()
        .any(|context| context.contains("RichComposer"))
    {
        if let Some(action) = bruhs_ui::input::composer_format_action(&event.keystroke) {
            cx.stop_propagation();
            window.dispatch_action(action, cx);
            return;
        }
    }
    let decision = workspace.update(cx, |this, cx| {
        this.key_decision(event, editable_input, window, cx)
    });
    match decision {
        KeyDecision::Dispatch(action) => {
            window.prevent_default();
            cx.stop_propagation();
            window.dispatch_action(action, cx);
        }
        KeyDecision::Usage(shortcut) => {
            if workspace.update(cx, |this, cx| this.apply_usage_shortcut(shortcut, cx)) {
                window.prevent_default();
                cx.stop_propagation();
            }
        }
        KeyDecision::WorkspaceFocus(backwards) => {
            if workspace.update(cx, |this, cx| {
                this.step_workspace_focus(backwards, window, cx)
            }) {
                window.prevent_default();
                cx.stop_propagation();
            }
        }
        KeyDecision::Dictation => {
            cx.stop_propagation();
            window.prevent_default();
            workspace.update(cx, |this, cx| {
                this.press_voice_shortcut(&event.keystroke, cx)
            });
        }
        KeyDecision::ProjectAction(action, cwd) => {
            cx.stop_propagation();
            workspace.update(cx, |this, cx| this.run_action_in(&action, cwd, window, cx));
        }
        KeyDecision::Undo => {
            cx.stop_propagation();
            workspace.update(cx, |this, cx| this.undo_sidebar(window, cx));
        }
        KeyDecision::CloseThreadMenu(backwards) => {
            cx.stop_propagation();
            workspace.update(cx, |this, cx| {
                this.close_thread_menu(window, cx);
                this.sidebar_viewport.borrow().tab(backwards, window, cx);
                cx.notify();
            });
        }
        KeyDecision::DialogFocus(backwards) => {
            cx.stop_propagation();
            workspace.update(cx, |this, cx| this.step_modal_focus(backwards, window, cx));
        }
        KeyDecision::SidebarFocus(backwards) => {
            workspace.update(cx, |this, cx| {
                if this.sidebar_viewport.borrow().tab(backwards, window, cx) {
                    cx.stop_propagation();
                    cx.notify();
                }
            });
        }
        KeyDecision::Swallow => cx.stop_propagation(),
        KeyDecision::Pass => {}
    }
}

impl Workspace {
    pub(super) fn sync_keybindings(&mut self, cx: &mut Context<'_, Self>) {
        let mut remove = Vec::new();
        for installed in &self.keybinding_installed {
            if !parses(&installed.shortcut) {
                continue;
            }
            let unbind = Box::new(Unbind(installed.action.into()));
            if let Ok(binding) =
                KeyBinding::load(&installed.shortcut, unbind, None, false, None, cx)
            {
                remove.push(binding);
            }
        }
        if !remove.is_empty() {
            cx.bind_keys(remove);
        }
        let mut add = Vec::new();
        let mut installed = Vec::new();
        for rule in &self.keybindings {
            push_rule_bindings(rule, &mut add, &mut installed);
        }
        let add: Vec<KeyBinding> = add
            .into_iter()
            .filter_map(|pending| pending.load(cx))
            .collect();
        if !add.is_empty() {
            cx.bind_keys(add);
        }
        self.keybinding_installed = installed;
        self.sync_capture_shortcut(cx);
    }

    fn key_decision(
        &self,
        event: &KeystrokeEvent,
        editable_input: bool,
        window: &Window,
        cx: &App,
    ) -> KeyDecision {
        let modifiers = event.keystroke.modifiers;
        // Only while focus is in this window's thread menu: the interceptor
        // sees every window's keys.
        if event.keystroke.key == "tab"
            && !modifiers.platform
            && !modifiers.control
            && !modifiers.alt
            && self.thread_menu_id.is_some()
            && self
                .thread_menu_items
                .iter()
                .any(|item| item.is_focused(window))
        {
            return KeyDecision::CloseThreadMenu(modifiers.shift);
        }
        // Only while focus is in this window's open dialog.
        if event.keystroke.key == "tab"
            && !modifiers.platform
            && !modifiers.control
            && !modifiers.alt
            && self.overlay_open()
            && self.modal_contains_focus(window, cx)
        {
            return KeyDecision::DialogFocus(modifiers.shift);
        }
        let chord = chord_from_event(
            "",
            &event.keystroke.key,
            modifiers.platform,
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
        );
        let ctx = self.key_context(event, editable_input, window, cx);
        let mac = cfg!(target_os = "macos");
        let (matched, shared) = bruhs_models::logic::keybindings::matching_binding(
            &self.keybindings,
            &chord,
            &ctx,
            mac,
        );
        if ctx.usage_page_open && !editable_input {
            let usage_rule = matched.or_else(|| {
                if !mac || !chord.command || chord.shift || chord.alt {
                    return None;
                }
                // GPUI on macOS folds Shift plus a digit into its symbol.
                // An explicitly assigned symbol shortcut takes precedence.
                let key = match chord.key.as_str() {
                    "!" => "1",
                    "@" => "2",
                    "#" => "3",
                    "$" => "4",
                    _ => return None,
                };
                let shifted_digit = chord_from_event("", key, true, chord.ctrl, false, true);
                bruhs_models::logic::keybindings::matching_binding(
                    &self.keybindings,
                    &shifted_digit,
                    &ctx,
                    mac,
                )
                .0
            });
            if let Some(shortcut) = usage_rule
                .and_then(|rule| crate::views::usage::UsageShortcut::from_command(&rule.command))
            {
                return KeyDecision::Usage(shortcut);
            }
        }
        if let Some(rule) = matched {
            let command = rule.command.as_str();
            if matches!(command, "workspace.focusNext" | "workspace.focusPrevious") {
                if !self.workspace_focus_handle.contains_focused(window, cx)
                    || (event.keystroke.key == "f6"
                        && event
                            .context_stack
                            .iter()
                            .any(|context| context.contains("LiveComputerInput")))
                {
                    return KeyDecision::Pass;
                }
                return KeyDecision::WorkspaceFocus(command == "workspace.focusPrevious");
            }
            if command == "voice.dictate"
                && !self.overlay_open()
                && !ctx.terminal_focus
                && event.context_stack.iter().any(|stack| {
                    stack
                        .primary()
                        .is_some_and(|entry| entry.key.as_ref() == "Chat")
                })
            {
                return KeyDecision::Dictation;
            }
            if command == "thread.undo" && self.sidebar_undo.is_some() {
                return KeyDecision::Undo;
            }
            if shared && self.workspace_focus_handle.contains_focused(window, cx) {
                // GPUI uses static contexts. A user-defined condition can
                // select an earlier binding than the last registered action.
                let mut candidates = Vec::new();
                push_rule_bindings(rule, &mut candidates, &mut Vec::new());
                let action = candidates
                    .into_iter()
                    .filter_map(|binding| {
                        let rank = match binding.context.as_deref() {
                            Some(name) => {
                                event
                                    .context_stack
                                    .iter()
                                    .rposition(|ctx| ctx.contains(name))?
                                    + 1
                            }
                            None => 0,
                        };
                        Some((rank, binding.action))
                    })
                    .max_by_key(|(rank, _)| *rank);
                return action.map_or(KeyDecision::Swallow, |(_, action)| {
                    KeyDecision::Dispatch(action)
                });
            }
            return KeyDecision::Pass;
        }
        // GPUI bindings ignore `when`. Swallowing `Cmd+[` here would take the
        // key from a focused terminal. Pass it so the action can propagate.
        if ctx.terminal_focus {
            let mut typing = ctx;
            typing.terminal_focus = false;
            if let Some(command) = resolve(&self.keybindings, &chord, &typing, mac) {
                if matches!(command, "navigation.back" | "navigation.forward") {
                    return KeyDecision::Pass;
                }
            }
        }
        let claimed = chord_claimed(&self.keybindings, &chord, mac);
        if claimed && !plain_text_key(event) {
            return KeyDecision::Swallow;
        }
        if !claimed
            && !self.overlay_open()
            && !ctx.terminal_focus
            && self.workspace_focus_handle.contains_focused(window, cx)
        {
            if let Some((action, cwd)) = self.shortcut_project_action(&chord, cx) {
                return KeyDecision::ProjectAction(action, cwd);
            }
        }
        if !claimed
            && event.keystroke.key == "tab"
            && !modifiers.platform
            && !modifiers.control
            && !modifiers.alt
            && event.context_stack.iter().any(|stack| {
                stack
                    .primary()
                    .is_some_and(|entry| entry.key.as_ref() == "Sidebar")
            })
        {
            return KeyDecision::SidebarFocus(modifiers.shift);
        }
        KeyDecision::Pass
    }

    fn key_context(
        &self,
        event: &KeystrokeEvent,
        editable_input: bool,
        window: &Window,
        cx: &App,
    ) -> KeyContext {
        let chat = self.active_thread().map(|thread| thread.chat.read(cx));
        let terminal_focus = self.terminal_has_focus(window, cx);
        let composer_focus = chat
            .as_ref()
            .is_some_and(|chat| chat.input_state.focus_handle(cx).is_focused(window));
        let composer_draft = chat
            .as_ref()
            .is_some_and(|chat| !chat.input_state.read(cx).value().trim().is_empty());
        KeyContext {
            terminal_focus,
            terminal_open: self.terminal_drawer_open,
            preview_focus: event.context_stack.iter().any(|stack| {
                stack
                    .primary()
                    .is_some_and(|entry| entry.key.as_ref() == "Preview")
            }),
            preview_open: self.preview_panel_visible,
            model_picker_open: chat.as_ref().is_some_and(|chat| chat.model_picker_open),
            usage_page_open: matches!(self.overlay, Some(super::Overlay::Usage { .. }))
                && self.modal_contains_focus(window, cx),
            composer_focus,
            composer_draft,
            turn_running: chat.as_ref().is_some_and(|chat| chat.is_streaming),
            editable_focus: editable_input || terminal_focus,
            is_desktop: true,
            ..KeyContext::default()
        }
    }

    pub(super) fn select_keybinding(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if index >= self.settings_keybindings().len() || !self.linked_keybindings.editable() {
            return;
        }
        self.keybinding_when_open = None;
        self.keybinding_recording = None;
        self.keybinding_index = index;
        self.keybinding_edit_open = true;
        let key = self.settings_keybindings()[index].key.clone();
        let when = self.settings_keybindings()[index]
            .when
            .clone()
            .unwrap_or_default();
        self.keybinding_key_input
            .update(cx, |input, cx| input.set_value(key, window, cx));
        self.keybinding_when_input
            .update(cx, |input, cx| input.set_value(when, window, cx));
        cx.notify();
    }

    pub(super) fn commit_keybinding_edit(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.keybinding_edit_open {
            return;
        }
        let index = self.keybinding_index;
        let Some(row) = self.settings_keybindings().get(index) else {
            return;
        };
        let key = self.keybinding_key_input.read(cx).value().to_string();
        let when_text = self.keybinding_when_input.read(cx).value().to_string();
        let when = if when_text.trim().is_empty() {
            None
        } else {
            Some(when_text)
        };
        if !bruhs_models::logic::keybindings::when_syntax_ok(when.as_deref().unwrap_or(""))
            || (!key.trim().is_empty()
                && !gpui_shortcut(&key).is_some_and(|shortcut| parses(&shortcut)))
            || (row.key == key && row.when == when)
        {
            return;
        }
        let rule = Keybinding {
            command: row.command.clone(),
            key,
            when,
        };
        self.apply_binding_edit(
            KeybindingEdit::Upsert {
                rule,
                replace: Some(row.clone()),
            },
            cx,
        );
    }

    pub(super) fn reset_selected_keybinding(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let index = self.keybinding_index;
        let Some(command) = self
            .settings_keybindings()
            .get(index)
            .map(|row| row.command.clone())
        else {
            return;
        };
        let nth = self
            .settings_keybindings()
            .iter()
            .take(index)
            .filter(|row| row.command == command)
            .count();
        let Some(original) = default_keybindings()
            .into_iter()
            .filter(|row| row.command == command)
            .nth(nth)
        else {
            return;
        };
        if self
            .settings_keybindings()
            .get(index)
            .is_some_and(|row| row == &original)
        {
            return;
        }
        self.apply_binding_edit(
            KeybindingEdit::Upsert {
                rule: original,
                replace: Some(self.settings_keybindings()[index].clone()),
            },
            cx,
        );
        if self.linked_keybindings.editable() {
            self.select_keybinding(index, window, cx);
        } else {
            self.keybinding_edit_open = false;
        }
    }

    pub(super) fn undo_sidebar_action(
        &mut self,
        _: &UndoSidebar,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.sidebar_undo.is_none() {
            cx.propagate();
            return;
        }
        self.undo_sidebar(window, cx);
    }

    pub(super) fn open_project_in_editor_action(
        &mut self,
        _: &OpenProjectInEditor,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let path = self
            .active_thread()
            .map(|thread| thread.project_path.clone())
            .filter(|path| !path.is_empty())
            .or_else(|| {
                self.active_project
                    .as_ref()
                    .map(|project| project.path.clone())
            })
            .filter(|path| !path.is_empty());
        let Some(path) = path else {
            self.meetings_hub.update(cx, |hub, cx| {
                hub.show_notice("No project to open", false, cx)
            });
            return;
        };
        self.open_path_in_editor(&path, None, cx);
    }

    pub(super) fn render_keybinding_settings(&self, cx: &Context<'_, Self>) -> AnyElement {
        let add = super::web_button(
            "desktop-keybindings-add",
            super::WebButtonVariant::Outline,
            super::WebButtonSize::Small,
            super::ButtonState::disabled_if(
                self.keybinding_draft.is_some() || !self.linked_keybindings.editable(),
            ),
        )
        .debug_selector(|| "desktop-keybindings-add".into())
        .track_focus(&self.keybinding_add_focus)
        .aria_label("Add keybinding")
        .child("+ Add keybinding");
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                div().flex_1().min_w(px(130.)).child(
                    Input::new(&self.keybinding_filter)
                        .aria_label("Search keybindings")
                        .cleanable(true),
                ),
            )
            .child(bruhs_ui::press::on_press_view(add, cx, |this, w, cx| {
                if this.keybinding_draft.is_none() {
                    this.open_new_binding(None, w, cx);
                }
            }))
            .child(self.keymap_file_buttons(cx));
        let mut layout = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.settings_card("Keybindings", "", vec![toolbar.into_any_element()]));
        if let Some(status) = self.linked_shortcut_status(cx) {
            layout = layout.child(status);
        }
        if let Some(draft) = self
            .keybinding_draft
            .as_ref()
            .filter(|_| self.linked_keybindings.editable())
        {
            layout = layout.child(self.new_binding_card(draft, cx));
        }
        let query = self.keybinding_filter.read(cx).value();
        let mut groups: [Vec<AnyElement>; 4] = Default::default();
        for (index, rule) in self.settings_keybindings().iter().enumerate() {
            let label = command_label(&rule.command);
            if !super::palette_ui::text_matches_query(
                &format!(
                    "{label} {} {} {}",
                    rule.command,
                    rule.key,
                    rule.when.as_deref().unwrap_or("")
                ),
                &query,
            ) {
                continue;
            }
            let group = match rule.command.split('.').next().unwrap_or("") {
                "thread" | "chat" => 1,
                "composer" | "draft" | "model" | "effort" => 2,
                "workspace" | "project" | "git" | "terminal" => 3,
                _ => 0,
            };
            let rows = &mut groups[group];
            rows.push(self.render_keybinding_rule(index, rule, cx));
            if self.linked_keybindings.editable()
                && self.keybinding_edit_open
                && index == self.keybinding_index
            {
                rows.push(self.render_keybinding_editor(&rule.command, cx));
            }
        }
        if groups.iter().all(Vec::is_empty) && !self.linked_keybindings.selected_unavailable() {
            layout = layout.child(div().p_4().child("No matching keybindings"));
        }
        for (title, rows) in ["Navigation", "Threads", "Composer", "Workspace"]
            .into_iter()
            .zip(groups)
        {
            if !rows.is_empty() {
                layout = layout.child(self.settings_card(title, "", rows));
            }
        }
        layout.into_any_element()
    }

    fn render_keybinding_editor(&self, command: &str, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .debug_selector(|| "desktop-settings-keybinding-editor".to_string())
            .px_4()
            .py_2()
            .flex()
            .flex_wrap()
            .min_w_0()
            .items_center()
            .gap_2()
            .child(
                super::web_button(
                    "desktop-record-shortcut",
                    super::WebButtonVariant::Outline,
                    super::WebButtonSize::Small,
                    super::ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-record-shortcut".to_string())
                .track_focus(&self.keybinding_record_focus)
                .aria_label("Record shortcut")
                .child(
                    if self.keybinding_recording == Some(BindingEditor::Existing) {
                        "Press shortcut… Esc cancels"
                    } else {
                        "Record shortcut"
                    },
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.keybinding_recording = (this.keybinding_recording
                        != Some(BindingEditor::Existing))
                    .then_some(BindingEditor::Existing);
                    this.keybinding_record_focus.focus(window, cx);
                    cx.notify();
                })),
            )
            .child(keybinding_field(
                "desktop-settings-keybinding-key",
                SharedString::from(format!("Key for {command}")),
                KEY_INPUT_WIDTH_PX,
                false,
                &self.keybinding_key_input,
            ))
            .child(self.binding_condition(BindingEditor::Existing, &self.keybinding_when_input, cx))
            .when_some(
                draft::conflict_message(
                    self.settings_keybindings(),
                    Some(self.keybinding_index),
                    &self.keybinding_key_input.read(cx).value(),
                    &self.keybinding_when_input.read(cx).value(),
                ),
                |el, message| el.child(div().text_xs().child(message)),
            )
            .child(
                div()
                    .id("desktop-settings-keybinding-reset")
                    .debug_selector(|| "desktop-settings-keybinding-reset".to_string())
                    .h(px(FIELD_HEIGHT_PX))
                    .px_2()
                    .rounded(px(SETTINGS_CODE_RADIUS_PX))
                    .flex()
                    .items_center()
                    .text_size(px(12.0))
                    .text_color(crate::theme::rambutan_300())
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Reset selected shortcut")
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(crate::theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(crate::theme::longan_800())
                            .text_color(crate::theme::rambutan_100())
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.reset_selected_keybinding(window, cx);
                    }))
                    .child("Reset"),
            )
            .child(
                super::web_button(
                    "desktop-add-shortcut",
                    super::WebButtonVariant::Ghost,
                    super::WebButtonSize::Small,
                    super::ButtonState::Enabled,
                )
                .aria_label("Add shortcut for this command")
                .child("Add shortcut")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_new_binding(Some(this.keybinding_index), window, cx);
                })),
            )
            .child(
                super::web_button(
                    "desktop-remove-shortcut",
                    super::WebButtonVariant::Ghost,
                    super::WebButtonSize::Small,
                    super::ButtonState::Enabled,
                )
                .aria_label("Remove selected shortcut")
                .child("Remove")
                .on_click(cx.listener(|this, _, _, cx| {
                    let index = this.keybinding_index;
                    if let Some(rule) = this.settings_keybindings().get(index).cloned() {
                        this.apply_binding_edit(KeybindingEdit::Remove { rule }, cx);
                    }
                    this.keybinding_edit_open = false;
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    fn render_keybinding_rule(
        &self,
        index: usize,
        rule: &Keybinding,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let selected = self.keybinding_edit_open && index == self.keybinding_index;
        let command = command_label(&rule.command);
        let key = shortcut_label(&rule.key);
        let when = rule.when.clone().unwrap_or_default();
        let label = if when.is_empty() {
            format!("{command}: {key}")
        } else {
            format!("{command}: {key}, when {when}")
        };
        div()
            .id(SharedString::from(format!(
                "desktop-keybinding-row-{index}"
            )))
            .debug_selector(move || format!("desktop-settings-keybinding-row-{index}"))
            .px_4()
            .py_2()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .when(self.linked_keybindings.editable(), |el| {
                el.cursor_pointer().tab_index(0).role(Role::Button)
            })
            .aria_label(label)
            .aria_toggled(selected.into())
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(crate::theme::persimmon_400()))
            .when(selected, |el| el.bg(crate::theme::longan_800()))
            .when(self.linked_keybindings.editable(), |el| {
                let el = el.hover(|el| el.bg(crate::theme::longan_800()));
                bruhs_ui::press::on_press_view(el, cx, move |this, window, cx| {
                    if this.keybinding_edit_open && this.keybinding_index == index {
                        this.keybinding_edit_open = false;
                        cx.notify();
                    } else {
                        this.select_keybinding(index, window, cx);
                    }
                })
            })
            .child(
                div()
                    .min_w(px(KEY_INPUT_WIDTH_PX))
                    .flex_1()
                    .text_size(px(SETTINGS_MODEL_LABEL_TEXT_SIZE_PX))
                    .text_color(crate::theme::rambutan_200())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(command)
                    .child(
                        div()
                            .text_xs()
                            .text_color(crate::theme::rambutan_500())
                            .child(if when.is_empty() {
                                "Always".to_owned()
                            } else {
                                format!("When {when}")
                            }),
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .max_w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.0))
                    .text_color(crate::theme::rambutan_300())
                    .font_family(crate::theme::CODE_FONT_FAMILY)
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(crate::theme::longan_800())
                    .child(key),
            )
            .into_any_element()
    }
}

pub(super) fn shortcut_label(raw: &str) -> String {
    gpui_shortcut(raw)
        .and_then(|shortcut| Keystroke::parse_with_os(&shortcut, host_os()).ok())
        .map_or_else(
            || {
                if raw.is_empty() {
                    "Unassigned".into()
                } else {
                    raw.into()
                }
            },
            |key| key.display(host_os()).to_string(),
        )
}

pub(super) fn command_label(command: &str) -> String {
    match command {
        "command.palette" => "Open command palette".into(),
        "settings.open" => "Open settings".into(),
        "chat.new" => "New thread".into(),
        "workspace.focusNext" => "Focus next workspace area".into(),
        "workspace.focusPrevious" => "Focus previous workspace area".into(),
        "snapshot.capture" => "Capture a window".into(),
        "usage.cost" => "Show usage cost".into(),
        "usage.tokens" => "Show usage tokens".into(),
        "usage.limits" => "Show usage limits".into(),
        "usage.period.day" => "Show usage for the past 24 hours".into(),
        "usage.period.week" => "Show usage for the past 7 days".into(),
        "usage.period.month" => "Show usage for the past 30 days".into(),
        "usage.period.quarter" => "Show usage for the past 90 days".into(),
        _ => command
            .split('.')
            .map(|part| {
                let mut label = String::new();
                for (i, ch) in part.chars().enumerate() {
                    if i == 0 {
                        label.extend(ch.to_uppercase());
                    } else if ch.is_uppercase() {
                        label.push(' ');
                        label.extend(ch.to_lowercase());
                    } else {
                        label.push(ch);
                    }
                }
                label
            })
            .collect::<Vec<_>>()
            .join(" · "),
    }
}

fn keybinding_field(
    selector: &'static str,
    label: SharedString,
    width: f32,
    broken: bool,
    input: &Entity<InputState>,
) -> AnyElement {
    div()
        .id(selector)
        .debug_selector(move || selector.to_string())
        .w(px(width))
        .max_w_full()
        .flex_shrink_0()
        .h(px(FIELD_HEIGHT_PX))
        .rounded(px(SETTINGS_CODE_RADIUS_PX))
        .bg(crate::theme::longan_800())
        .when(broken, |el| {
            el.border_2().border_color(crate::theme::persimmon_400())
        })
        .child(Input::new(input).aria_label(label))
        .into_any_element()
}

fn plain_text_key(event: &KeystrokeEvent) -> bool {
    event.keystroke.key_char.is_some() && !event.keystroke.modifiers.modified()
}

fn push_rule_bindings(
    rule: &Keybinding,
    add: &mut Vec<PendingBinding>,
    installed: &mut Vec<InstalledBinding>,
) {
    let Some(shortcut) = gpui_shortcut(&rule.key) else {
        return;
    };
    if !parses(&shortcut) {
        return;
    }
    let names = binding_names(&rule.command);
    if names.is_empty() {
        return;
    }
    let name = names[0];
    match rule.command.as_str() {
        "command.palette" => bind(&shortcut, FocusCommandBar, name, None, add, installed),
        "capture.open" => bind(&shortcut, OpenCapture, name, None, add, installed),
        "view.today" => bind(&shortcut, OpenTodayView, name, None, add, installed),
        "view.knowledge" => bind(&shortcut, OpenKnowledgeView, name, None, add, installed),
        "view.inbox" => bind(&shortcut, OpenInboxView, name, None, add, installed),
        "chat.new" => bind(&shortcut, NewThread, name, None, add, installed),
        "sidebar.toggle" => bind(&shortcut, ToggleSidebar, name, None, add, installed),
        "rightPanel.toggle" => bind(&shortcut, TogglePanel, name, None, add, installed),
        "rightPanel.close" => bind(&shortcut, ClosePanel, name, None, add, installed),
        "diff.toggle" => bind(&shortcut, ToggleDiff, name, None, add, installed),
        "tasks.toggle" => bind(&shortcut, ToggleTasks, name, None, add, installed),
        "model.picker" => bind(&shortcut, ToggleModelPicker, name, None, add, installed),
        "workspace.menu" => bind(&shortcut, ToggleWorkspaceMenu, name, None, add, installed),
        "branch.menu" => bind(&shortcut, ToggleBranchMenu, name, None, add, installed),
        "effort.menu" => bind(&shortcut, ToggleEffortMenu, name, None, add, installed),
        "worktree.previous" => bind(&shortcut, UsePreviousWorktree, name, None, add, installed),
        "mention.file" => bind(&shortcut, MentionFile, name, None, add, installed),
        "thread.previous" => bind(&shortcut, PrevThread, name, None, add, installed),
        "thread.next" => bind(&shortcut, NextThread, name, None, add, installed),
        "navigation.back" => bind(&shortcut, NavigateBack, name, None, add, installed),
        "navigation.forward" => bind(&shortcut, NavigateForward, name, None, add, installed),
        "thread.pin" => bind(&shortcut, PinThread, name, None, add, installed),
        "thread.settle" => bind(&shortcut, SettleThread, name, None, add, installed),
        "thread.copyLink" => bind(&shortcut, CopyThreadLink, name, None, add, installed),
        "pull.copyLink" => bind(&shortcut, CopyPullLink, name, None, add, installed),
        "pull.copyNumber" => bind(&shortcut, CopyPullNumber, name, None, add, installed),
        "thread.import" => bind(&shortcut, ImportThread, name, None, add, installed),
        "thread.copyResumeCommand" => {
            bind(&shortcut, CopyResumeCommand, name, None, add, installed)
        }
        "thread.copyMarkdown" => bind(&shortcut, CopyThreadMarkdown, name, None, add, installed),
        "thread.continueInWorkbench" => {
            bind(&shortcut, ContinueInWorkbench, name, None, add, installed)
        }
        "settings.open" => bind(&shortcut, OpenSettings, name, None, add, installed),
        "terminal.toggle" => bind(&shortcut, ToggleTerminal, name, None, add, installed),
        "preview.toggle" => bind(&shortcut, TogglePreview, name, None, add, installed),
        "search.open" => bind(&shortcut, SearchThreads, name, None, add, installed),
        "project.openInEditor" => bind(&shortcut, OpenProjectInEditor, name, None, add, installed),
        "dialog.dismiss" => bind(&shortcut, CloseOverlays, name, None, add, installed),
        "composer.focus" => bind(&shortcut, FocusChatInput, name, None, add, installed),
        "crew.focus" => bind(&shortcut, FocusCrew, name, None, add, installed),
        "agents.stopAll" => bind(&shortcut, StopAll, name, None, add, installed),
        "task.new" => bind(
            &shortcut,
            OpenTaskCreateShortcut,
            name,
            None,
            add,
            installed,
        ),
        "snapshot.capture" => bind(&shortcut, CaptureWindow, name, None, add, installed),
        "voice.dictate" => bind(
            &shortcut,
            crate::views::chat::ToggleDictation,
            name,
            Some("Chat"),
            add,
            installed,
        ),
        "draft.stash" => {
            bind(
                &shortcut,
                StashDraft,
                names[0],
                Some("Chat"),
                add,
                installed,
            );
            bind(
                &shortcut,
                SaveKnowledgeDocument,
                names[1],
                Some("Workspace"),
                add,
                installed,
            );
        }
        "chat.find" => {
            bind(
                &shortcut,
                FindInThread,
                names[0],
                Some("Chat"),
                add,
                installed,
            );
            bind(
                &shortcut,
                FindInDiff,
                names[1],
                Some("Diff"),
                add,
                installed,
            );
        }
        command => {
            if let Some(digit) = command.strip_prefix("thread.jump.") {
                if let Ok(n) = digit.parse::<usize>() {
                    if (1..=9).contains(&n) {
                        bind_action(
                            &shortcut,
                            JumpThread(n - 1),
                            "workspace::JumpThread",
                            None,
                            add,
                            installed,
                        );
                    }
                }
            }
        }
    }
}

/// A user shortcut, kept as text until a context can load it: gpui only
/// parses `&'static str` shortcuts without one.
struct PendingBinding {
    shortcut: String,
    action: Box<dyn Action>,
    context: Option<String>,
}

impl PendingBinding {
    fn load(self, cx: &App) -> Option<KeyBinding> {
        let context = match self.context.as_deref() {
            Some(context) => Some(KeyBindingContextPredicate::parse(context).ok()?.into()),
            None => None,
        };
        KeyBinding::load(&self.shortcut, self.action, context, false, None, cx).ok()
    }
}

/// The host OS, which resolves `secondary-` in a shortcut.
pub(super) fn host_os() -> OperatingSystem {
    if cfg!(target_os = "macos") {
        OperatingSystem::Mac
    } else if cfg!(target_os = "windows") {
        OperatingSystem::Windows
    } else {
        OperatingSystem::Linux
    }
}

/// Whether `shortcut` is a keystroke gpui accepts.
fn parses(shortcut: &str) -> bool {
    Keystroke::parse_with_os(shortcut, host_os()).is_ok()
}

fn bind(
    shortcut: &str,
    action: impl Action,
    name: &'static str,
    context: Option<&str>,
    add: &mut Vec<PendingBinding>,
    installed: &mut Vec<InstalledBinding>,
) {
    bind_action(shortcut, action, name, context, add, installed);
}

fn bind_action(
    shortcut: &str,
    action: impl Action,
    name: &'static str,
    context: Option<&str>,
    add: &mut Vec<PendingBinding>,
    installed: &mut Vec<InstalledBinding>,
) {
    add.push(PendingBinding {
        shortcut: shortcut.to_string(),
        action: Box::new(action),
        context: context.map(str::to_string),
    });
    installed.push(InstalledBinding {
        shortcut: shortcut.to_string(),
        action: name,
    });
}

#[cfg(test)]
mod tests {
    use super::super::{
        ContinueInWorkbench, FocusCommandBar, JumpThread, OpenProjectInEditor, UndoSidebar,
    };
    use super::{binding_names, rules_from_saved};
    use crate::views::chat::StashDraft;
    use crate::views::diff_panel::FindInDiff;
    use ::core::prelude::v1::test;
    use bruhs_models::logic::keybindings::{default_keybindings, gpui_shortcut};
    use gpui::Action;

    #[test]
    fn every_default_command_is_bound_or_handled_by_the_interceptor() {
        for rule in default_keybindings() {
            let names = binding_names(&rule.command);
            if rule.command == "thread.undo"
                || matches!(
                    rule.command.as_str(),
                    "workspace.focusNext" | "workspace.focusPrevious"
                )
                || crate::views::usage::UsageShortcut::from_command(&rule.command).is_some()
            {
                assert!(names.is_empty());
            } else {
                assert!(!names.is_empty(), "{}", rule.command);
            }
            if rule.key.is_empty() {
                assert!(
                    bruhs_models::logic::keybindings::UNBOUND_BY_DEFAULT
                        .contains(&rule.command.as_str()),
                    "{}",
                    rule.command
                );
                assert!(gpui_shortcut(&rule.key).is_none());
                continue;
            }
            let shortcut = gpui_shortcut(&rule.key).unwrap_or_else(|| panic!("{}", rule.key));
            assert!(super::parses(&shortcut), "{shortcut} from {}", rule.key);
        }
    }

    #[test]
    fn empty_storage_loads_the_defaults() {
        assert_eq!(
            rules_from_saved(Vec::new(), 0).len(),
            default_keybindings().len()
        );
    }

    #[test]
    fn binding_names_match_the_action_types() {
        assert_eq!(
            <FocusCommandBar as Action>::name(&FocusCommandBar),
            "workspace::FocusCommandBar"
        );
        assert_eq!(
            <JumpThread as Action>::name(&JumpThread(0)),
            "workspace::JumpThread"
        );
        assert_eq!(
            <StashDraft as Action>::name(&StashDraft),
            "chat::StashDraft"
        );
        assert_eq!(
            <FindInDiff as Action>::name(&FindInDiff),
            "diff_panel::FindInDiff"
        );
        assert_eq!(
            <OpenProjectInEditor as Action>::name(&OpenProjectInEditor),
            binding_names("project.openInEditor")[0]
        );
        assert_eq!(
            <UndoSidebar as Action>::name(&UndoSidebar),
            "workspace::UndoSidebar"
        );
        assert_eq!(
            <ContinueInWorkbench as Action>::name(&ContinueInWorkbench),
            binding_names("thread.continueInWorkbench")[0]
        );
    }
}
