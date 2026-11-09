//! Native provider-neutral Inbox, sharing the same persisted records and
//! triage semantics as the web surface.

use gpui::prelude::*;
use gpui::*;
use gpui_component::input::Input;
use std::cmp::Reverse;

use super::*;

impl Workspace {
    pub(super) fn open_inbox_view(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        self.active_view = AppView::Inbox;
        self.inbox_import_visible = false;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    pub(super) fn start_inbox_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        for input in [
            &self.inbox_sender_input,
            &self.inbox_subject_input,
            &self.inbox_body_input,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.active_view = AppView::Inbox;
        self.inbox_import_visible = true;
        self.inbox_import_source = InboxSource::Email;
        self.persistence_error = None;
        cx.notify();
        self.inbox_sender_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn cycle_inbox_source(&mut self, cx: &mut Context<'_, Self>) {
        self.inbox_import_source = match self.inbox_import_source {
            InboxSource::Email => InboxSource::Slack,
            InboxSource::Slack => InboxSource::Discord,
            InboxSource::Discord => InboxSource::IMessage,
            InboxSource::IMessage => InboxSource::X,
            InboxSource::X => InboxSource::Import,
            InboxSource::Import => InboxSource::Email,
        };
        cx.notify();
    }

    fn save_inbox_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let sender = self.inbox_sender_input.read(cx).value().trim().to_string();
        let body = self.inbox_body_input.read(cx).value().trim().to_string();
        if sender.is_empty() || body.is_empty() {
            self.persistence_error = Some("A sender and message are required.".to_string());
            cx.notify();
            return;
        }
        let now = EpochMs::now().as_millis();
        let linked_person_id = self
            .meetings_hub
            .read(cx)
            .people
            .iter()
            .find(|person| person.display_name.eq_ignore_ascii_case(&sender))
            .map(|person| person.id.clone());
        let mut conversation = InboxConversation::new(
            self.inbox_import_source,
            self.inbox_subject_input.read(cx).value().to_string(),
            vec![InboxParticipant {
                id: uuid::Uuid::new_v4().to_string(),
                display_name: sender.clone(),
                address: String::new(),
                person_id: linked_person_id,
            }],
            now,
        );
        conversation.receive(InboxMessage::incoming(sender, body, now));
        let id = conversation.id.clone();
        self.inbox_conversations.insert(0, conversation);
        self.inbox_import_visible = false;
        self.select_inbox_conversation(&id, window, cx);
    }

    pub(super) fn select_inbox_conversation(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(conversation) = self
            .inbox_conversations
            .iter_mut()
            .find(|conversation| conversation.id == id)
        else {
            return;
        };
        conversation.mark_read(true);
        let draft = conversation.draft_body().to_string();
        self.inbox_reply_input
            .update(cx, |input, cx| input.set_value(draft, window, cx));
        self.active_view = AppView::ConversationDetail {
            conversation_id: id.to_string(),
        };
        self.persist_and_notify(cx);
    }

    fn mutate_current_conversation(
        &mut self,
        update: impl FnOnce(&mut InboxConversation),
        cx: &mut Context<'_, Self>,
    ) {
        let AppView::ConversationDetail { conversation_id } = &self.active_view else {
            return;
        };
        if let Some(conversation) = self
            .inbox_conversations
            .iter_mut()
            .find(|conversation| conversation.id == *conversation_id)
        {
            update(conversation);
            self.persist_and_notify(cx);
        }
    }

    fn save_inbox_draft(&mut self, cx: &mut Context<'_, Self>) {
        let body = self.inbox_reply_input.read(cx).value().to_string();
        self.mutate_current_conversation(
            move |conversation| conversation.upsert_draft(body, EpochMs::now().as_millis()),
            cx,
        );
    }

    fn send_inbox_draft(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let body = self.inbox_reply_input.read(cx).value().to_string();
        if body.trim().is_empty() {
            return;
        }
        self.mutate_current_conversation(
            move |conversation| {
                let now = EpochMs::now().as_millis();
                conversation.upsert_draft(body, now);
                conversation.send_draft(now);
            },
            cx,
        );
        self.inbox_reply_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn delete_current_conversation(&mut self, cx: &mut Context<'_, Self>) {
        let AppView::ConversationDetail { conversation_id } = &self.active_view else {
            return;
        };
        self.inbox_conversations
            .retain(|conversation| conversation.id != *conversation_id);
        self.active_view = AppView::Inbox;
        self.persist_and_notify(cx);
    }

    fn ask_agent_about_current_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let AppView::ConversationDetail { conversation_id } = &self.active_view else {
            return;
        };
        let Some(conversation) = self
            .inbox_conversations
            .iter()
            .find(|conversation| conversation.id == *conversation_id)
        else {
            return;
        };
        let prompt = bruhs_models::inbox_agent_prompt(conversation);
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        self.active_view = AppView::Chat;
        chat.update(cx, |chat, cx| chat.set_composer_draft(prompt, window, cx));
        self.persist_and_notify(cx);
    }

    pub(super) fn render_inbox_page(&self, cx: &Context<'_, Self>) -> Div {
        if self.inbox_import_visible {
            return self.render_inbox_import(cx);
        }
        let now = EpochMs::now().as_millis();
        let open = self
            .inbox_conversations
            .iter()
            .filter(|conversation| conversation.is_visible_in_inbox(now))
            .count();
        let unread = self
            .inbox_conversations
            .iter()
            .filter(|conversation| conversation.unread && !conversation.archived)
            .count();
        let needs_reply = self
            .inbox_conversations
            .iter()
            .filter(|conversation| conversation.needs_reply() && !conversation.archived)
            .count();
        let mut conversations = self
            .inbox_conversations
            .iter()
            .filter(|conversation| conversation.is_visible_in_inbox(now))
            .collect::<Vec<_>>();
        conversations.sort_unstable_by_key(|conversation| Reverse(conversation.updated_at_ms));
        let cards = conversations
            .into_iter()
            .map(|conversation| self.render_inbox_card(conversation, cx))
            .collect::<Vec<_>>();

        div()
            .debug_selector(|| "desktop-inbox-page".to_string())
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .child(
                                div()
                                    .font_family(theme::HEADING_FONT_FAMILY)
                                    .text_size(px(16.0))
                                    .text_color(theme::rambutan_100())
                                    .child("Inbox"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child("Every conversation, one decisive local queue"),
                            ),
                    )
                    .child(
                        web_button(
                            "desktop-import-conversation",
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("+ Import conversation")
                        .on_click(cx.listener(|this, _event, window, cx| {
                            this.start_inbox_import(window, cx);
                        })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_5()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(900.0))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(inbox_metric("Open", open))
                                    .child(inbox_metric("Unread", unread))
                                    .child(inbox_metric("Needs reply", needs_reply)),
                            )
                            .children(cards)
                            .when(self.inbox_conversations.is_empty(), |element| {
                                element.child(
                                    div()
                                        .py_12()
                                        .text_center()
                                        .text_size(px(13.0))
                                        .text_color(theme::rambutan_500())
                                        .child("Nothing waiting here. A clear inbox is a good place to be."),
                                )
                            })
                            .child(
                                div()
                                    .pt_2()
                                    .text_size(px(12.0))
                                    .text_color(theme::rambutan_400())
                                    .child("Email · Slack · Discord · iMessage · X use one adapter contract; import/share capture works offline now."),
                            ),
                    ),
            )
    }

    fn render_inbox_card(
        &self,
        conversation: &InboxConversation,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = conversation.id.clone();
        let keyboard_id = id.clone();
        let sender = conversation
            .participants
            .first()
            .map_or("Unknown", |participant| participant.display_name.as_str())
            .to_string();
        let preview = conversation
            .last_message()
            .map_or("Draft only", |message| message.body.as_str())
            .replace('\n', " ");
        div()
            .id(("inbox-card", element_key(&id)))
            .role(Role::Button)
            .aria_label(format!("Open conversation: {}", conversation.subject))
            .tab_index(0)
            .cursor_pointer()
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.55))
            .border_1()
            .border_color(theme::longan_800())
            .px_3()
            .py_2()
            .hover(|element| element.bg(theme::longan_800().opacity(0.7)))
            .focus_visible(|element| element.border_2().border_color(theme::persimmon_400()))
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.select_inbox_conversation(&id, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    this.select_inbox_conversation(&keyboard_id, window, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(theme::rambutan_100())
                            .child(sender),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(if conversation.unread {
                                theme::persimmon_300()
                            } else {
                                theme::rambutan_500()
                            })
                            .child(conversation.source.label()),
                    ),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_300())
                    .child(conversation.subject.clone()),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(11.0))
                    .text_color(theme::rambutan_500())
                    .overflow_hidden()
                    .child(preview),
            )
            .into_any_element()
    }

    fn render_inbox_import(&self, cx: &Context<'_, Self>) -> Div {
        div()
            .flex_1()
            .h_full()
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .text_size(px(16.0))
                            .text_color(theme::rambutan_100())
                            .child("Import conversation"),
                    )
                    .child(
                        web_button(
                            "cancel-inbox-import",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Cancel")
                        .on_click(cx.listener(
                            |this, _event, _window, cx| {
                                this.inbox_import_visible = false;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(
                div().p_5().flex().justify_center().child(
                    div()
                        .w_full()
                        .max_w(px(680.0))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme::rambutan_500())
                                .child("Provider"),
                        )
                        .child(
                            web_button(
                                "cycle-inbox-provider",
                                WebButtonVariant::Outline,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .child(self.inbox_import_source.label())
                            .on_click(
                                cx.listener(|this, _event, _window, cx| {
                                    this.cycle_inbox_source(cx)
                                }),
                            ),
                        )
                        .child(Input::new(&self.inbox_sender_input))
                        .child(Input::new(&self.inbox_subject_input))
                        .child(Input::new(&self.inbox_body_input))
                        .when_some(self.persistence_error.clone(), |element, error| {
                            element.child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::lychee_300())
                                    .child(error),
                            )
                        })
                        .child(
                            web_button(
                                "save-inbox-import",
                                WebButtonVariant::Primary,
                                WebButtonSize::Default,
                                ButtonState::Enabled,
                            )
                            .child("Import")
                            .on_click(cx.listener(
                                |this, _event, window, cx| this.save_inbox_import(window, cx),
                            )),
                        ),
                ),
            )
    }

    pub(super) fn render_conversation_detail(
        &self,
        conversation_id: &str,
        cx: &Context<'_, Self>,
    ) -> Div {
        let Some(conversation) = self
            .inbox_conversations
            .iter()
            .find(|conversation| conversation.id == conversation_id)
        else {
            return self.render_inbox_page(cx);
        };
        let messages = conversation
            .messages
            .iter()
            .filter(|message| message.send_state != bruhs_models::MessageSendState::Draft)
            .enumerate()
            .map(|(index, message)| {
                let outgoing = message.direction == bruhs_models::MessageDirection::Outgoing;
                div()
                    .id(("inbox-message", index))
                    .max_w(px(660.0))
                    .rounded(px(6.0))
                    .bg(if outgoing {
                        theme::persimmon_500().opacity(0.10)
                    } else {
                        theme::longan_900()
                    })
                    .border_1()
                    .border_color(if outgoing {
                        theme::persimmon_500().opacity(0.2)
                    } else {
                        theme::longan_800()
                    })
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme::rambutan_500())
                            .child(message.sender_name.clone()),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(13.0))
                            .text_color(theme::rambutan_200())
                            .child(message.body.clone()),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let archived = conversation.archived;
        let unread = conversation.unread;
        let important = conversation.labels.iter().any(|label| label == "important");
        let snoozed = conversation
            .snoozed_until_ms
            .is_some_and(|at| at > EpochMs::now().as_millis());

        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .child(
                div()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        web_button(
                            "back-to-inbox",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("← Inbox")
                        .on_click(
                            cx.listener(|this, _event, window, cx| {
                                this.open_inbox_view(window, cx)
                            }),
                        ),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .text_size(px(15.0))
                            .text_color(theme::rambutan_100())
                            .child(conversation.subject.clone()),
                    )
                    .child(
                        web_button(
                            "inbox-read",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(if unread { "Mark read" } else { "Mark unread" })
                        .on_click(cx.listener(
                            move |this, _event, _window, cx| {
                                this.mutate_current_conversation(
                                    |conversation| conversation.mark_read(unread),
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(
                        web_button(
                            "inbox-snooze",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(if snoozed { "Unsnooze" } else { "Tomorrow" })
                        .on_click(cx.listener(
                            move |this, _event, _window, cx| {
                                this.mutate_current_conversation(
                                    |conversation| {
                                        conversation.snooze_until(if snoozed {
                                            None
                                        } else {
                                            Some(EpochMs::now().as_millis() + 86_400_000)
                                        })
                                    },
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(
                        web_button(
                            "inbox-important",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(if important { "Unstar" } else { "Important" })
                        .on_click(cx.listener(
                            |this, _event, _window, cx| {
                                this.mutate_current_conversation(
                                    |conversation| conversation.toggle_label("important"),
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(
                        web_button(
                            "inbox-archive",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(if archived { "Unarchive" } else { "Archive" })
                        .on_click(cx.listener(
                            move |this, _event, _window, cx| {
                                this.mutate_current_conversation(
                                    |conversation| conversation.set_archived(!archived),
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(
                        web_button(
                            "inbox-ask-agent",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Ask agent")
                        .on_click(cx.listener(
                            |this, _event, window, cx| {
                                this.ask_agent_about_current_conversation(window, cx)
                            },
                        )),
                    )
                    .child(
                        web_button(
                            "inbox-delete",
                            WebButtonVariant::Destructive,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Delete")
                        .on_click(cx.listener(
                            |this, _event, _window, cx| this.delete_current_conversation(cx),
                        )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(messages),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(theme::longan_800())
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&self.inbox_reply_input))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                web_button(
                                    "save-inbox-draft",
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Save draft")
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| this.save_inbox_draft(cx),
                                )),
                            )
                            .child(
                                web_button(
                                    "send-inbox-draft",
                                    WebButtonVariant::Primary,
                                    WebButtonSize::Small,
                                    ButtonState::disabled_if(
                                        self.inbox_reply_input.read(cx).value().trim().is_empty(),
                                    ),
                                )
                                .child("Log reply")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| this.send_inbox_draft(window, cx),
                                )),
                            ),
                    ),
            )
    }
}

fn inbox_metric(label: &'static str, value: usize) -> Div {
    div()
        .flex_1()
        .rounded(px(6.0))
        .bg(theme::longan_900().opacity(0.55))
        .border_1()
        .border_color(theme::longan_800())
        .px_3()
        .py_2()
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme::rambutan_500())
                .child(label),
        )
        .child(
            div()
                .mt_1()
                .font_family(theme::HEADING_FONT_FAMILY)
                .text_size(px(18.0))
                .text_color(theme::rambutan_100())
                .child(value.to_string()),
        )
}

/// The conversation import form and the reply box.
pub(super) struct InboxInputs {
    pub(super) inbox_sender_input: Entity<InputState>,
    pub(super) inbox_subject_input: Entity<InputState>,
    pub(super) inbox_body_input: Entity<InputState>,
    pub(super) inbox_reply_input: Entity<InputState>,
}

impl Workspace {
    /// The conversation import form and the reply box; edits redraw.
    pub(super) fn new_inbox_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> InboxInputs {
        let inbox_sender_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name or handle"));
        subscriptions.push(redraw_on_edit(&inbox_sender_input, window, cx));
        let inbox_subject_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Subject (optional)"));
        subscriptions.push(redraw_on_edit(&inbox_subject_input, window, cx));
        let inbox_body_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Paste the message…")
                .multi_line(true)
                .rows(8)
        });
        subscriptions.push(redraw_on_edit(&inbox_body_input, window, cx));
        let inbox_reply_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Write a reply…")
                .multi_line(true)
                .rows(4)
        });
        subscriptions.push(redraw_on_edit(&inbox_reply_input, window, cx));
        InboxInputs {
            inbox_sender_input,
            inbox_subject_input,
            inbox_body_input,
            inbox_reply_input,
        }
    }
}
