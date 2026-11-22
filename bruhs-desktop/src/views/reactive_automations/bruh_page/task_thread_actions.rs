//! Native thread actions for a scheduled conversation.
use super::*;
use bruhs_models::ids::ThreadId;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) enum TaskThreadDeletion {
    Requested {
        thread_id: ThreadId,
        title: String,
        tasks: BTreeMap<String, String>,
    },
    Deleted(ThreadId),
    Failed(String),
}

pub(super) struct TaskThreadMenu {
    trigger: FocusHandle,
    delete: FocusHandle,
    open: bool,
}

impl TaskThreadMenu {
    pub(super) fn new(cx: &App) -> Self {
        Self {
            trigger: cx.focus_handle().tab_stop(true),
            delete: cx.focus_handle(),
            open: false,
        }
    }
}

fn linked_tasks(snapshot: &ReactiveSnapshot, id: &ThreadId) -> BTreeMap<String, String> {
    snapshot
        .subscriptions
        .iter()
        .filter(|sub| snapshot.task_threads.get(&sub.id) == Some(id))
        .filter_map(|sub| {
            Some((
                sub.id.clone(),
                sub.title(snapshot.agents.iter().find(|a| a.id == sub.agent_id)?)
                    .to_owned(),
            ))
        })
        .collect()
}

impl ReactiveAutomations {
    fn toggle_thread_menu(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(menu) = self.bruh.as_mut().and_then(|p| p.thread_menu.as_mut()) else {
            return;
        };
        menu.open = !menu.open;
        if menu.open {
            menu.delete.focus(window, cx)
        } else {
            menu.trigger.focus(window, cx)
        }
        cx.notify();
    }

    pub(crate) fn delete_task_thread(
        &mut self,
        thread_id: ThreadId,
        linked_tasks: BTreeSet<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.send_all(
            vec![ReactiveCommand::DeleteTaskThread {
                thread_id,
                linked_tasks,
            }],
            Scope::General,
            cx,
        );
    }

    pub(super) fn thread_actions(
        &self,
        location: &TaskThreadLocation,
        title: &str,
        snapshot: &ReactiveSnapshot,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let Some(menu) = self.bruh.as_ref().and_then(|p| p.thread_menu.as_ref()) else {
            return div().into_any_element();
        };
        let trigger = web_button(
            "task-thread-actions",
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::disabled_if(self.busy),
        )
        .track_focus(&menu.trigger)
        .aria_label("Thread actions")
        .aria_expanded(menu.open)
        .debug_selector(|| "task-thread-actions".into())
        .child(
            svg()
                .path(bruhs_ui::icons::ELLIPSIS)
                .size(px(16.))
                .text_color(theme::rambutan_300()),
        )
        .on_activate(cx, |this, w, cx| this.toggle_thread_menu(w, cx));
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = bounds.clone();
        let mut anchor = div()
            .relative()
            .flex_shrink_0()
            .on_children_prepainted(move |b, _, _| {
                if let Some(b) = b.first() {
                    measured.set(*b);
                }
            })
            .child(trigger);
        if menu.open {
            let id = location.thread_id.clone();
            let title = title.to_owned();
            let tasks = linked_tasks(snapshot, &id);
            let delete = web_button(
                "task-thread-delete",
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::disabled_if(self.busy),
            )
            .track_focus(&menu.delete)
            .w_full()
            .justify_start()
            .role(Role::MenuItem)
            .aria_label("Delete thread")
            .debug_selector(|| "task-thread-delete".into())
            .text_color(theme::lychee_300())
            .child(
                svg()
                    .path(bruhs_ui::icons::IconName::Delete.path())
                    .size(px(14.)),
            )
            .child("Delete thread")
            .on_activate(cx, move |this, w, cx| {
                this.toggle_thread_menu(w, cx);
                cx.emit(TaskThreadDeletion::Requested {
                    thread_id: id.clone(),
                    title: title.clone(),
                    tasks: tasks.clone(),
                });
            });
            let popup = div()
                .id("task-thread-action-menu")
                .role(Role::Menu)
                .aria_label("Thread actions")
                .occlude()
                .p_1()
                .w(px(160.))
                .rounded(px(8.))
                .border_1()
                .border_color(theme::ring_floating())
                .bg(theme::longan_800())
                .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if !bounds.get().contains(&event.position) {
                        if let Some(menu) = this.bruh.as_mut().and_then(|p| p.thread_menu.as_mut())
                        {
                            menu.open = false;
                            cx.notify();
                        }
                    }
                }))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                    if matches!(event.keystroke.key.as_str(), "escape" | "tab") {
                        this.toggle_thread_menu(w, cx);
                        if event.keystroke.key == "escape" {
                            w.prevent_default();
                            cx.stop_propagation();
                        }
                    }
                }))
                .on_action(
                    cx.listener(|this, _: &bruhs_ui::actions::CloseOverlays, w, cx| {
                        this.toggle_thread_menu(w, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(delete);
            anchor = anchor.child(div().absolute().right_0().top_0().w(px(160.)).child(
                bruhs_ui::menu_popover::menu_popover(
                    gpui::size(px(160.), px(28.)),
                    popup.into_any_element(),
                ),
            ));
        }
        anchor.into_any_element()
    }
}
