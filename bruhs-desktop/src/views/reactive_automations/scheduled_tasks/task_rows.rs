//! The task enable switch and keyboard-accessible actions menu.
use super::*;
use bruhs_ui::{menu_popover::menu_popover, press::Interactivity, switch::switch};
use std::cell::Cell;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskAction {
    Edit,
    Deliveries,
    Run,
    Delete,
}

impl TaskAction {
    fn presentation(self) -> (&'static str, &'static str) {
        match self {
            Self::Edit => ("Edit", crate::assets::TOOL_PENCIL_ICON_PATH),
            Self::Deliveries => ("Deliveries", crate::assets::CLIPBOARD_LIST_ICON_PATH),
            Self::Run => ("Run now", crate::assets::ACTIVITY_ICON_PATH),
            Self::Delete => ("Delete", bruhs_ui::icons::IconName::Delete.path()),
        }
    }
}

impl ReactiveAutomations {
    pub(crate) fn dismiss_task_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.task_dialog.menu_task.take().is_none() {
            return false;
        }
        self.task_dialog.return_focus.focus(window, cx);
        cx.notify();
        true
    }

    pub(super) fn scheduled_row_actions(
        &self,
        setup: AutomationSetup,
        cx: &Context<'_, Self>,
    ) -> Div {
        let id = setup.subscription.id.clone();
        let open = self.task_dialog.menu_task.as_ref() == Some(&id);
        let active = setup.enabled();
        let toggle_id = id.clone();
        let toggle = switch(
            format!("settings-enable-task-{id}"),
            format!("Enable {}", setup.title()),
            active,
            Interactivity::disabled_if(self.busy),
        )
        .on_activate(cx, move |this, _, cx| {
            this.send(
                ReactiveCommand::SetAutomationEnabled {
                    id: toggle_id.clone(),
                    enabled: !active,
                },
                cx,
            )
        });
        let menu_id = id.clone();
        let trigger = web_button(
            SharedString::from(format!("settings-task-actions-{id}")),
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::disabled_if(self.busy),
        )
        .size(px(28.))
        .aria_label(format!("Actions for {}", setup.title()))
        .aria_expanded(open)
        .debug_selector(move || format!("task-actions-{menu_id}"))
        .when(self.task_dialog.origin.as_ref() == Some(&id), |b| {
            b.track_focus(&self.task_dialog.return_focus)
        })
        .child(
            svg()
                .path(bruhs_ui::icons::ELLIPSIS)
                .size(px(16.))
                .text_color(theme::rambutan_300()),
        )
        .on_activate(cx, move |this, window, cx| {
            if open {
                this.dismiss_task_menu(window, cx);
            } else {
                this.task_dialog.origin = Some(id.clone());
                this.task_dialog.menu_task = Some(id.clone());
                this.task_dialog.menu_items[0].focus(window, cx);
                cx.notify();
            }
        });
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = bounds.clone();
        let anchor =
            div()
                .relative()
                .flex_shrink_0()
                .on_children_prepainted(move |children, _, _| {
                    if let Some(first) = children.first() {
                        measured.set(*first);
                    }
                })
                .child(trigger)
                .when(open, |anchor| {
                    let menu = self.scheduled_row_menu(setup, bounds, cx);
                    anchor.child(div().absolute().right_0().top_0().w(px(160.)).child(
                        menu_popover(size(px(160.), px(28.)), menu.into_any_element()),
                    ))
                });
        div()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .child(toggle)
            .child(anchor)
    }

    fn scheduled_row_menu(
        &self,
        setup: AutomationSetup,
        trigger: Rc<Cell<Bounds<Pixels>>>,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let mut menu = div()
            .id("scheduled-task-actions-menu")
            .debug_selector(|| "scheduled-task-actions-menu".into())
            .role(Role::Menu)
            .aria_label(format!("Actions for {}", setup.title()))
            .w(px(160.))
            .occlude()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_800())
            .p_1()
            .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                if !trigger.get().contains(&event.position) {
                    this.task_dialog.menu_task = None;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let handles = &this.task_dialog.menu_items;
                let current = handles.iter().position(|handle| handle.is_focused(window));
                if let Some(next) = bruhs_models::logic::menu::menu_step(
                    current,
                    handles.len(),
                    event.keystroke.key.as_str(),
                ) {
                    window.prevent_default();
                    cx.stop_propagation();
                    handles[next].focus(window, cx);
                }
            }));
        let run_action = if setup.source.kind == SourceKind::Webhook {
            TaskAction::Deliveries
        } else {
            TaskAction::Run
        };
        for (slot, action) in [TaskAction::Edit, run_action, TaskAction::Delete]
            .into_iter()
            .enumerate()
        {
            let (label, icon) = action.presentation();
            if action == TaskAction::Delete {
                menu = menu.child(div().h(px(1.)).my_1().bg(theme::ring_modal()));
            }
            let task = setup.clone();
            menu =
                menu.child(
                    web_button(
                        SharedString::from(format!("task-action-{slot}")),
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::disabled_if(self.busy),
                    )
                    .aria_label(label)
                    .debug_selector(move || format!("reactive-{label}"))
                    .child(svg().path(icon).size(px(14.)).text_color(
                        if action == TaskAction::Delete {
                            theme::lychee_300()
                        } else {
                            theme::rambutan_400()
                        },
                    ))
                    .child(label)
                    .role(Role::MenuItem)
                    .track_focus(&self.task_dialog.menu_items[slot])
                    .w_full()
                    .justify_start()
                    .when(action == TaskAction::Delete, |b| {
                        b.text_color(theme::lychee_300())
                    })
                    .on_activate(cx, move |this, window, cx| {
                        this.dismiss_task_menu(window, cx);
                        match action {
                            TaskAction::Edit => {
                                this.edit_scheduled_task(Some(task.clone()), window, cx)
                            }
                            TaskAction::Deliveries => {
                                this.task_dialog.restore_focus = true;
                                this.task_dialog.focus_on_open = true;
                                this.task_dialog.scroll = ScrollHandle::new();
                                this.open_task_deliveries(
                                    task.subscription.id.clone(),
                                    task.title().to_owned(),
                                    cx,
                                );
                            }
                            TaskAction::Run => this.send(
                                ReactiveCommand::RunScheduledTask {
                                    id: task.subscription.id.clone(),
                                    run_id: uuid::Uuid::new_v4().to_string(),
                                },
                                cx,
                            ),
                            TaskAction::Delete => this.confirm_delete_scheduled_task(
                                &task.subscription.id,
                                task.title(),
                                window,
                                cx,
                            ),
                        }
                    }),
                );
        }
        menu
    }
}
