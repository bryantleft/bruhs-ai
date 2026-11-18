//! Keyboard-first universal capture for the native workspace.

use super::*;
use bruhs_ui::press::on_press_view;
use gpui_component::zoom::px;

const CAPTURE_TEXTAREA_HEIGHT_PX: f32 = 118.0;

fn capture_guidance(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Task => "Creates a backlog task immediately.",
        CaptureKind::Note => "Creates a local Markdown document you can link and cite.",
        CaptureKind::Event => "Adds a one hour event to your calendar.",
        CaptureKind::Contact => "Adds a private contact and syncs when the server is available.",
        CaptureKind::MessageDraft => "Saved locally until you choose where to send it.",
    }
}

impl Workspace {
    pub(super) fn open_capture_dialog(
        &mut self,
        _: &OpenCapture,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_transient_surfaces();
        self.open_overlay(Overlay::Capture);
        self.capture_kind = CaptureKind::Task;
        self.capture_edit_id = None;
        self.capture_error = None;
        self.capture_input.update(cx, |input, cx| {
            input.set_placeholder(CaptureKind::Task.placeholder(), window, cx);
            input.set_value("", window, cx);
        });
        let input = self.capture_input.clone();
        cx.defer_in(window, move |_this, window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.notify();
    }

    pub(super) fn open_capture_record(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(record) = self.captures.iter().find(|record| record.id == id).cloned() else {
            return;
        };
        self.close_transient_surfaces();
        self.open_overlay(Overlay::Capture);
        let content = record.content();
        self.capture_kind = record.kind;
        self.capture_edit_id = Some(record.id);
        self.capture_error = None;
        self.capture_input.update(cx, |input, cx| {
            input.set_placeholder(record.kind.placeholder(), window, cx);
            input.set_value(&content, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn select_capture_kind(
        &mut self,
        kind: CaptureKind,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.capture_kind = kind;
        self.capture_error = None;
        self.capture_input.update(cx, |input, cx| {
            input.set_placeholder(kind.placeholder(), window, cx);
        });
        cx.notify();
    }

    fn submit_capture(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let content = self.capture_input.read(cx).value().to_string();
        let now_ms = EpochMs::now().as_millis();
        let Ok(mut record) = CaptureRecord::new(self.capture_kind, &content, now_ms) else {
            self.capture_error = Some("Capture text is required.".to_string());
            self.capture_input
                .update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        };

        if let Some(editing_id) = self.capture_edit_id.as_deref() {
            if let Some(existing) = self
                .captures
                .iter_mut()
                .find(|existing| existing.id == editing_id)
            {
                existing.kind = record.kind;
                existing.title = record.title;
                existing.body = record.body;
                existing.updated_at_ms = now_ms;
            }
        } else {
            match record.kind {
                CaptureKind::Task => {
                    let target_id = uuid::Uuid::new_v4().to_string();
                    self.work_items.push(DesktopWorkItem {
                        id: target_id.clone(),
                        project_id: None,
                        title: record.title.clone(),
                        description: (!record.body.is_empty()).then(|| record.body.clone()),
                        status: DesktopWorkItemStatus::Backlog,
                        priority: DesktopPriority::None,
                        due_date: None,
                        planning: bruhs_models::WorkItemPlanning::default(),
                        updated_at: EpochMs::now(),
                    });
                    record.target_id = Some(target_id);
                }
                CaptureKind::Event => {
                    let target_id = uuid::Uuid::new_v4().to_string();
                    let start_at = default_event_start(Local::now().naive_local());
                    self.calendar_events.push(DesktopCalendarEvent {
                        id: target_id.clone(),
                        title: record.title.clone(),
                        start_at,
                        end_at: start_at + Duration::hours(1),
                        all_day: false,
                        color: Some("persimmon".to_string()),
                        notes: (!record.body.is_empty()).then(|| record.body.clone()),
                        ..Default::default()
                    });
                    record.target_id = Some(target_id);
                }
                CaptureKind::Contact => {
                    let mut person = PersonProfile::new(&record.title);
                    person.created_at = record.created_at_ms;
                    person.updated_at = record.updated_at_ms;
                    record.target_id = Some(person.id.clone());
                    self.meetings_hub
                        .update(cx, |hub, cx| hub.capture_person(person, cx));
                }
                CaptureKind::Note => {
                    let document = KnowledgeDocument::new(
                        record.title.clone(),
                        record.body.clone(),
                        record.created_at_ms,
                    );
                    record.target_id = Some(document.id.clone());
                    self.knowledge_documents.insert(0, document);
                }
                CaptureKind::MessageDraft => {
                    let mut conversation = InboxConversation::new(
                        InboxSource::Import,
                        record.title.clone(),
                        vec![InboxParticipant {
                            id: uuid::Uuid::new_v4().to_string(),
                            display_name: "Recipient".to_string(),
                            address: String::new(),
                            person_id: None,
                        }],
                        record.created_at_ms,
                    );
                    conversation.upsert_draft(record.body.clone(), record.updated_at_ms);
                    record.target_id = Some(conversation.id.clone());
                    self.inbox_conversations.insert(0, conversation);
                }
            }
            self.captures.insert(0, record);
        }

        *self.universal_search_index.borrow_mut() = None;
        self.dismiss_overlay(|overlay| matches!(overlay, Overlay::Capture));
        self.capture_edit_id = None;
        self.capture_error = None;
        self.capture_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.persist_and_notify(cx);
    }

    pub(super) fn render_capture_dialog(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let editing = self.capture_edit_id.is_some();
        let selected = self.capture_kind;
        let empty = self.capture_input.read(cx).value().trim().is_empty();
        let type_tabs = div()
            .id("capture-type-tabs")
            .role(Role::TabList)
            .aria_label("Capture type")
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|element| element.border_color(theme::persimmon_400()))
            .flex()
            .gap_1()
            .rounded(px(8.0))
            .bg(theme::longan_900().opacity(0.5))
            .p_1()
            .children(
                CaptureKind::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, kind)| {
                        let is_selected = kind == selected;
                        let group = SharedString::from(format!("capture-kind-{index}"));
                        let color = if is_selected {
                            theme::rambutan_100()
                        } else {
                            theme::rambutan_500()
                        };
                        div()
                            .id(("capture-kind", index))
                            .group(group.clone())
                            .debug_selector(move || {
                                format!("desktop-capture-kind-{}", kind.label().to_lowercase())
                            })
                            .role(Role::Tab)
                            .aria_label(kind.label())
                            .aria_selected(is_selected)
                            .tab_index(0)
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .focus_visible(|element| element.border_color(theme::persimmon_400()))
                            .flex_1()
                            .min_w_0()
                            .rounded(px(6.0))
                            .px_1()
                            .py_2()
                            // Web: the destination's icon above its label.
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_center()
                            .text_color(color)
                            .when(is_selected, |element| element.bg(theme::longan_700()))
                            .cursor_pointer()
                            .hover(|element| {
                                element
                                    .bg(theme::longan_800())
                                    .text_color(theme::rambutan_200())
                            })
                            .map(|el| {
                                on_press_view(el, cx, move |this, window, cx| {
                                    if this.capture_edit_id.is_none() {
                                        this.select_capture_kind(kind, window, cx);
                                    }
                                })
                            })
                            .child(
                                svg()
                                    .debug_selector(move || {
                                        format!(
                                            "desktop-capture-kind-{}-icon",
                                            kind.label().to_lowercase()
                                        )
                                    })
                                    .path(SharedString::from(format!("icons/{}.svg", kind.icon())))
                                    .size(px(theme::ICON_INLINE_PX))
                                    .flex_shrink_0()
                                    .text_color(color)
                                    .when(!is_selected, |el| {
                                        el.group_hover(group, |el| {
                                            el.text_color(theme::rambutan_200())
                                        })
                                    }),
                            )
                            .child(div().max_w_full().truncate().child(kind.label()))
                    }),
            );

        let body = div()
            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
            .py_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(type_tabs)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::rambutan_400())
                            .child(selected.label()),
                    )
                    .child(
                        form_textarea(Input::new(&self.capture_input), CAPTURE_TEXTAREA_HEIGHT_PX)
                            .aria_label("Capture text"),
                    )
                    .when_some(self.capture_error.clone(), |element, error| {
                        element.child(
                            div()
                                .id("desktop-capture-error")
                                .role(Role::Alert)
                                .text_xs()
                                .text_color(theme::lychee_300())
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::rambutan_500())
                            .child(capture_guidance(selected)),
                    ),
            );

        let footer = div()
            .mt_1()
            .pt_3()
            .border_t_1()
            .border_color(theme::longan_700())
            .flex()
            .justify_end()
            .gap_2()
            .child(
                web_button(
                    "capture-cancel",
                    WebButtonVariant::Ghost,
                    WebButtonSize::Default,
                    ButtonState::Enabled,
                )
                .aria_label("Cancel capture")
                .track_focus(&self.capture_cancel_focus)
                .map(|el| {
                    on_press_view(el, cx, |this, _window, cx| {
                        this.close_transient_surfaces();
                        cx.notify();
                    })
                })
                .child("Cancel"),
            )
            .child(
                web_button(
                    "capture-submit",
                    WebButtonVariant::Primary,
                    WebButtonSize::Default,
                    ButtonState::disabled_if(empty),
                )
                .aria_label(if editing { "Save capture" } else { "Capture" })
                .track_focus(&self.capture_submit_focus.clone().tab_stop(!empty))
                .debug_selector(|| "desktop-capture-submit".to_string())
                // Disabled while empty, like web's `<button disabled>`.
                .when(!empty, |el| {
                    on_press_view(el, cx, |this, window, cx| {
                        this.submit_capture(window, cx);
                    })
                })
                .child(if editing { "Save" } else { "Capture" }),
            );

        self.render_modal_shell(
            if editing {
                "Edit capture"
            } else {
                "Capture anything"
            },
            body,
            Some(footer),
            window,
            cx,
        )
    }
}

impl Workspace {
    /// The capture dialog input; editing clears the last error.
    pub(super) fn new_capture_input(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let capture_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(CaptureKind::Task.placeholder())
                .multi_line(true)
                .rows(5)
        });
        subscriptions.push(cx.subscribe_in(
            &capture_input,
            window,
            |this: &mut Self, _input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.capture_error = None;
                    cx.notify();
                }
            },
        ));
        capture_input
    }
}
