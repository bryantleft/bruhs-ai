use super::*;

impl Workspace {
    pub(in crate::views::workspace) fn open_local_meeting_note(
        &mut self,
        note_id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.active_view = AppView::MeetingNoteDetail { note_id };
        self.close_transient_surfaces();
        self.search_record_focus.focus(window, cx);
        cx.notify();
    }

    pub(in crate::views::workspace) fn render_local_meeting_note(
        &self,
        note_id: &str,
        cx: &Context<'_, Self>,
    ) -> Div {
        let content = match bruhs_models::MeetingNote::find_unique(&self.meeting_notes, note_id) {
            Ok(note) => {
                let edit_id = note.id.clone();
                let key_id = edit_id.clone();
                div()
                    .min_w_0()
                    .child(
                        web_button(
                            "desktop-edit-meeting-note",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Default,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-edit-meeting-note".into())
                        .aria_label("Edit note")
                        .min_h(px(44.0))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_note_by_id(&edit_id, window, cx)
                        }))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.edit_note_by_id(&key_id, window, cx);
                            }
                        }))
                        .child("Edit note"),
                    )
                    .child(body(note.title.clone(), BODY_PX, theme::rambutan_100()))
                    .when_some(note.recording_id.as_ref(), |el, id| {
                        let available = self.meetings_hub.read(cx).meetings.iter().filter(|meeting| meeting.id == *id).count() == 1;
                        let click_id = id.clone(); let key_id = id.clone();
                        el.child(web_button("desktop-open-linked-recording", WebButtonVariant::Ghost, WebButtonSize::Default, ButtonState::disabled_if(!available))
                            .debug_selector(|| "desktop-open-linked-recording".into()).aria_label("Open linked recording").min_h(px(44.0))
                            .when(available, |button| button
                                .on_click(cx.listener(move |this, _, _, cx| this.open_meeting_detail(click_id.clone(), cx)))
                                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default(); cx.stop_propagation(); this.open_meeting_detail(key_id.clone(), cx);
                                    }
                                })))
                            .child("Open linked recording"))
                          .when(!available, |el| el.child(body("Linked recording is not available locally. The note and link are retained.", BODY_SM_PX, theme::rambutan_400())))
                    })
                    .child(
                        body(
                            "Separate from recordings and generated summaries.",
                            BODY_SM_PX,
                            theme::rambutan_400(),
                        )
                        .mt_1(),
                    )
                    .child(
                        body(note.body.clone(), BODY_SM_PX, theme::rambutan_200())
                            .id("desktop-local-meeting-note-content")
                            .debug_selector(|| "desktop-local-meeting-note-content".into())
                            .aria_label("Meeting note content")
                            .tab_index(0)
                            .border_2()
                            .border_color(gpui::transparent_black())
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .mt_4(),
                    )
            }
            Err(message) => div().child(
                body(message.message(), BODY_SM_PX, theme::rambutan_300())
                    .debug_selector(|| "desktop-local-meeting-note-error".into()),
            ),
        };
        div().flex_1().min_h_0().min_w_0().child(
            div()
                .id("desktop-local-meeting-note")
                .debug_selector(|| "desktop-local-meeting-note".into())
                .aria_label("Local meeting note")
                .track_focus(&self.search_record_focus)
                .border_2()
                .border_color(gpui::transparent_black())
                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                .h_full()
                .min_w_0()
                .overflow_y_scroll()
                .p_6()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .mb_4()
                        .child(
                            web_button(
                                "desktop-back-to-meeting-notes",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Default,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-back-to-meeting-notes".into())
                            .min_h(px(44.0))
                            .aria_label("Back to meeting notes")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.activate_sidebar_view(AppView::Meetings, window, cx)
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    this.activate_sidebar_view(AppView::Meetings, window, cx);
                                }
                            }))
                            .child("Back to meeting notes"),
                        )
                        .child(label("Local text note", LABEL_SM_PX, theme::rambutan_400())),
                )
                .child(self.render_note_editor(cx))
                .child(content),
        )
    }
}
