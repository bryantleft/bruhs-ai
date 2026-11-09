use super::*;

#[derive(Clone, Copy)]
pub(super) enum MeetingButtonAction {
    Generate,
    Reextract,
    LoadLatest,
    CancelGeneration,
    DiscoverPending,
    CancelForeign,
}

impl Workspace {
    pub(super) fn meeting_action_button(
        &self,
        selector: &'static str,
        title: &'static str,
        meeting_id: &str,
        action: MeetingButtonAction,
        disabled: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = meeting_id.to_string();
        let key_id = id.clone();
        web_button(
            selector,
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            ButtonState::disabled_if(disabled),
        )
        .debug_selector(move || selector.into())
        .min_h(px(44.0))
        .aria_label(title)
        .when(!disabled, |el| {
            el.on_click(
                cx.listener(move |this, _, _, cx| {
                    this.invoke_meeting_action(id.clone(), action, cx)
                }),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    this.invoke_meeting_action(key_id.clone(), action, cx);
                }
            }))
        })
        .child(title)
        .into_any_element()
    }

    fn invoke_meeting_action(
        &mut self,
        id: String,
        action: MeetingButtonAction,
        cx: &mut Context<'_, Self>,
    ) {
        self.meetings_hub.update(cx, |hub, cx| match action {
            MeetingButtonAction::LoadLatest => hub.retry_meeting_action(id, cx),
            MeetingButtonAction::CancelGeneration => hub.cancel_meeting_action(id, cx),
            MeetingButtonAction::DiscoverPending => hub.discover_pending_generation(id, cx),
            MeetingButtonAction::CancelForeign => hub.cancel_foreign_generation(id, cx),
            MeetingButtonAction::Generate | MeetingButtonAction::Reextract => {
                let extract = hub.prefs.personality.extract_observations;
                hub.summarize(
                    id,
                    extract,
                    matches!(action, MeetingButtonAction::Reextract),
                    cx,
                );
            }
        });
    }

    pub(super) fn render_meeting_action_status(
        &self,
        id: &str,
        status: &crate::views::meetings_hub::MeetingActionStatus,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .id("desktop-meeting-action-status")
            .debug_selector(|| "desktop-meeting-action-status".into())
            .role(if status.recovery {
                Role::Alert
            } else {
                Role::Status
            })
            .flex()
            .flex_col()
            .items_start()
            .gap_2()
            .min_w_0()
            .child(body(
                status.message.clone(),
                BODY_SM_PX,
                theme::rambutan_300(),
            ))
            .when(status.recovery, |el| {
                el.child(self.meeting_action_button(
                    "desktop-meeting-load-latest",
                    "Load latest saved meeting",
                    id,
                    MeetingButtonAction::LoadLatest,
                    false,
                    cx,
                ))
            })
            .when(status.cancellable, |el| {
                el.child(self.meeting_action_button(
                    "desktop-meeting-cancel-generation",
                    "Cancel saved generation",
                    id,
                    MeetingButtonAction::CancelGeneration,
                    false,
                    cx,
                ))
            })
            .when(status.foreign_pending, |el| {
                el.child(self.meeting_action_button(
                    "desktop-meeting-cancel-foreign",
                    "Cancel unresolved server request",
                    id,
                    MeetingButtonAction::CancelForeign,
                    status.busy,
                    cx,
                ))
            })
            .into_any_element()
    }
}
