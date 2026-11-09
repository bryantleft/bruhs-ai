use super::*;

const PAGE_SIZE: usize = 50;

impl Workspace {
    pub(super) fn render_meeting_notes(&self, cx: &Context<'_, Self>) -> AnyElement {
        let total = self.meeting_notes.len();
        let page = self
            .meeting_notes_page
            .min(total.saturating_sub(1) / PAGE_SIZE);
        let start = page * PAGE_SIZE;
        let selected = self
            .selected_meeting_note_id
            .as_deref()
            .and_then(|id| bruhs_models::MeetingNote::find_unique(&self.meeting_notes, id).ok());
        div()
            .id("desktop-meeting-notes")
            .debug_selector(|| "desktop-meeting-notes".into())
            .aria_label("Meeting notes")
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_w_0()
            .px_6()
            .py_3()
            .border_b_1()
            .border_color(theme::longan_800())
            .child(body("Meeting notes", BODY_PX, theme::rambutan_100()))
            .child(self.new_note_button(cx))
            .child(self.render_note_editor(cx))
            .child(body(
                "Local text notes, separate from recordings and generated summaries.",
                BODY_SM_PX,
                theme::rambutan_400(),
            ))
            .when(total == 0, |el| {
                el.child(
                    body(
                        "Create a note here or through an approved workflow.",
                        BODY_SM_PX,
                        theme::rambutan_400(),
                    )
                    .mt_2(),
                )
            })
            .when(total > 0, |el| {
                el.child(
                    div()
                        .id("desktop-meeting-note-titles")
                        .max_h(px(176.0))
                        .overflow_y_scroll()
                        .mt_2()
                        .children(self.meeting_notes.iter().skip(start).take(PAGE_SIZE).map(
                            |note| {
                                let id = note.id.clone();
                                let key_id = id.clone();
                                let key = element_key(&id);
                                web_button(
                                    ("meeting-note", key),
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Default,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(move || format!("desktop-meeting-note-{key}"))
                                .min_h(px(44.0))
                                .justify_start()
                                .w_full()
                                .aria_label(note.title.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.selected_meeting_note_id = Some(id.clone());
                                    cx.notify();
                                }))
                                .on_key_down(cx.listener(
                                    move |this, event: &KeyDownEvent, _, cx| {
                                        if is_web_button_activation_key(
                                            event.keystroke.key.as_str(),
                                        ) {
                                            this.selected_meeting_note_id = Some(key_id.clone());
                                            cx.notify();
                                        }
                                    },
                                ))
                                .child(note.title.clone())
                            },
                        )),
                )
            })
            .when(total > PAGE_SIZE, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(self.meeting_note_page_button(
                            "desktop-previous-notes",
                            "Previous notes",
                            page == 0,
                            page.saturating_sub(1),
                            cx,
                        ))
                        .child(label(
                            format!(
                                "{}–{} of {total}",
                                start + 1,
                                (start + PAGE_SIZE).min(total)
                            ),
                            LABEL_SM_PX,
                            theme::rambutan_300(),
                        ))
                        .child(self.meeting_note_page_button(
                            "desktop-next-notes",
                            "Next notes",
                            start + PAGE_SIZE >= total,
                            page + 1,
                            cx,
                        )),
                )
            })
            .when_some(selected, |el, note| {
                let id = note.id.clone();
                let key_id = id.clone();
                el.child(
                    div()
                        .id("desktop-meeting-note-content")
                        .debug_selector(|| "desktop-meeting-note-content".into())
                        .aria_label("Meeting note content")
                        .tab_index(0)
                        .max_h(px(200.0))
                        .overflow_y_scroll()
                        .mt_2()
                        .p_3()
                        .border_1()
                        .border_color(theme::longan_800())
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .rounded(px(8.0))
                        .child(body(note.body.clone(), BODY_SM_PX, theme::rambutan_200())),
                )
                .child(
                    web_button(
                        "desktop-open-selected-note",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Default,
                        ButtonState::Enabled,
                    )
                    .debug_selector(|| "desktop-open-selected-note".into())
                    .aria_label("Open note")
                    .min_h(px(44.0))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_local_meeting_note(id.clone(), window, cx)
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.open_local_meeting_note(key_id.clone(), window, cx);
                        }
                    }))
                    .child("Open note"),
                )
            })
            .into_any_element()
    }

    fn meeting_note_page_button(
        &self,
        id: &'static str,
        title: &'static str,
        disabled: bool,
        page: usize,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            ButtonState::disabled_if(disabled),
        )
        .debug_selector(move || id.into())
        .min_h(px(44.0))
        .aria_label(title)
        .when(!disabled, |el| {
            el.on_click(cx.listener(move |this, _, _, cx| {
                this.meeting_notes_page = page;
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    this.meeting_notes_page = page;
                    cx.notify();
                }
            }))
        })
        .child(title)
        .into_any_element()
    }
}
