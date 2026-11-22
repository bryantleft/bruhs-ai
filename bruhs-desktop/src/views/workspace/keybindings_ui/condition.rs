//! The raw expression and graphical editor both use the shortcut matcher's grammar.
use super::*;
use crate::views::workspace::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use bruhs_models::logic::keybindings::{condition_variables, parse_when, WhenExpr};
use bruhs_ui::dropdown::{saved_option_value, Dropdown, DropdownGroup, DropdownOption};
use bruhs_ui::press::on_press_view;
use std::rc::Rc;

#[derive(Clone, Copy)]
enum Change {
    Condition,
    Group,
    Negate,
    Operator,
    Remove,
    Variable(&'static str),
}

fn default_node(group: bool) -> WhenExpr {
    let flag = WhenExpr::Flag("terminalFocus".into());
    if group {
        WhenExpr::Or(
            Box::new(flag.clone()),
            Box::new(WhenExpr::Not(Box::new(flag))),
        )
    } else {
        flag
    }
}

/// Rebuild only the path to the selected node. Removing one side of a
/// binary operation promotes its remaining operand.
fn change_node(node: WhenExpr, path: &[bool], change: Change) -> Option<WhenExpr> {
    use WhenExpr::{And, Flag, Not, Or};
    if let Some((&right, tail)) = path.split_first() {
        if let Not(inner) = node {
            return change_node(*inner, path, change).map(|n| Not(Box::new(n)));
        }
        let either = matches!(node, Or(..));
        let (left, rhs) = match node {
            And(l, r) | Or(l, r) => (l, r),
            other => return Some(other),
        };
        let (target, keep) = if right { (rhs, left) } else { (left, rhs) };
        let Some(updated) = change_node(*target, tail, change) else {
            return Some(*keep);
        };
        let (l, r) = if right {
            (keep, Box::new(updated))
        } else {
            (Box::new(updated), keep)
        };
        return Some(if either { Or(l, r) } else { And(l, r) });
    }
    match change {
        Change::Negate => Some(match node {
            Not(inner) => *inner,
            other => Not(Box::new(other)),
        }),
        Change::Remove => None,
        change if matches!(node, Not(_)) => {
            let Not(inner) = node else { unreachable!() };
            change_node(*inner, &[], change).map(|n| Not(Box::new(n)))
        }
        Change::Variable(name) => Some(Flag(name.into())),
        Change::Operator => Some(match node {
            And(l, r) => Or(l, r),
            Or(l, r) => And(l, r),
            other => other,
        }),
        Change::Condition | Change::Group => {
            let child = Box::new(default_node(matches!(change, Change::Group)));
            Some(if matches!(node, Or(..)) {
                Or(Box::new(node), child)
            } else {
                And(Box::new(node), child)
            })
        }
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn dismiss_binding_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.settings_open_dropdown().is_some() {
            return false; // The listbox handles Escape and restores its trigger.
        }
        if let Some(editor) = self.keybinding_when_open.take() {
            if let Some(focus) = self.condition_focus(editor) {
                focus.focus(window, cx);
            }
            cx.notify();
            return true;
        }
        if self.keybinding_draft.is_some() {
            self.close_binding_draft(window, cx);
            return true;
        }
        false
    }

    fn condition_focus(&self, editor: BindingEditor) -> Option<&gpui::FocusHandle> {
        match editor {
            BindingEditor::Existing => Some(&self.keybinding_condition_focus),
            BindingEditor::New => self.keybinding_draft.as_ref().map(|d| &d.condition_focus),
        }
    }

    fn change_binding_condition(
        &mut self,
        path: &[bool],
        change: Change,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (field, editor) = match self.keybinding_when_open {
            Some(BindingEditor::New) => {
                let Some(draft) = self.keybinding_draft.as_ref() else {
                    return;
                };
                (draft.when.clone(), BindingEditor::New)
            }
            Some(editor) => (self.keybinding_when_input.clone(), editor),
            None => return,
        };
        let current = field.read(cx).value();
        let next = if current.trim().is_empty() {
            match change {
                Change::Condition | Change::Group => {
                    Some(default_node(matches!(change, Change::Group)))
                }
                _ => return,
            }
        } else if let Some(node) = parse_when(current.trim()) {
            change_node(node, path, change)
        } else {
            return;
        };
        field.update(cx, |f, cx| {
            f.set_value(next.map_or_else(String::new, |e| e.to_string()), window, cx)
        });
        if editor == BindingEditor::Existing {
            self.commit_keybinding_edit(window, cx);
        }
        cx.notify();
    }

    pub(super) fn binding_condition(
        &self,
        editor: BindingEditor,
        field: &Entity<InputState>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let when = field.read(cx).value();
        let id = match editor {
            BindingEditor::New => "keybinding-new-when",
            BindingEditor::Existing => "keybinding-when-toggle",
        };
        let trigger = web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(move || id.into())
        .when_some(self.condition_focus(editor), |el, focus| {
            el.track_focus(focus)
        })
        .max_w_full()
        .overflow_hidden()
        .aria_label("Edit when clause")
        .child(if when.is_empty() {
            "Always".into()
        } else {
            when.to_string()
        });
        let mut control = div()
            .min_w_0()
            .max_w_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child("When")
                    .child(on_press_view(trigger, cx, move |ws, _, cx| {
                        ws.keybinding_when_open =
                            (ws.keybinding_when_open != Some(editor)).then_some(editor);
                        ws.set_settings_open_dropdown(None);
                        cx.notify();
                    })),
            );
        if self.keybinding_when_open == Some(editor) {
            let node = parse_when(when.trim());
            let invalid = !when.trim().is_empty() && node.is_none();
            let mut panel = div()
                .max_w_full()
                .flex()
                .flex_col()
                .gap_1()
                .border_1()
                .border_color(crate::theme::longan_700())
                .rounded(px(6.))
                .p_2()
                .child(keybinding_field(
                    "desktop-settings-keybinding-when",
                    "When expression".into(),
                    260.,
                    invalid,
                    field,
                ));
            if let Some(node) = node {
                panel = panel.child(self.condition_node(&node, Vec::new(), cx));
            } else if invalid {
                panel = panel.child(
                    div()
                        .text_xs()
                        .child("Use variables with !, &&, || and parentheses."),
                );
            } else {
                panel = panel.child(
                    div()
                        .flex()
                        .gap_1()
                        .child(self.condition_button(&[], "+ Condition", Change::Condition, cx))
                        .child(self.condition_button(&[], "+ Group", Change::Group, cx)),
                );
            }
            control = control.child(panel);
        }
        control.into_any_element()
    }

    fn condition_button(
        &self,
        path: &[bool],
        label: &'static str,
        change: Change,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = format!("when-{}-{label}", path_id(path));
        let button = web_button(
            SharedString::from(id.clone()),
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(move || id)
        .aria_label(label)
        .when(matches!(change, Change::Remove), |el| {
            el.child(crate::views::workspace::updates_ui::icon_svg(
                crate::assets::MINUS_ICON_PATH,
                12.,
                crate::theme::rambutan_400(),
            ))
        })
        .when(!matches!(change, Change::Remove), |el| el.child(label));
        let path = path.to_vec();
        on_press_view(button, cx, move |ws, w, cx| {
            ws.change_binding_condition(&path, change, w, cx)
        })
        .into_any_element()
    }

    fn condition_node(
        &self,
        node: &WhenExpr,
        path: Vec<bool>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        use WhenExpr::{And, Flag, Not, Or};
        if path.len() > 12 {
            return div().text_xs().child(node.to_string()).into_any_element();
        }
        let (node, negated) = if let Not(inner) = node {
            (inner.as_ref(), true)
        } else {
            (node, false)
        };
        let mut header =
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .child(self.condition_button(
                    &path,
                    if negated { "Undo Not" } else { "Not" },
                    Change::Negate,
                    cx,
                ));
        let mut children = Vec::new();
        match node {
            Flag(name) => {
                let id = saved_option_value(&format!("keybinding-variable-{}", path_id(&path)));
                let options = condition_variables()
                    .map(|value| DropdownOption {
                        value,
                        label: value.into(),
                        description: None,
                    })
                    .collect();
                let (toggle, _) = self.settings_dropdown_handlers(id, cx);
                let view = cx.weak_entity();
                let selected = path.clone();
                let select = Rc::new(move |value: &'static str, w: &mut Window, cx: &mut App| {
                    let _ = view.update(cx, |ws, cx| {
                        ws.set_settings_open_dropdown(None);
                        ws.change_binding_condition(&selected, Change::Variable(value), w, cx);
                    });
                });
                header = header.child(
                    Dropdown::new(
                        id,
                        "Condition",
                        name,
                        vec![DropdownGroup {
                            label: None,
                            mark: None,
                            options,
                        }],
                        self.settings_listbox(id),
                        toggle,
                        select,
                    )
                    .compact(),
                );
                if !condition_variables().any(|v| v == name) {
                    header = header.child(format!("Unknown condition: {name}"));
                }
            }
            And(left, right) | Or(left, right) => {
                header = header.child(self.condition_button(
                    &path,
                    if matches!(node, Or(..)) { "or" } else { "and" },
                    Change::Operator,
                    cx,
                ));
                for (side, n) in [(false, left), (true, right)] {
                    let mut p = path.clone();
                    p.push(side);
                    children.push(self.condition_node(n, p, cx));
                }
            }
            Not(_) => {
                header = header.child(node.to_string());
            }
        }
        if path.is_empty() || matches!(node, And(..) | Or(..)) {
            header = header
                .child(self.condition_button(&path, "+ Condition", Change::Condition, cx))
                .child(self.condition_button(&path, "+ Group", Change::Group, cx));
        }
        header = header.child(self.condition_button(&path, "Remove", Change::Remove, cx));
        div()
            .min_w_0()
            .max_w_full()
            .pl_1()
            .border_l_1()
            .border_color(crate::theme::longan_700())
            .flex()
            .flex_col()
            .gap_1()
            .child(header)
            .children(children)
            .into_any_element()
    }
}

fn path_id(path: &[bool]) -> String {
    let mut id = "root".to_owned();
    id.extend(path.iter().map(|right| if *right { 'r' } else { 'l' }));
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_a_nested_group_preserves_its_siblings_and_negation() {
        let mut node = parse_when("isDesktop && !(composerFocus || terminalFocus)").unwrap();
        node = change_node(node, &[true, false], Change::Variable("turnRunning")).unwrap();
        node = change_node(node, &[true], Change::Operator).unwrap();
        assert_eq!(
            node.to_string(),
            "isDesktop && !(turnRunning && terminalFocus)"
        );
        assert_eq!(parse_when(&node.to_string()), Some(node.clone()));
        node = change_node(node, &[true, false], Change::Remove).unwrap();
        assert_eq!(node.to_string(), "isDesktop && !terminalFocus");
        assert!(change_node(node, &[], Change::Remove).is_none());
    }
}
