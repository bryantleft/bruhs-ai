//! Native provider-neutral Inbox, sharing the same persisted records and
//! triage semantics as the web surface.

use bruhs_models::{InboxCard, InboxDirectory, InboxFilter, INBOX_PAGE_SIZE};
use gpui::Toggled;
use gpui_component::input::{Input, InputEvent};
use gpui_component::zoom::px;

use super::*;
use bruhs_ui::press::on_press_view;

/// The Linked person dropdown's trigger and options.
const LINKED_PERSON_DROPDOWN_ID: &str = "desktop-inbox-linked-person";
/// Web `sm:px-5` and `px-3`: a conversation's header, Linked person row and
/// reply bar.
const CONVERSATION_GUTTER_PX: f32 = 20.0;
const CONVERSATION_GUTTER_COMPACT_PX: f32 = 12.0;
/// Web `max-w-3xl`: the message column and the reply box.
const CONVERSATION_COLUMN_PX: f32 = 768.0;
/// Web `max-w-[88%]`: a message's share of the column.
const CONVERSATION_MESSAGE_WIDTH: f32 = 0.88;
/// Web `rows=3` at `text-body` with `py-1.5`.
const REPLY_BOX_HEIGHT_PX: f32 = 75.0;

/// A dropdown value is `'static` and a person's id is not, so each distinct
/// id is kept once for the life of the app (like Settings' project names).
fn person_value(id: &str) -> &'static str {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, PoisonError};
    static VALUES: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut values = VALUES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(known) = values.get(id) {
        return known;
    }
    let kept: &'static str = Box::leak(id.to_string().into_boxed_str());
    values.insert(kept);
    kept
}

/// A row's accessible name, in the order the web row reads from its
/// content: read state, sender, date, subject, Important, preview and source.
pub(super) fn inbox_row_label(card: &InboxCard) -> String {
    let subject = if card.subject.trim().is_empty() {
        "(No subject)"
    } else {
        card.subject.as_str()
    };
    let state = if card.unread {
        "Unread conversation:"
    } else {
        "Read conversation:"
    };
    let date = inbox_date(card.message_at_ms);
    let source = format!("{} import", card.source.label());
    [
        state,
        card.sender.as_str(),
        date.as_str(),
        subject,
        if card.important { "Important" } else { "" },
        card.preview.as_str(),
        source.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

impl Workspace {
    pub(super) fn open_inbox_view(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.active_view = AppView::Inbox;
        self.inbox_import_visible = false;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.workspace_focus_handle.focus(window, cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn start_inbox_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let inputs: [&dyn TextInput; 3] = [
            &self.inbox_sender_input,
            &self.inbox_subject_input,
            &self.inbox_body_input,
        ];
        for input in inputs {
            input.set_text_value("".into(), window, cx);
        }
        // Like the web dialog, closing the form returns focus to the button
        // that opened it.
        self.inbox_import_returns_focus = self.inbox_import_focus.is_focused(window);
        // The command bar's Import conversation closes the bar, as the web's.
        self.close_transient_surfaces();
        self.active_view = AppView::Inbox;
        self.inbox_import_visible = true;
        self.inbox_mail_visible = false;
        self.inbox_import_source = InboxSource::Email;
        self.persistence_error = None;
        cx.notify();
        self.inbox_sender_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Cancel or Escape: close the import form. Focus returns to Import
    /// conversation when it opened the form, as the web dialog restores its
    /// invoker; otherwise (the command bar opened it) to the page.
    pub(super) fn cancel_inbox_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.inbox_import_visible = false;
        if std::mem::take(&mut self.inbox_import_returns_focus) {
            self.inbox_import_focus.focus(window, cx);
        } else {
            self.workspace_focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    /// The import form's provider choice, a pressed button as on the web.
    fn inbox_source_button(&self, source: InboxSource, cx: &Context<'_, Self>) -> AnyElement {
        let selected = self.inbox_import_source == source;
        web_button(
            ("inbox-import-source", element_key(source.label())),
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            ButtonState::Enabled,
        )
        .debug_selector(move || format!("desktop-inbox-import-source-{}", source.label()))
        .aria_label(source.label())
        .aria_toggled(if selected {
            Toggled::True
        } else {
            Toggled::False
        })
        .text_size(px(12.0))
        .rounded(px(4.0))
        .border_color(if selected {
            theme::blueberry_500().opacity(0.3)
        } else {
            theme::longan_700()
        })
        .when(selected, |button| {
            button
                .bg(theme::blueberry_500().opacity(0.15))
                .text_color(theme::blueberry_400())
        })
        .when(!selected, |button| button.text_color(theme::rambutan_400()))
        .child(source.label())
        .map(|el| {
            on_press_view(el, cx, move |this, _, cx| {
                this.inbox_import_source = source;
                cx.notify();
            })
        })
        .into_any_element()
    }

    pub(super) fn save_inbox_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let sender = self.inbox_sender_input.read(cx).value().trim().to_string();
        let body = self.inbox_body_input.read(cx).value().trim().to_string();
        if sender.is_empty() || body.is_empty() {
            self.persistence_error = Some("A sender and message are required.".to_string());
            cx.notify();
            return;
        }
        let now = EpochMs::now().as_millis();
        // Display names and aliases, Unicode case folded; a name two
        // profiles answer to links to neither.
        let linked_person_id = bruhs_models::unique_person_named(
            self.meetings_hub.read(cx).people.iter().map(Arc::as_ref),
            &sender,
        )
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
        self.inbox_person_menu_open = false;
        self.inbox_reply_input
            .update(cx, |input, cx| input.set_value(draft, window, cx));
        self.active_view = AppView::ConversationDetail {
            conversation_id: id.to_string(),
        };
        self.inbox_mail_visible = false;
        self.workspace_focus_handle.focus(window, cx);
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

    /// Web's Linked person: link the conversation's first participant to
    /// `person_id`, or unlink it with an empty id, and save.
    pub(super) fn link_current_conversation_person(
        &mut self,
        person_id: &str,
        cx: &mut Context<'_, Self>,
    ) {
        self.inbox_person_menu_open = false;
        let AppView::ConversationDetail { conversation_id } = &self.active_view else {
            return;
        };
        let changed = self
            .inbox_conversations
            .iter_mut()
            .find(|conversation| conversation.id == *conversation_id)
            .is_some_and(|conversation| conversation.link_person(Some(person_id.to_string())));
        if changed {
            self.persist_and_notify(cx);
        } else {
            cx.notify();
        }
    }

    /// Web `ConversationDetail`'s Linked person row: Unlinked, then every
    /// person in the directory.
    fn render_linked_person(
        &self,
        conversation: &InboxConversation,
        narrow: bool,
        gutter: f32,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let options = std::iter::once(DropdownOption {
            value: "",
            label: "Unlinked".to_string(),
            description: None,
        })
        .chain(
            self.meetings_hub
                .read(cx)
                .people
                .iter()
                .map(|person| DropdownOption {
                    value: person_value(&person.id),
                    label: person.display_name.clone(),
                    description: None,
                }),
        )
        .collect();
        let entity = cx.entity().downgrade();
        let toggle_entity = entity.clone();
        let on_toggle: ToggleHandler = Rc::new(move |_window, cx| {
            let _ = toggle_entity.update(cx, |this, cx| {
                this.inbox_person_menu_open = !this.inbox_person_menu_open;
                cx.notify();
            });
        });
        let on_select: SelectHandler = Rc::new(move |value, _window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.link_current_conversation_person(value, cx);
            });
        });
        div()
            .id("desktop-inbox-linked-person-row")
            .flex_shrink_0()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .border_b_1()
            .border_color(theme::longan_800().opacity(0.7))
            .px(px(gutter))
            .py_2()
            .child(
                div()
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::rambutan_500())
                    .child("Linked person"),
            )
            .child(
                div()
                    // Web `w-full sm:w-48`.
                    .when(narrow, |slot| slot.w_full())
                    .when(!narrow, |slot| slot.w(px(192.0)))
                    .child(dropdown(
                        LINKED_PERSON_DROPDOWN_ID,
                        "Linked person",
                        conversation.linked_person_id().unwrap_or_default(),
                        vec![DropdownGroup {
                            label: None,
                            mark: None,
                            options,
                        }],
                        Listbox::open_if(self.inbox_person_menu_open),
                        on_toggle,
                        on_select,
                    )),
            )
            .into_any_element()
    }

    pub(super) fn save_inbox_draft(&mut self, cx: &mut Context<'_, Self>) {
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

    pub(super) fn delete_current_conversation(&mut self, cx: &mut Context<'_, Self>) {
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
        if self.inbox_mail_visible {
            return div()
                .flex_1()
                .h_full()
                .min_w_0()
                .child(self.connected_inbox.clone());
        }
        let now = EpochMs::now().as_millis();
        let query = self.inbox_search_input.read(cx).value();
        let mut cache = self.inbox_directory.borrow_mut();
        if cache
            .as_ref()
            .is_some_and(|(count, _)| *count != self.inbox_conversations.len())
        {
            cache.take();
        }
        let (_, directory) = cache.get_or_insert_with(|| {
            (
                self.inbox_conversations.len(),
                InboxDirectory::new(&self.inbox_conversations),
            )
        });
        let matches = directory.matching(&query, self.inbox_filter, now);
        let counts = directory.counts(now);
        let total = matches.len();
        let start = InboxDirectory::page_start(total, self.inbox_page);
        let cards = directory
            .page(&matches, self.inbox_page)
            .into_iter()
            .map(|card| self.render_inbox_card(&card, cx))
            .collect::<Vec<_>>();
        let (empty, next_step) =
            inbox_empty_state(&query, self.inbox_filter, self.saved_records_loaded);

        div().debug_selector(|| "desktop-inbox-page".into())
            .flex_1().h_full().min_w_0().bg(cx.theme().background).flex().flex_col()
            .child(div().px_4().py_3().border_b_1().border_color(theme::longan_800())
                .flex().flex_wrap().gap_3().items_center().justify_between()
                .child(div().min_w_0()
                    .child(div().id("desktop-inbox-title").role(Role::Heading).aria_level(1).aria_label("Inbox").font_family(theme::HEADING_FONT_FAMILY).text_size(px(18.0)).text_color(theme::rambutan_100()).child("Inbox"))
                    .child(div().mt_0p5().text_size(px(12.0)).text_color(theme::rambutan_500()).child("Imported conversations and local reply drafts.")))
                .child(div().flex().flex_wrap().gap_2()
                .child(web_button("desktop-inbox-email", WebButtonVariant::Outline, WebButtonSize::Small, ButtonState::Enabled)
                    .debug_selector(|| "desktop-inbox-email".into()).aria_label("Email").child("Email")
                    .map(|el| on_press_view(el, cx, |this, _, cx| this.open_connected_inbox(cx))))
                .child(web_button("desktop-import-conversation", WebButtonVariant::Primary, WebButtonSize::Small, ButtonState::Enabled)
                    .debug_selector(|| "desktop-import-conversation".into())
                    .aria_label("Import conversation").child("Import conversation")
                    .track_focus(&self.inbox_import_focus)
                    .on_click(cx.listener(|this, _, window, cx| this.start_inbox_import(window, cx))))))
            .child(div().flex_1().min_h_0().overflow_y_scrollbar().p_4().flex().justify_center()
                .child(div().w_full().max_w(px(1024.0)).flex().flex_col().gap_4()
                    .child(Input::new(&self.inbox_search_input).aria_label("Search inbox").role(Role::SearchInput))
                    .child(div().id("desktop-inbox-filter-group").role(Role::Group).aria_label("Inbox filter").flex().flex_wrap().gap_1()
                        .children(InboxFilter::ALL.into_iter().map(|value| self.inbox_filter_button(value, cx))))
                    .child(div().id("desktop-inbox-conversations").role(Role::Region).aria_label("Conversations")
                        .flex().flex_col().rounded(px(8.0)).border_1().border_color(theme::longan_800()).overflow_hidden()
                        .children(cards)
                        .when(total == 0, |element| element.child(div().py_12().px_4().text_center()
                            .child(div().id("desktop-inbox-empty").debug_selector(|| "desktop-inbox-empty".into()).role(Role::Status).aria_label(empty).text_size(px(13.0)).text_color(theme::rambutan_300()).child(empty))
                            .child(div().mt_1().text_size(px(12.0)).text_color(theme::rambutan_500())
                                .child(next_step)))))
                    .child(div().id("desktop-inbox-counts").role(Role::Status).aria_label("Inbox counts").flex().flex_wrap().gap_4().text_size(px(12.0)).text_color(theme::rambutan_400())
                        .child(format!("{} open", counts.open)).child(format!("{} unread", counts.unread))
                        .child(format!("{} need reply", counts.needs_reply)).child(format!("{} snoozed", counts.snoozed)))
                    .child(div().text_size(px(12.0)).text_color(theme::rambutan_400())
                        .child("Imported messages and replies stay on this device. Choose Email to read and sync connected Gmail."))))
            .when(total > INBOX_PAGE_SIZE, |element| element.child(
                div().id("desktop-inbox-pages").role(Role::Navigation).aria_label("Inbox pages")
                    .flex().items_center().justify_between().px_3().py_2()
                    .border_t_1().border_color(theme::longan_800())
                    .child(self.inbox_page_button("desktop-inbox-previous", "Previous", start == 0, (start / INBOX_PAGE_SIZE).saturating_sub(1), cx))
                    .child(div().id("desktop-inbox-page-status").role(Role::Status).text_size(px(12.0)).text_color(theme::rambutan_400()).child(format!("{} to {} of {}", start + 1, (start + INBOX_PAGE_SIZE).min(total), total)))
                    .child(self.inbox_page_button("desktop-inbox-next", "Next", start + INBOX_PAGE_SIZE >= total, start / INBOX_PAGE_SIZE + 1, cx))))
    }

    fn inbox_filter_button(&self, value: InboxFilter, cx: &Context<'_, Self>) -> AnyElement {
        let selected = self.inbox_filter == value;
        web_button(
            ("inbox-filter", element_key(value.label())),
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            ButtonState::Enabled,
        )
        .debug_selector(move || format!("desktop-inbox-filter-{}", value.label()))
        .aria_label(value.label())
        .aria_toggled(if selected {
            Toggled::True
        } else {
            Toggled::False
        })
        .text_size(px(12.0))
        .rounded(px(4.0))
        .when(selected, |button| {
            button
                .bg(theme::blueberry_500().opacity(0.15))
                .text_color(theme::blueberry_400())
        })
        .child(value.label())
        .map(|el| {
            on_press_view(el, cx, move |this, _, cx| {
                this.inbox_filter = value;
                this.inbox_page = 0;
                cx.notify();
            })
        })
        .into_any_element()
    }

    fn inbox_page_button(
        &self,
        id: &'static str,
        label: &'static str,
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
        .aria_label(label)
        .min_h(px(theme::CONTROL_PX))
        .when(!disabled, |button| {
            on_press_view(button, cx, move |this, _, cx| {
                this.inbox_page = page;
                cx.notify();
            })
        })
        .child(label)
        .into_any_element()
    }

    fn render_inbox_card(&self, conversation: &InboxCard, cx: &Context<'_, Self>) -> AnyElement {
        let id = conversation.id.clone();
        let sender = conversation.sender.clone();
        let preview = conversation.preview.clone();
        let key = element_key(&id);
        div()
            .id(("inbox-card", key))
            .debug_selector(move || format!("desktop-inbox-card-{key}"))
            .role(Role::Button)
            .aria_label(inbox_row_label(conversation))
            .tab_index(0)
            .cursor_pointer()
            .w_full()
            .min_w_0()
            .bg(theme::longan_900().opacity(0.45))
            .border_b_1()
            .border_color(theme::longan_800())
            .px_3()
            .py_3()
            .hover(|element| element.bg(theme::longan_800().opacity(0.7)))
            .focus_visible(|element| element.border_color(theme::persimmon_400()))
            .map(|el| {
                on_press_view(el, cx, move |this, window, cx| {
                    this.select_inbox_conversation(&id, window, cx);
                })
            })
            .flex()
            .items_start()
            .gap_3()
            // Web: a dot marks unread rows (blueberry), read ones stay muted.
            .child(
                div()
                    .debug_selector(move || format!("desktop-inbox-card-dot-{key}"))
                    .mt(px(6.0))
                    .size(px(8.0))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(if conversation.unread {
                        theme::blueberry_400()
                    } else {
                        theme::longan_700()
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .min_w_0()
                                    .flex_1()
                                    .font_weight(if conversation.unread {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if conversation.unread {
                                        theme::rambutan_100()
                                    } else {
                                        theme::rambutan_300()
                                    })
                                    .child(sender),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(12.0))
                                    .text_color(theme::rambutan_500())
                                    .child(inbox_date(conversation.message_at_ms)),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_300())
                            .child(if conversation.subject.trim().is_empty() {
                                "(No subject)".to_string()
                            } else {
                                conversation.subject.clone()
                            }),
                    )
                    .child(
                        div()
                            .mt_1()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_500())
                            .overflow_hidden()
                            .child(preview),
                    )
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .gap_3()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_500())
                            .child(format!("{} import", conversation.source.label()))
                            .when(conversation.important, |element| element.child("Important")),
                    ),
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
                        .debug_selector(|| "desktop-inbox-import-cancel".into())
                        .aria_label("Cancel")
                        .child("Cancel")
                        .on_click(cx.listener(
                            |this, _event, window, cx| this.cancel_inbox_import(window, cx),
                        )),
                    ),
            )
            .child(
                div()
                    .id("desktop-inbox-import-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_5()
                    .flex()
                    .justify_center()
                    .items_start()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(680.0))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(theme::rambutan_500())
                                    .child("Provider"),
                            )
                            .child(
                                div().flex().flex_wrap().gap_1().children(
                                    InboxSource::ALL
                                        .into_iter()
                                        .map(|source| self.inbox_source_button(source, cx)),
                                ),
                            )
                            .child(Input::new(&self.inbox_sender_input).aria_label("Sender"))
                            .child(Input::new(&self.inbox_subject_input).aria_label("Subject"))
                            .child(Textarea::new(&self.inbox_body_input).aria_label("Message"))
                            .when_some(self.persistence_error.clone(), |element, error| {
                                element.child(
                                    div()
                                        .text_size(px(12.0))
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
                                .debug_selector(|| "desktop-inbox-import-submit".into())
                                .aria_label("Import")
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
        narrow: bool,
        cx: &Context<'_, Self>,
    ) -> Div {
        let Some(conversation) = self
            .inbox_conversations
            .iter()
            .find(|conversation| conversation.id == conversation_id)
        else {
            return self.render_inbox_page(cx);
        };
        // Web `px-3 sm:px-5`; `narrow` is below `sm`.
        let gutter = if narrow {
            CONVERSATION_GUTTER_COMPACT_PX
        } else {
            CONVERSATION_GUTTER_PX
        };
        let messages = conversation
            .messages
            .iter()
            .filter(|message| message.send_state != bruhs_models::MessageSendState::Draft)
            .enumerate()
            .map(|(index, message)| {
                let outgoing = message.direction == bruhs_models::MessageDirection::Outgoing;
                // Web: a block 88% of the column, at its left when
                // received and its right when sent.
                div()
                    .w_full()
                    .flex()
                    .when(outgoing, |row| row.justify_end())
                    .child(
                        div()
                            .id(("inbox-message", index))
                            .debug_selector(move || format!("desktop-inbox-message-{index}"))
                            .min_w_0()
                            .w(relative(CONVERSATION_MESSAGE_WIDTH))
                            .rounded(px(8.0))
                            .when(outgoing, |message| message.rounded_br(px(4.0)))
                            .when(!outgoing, |message| message.rounded_bl(px(4.0)))
                            .bg(if outgoing {
                                theme::longan_800()
                            } else {
                                theme::longan_900()
                            })
                            .border_1()
                            .border_color(if outgoing {
                                theme::longan_700()
                            } else {
                                theme::longan_800()
                            })
                            .px_3()
                            .py(px(10.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_baseline()
                                    .justify_between()
                                    .gap_x(px(16.0))
                                    .gap_y_1()
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(12.0))
                                    .text_color(theme::rambutan_500())
                                    .child(div().min_w_0().child(message.sender_name.clone()))
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("desktop-inbox-message-time-{index}")
                                            })
                                            .flex_shrink_0()
                                            .font_features(theme::tabular_nums())
                                            .child(inbox_date(message.sent_at_ms)),
                                    ),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .font_family(theme::CODE_FONT_FAMILY)
                                    .text_size(px(14.0))
                                    .text_color(theme::rambutan_200())
                                    .child(message.body.clone()),
                            ),
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
                    .flex_shrink_0()
                    .px(px(gutter))
                    .py(px(10.0))
                    .border_b_1()
                    .border_color(theme::longan_800())
                    // Web `flex-wrap`: the actions move under the title
                    // when the row cannot hold them.
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        // Web: a quiet "← Inbox".
                        div()
                            .id("back-to-inbox")
                            .debug_selector(|| "desktop-inbox-back".to_string())
                            .role(Role::Button)
                            .tab_index(0)
                            .aria_label("Back to Inbox")
                            .flex_shrink_0()
                            .min_h(px(theme::CONTROL_PX))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .hover(|el| el.bg(theme::longan_800()))
                            .font_family(theme::UI_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_400())
                            .cursor_pointer()
                            .whitespace_nowrap()
                            .child("← Inbox")
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.open_inbox_view(window, cx)
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .id("desktop-inbox-conversation-title")
                                    .role(Role::Heading)
                                    .aria_level(1)
                                    .aria_label(conversation.subject.clone())
                                    .debug_selector(|| {
                                        "desktop-inbox-conversation-title".to_string()
                                    })
                                    .font_family(theme::HEADING_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(16.0))
                                    .text_color(theme::rambutan_100())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(conversation.subject.clone()),
                            )
                            .child(
                                div()
                                    .debug_selector(|| {
                                        "desktop-inbox-conversation-subtitle".to_string()
                                    })
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(12.0))
                                    .text_color(theme::rambutan_500())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(conversation.source_and_participants()),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "desktop-inbox-actions".to_string())
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(
                                inbox_action_button(
                                    "inbox-read",
                                    if unread { "Mark read" } else { "Mark unread" },
                                )
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
                                inbox_action_button(
                                    "inbox-snooze",
                                    if snoozed { "Unsnooze" } else { "Tomorrow" },
                                )
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
                                inbox_action_button(
                                    "inbox-important",
                                    if important { "Unstar" } else { "Important" },
                                )
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
                                inbox_action_button(
                                    "inbox-archive",
                                    if archived { "Unarchive" } else { "Archive" },
                                )
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
                                inbox_action_button("inbox-ask-agent", "Ask agent").on_click(
                                    cx.listener(|this, _event, window, cx| {
                                        this.ask_agent_about_current_conversation(window, cx)
                                    }),
                                ),
                            )
                            .child(inbox_action_button("inbox-delete", "Delete").on_click(
                                cx.listener(|this, _event, window, cx| {
                                    this.request_record_delete(window, cx)
                                }),
                            )),
                    ),
            )
            .child(self.render_linked_person(conversation, narrow, gutter, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .flex()
                    .justify_center()
                    .child(
                        // Web `mx-auto max-w-3xl px-4 py-5 space-y-3`.
                        div()
                            .id("desktop-inbox-messages")
                            .role(Role::Region)
                            .aria_label("Conversation messages")
                            .debug_selector(|| "desktop-inbox-message-column".to_string())
                            .w_full()
                            .max_w(px(CONVERSATION_COLUMN_PX))
                            .px_4()
                            .py_5()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .children(messages),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme::longan_800())
                    .bg(theme::longan_950())
                    .px(px(gutter))
                    .py_3()
                    .flex()
                    .justify_center()
                    .child(
                        // Web: a three-row reply box in the message column's
                        // width, the hint at its left and the actions at its
                        // right.
                        div()
                            .debug_selector(|| "desktop-inbox-reply".to_string())
                            .w_full()
                            .max_w(px(CONVERSATION_COLUMN_PX))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .debug_selector(|| "desktop-inbox-reply-box".to_string())
                                    .child(
                                        Textarea::new(&self.inbox_reply_input)
                                            .h(px(REPLY_BOX_HEIGHT_PX))
                                            .aria_label("Reply draft"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        div()
                                            .debug_selector(|| {
                                                "desktop-inbox-reply-hint".to_string()
                                            })
                                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_size(px(12.0))
                                            .text_color(theme::rambutan_400())
                                            .child(bruhs_models::REPLY_HINT),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(
                                                web_button(
                                                    "save-inbox-draft",
                                                    WebButtonVariant::Outline,
                                                    WebButtonSize::Default,
                                                    ButtonState::Enabled,
                                                )
                                                .border_color(theme::longan_700())
                                                .text_color(theme::rambutan_300())
                                                .debug_selector(|| {
                                                    "desktop-inbox-save-draft".into()
                                                })
                                                .aria_label("Save draft")
                                                .child("Save draft")
                                                .on_click(cx.listener(
                                                    |this, _event, _window, cx| {
                                                        this.save_inbox_draft(cx)
                                                    },
                                                )),
                                            )
                                            .child(
                                                web_button(
                                                    "send-inbox-draft",
                                                    WebButtonVariant::Primary,
                                                    WebButtonSize::Default,
                                                    ButtonState::disabled_if(
                                                        self.inbox_reply_input
                                                            .read(cx)
                                                            .value()
                                                            .trim()
                                                            .is_empty(),
                                                    ),
                                                )
                                                .debug_selector(|| "desktop-inbox-log-reply".into())
                                                .aria_label("Log reply")
                                                .child("Log reply")
                                                .on_click(cx.listener(
                                                    |this, _event, window, cx| {
                                                        this.send_inbox_draft(window, cx)
                                                    },
                                                )),
                                            ),
                                    ),
                            ),
                    ),
            )
    }
}

/// What an empty conversation list says and the next step it offers, in
/// the web Inbox's words: saved conversations that could not be loaded, a
/// search with no match, or the filter's own empty view. The desktop reads
/// its state file before the first frame, so it never shows the web's
/// loading state.
pub(super) fn inbox_empty_state(
    query: &str,
    filter: InboxFilter,
    records_loaded: bool,
) -> (&'static str, &'static str) {
    if !records_loaded {
        (
            bruhs_models::INBOX_LOAD_FAILED,
            bruhs_models::INBOX_LOAD_DETAIL,
        )
    } else if query.trim().is_empty() {
        (
            filter.empty_message(),
            "Import a message or choose another view.",
        )
    } else {
        (
            "No matching conversations.",
            "Try another search or choose All.",
        )
    }
}

/// Web `ActionButton`: an outlined text button in a conversation's header.
fn inbox_action_button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || format!("desktop-{id}"))
        .role(Role::Button)
        .tab_index(0)
        .aria_label(label)
        .flex_shrink_0()
        .min_h(px(theme::CONTROL_PX))
        .px_2()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(theme::longan_700())
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_200()))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(12.0))
        .text_color(theme::rambutan_400())
        .whitespace_nowrap()
        .cursor_pointer()
        .child(label)
}

/// ⌘↵ in the reply box logs the reply, as on the web. gpui-component inserts
/// a newline at the cursor before it reports the key, and the web prevents
/// it, so the reply is the text without that newline.
fn reply_without_submit_newline(value: &str, cursor: usize) -> String {
    match cursor.checked_sub(1) {
        Some(at) if value.get(at..cursor) == Some("\n") => {
            format!("{}{}", &value[..at], &value[cursor..])
        }
        _ => value.to_string(),
    }
}

pub(super) fn inbox_date(timestamp_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(timestamp_ms).map_or_else(
        || "Unknown date".into(),
        |date| {
            let date = date.with_timezone(&Local);
            date.format(if date.date_naive() == Local::now().date_naive() {
                "%H:%M"
            } else {
                "%Y-%m-%d"
            })
            .to_string()
        },
    )
}

/// The conversation import form and the reply box.
pub(super) struct InboxInputs {
    pub(super) inbox_search_input: Entity<InputState>,
    pub(super) inbox_sender_input: Entity<InputState>,
    pub(super) inbox_subject_input: Entity<InputState>,
    pub(super) inbox_body_input: Entity<TextareaState>,
    pub(super) inbox_reply_input: Entity<TextareaState>,
}

impl Workspace {
    /// The conversation import form and the reply box; edits redraw.
    pub(super) fn new_inbox_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> InboxInputs {
        let inbox_search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search people, subjects, and messages…")
        });
        subscriptions.push(cx.subscribe_in(
            &inbox_search_input,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.inbox_page = 0;
                    cx.notify();
                }
            },
        ));
        let inbox_sender_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name or handle"));
        subscriptions.push(redraw_on_edit(&inbox_sender_input, window, cx));
        let inbox_subject_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Subject (optional)"));
        subscriptions.push(redraw_on_edit(&inbox_subject_input, window, cx));
        let inbox_body_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Paste the message…")
                .rows(8)
        });
        subscriptions.push(redraw_on_edit(&inbox_body_input, window, cx));
        let inbox_reply_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Write a reply…")
                .rows(4)
        });
        subscriptions.push(redraw_on_edit(&inbox_reply_input, window, cx));
        subscriptions.push(cx.subscribe_in(
            &inbox_reply_input,
            window,
            |this: &mut Self, input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true,
                    shift: false,
                } = event
                {
                    let (value, cursor) = {
                        let input = input.read(cx);
                        (input.value().to_string(), input.cursor())
                    };
                    let body = reply_without_submit_newline(&value, cursor);
                    input.update(cx, |input, cx| input.set_value(body, window, cx));
                    this.send_inbox_draft(window, cx);
                }
            },
        ));
        InboxInputs {
            inbox_search_input,
            inbox_sender_input,
            inbox_subject_input,
            inbox_body_input,
            inbox_reply_input,
        }
    }
}

#[cfg(test)]
mod parity_tests;

#[cfg(test)]
mod tests {
    use super::{
        element_key, inbox_date, inbox_row_label, AppView, InboxCard, InboxConversation,
        InboxMessage, InboxSource,
    };
    use gpui::{Modifiers, TestAppContext};

    #[gpui::test]
    fn inbox_search_and_filters_reach_archived_full_text_and_refresh_after_edits(
        cx: &mut TestAppContext,
    ) {
        let (workspace, cx) = super::super::performance_tests::fixture(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.active_view = AppView::Inbox;
            workspace.inbox_conversations.clear();
            for (id, archived) in [("open", false), ("archived", true)] {
                let mut conversation = InboxConversation::new(InboxSource::Email, id, vec![], 10);
                conversation.id = id.into();
                conversation.receive(InboxMessage::incoming(
                    "Ada",
                    format!("{} Deep CAFÉ 世界", "x".repeat(300)),
                    20,
                ));
                conversation.set_archived(archived);
                workspace.inbox_conversations.push(conversation);
            }
            workspace.persist_and_notify(cx);
        });
        cx.run_until_parked();
        let archived_row =
            Box::leak(format!("desktop-inbox-card-{}", element_key("archived")).into_boxed_str());
        let open_row =
            Box::leak(format!("desktop-inbox-card-{}", element_key("open")).into_boxed_str());
        assert!(cx.debug_bounds(open_row).is_some());
        assert!(cx.debug_bounds(archived_row).is_none());
        let archived = cx.debug_bounds("desktop-inbox-filter-Archived").unwrap();
        cx.simulate_click(archived.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds(archived_row).is_some(),
            "archived conversations must remain reachable"
        );
        assert!(cx.debug_bounds(open_row).is_none());
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.inbox_page = usize::MAX;
            workspace
                .inbox_search_input
                .update(cx, |input, cx| input.focus(window, cx));
        });
        cx.simulate_input("deep café 世界");
        cx.run_until_parked();
        assert!(
            cx.debug_bounds(archived_row).is_some(),
            "search includes full text beyond the row preview"
        );
        workspace.read_with(cx, |workspace, _| assert_eq!(workspace.inbox_page, 0));
        workspace.update(cx, |workspace, cx| {
            workspace.inbox_conversations[1].messages[0].body =
                "Changed without changing the record count or timestamp".into();
            workspace.persist_and_notify(cx);
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds(archived_row).is_none(),
            "saved edits must invalidate the full-text index"
        );
        assert!(cx.debug_bounds("desktop-inbox-empty").is_some());
        workspace.update_in(cx, |workspace, window, cx| {
            workspace
                .inbox_search_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        });
        cx.run_until_parked();
        let all = cx.debug_bounds("desktop-inbox-filter-All").unwrap();
        cx.simulate_click(all.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds(open_row).is_some());
        assert!(cx.debug_bounds(archived_row).is_some());
    }

    /// A row reads as the web's row does, from its content: read state,
    /// sender, date, subject, Important, preview and source.
    #[test]
    fn inbox_rows_read_like_the_web_row() {
        let card = InboxCard {
            id: "launch".into(),
            source: InboxSource::Email,
            subject: "Launch review".into(),
            sender: "Ada".into(),
            preview: "Can we walk through it?".into(),
            unread: true,
            important: true,
            message_at_ms: i64::MAX,
        };
        assert_eq!(
            inbox_row_label(&card),
            "Unread conversation: Ada Unknown date Launch review Important Can we walk through it? Email import"
        );
        let read = InboxCard {
            subject: " ".into(),
            unread: false,
            important: false,
            source: InboxSource::Import,
            ..card
        };
        assert_eq!(
            inbox_row_label(&read),
            "Read conversation: Ada Unknown date (No subject) Can we walk through it? Imported import"
        );
    }

    #[test]
    fn inbox_rejects_dates_outside_the_calendar_range() {
        assert_eq!(inbox_date(i64::MAX), "Unknown date");
        assert_eq!(inbox_date(i64::MIN), "Unknown date");
    }
}
