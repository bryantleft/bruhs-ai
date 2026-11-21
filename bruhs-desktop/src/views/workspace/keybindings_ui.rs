//! Settings → Keybindings. Rules live in the desktop state file. The last
//! matching rule wins. `mod` is Command on macOS and Control elsewhere.
//!
//! GPUI still owns dispatch for every command except sidebar undo. Undo stays
//! out of the keymap so a text field keeps ⌘Z. A rule whose `when` fails
//! swallows a modified chord, so a customized condition can turn a shortcut
//! off. A plain key such as `/` or `c` is left alone when its condition fails,
//! or the character would never reach the field.

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
    chord_claimed, chord_from_event, default_keybindings, gpui_shortcut, resolve,
    stored_keybindings, KeyContext, Keybinding,
};

use crate::views::chat::{FindInThread, StashDraft};
use crate::views::diff_panel::FindInDiff;

use super::{
    CaptureWindow, CloseOverlays, ClosePanel, ContinueInT3, ContinueInWorkbench, CopyPullLink, CopyPullNumber,
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
const WHEN_INPUT_WIDTH_PX: f32 = 160.0;
const FIELD_HEIGHT_PX: f32 = crate::theme::CONTROL_PX;

#[derive(Clone)]
pub(super) struct InstalledBinding {
    shortcut: String,
    action: &'static str,
}

enum KeyDecision {
    Dictation,
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

pub(super) fn rules_from_saved(mut saved: Vec<Keybinding>, schema: u8) -> Vec<Keybinding> {
    if saved.is_empty() {
        return stored_keybindings(None);
    }
    if schema == 0 {
        let is_mac = cfg!(target_os = "macos");
        let chord = bruhs_models::logic::keybindings::parse_chord("mod+,", is_mac)
            .expect("shipped shortcut");
        let mut settings = saved.iter().filter(|rule| rule.command == "settings.open");
        let untouched = settings
            .next()
            .is_some_and(|rule| rule.key == "mod+/" && rule.when.is_none())
            && settings.next().is_none();
        if untouched && !chord_claimed(&saved, &chord, is_mac) {
            saved.push(Keybinding {
                key: "mod+,".into(),
                command: "settings.open".into(),
                when: None,
            });
        }
    }
    stored_keybindings(Some(saved))
}

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
        "thread.continueInT3" => &["workspace::ContinueInT3"],
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
            if commits_keybinding(event) {
                this.commit_keybinding_edit(window, cx);
            }
        },
    ));
    subscriptions.push(cx.subscribe_in(
        &when_input,
        window,
        |this, _, event: &InputEvent, window, cx| {
            if commits_keybinding(event) {
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
    if workspace.read(cx).keybinding_recording
        && workspace
            .read(cx)
            .keybinding_record_focus
            .is_focused(window)
    {
        cx.stop_propagation();
        workspace.update(cx, |this, cx| {
            let stroke = &event.keystroke;
            let add_for = this.keybinding_new_for.take();
            this.keybinding_recording = false;
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
                if let Some(index) = add_for {
                    let mut rule = this.keybindings[index].clone();
                    rule.key = shortcut;
                    let existing = this.keybindings.iter().position(|candidate| candidate == &rule);
                    let index = existing.unwrap_or_else(|| {
                        this.keybindings.push(rule);
                        this.keybindings.len() - 1
                    });
                    this.select_keybinding(index, window, cx);
                    this.keymap_changed(cx);
                    this.persist_and_notify(cx);
                    return;
                }
                this.keybinding_key_input
                    .update(cx, |field, cx| field.set_value(shortcut, window, cx));
                this.commit_keybinding_edit(window, cx);
            }
            cx.notify();
        });
        return;
    }
    let editable_input = window.has_focused_input(cx);
    let decision = workspace.update(cx, |this, cx| {
        this.key_decision(event, editable_input, window, cx)
    });
    match decision {
        KeyDecision::Dictation => {
            cx.stop_propagation();
            window.prevent_default();
            workspace.update(cx, |this, cx| this.press_voice_shortcut(&event.keystroke, cx));
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
            && self.active_modal_focus_bounds(cx).0.contains_focused(window, cx)
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
        if let Some(command) = resolve(&self.keybindings, &chord, &ctx, mac) {
            if command == "voice.dictate" && !self.overlay_open() && !ctx.terminal_focus
                && event.context_stack.iter().any(|stack| stack.primary().is_some_and(|entry| entry.key.as_ref() == "Chat"))
            {
                return KeyDecision::Dictation;
            }
            if command == "thread.undo" && self.sidebar_undo.is_some() {
                return KeyDecision::Undo;
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
        if !claimed && !self.overlay_open() && !ctx.terminal_focus
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
            composer_focus,
            composer_draft,
            turn_running: chat.as_ref().is_some_and(|chat| chat.is_streaming),
            editable_focus: editable_input || terminal_focus,
            is_desktop: true,
            ..KeyContext::default()
        }
    }

    fn record_new_binding(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.keybinding_new_for = Some(index);
        self.keybinding_recording = true;
        self.keybinding_choosing_command = false;
        self.keybinding_edit_open = false;
        let command = self.keybindings[index].command.clone();
        self.keybinding_filter.update(cx, |input, cx| input.set_value(command, window, cx));
        self.settings_scroll.set_offset(Default::default());
        self.keybinding_record_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn select_keybinding(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if index >= self.keybindings.len() {
            return;
        }
        self.keybinding_recording = false;
        self.keybinding_index = index;
        self.keybinding_edit_open = true;
        let key = self.keybindings[index].key.clone();
        let when = self.keybindings[index].when.clone().unwrap_or_default();
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
        let index = self.keybinding_index;
        let Some(row) = self.keybindings.get(index) else {
            return;
        };
        let key = self.keybinding_key_input.read(cx).value().to_string();
        let when_text = self.keybinding_when_input.read(cx).value().to_string();
        let when = if when_text.trim().is_empty() {
            None
        } else {
            Some(when_text)
        };
        if row.key == key && row.when == when {
            return;
        }
        if let Some(row) = self.keybindings.get_mut(index) {
            row.key = key;
            row.when = when;
        }
        self.keymap_changed(cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn reset_selected_keybinding(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let index = self.keybinding_index;
        let Some(command) = self.keybindings.get(index).map(|row| row.command.clone()) else {
            return;
        };
        let nth = self
            .keybindings
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
            .keybindings
            .get(index)
            .is_some_and(|row| row == &original)
        {
            return;
        }
        self.keybindings[index] = original;
        self.select_keybinding(index, window, cx);
        self.keymap_changed(cx);
        self.persist_and_notify(cx);
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
        let choosing = self.keybinding_choosing_command;
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.keybinding_filter)
                        .aria_label("Search keybindings")
                        .cleanable(true),
                ),
            )
            .child(
                super::web_button(
                    "desktop-keybindings-add",
                    super::WebButtonVariant::Outline,
                    super::WebButtonSize::Small,
                    super::ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-keybindings-add".to_string())
                .aria_label(if choosing {
                    "Cancel adding keybinding"
                } else {
                    "Add keybinding"
                })
                .child(if choosing {
                    "Cancel"
                } else {
                    "+ Add keybinding"
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    this.keybinding_choosing_command = !this.keybinding_choosing_command;
                    this.keybinding_edit_open = false;
                    this.keybinding_recording = false;
                    this.keybinding_new_for = None;
                    this.keybinding_filter.update(cx, |field, cx| {
                        field.set_value("", window, cx);
                        field.focus(window, cx);
                    });
                    cx.notify();
                })),
            );
        let mut layout = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.settings_card("Keybindings", "", vec![toolbar.into_any_element(),self.keymap_file_buttons(cx)]));
        if let Some(index) = self.keybinding_new_for {
            let command = &self.keybindings[index].command;
            let recorder = super::web_button(
                "desktop-record-new-shortcut",
                super::WebButtonVariant::Outline,
                super::WebButtonSize::Small,
                super::ButtonState::Enabled,
            )
            .debug_selector(|| "desktop-record-new-shortcut".to_string())
            .track_focus(&self.keybinding_record_focus)
            .child(format!("Press a shortcut for {} · Esc cancels", command_label(command)))
            .on_click(cx.listener(|this, _, window, cx| this.keybinding_record_focus.focus(window, cx)));
            return layout.child(recorder).into_any_element();
        }
        let query = self.keybinding_filter.read(cx).value();
        let mut commands = std::collections::HashSet::new();
        let mut groups: [Vec<AnyElement>; 4] = Default::default();
        for (index, rule) in self.keybindings.iter().enumerate() {
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
            if choosing && !commands.insert(&rule.command) {
                continue;
            }
            let group = match rule.command.split('.').next().unwrap_or("") {
                "thread" | "chat" => 1,
                "composer" | "draft" | "model" | "effort" => 2,
                "workspace" | "project" | "git" | "terminal" => 3,
                _ => 0,
            };
            let rows = &mut groups[group];
            if choosing {
                let button = super::web_button(
                    ("desktop-choose-command", index),
                    super::WebButtonVariant::Ghost,
                    super::WebButtonSize::Small,
                    super::ButtonState::Enabled,
                )
                .debug_selector(move || format!("desktop-choose-command-{index}"))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.record_new_binding(index, window, cx);
                }));
                rows.push(button.into_any_element());
                continue;
            }
            rows.push(self.render_keybinding_rule(index, rule, cx));
            if self.keybinding_edit_open && index == self.keybinding_index {
                let when = self.keybinding_when_input.read(cx).value();
                rows.push(self.render_keybinding_editor(
                    &rule.command,
                    !bruhs_models::logic::keybindings::when_syntax_ok(&when),
                    cx,
                ));
            }
        }
        if groups.iter().all(Vec::is_empty) {
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

    fn render_keybinding_editor(
        &self,
        command: &str,
        when_broken: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
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
                .child(if self.keybinding_recording {
                    "Press shortcut… Esc cancels"
                } else {
                    "Record shortcut"
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    this.keybinding_recording = !this.keybinding_recording;
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
            .child(keybinding_field(
                "desktop-settings-keybinding-when",
                SharedString::from(format!("When for {command}")),
                WHEN_INPUT_WIDTH_PX,
                when_broken,
                &self.keybinding_when_input,
            ))
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
                    this.record_new_binding(this.keybinding_index, window, cx);
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
                .on_click(cx.listener(|this, _, window, cx| {
                    let index = this.keybinding_index;
                    let command = &this.keybindings[index].command;
                    // Keep a disabled entry for a default command so restoring an older
                    // profile will not reinstate the shortcut on the next launch.
                    if this
                        .keybindings
                        .iter()
                        .filter(|rule| &rule.command == command)
                        .count()
                        == 1
                    {
                        this.keybindings[index].key.clear();
                    } else {
                        this.keybindings.remove(index);
                    }
                    this.select_keybinding(index.min(this.keybindings.len() - 1), window, cx);
                    this.keybinding_edit_open = false;
                    this.keymap_changed(cx);
                    this.persist_and_notify(cx);
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
        let key = gpui_shortcut(&rule.key)
            .and_then(|shortcut| Keystroke::parse_with_os(&shortcut, host_os()).ok())
            .map(|key| key.display(host_os()).to_string())
            .unwrap_or_else(|| {
                if rule.key.is_empty() {
                    "Unassigned".into()
                } else {
                    rule.key.clone()
                }
            });
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
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .aria_toggled(selected.into())
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(crate::theme::persimmon_400()))
            .when(selected, |el| el.bg(crate::theme::longan_800()))
            .hover(|el| el.bg(crate::theme::longan_800()))
            .map(|el| {
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

fn command_label(command: &str) -> String {
    match command {
        "command.palette" => "Open command palette".into(),
        "settings.open" => "Open settings".into(),
        "chat.new" => "New thread".into(),
        "snapshot.capture" => "Capture a window".into(),
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
        "thread.continueInT3" => bind(&shortcut, ContinueInT3, name, None, add, installed),
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
        "voice.dictate" => bind(&shortcut, crate::views::chat::ToggleDictation, name, Some("Chat"), add, installed),
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
    fn every_default_command_is_bound_or_is_sidebar_undo() {
        for rule in default_keybindings() {
            let names = binding_names(&rule.command);
            if rule.command == "thread.undo" {
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
