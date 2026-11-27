//! The new binding stays separate from installed shortcuts until Save.
use super::*;
use crate::views::workspace::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use bruhs_models::logic::keybindings::{parse_chord, when_syntax_ok};
use bruhs_ui::dropdown::{Dropdown, DropdownGroup, DropdownOption};
use bruhs_ui::press::on_press_view;
use std::collections::BTreeSet;

pub(in crate::views::workspace) struct NewBinding {
    pub(in crate::views::workspace) command: Option<String>,
    pub(super) key: String,
    pub(super) focus: gpui::FocusHandle,
    pub(super) condition_focus: gpui::FocusHandle,
    pub(super) when: Entity<InputState>,
    _watch: Subscription,
}

impl Workspace {
    pub(super) fn open_new_binding(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let command = index
            .and_then(|i| self.settings_keybindings().get(i))
            .map(|r| r.command.clone());
        self.open_new_binding_for(command, window, cx);
    }

    pub(in crate::views::workspace) fn open_new_binding_for(
        &mut self,
        command: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.linked_keybindings.editable() {
            return;
        }
        if let Some(draft) = &mut self.keybinding_draft {
            if command.is_some() {
                draft.command = command;
            }
        } else {
            let when = cx.new(|cx| InputState::new(window, cx).placeholder("Always"));
            let watch = crate::views::workspace::redraw_on_edit(&when, window, cx);
            self.keybinding_draft = Some(NewBinding {
                command,
                key: String::new(),
                focus: cx.focus_handle().tab_stop(true),
                condition_focus: cx.focus_handle().tab_stop(true),
                when,
                _watch: watch,
            });
        }
        self.keybinding_when_open = None;
        self.keybinding_recording = None;
        self.keybinding_edit_open = false;
        self.settings_scroll.set_offset(Default::default());
        cx.notify();
    }

    pub(super) fn close_binding_draft(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.keybinding_draft = None;
        self.keybinding_when_open = None;
        self.keybinding_recording = None;
        self.set_settings_open_dropdown(None);
        self.keybinding_add_focus.focus(window, cx);
        cx.notify();
    }

    fn new_rule(&self, cx: &App) -> Option<Keybinding> {
        let draft = self.keybinding_draft.as_ref()?;
        let command = draft.command.as_ref()?;
        let key = &draft.key;
        let when = draft.when.read(cx).value();
        let key = key.trim();
        let when = when.trim();
        if !gpui_shortcut(key).is_some_and(|shortcut| parses(&shortcut)) || !when_syntax_ok(when) {
            return None;
        }
        Some(Keybinding {
            command: command.clone(),
            key: key.into(),
            when: (!when.is_empty()).then(|| when.into()),
        })
    }

    fn save_binding_draft(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.linked_keybindings.editable() {
            return;
        }
        let Some(rule) = self.new_rule(cx) else {
            return;
        };
        self.apply_binding_edit(
            KeybindingEdit::Upsert {
                rule,
                replace: None,
            },
            cx,
        );
        self.close_binding_draft(window, cx);
    }

    pub(super) fn new_binding_card(
        &self,
        draft: &NewBinding,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let command = draft.command.as_deref().unwrap_or("");
        let id = "keybinding-new-command";
        let mut commands = BTreeSet::new();
        let defaults = bruhs_models::logic::keybindings::default_keybindings();
        let mut options = self
            .settings_keybindings()
            .iter()
            .chain(defaults.iter())
            .filter(|r| commands.insert(r.command.as_str()))
            .map(|r| DropdownOption {
                value: r.command.clone().into(),
                label: command_label(&r.command),
                description: None,
            })
            .collect::<Vec<_>>();
        options.sort_by(|a, b| a.label.cmp(&b.label));
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let picker = Dropdown::new(
            id,
            "Command",
            command,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            self.settings_listbox(id),
            toggle,
            select,
        )
        .placeholder("Command");

        let key = draft.key.as_str();
        let when = draft.when.read(cx).value();
        let label = gpui_shortcut(key)
            .and_then(|s| Keystroke::parse_with_os(&s, host_os()).ok())
            .map_or_else(
                || "Unassigned".to_string(),
                |k| k.display(host_os()).to_string(),
            );
        let record = web_button(
            "keybinding-new-record",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .track_focus(&draft.focus)
        .debug_selector(|| "keybinding-new-record".into())
        .aria_label("Record new shortcut")
        .child(if self.keybinding_recording == Some(BindingEditor::New) {
            "Press a shortcut…".to_string()
        } else {
            label
        });
        let save = web_button(
            "keybinding-new-save",
            WebButtonVariant::Primary,
            WebButtonSize::Small,
            ButtonState::disabled_if(
                !self.linked_keybindings.editable() || self.new_rule(cx).is_none(),
            ),
        )
        .debug_selector(|| "keybinding-new-save".into())
        .aria_label("Save new keybinding")
        .child("Save");
        let cancel = web_button(
            "keybinding-new-cancel",
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::Enabled,
        )
        .debug_selector(|| "keybinding-new-cancel".into())
        .aria_label("Cancel new keybinding")
        .tooltip(|w, cx| bruhs_ui::tooltip::Tooltip::new("Cancel").build(w, cx))
        .child(crate::views::workspace::updates_ui::icon_svg(
            crate::assets::CLOSE_ICON_PATH,
            14.,
            crate::theme::rambutan_400(),
        ));
        let controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(div().w(px(175.)).max_w_full().child(picker))
            .child(on_press_view(record, cx, |ws, w, cx| {
                if let Some(draft) = &ws.keybinding_draft {
                    ws.keybinding_recording = Some(BindingEditor::New);
                    draft.focus.focus(w, cx);
                }
                cx.notify();
            }))
            .child(on_press_view(save, cx, Self::save_binding_draft))
            .child(on_press_view(cancel, cx, Self::close_binding_draft));
        let details = div()
            .flex()
            .flex_col()
            .min_w_0()
            .max_w_full()
            .gap_1()
            .child(
                div()
                    .text_color(crate::theme::rambutan_100())
                    .child("New keybinding"),
            )
            .child(self.binding_condition(BindingEditor::New, &draft.when, cx));
        div()
            .px_4()
            .py_3()
            .rounded(px(12.))
            .bg(crate::theme::longan_900())
            .border_1()
            .border_color(crate::theme::longan_800())
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(details)
                    .child(controls),
            )
            .when_some(
                conflict_message(self.settings_keybindings(), None, key, &when),
                |el, message| {
                    el.child(
                        div()
                            .debug_selector(|| "keybinding-new-conflict".into())
                            .text_xs()
                            .text_color(crate::theme::durian_300())
                            .child(message),
                    )
                },
            )
            .into_any_element()
    }
}

pub(super) fn conflict_message(
    rules: &[Keybinding],
    except: Option<usize>,
    key: &str,
    when: &str,
) -> Option<String> {
    let mac = cfg!(target_os = "macos");
    let key = parse_chord(key, mac)?;
    let when = when.trim();
    let expression = bruhs_models::logic::keybindings::parse_when(when);
    let labels: BTreeSet<_> = rules
        .iter()
        .enumerate()
        .filter(|(i, r)| {
            let clause = r.when.as_deref().unwrap_or("").trim();
            Some(*i) != except
                && parse_chord(&r.key, mac).as_ref() == Some(&key)
                && (when.is_empty()
                    || clause.is_empty()
                    || clause == when
                    || expression.as_ref().is_some_and(|e| {
                        bruhs_models::logic::keybindings::parse_when(clause).as_ref() == Some(e)
                    }))
        })
        .map(|(_, r)| command_label(&r.command))
        .collect();
    if labels.is_empty() {
        return None;
    }
    Some(format!(
        "Conflicts with {}. The most recent matching shortcut wins.",
        labels.into_iter().collect::<Vec<_>>().join(", ")
    ))
}
