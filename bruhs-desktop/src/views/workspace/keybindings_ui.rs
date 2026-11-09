//! Settings → Keybindings. Rules live in the desktop state file. The last
//! matching rule wins. `mod` is Command on macOS and Control elsewhere.
//!
//! GPUI still owns dispatch for every command except sidebar undo. Undo stays
//! out of the keymap so a text field keeps ⌘Z. A rule whose `when` fails
//! swallows a modified chord, so a customized condition can turn a shortcut
//! off. A plain key such as `/` or `c` is left alone when its condition fails,
//! or the character would never reach the field.

use gpui::prelude::*;
use gpui::{
    div, px, Action, AnyElement, App, Context, Entity, Focusable, KeyBinding, Keystroke,
    KeystrokeEvent, SharedString, Subscription, Unbind, WeakEntity, Window,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::WindowExt;

use bruhs_models::logic::keybindings::{
    chord_claimed, chord_from_event, default_keybindings, gpui_shortcut, resolve,
    stored_keybindings, KeyContext, Keybinding,
};

use crate::views::chat::{FindInThread, StashDraft};
use crate::views::diff_panel::FindInDiff;

use super::{
    CloseOverlays, ClosePanel, ContinueInT3, CopyPullLink, CopyPullNumber, CopyResumeCommand,
    CopyThreadLink, CopyThreadMarkdown, FocusChatInput, FocusCommandBar, ImportThread, JumpThread,
    MentionFile, NavigateBack, NavigateForward, NewThread, NextThread, OpenCapture, OpenInboxView,
    OpenKnowledgeView, OpenProjectInEditor, OpenSettings, OpenTaskCreateShortcut, OpenTodayView,
    PinThread, PrevThread, SaveKnowledgeDocument, SearchThreads, SettleThread, ToggleBranchMenu,
    ToggleDiff, ToggleEffortMenu, ToggleModelPicker, TogglePanel, TogglePreview, ToggleSidebar,
    ToggleTasks, ToggleTerminal, ToggleWorkspaceMenu, UndoSidebar, UsePreviousWorktree, Workspace,
    SETTINGS_CODE_RADIUS_PX, SETTINGS_MODEL_LABEL_TEXT_SIZE_PX,
};

const KEY_INPUT_WIDTH_PX: f32 = 144.0;
const WHEN_INPUT_WIDTH_PX: f32 = 160.0;
const FIELD_HEIGHT_PX: f32 = 32.0;

#[derive(Clone)]
pub(super) struct InstalledBinding {
    shortcut: String,
    action: &'static str,
}

enum KeyDecision {
    Undo,
    Swallow,
    Pass,
}

pub(super) fn rules_from_saved(saved: Vec<Keybinding>) -> Vec<Keybinding> {
    if saved.is_empty() {
        stored_keybindings(None)
    } else {
        stored_keybindings(Some(saved))
    }
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
        "settings.open" => &["workspace::OpenSettings"],
        "terminal.toggle" => &["workspace::ToggleTerminal"],
        "preview.toggle" => &["workspace::TogglePreview"],
        "draft.stash" => &["chat::StashDraft", "workspace::SaveKnowledgeDocument"],
        "search.open" => &["workspace::SearchThreads"],
        "chat.find" => &["chat::FindInThread", "diff_panel::FindInDiff"],
        "project.openInEditor" => &["workspace::OpenProjectInEditor"],
        "dialog.dismiss" => &["workspace::CloseOverlays"],
        "composer.focus" => &["workspace::FocusChatInput"],
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
    let editable_input = window.has_focused_input(cx);
    let decision = workspace.update(cx, |this, cx| {
        this.key_decision(event, editable_input, window, cx)
    });
    match decision {
        KeyDecision::Undo => {
            cx.stop_propagation();
            workspace.update(cx, |this, cx| this.undo_sidebar(window, cx));
        }
        KeyDecision::Swallow => cx.stop_propagation(),
        KeyDecision::Pass => {}
    }
}

impl Workspace {
    pub(super) fn sync_keybindings(&mut self, cx: &mut Context<'_, Self>) {
        let mut remove = Vec::new();
        for installed in &self.keybinding_installed {
            if Keystroke::parse(&installed.shortcut).is_err() {
                continue;
            }
            remove.push(KeyBinding::new(
                &installed.shortcut,
                Unbind(installed.action.into()),
                None,
            ));
        }
        if !remove.is_empty() {
            cx.bind_keys(remove);
        }
        let mut add = Vec::new();
        let mut installed = Vec::new();
        for rule in &self.keybindings {
            push_rule_bindings(rule, &mut add, &mut installed);
        }
        if !add.is_empty() {
            cx.bind_keys(add);
        }
        self.keybinding_installed = installed;
    }

    fn key_decision(
        &self,
        event: &KeystrokeEvent,
        editable_input: bool,
        window: &Window,
        cx: &App,
    ) -> KeyDecision {
        let modifiers = event.keystroke.modifiers;
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
        if chord_claimed(&self.keybindings, &chord, mac) && !plain_text_key(event) {
            return KeyDecision::Swallow;
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

    pub(super) fn select_keybinding(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if index >= self.keybindings.len() {
            return;
        }
        self.keybinding_index = index;
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
        self.sync_keybindings(cx);
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
        self.sync_keybindings(cx);
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
            return;
        };
        self.open_path_in_editor(&path, None, cx);
    }

    pub(super) fn render_keybinding_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let command = self
            .keybindings
            .get(self.keybinding_index)
            .map(|row| row.command.clone())
            .unwrap_or_default();
        let when_text = self.keybinding_when_input.read(cx).value();
        let when_broken = !when_text.trim().is_empty()
            && !bruhs_models::logic::keybindings::when_syntax_ok(&when_text);
        let mut rows = vec![self.render_keybinding_editor(&command, when_broken, cx)];
        for (index, rule) in self.keybindings.iter().enumerate() {
            rows.push(self.render_keybinding_rule(index, rule, cx));
        }
        rows
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
            .items_center()
            .gap_2()
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
            .into_any_element()
    }

    fn render_keybinding_rule(
        &self,
        index: usize,
        rule: &Keybinding,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let selected = index == self.keybinding_index;
        let command = rule.command.clone();
        let key = rule.key.clone();
        let when = rule.when.clone().unwrap_or_default();
        div()
            .id(SharedString::from(format!(
                "desktop-keybinding-row-{index}"
            )))
            .debug_selector(move || format!("desktop-settings-keybinding-row-{index}"))
            .px_4()
            .py_2()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .cursor_pointer()
            .when(selected, |el| el.bg(crate::theme::longan_800()))
            .hover(|el| el.bg(crate::theme::longan_800()))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_keybinding(index, window, cx);
            }))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_size(px(SETTINGS_MODEL_LABEL_TEXT_SIZE_PX))
                    .text_color(crate::theme::rambutan_200())
                    .child(command),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.0))
                    .text_color(crate::theme::rambutan_300())
                    .font(crate::theme::tabular_system_font())
                    .child(key)
                    .when(!when.is_empty(), |el| {
                        el.child(div().text_color(crate::theme::rambutan_500()).child(when))
                    }),
            )
            .into_any_element()
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
        .aria_label(label)
        .w(px(width))
        .h(px(FIELD_HEIGHT_PX))
        .rounded(px(SETTINGS_CODE_RADIUS_PX))
        .bg(crate::theme::longan_800())
        .when(broken, |el| {
            el.border_2().border_color(crate::theme::persimmon_400())
        })
        .child(Input::new(input))
        .into_any_element()
}

fn plain_text_key(event: &KeystrokeEvent) -> bool {
    event.keystroke.key_char.is_some() && !event.keystroke.modifiers.modified()
}

fn push_rule_bindings(
    rule: &Keybinding,
    add: &mut Vec<KeyBinding>,
    installed: &mut Vec<InstalledBinding>,
) {
    let Some(shortcut) = gpui_shortcut(&rule.key) else {
        return;
    };
    if Keystroke::parse(&shortcut).is_err() {
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
        "settings.open" => bind(&shortcut, OpenSettings, name, None, add, installed),
        "terminal.toggle" => bind(&shortcut, ToggleTerminal, name, None, add, installed),
        "preview.toggle" => bind(&shortcut, TogglePreview, name, None, add, installed),
        "search.open" => bind(&shortcut, SearchThreads, name, None, add, installed),
        "project.openInEditor" => bind(&shortcut, OpenProjectInEditor, name, None, add, installed),
        "dialog.dismiss" => bind(&shortcut, CloseOverlays, name, None, add, installed),
        "composer.focus" => bind(&shortcut, FocusChatInput, name, None, add, installed),
        "task.new" => bind(
            &shortcut,
            OpenTaskCreateShortcut,
            name,
            None,
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

fn bind(
    shortcut: &str,
    action: impl Action,
    name: &'static str,
    context: Option<&str>,
    add: &mut Vec<KeyBinding>,
    installed: &mut Vec<InstalledBinding>,
) {
    bind_action(shortcut, action, name, context, add, installed);
}

fn bind_action(
    shortcut: &str,
    action: impl Action,
    name: &'static str,
    context: Option<&str>,
    add: &mut Vec<KeyBinding>,
    installed: &mut Vec<InstalledBinding>,
) {
    add.push(KeyBinding::new(shortcut, action, context));
    installed.push(InstalledBinding {
        shortcut: shortcut.to_string(),
        action: name,
    });
}

#[cfg(test)]
mod tests {
    use super::super::{FocusCommandBar, JumpThread, OpenProjectInEditor, UndoSidebar};
    use super::{binding_names, rules_from_saved};
    use crate::views::chat::StashDraft;
    use crate::views::diff_panel::FindInDiff;
    use bruhs_models::logic::keybindings::{default_keybindings, gpui_shortcut};
    use gpui::{Action, Keystroke};

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
            assert!(
                Keystroke::parse(&shortcut).is_ok(),
                "{shortcut} from {}",
                rule.key
            );
        }
    }

    #[test]
    fn empty_storage_loads_the_defaults() {
        assert_eq!(
            rules_from_saved(Vec::new()).len(),
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
    }
}
