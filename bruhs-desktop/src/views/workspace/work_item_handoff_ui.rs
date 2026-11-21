use super::*;
use bruhs_models::workbench_handoff::WorkItemHandoffRequest;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct HandoffUi {
    pending: Option<String>,
    error: Option<(String, String)>,
    pub(super) open: Option<bruhs_models::ids::ThreadId>,
    #[cfg(test)]
    client: Option<ServerClient>,
}

impl Workspace {
    fn prepare_work_item_handoff(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if self.work_item_handoff.pending.is_some() || !self.save_status.is_saved() {
            return;
        }
        let Some(item) = self.work_items.iter().find(|item| item.id == id) else {
            return;
        };
        let request = WorkItemHandoffRequest {
            item: item.to_work_item(),
            project_path: self
                .active_project
                .as_ref()
                .map_or_else(String::new, |project| project.path.clone()),
            model: self.current_model.clone(),
            effort: self
                .claude_effort
                .clone()
                .filter(|_| bruhs_models::coordination::task_effort_supported(&self.current_model)),
            permission_mode: self.permission_mode,
        };
        let source_id = id.to_owned();
        let reviewed_item = request.item.clone();
        self.work_item_handoff.pending = Some(source_id.clone());
        self.work_item_handoff.error = None;
        let client = {
            #[cfg(test)]
            if let Some(client) = self.work_item_handoff.client.clone() {
                client
            } else {
                ServerClient::workbench_client()
            }
            #[cfg(not(test))]
            ServerClient::workbench_client()
        };
        let prepared = crate::app_state::spawn_tokio(cx, async move {
            client
                .prepare_work_item_handoff(&request)
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = prepared.await;
            let _ = this.update(cx, |this, cx| {
                this.work_item_handoff.pending = None;
                match result {
                    Ok(handoff) => {
                        if this.active_view == AppView::Tasks
                            && this.selected_work_item_id.as_deref() == Some(source_id.as_str())
                            && this
                                .work_items
                                .iter()
                                .any(|item| item.to_work_item() == reviewed_item)
                            && this.save_status.is_saved()
                        {
                            this.work_item_handoff.open = Some(handoff.id);
                            this.active_view = AppView::Workbench;
                            this.persist_and_notify(cx);
                        }
                    }
                    Err(error) => this.work_item_handoff.error = Some((source_id, error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_work_item_handoff(
        &self,
        item: &DesktopWorkItem,
        cx: &Context<'_, Self>,
    ) -> Div {
        let id = item.id.clone();
        let keyboard_id = id.clone();
        let enabled = self.work_item_handoff.pending.is_none() && self.save_status.is_saved();
        div().flex().flex_col().gap(px(8.0))
            .child(web_button(("task-agent-review", element_key(&item.id)), WebButtonVariant::Outline,
                WebButtonSize::Small, if enabled { ButtonState::Enabled } else { ButtonState::Disabled })
                .debug_selector(|| "desktop-task-agent-review".into())
                .role(Role::Button).aria_label("Review agent work").tab_index(0)
                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                .child("Review agent work")
                .on_click(cx.listener(move |this, _, _, cx| this.prepare_work_item_handoff(&id, cx)))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default(); cx.stop_propagation();
                        this.prepare_work_item_handoff(&keyboard_id, cx);
                    }
                })))
            .child(div().text_size(px(12.0)).text_color(theme::rambutan_400())
                .child("Review this task in Workbench, then choose the project, model and access before starting. The personal task stays unchanged."))
            .when(self.work_item_handoff.pending.as_deref() == Some(item.id.as_str()), |el| el.child(div().child("Preparing agent work…")))
            .when(!self.save_status.is_saved(), |el| el.child(div().child("Save task changes before preparing agent work.")))
            .when_some(self.work_item_handoff.error.as_ref().filter(|(id,_)| id == &item.id), |el, (_,error)| el.child(div().text_color(theme::lychee_300()).child(error.clone())))
    }
}
