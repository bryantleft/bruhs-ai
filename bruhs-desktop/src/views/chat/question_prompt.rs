use super::*;
use bruhs_core::PermissionResolveError;
use bruhs_models::logic::user_question::{
    questions, validate_answers, QuestionAnswer, QuestionReply, UserQuestion,
    CHOICE_ADVANCE_DELAY_MS,
};
use bruhs_ui::input::Textarea;
use bruhs_ui::press::on_press_view;

mod files;

pub(super) struct QuestionPrompt {
    chat: WeakEntity<ChatView>,
    focus: FocusHandle,
    request: DesktopPermissionRequest,
    questions: Vec<UserQuestion>,
    answers: Vec<QuestionAnswer>,
    fields: Vec<Option<Entity<TextareaState>>>,
    uploads: Vec<files::Offer>,
    position: usize,
    collapsed_question: Option<usize>,
    busy: bool,
    error: Option<String>,
    auto_advance: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

pub(super) struct QuestionResponse {
    request_id: String,
    turn_generation: u64,
    task: Option<Task<()>>,
    pub(super) error: Option<String>,
}

impl QuestionResponse {
    pub(super) fn responding(&self) -> bool {
        self.task.is_some()
    }
}

impl ChatView {
    pub(super) fn current_question_response(&self) -> Option<&QuestionResponse> {
        let request = self.pending_permission.as_ref()?;
        self.question_response.as_ref().filter(|response| {
            response.request_id == request.id && response.turn_generation == self.turn_generation
        })
    }

    pub(super) fn start_question_reply(
        &mut self,
        request_id: &str,
        reply: QuestionReply,
        device: Option<uuid::Uuid>,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), &'static str> {
        let request = self
            .pending_permission
            .as_ref()
            .filter(|request| request.can_answer && request.id == request_id)
            .ok_or("This question is no longer waiting for an answer.")?;
        if self
            .current_question_response()
            .is_some_and(QuestionResponse::responding)
        {
            return Err("An answer is already being sent.");
        }
        if let QuestionReply::Answer { answers } = &reply {
            validate_answers(&request.input, answers)?;
        }
        let thread = self
            .thread_id
            .as_deref()
            .and_then(|id| ThreadId::try_from(id).ok())
            .ok_or("This conversation could not be identified.")?;
        let state = cx
            .try_global::<DesktopState>()
            .ok_or("The desktop could not send the answer.")?;
        let run = self.restart_turn.clone();
        let app = Arc::clone(&state.app);
        let request_id = request_id.to_owned();
        let id = request_id.clone();
        let generation = self.turn_generation;
        let operation = state.runtime.spawn(async move {
            if let Some(device) = device {
                app.answer_question_from_device(&thread, &id, reply, device)
                    .await
            } else {
                app.answer_question(&thread, &id, reply).await
            }
        });
        let id = request_id.clone();
        let task = cx.spawn(async move |this, cx| {
            let failure = match operation.await {
                Ok(Ok(())) | Ok(Err(PermissionResolveError::UnknownRequest)) => None,
                Ok(Err(error)) => Some(error.to_string()),
                Err(_) => Some("The answer could not be delivered.".to_owned()),
            };
            if failure.is_none() {
                if let Some(run) = run {
                    run.answered(&id);
                }
            }
            let _ = this.update(cx, |chat, cx| {
                if chat.turn_generation != generation
                    || chat
                        .pending_permission
                        .as_ref()
                        .is_none_or(|pending| pending.id != id)
                {
                    return;
                }
                if let Some(error) = failure {
                    chat.question_response = Some(QuestionResponse {
                        request_id: id.clone(),
                        turn_generation: generation,
                        task: None,
                        error: Some(error.clone()),
                    });
                    if let Some(prompt) = &chat.question_prompt {
                        prompt.update(cx, |prompt, cx| prompt.set_delivery(false, Some(error), cx));
                    }
                } else {
                    streaming::clear_permission_if_matches(&mut chat.pending_permission, &id);
                    chat.question_prompt = None;
                    chat.question_response = None;
                }
                cx.emit(ChatViewEvent::MessagesChanged);
                cx.notify();
            });
        });
        self.question_response = Some(QuestionResponse {
            request_id,
            turn_generation: generation,
            task: Some(task),
            error: None,
        });
        cx.emit(ChatViewEvent::MessagesChanged);
        cx.notify();
        Ok(())
    }

    pub(super) fn mark_question_prompt_sending(&self, cx: &mut Context<'_, Self>) {
        if let Some(prompt) = &self.question_prompt {
            prompt.update(cx, |prompt, cx| prompt.set_delivery(true, None, cx));
        }
    }

    pub(super) fn on_question_shortcut(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if window.default_prevented()
            || event.is_held
            || modifiers.platform
            || modifiers.control
            || modifiers.alt
            || modifiers.shift
            || self.model_picker_open
            || self.branch_menu_open
            || self.workspace_menu_open
            || self.effort_menu_open
            || self.composer_options_open
            || self.find_open
            || self.pending_secret.is_some()
            || self
                .input_state
                .focus_handle(cx)
                .contains_focused(window, cx)
        {
            return;
        }
        let Some(prompt) = self.question_prompt.as_ref() else {
            return;
        };
        let prompt_state = prompt.read(cx);
        let position = prompt_state.position;
        if prompt_state.collapsed_question == Some(position)
            || prompt_state.busy
            || prompt_state.fields[position]
                .as_ref()
                .is_some_and(|field| field.focus_handle(cx).contains_focused(window, cx))
        {
            return;
        }
        if let Some(index) = prompt_state.questions[position].shortcut_option(&event.keystroke.key)
        {
            prompt.update(cx, |prompt, cx| {
                prompt.select_option(position, index, window, cx)
            });
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    pub(super) fn render_pending_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        if let Some(request) = self
            .pending_permission
            .as_ref()
            .filter(|request| request.can_answer)
        {
            let replace = self
                .question_prompt
                .as_ref()
                .is_none_or(|prompt| prompt.read(cx).request.id != request.id);
            if replace {
                if let Some(questions) = questions(&request.input) {
                    let chat = cx.weak_entity();
                    let response = self.current_question_response();
                    let busy = response.is_some_and(QuestionResponse::responding);
                    let error = response.and_then(|response| response.error.clone());
                    self.question_prompt = Some(cx.new(|cx| {
                        let mut prompt =
                            QuestionPrompt::new(chat, request.clone(), questions, window, cx);
                        prompt.busy = busy;
                        prompt.error = error;
                        prompt
                    }));
                }
            }
            if let Some(prompt) = &self.question_prompt {
                return div()
                    .w_full()
                    .flex_shrink_0()
                    .px(px(CHAT_COMPOSER_PAGE_X_PADDING_PX))
                    .pb_2()
                    .child(
                        div()
                            .w_full()
                            .mx_auto()
                            .when_some(self.chat_width.limit(), |panel, limit| {
                                panel.max_w(px(limit))
                            })
                            .child(prompt.clone()),
                    )
                    .into_any_element();
            }
        }
        self.render_permission_prompt(cx).into_any_element()
    }
}

impl QuestionPrompt {
    fn new(
        chat: WeakEntity<ChatView>,
        request: DesktopPermissionRequest,
        questions: Vec<UserQuestion>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let fields: Vec<_> = questions
            .iter()
            .map(|question| {
                question
                    .allow_custom
                    .then(|| cx.new(|cx| TextareaState::new(window, cx).rows(2)))
            })
            .collect();
        let subscriptions = fields
            .iter()
            .enumerate()
            .filter_map(|(index, field)| {
                let field = field.as_ref()?;
                Some(cx.observe(field, move |this: &mut Self, field, cx| {
                    let value = field.read(cx).value().to_string();
                    if this.answers[index].text != value {
                        this.answers[index].text = value;
                        if this.position == index {
                            this.auto_advance = None;
                        }
                        cx.notify();
                    }
                }))
            })
            .collect();
        let answers = vec![QuestionAnswer::default(); questions.len()];
        Self {
            chat,
            focus: cx.focus_handle().tab_stop(false),
            request,
            questions,
            answers,
            fields,
            uploads: Vec::new(),
            position: 0,
            collapsed_question: None,
            busy: false,
            error: None,
            auto_advance: None,
            _subscriptions: subscriptions,
        }
    }

    fn select_option(
        &mut self,
        position: usize,
        index: usize,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.position != position {
            return;
        }
        let auto_advance = self.answers[position].select(&self.questions[position], index)
            && !self
                .uploads
                .iter()
                .any(|offer| offer.question() == position);
        self.auto_advance = auto_advance.then(|| {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(u64::from(CHOICE_ADVANCE_DELAY_MS)))
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    // Switching threads detaches this panel without dropping its draft.
                    let mounted = this
                        .chat
                        .upgrade()
                        .is_some_and(|chat| chat.read(cx).chat_focus.contains(&this.focus, window));
                    if mounted
                        && this.position == position
                        && this.collapsed_question != Some(position)
                        && this.answers[position].is_choice_only()
                        && !this
                            .uploads
                            .iter()
                            .any(|offer| offer.question() == position)
                        && !this.fields[position].as_ref().is_some_and(|field| {
                            field.focus_handle(cx).contains_focused(window, cx)
                        })
                    {
                        this.advance(cx);
                    }
                });
            })
        });
        cx.notify();
    }

    fn can_advance(&self) -> bool {
        self.answers[self.position].is_answered()
            && self
                .uploads
                .iter()
                .all(|offer| offer.question() != self.position || offer.ready())
    }

    fn can_send(&self) -> bool {
        self.uploads.iter().all(files::Offer::ready)
            && validate_answers(&self.request.input, &self.answers).is_ok()
    }

    fn advance(&mut self, cx: &mut Context<'_, Self>) {
        self.auto_advance = None;
        if self.busy || !self.can_advance() {
            return;
        }
        if self.position + 1 < self.questions.len() {
            self.position += 1;
            cx.notify();
        } else if self.can_send() {
            self.respond(false, cx);
        }
    }

    fn set_delivery(&mut self, busy: bool, error: Option<String>, cx: &mut Context<'_, Self>) {
        self.auto_advance = None;
        self.busy = busy;
        self.error = error;
        cx.notify();
    }

    fn respond(&mut self, skip: bool, cx: &mut Context<'_, Self>) {
        self.auto_advance = None;
        if self.busy {
            return;
        }
        let Some(chat) = self.chat.upgrade() else {
            return;
        };
        let reply = if skip {
            QuestionReply::Skip
        } else {
            QuestionReply::Answer {
                answers: self.answers.clone(),
            }
        };
        let result = chat.update(cx, |chat, cx| {
            chat.start_question_reply(&self.request.id, reply, None, cx)
        });
        match result {
            Ok(()) => self.set_delivery(true, None, cx),
            Err(error) => self.set_delivery(false, Some(error.to_owned()), cx),
        }
    }

    fn action(
        &self,
        id: &'static str,
        label: &'static str,
        variant: WebButtonVariant,
        disabled: bool,
        cx: &Context<'_, Self>,
        activate: impl Fn(&mut Self, &mut Context<'_, Self>) + 'static,
    ) -> impl IntoElement {
        web_button(
            id,
            variant,
            WebButtonSize::Small,
            ButtonState::disabled_if(disabled),
        )
        .min_h(px(theme::CONTROL_PX))
        .debug_selector(move || id.to_owned())
        .aria_label(label)
        .child(label)
        .map(|button| {
            on_press_view(button, cx, move |this, _, cx| {
                if !disabled {
                    activate(this, cx);
                }
            })
        })
    }
}

impl Render for QuestionPrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let position = self.position;
        let question = &self.questions[position];
        let total = self.questions.len();
        let expanded = self.collapsed_question != Some(position);
        let heading = if question.header.trim().is_empty() {
            "Question from the agent".to_owned()
        } else {
            question.header.clone()
        };
        let options = question.options.iter().enumerate().map(|(index, option)| {
            let selected = self.answers[position].selected.contains(&index);
            let description = option.description.clone();
            let busy = self.busy;
            let button = web_button(
                ("agent-question-option", index),
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(busy),
            )
            .w_full()
            .flex_shrink_0()
            .debug_selector(move || format!("agent-question-option-{index}"))
            .h_auto()
            .min_h(px(theme::CONTROL_PX))
            .justify_start()
            .aria_label(option.label.clone())
            .aria_toggled(selected.into())
            .when(selected, |button| {
                button
                    .border_color(theme::persimmon_400())
                    .bg(theme::longan_800())
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_color(theme::rambutan_100())
                                    .child(option.label.clone()),
                            )
                            .when(!description.is_empty(), |element| {
                                element.child(
                                    div()
                                        .font_weight(FontWeight::NORMAL)
                                        .text_size(px(11.0))
                                        .text_color(theme::rambutan_300())
                                        .child(description),
                                )
                            }),
                    )
                    .child(bruhs_ui::kbd::kbd((index + 1).to_string())),
            )
            .map(|button| {
                on_press_view(button, cx, move |this, window, cx| {
                    this.select_option(position, index, window, cx);
                })
            });
            div()
                .w_full()
                .flex_shrink_0()
                .min_w_0()
                .child(button)
                .when_some(option.preview.as_deref(), |row, preview| {
                    row.child(
                        div()
                            .id(("agent-question-preview", index))
                            .debug_selector(move || format!("agent-question-preview-{index}"))
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_300())
                            .child(
                                untrusted_markdown(
                                    ("agent-question-preview-text", index),
                                    preview.to_owned(),
                                )
                                .style(desktop_markdown_style(cx))
                                .selectable(true),
                            ),
                    )
                })
        });

        div()
            .id("agent-question")
            .track_focus(&self.focus)
            .capture_action(cx.listener(|this, _: &bruhs_ui::input::Paste, window, cx| {
                if this.paste_files(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .debug_selector(|| "agent-question".to_owned())
            .role(Role::Group)
            .aria_label("Question from the agent")
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.0))
            .border_1()
            .border_color(theme::longan_800())
            .bg(theme::longan_950())
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_size(px(12.0))
            .text_color(theme::rambutan_100())
            .child(
                web_button(
                    "agent-question-toggle",
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "agent-question-toggle".to_owned())
                .min_h(px(theme::CONTROL_PX))
                .w_full()
                .min_w_0()
                .justify_start()
                .aria_expanded(expanded)
                .aria_label(if expanded {
                    format!("Hide options for {heading}")
                } else {
                    format!("Show options for {heading}")
                })
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .text_color(theme::rambutan_300())
                        .child(
                            div()
                                .min_w_0()
                                .max_w(relative(0.5))
                                .truncate()
                                .child(heading),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .when(!expanded, |summary| {
                                    summary.child(question.question.clone())
                                }),
                        )
                        .when(total > 1, |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .child(format!("{} of {total}", position + 1)),
                            )
                        })
                        .child(
                            svg()
                                .path(if expanded {
                                    assets::CHEVRON_DOWN_ICON_PATH
                                } else {
                                    assets::CHEVRON_RIGHT_ICON_PATH
                                })
                                .size(px(12.0))
                                .flex_shrink_0()
                                .text_color(theme::rambutan_400()),
                        ),
                )
                .map(|button| {
                    on_press_view(button, cx, |this, _, cx| {
                        this.auto_advance = None;
                        this.collapsed_question = (this.collapsed_question != Some(this.position))
                            .then_some(this.position);
                        cx.notify();
                    })
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(expanded, |panel| {
                        panel
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(question.question.clone()),
                            )
                            .when(question.multi_select, |panel| {
                                panel.child(
                                    div()
                                        .text_color(theme::rambutan_300())
                                        .child("Choose one or more"),
                                )
                            })
                            .child(
                                div()
                                    .id(("agent-question-options", position))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .max_h(window.viewport_size().height * 0.26)
                                    .overflow_y_scroll()
                                    .children(options),
                            )
                    })
                    .when_some(self.fields[position].as_ref(), |panel, field| {
                        panel
                            .child(
                                div()
                                    .pt_1()
                                    .text_color(theme::rambutan_300())
                                    .child("Custom answer"),
                            )
                            .child(
                                Textarea::new(field)
                                    .aria_label("Custom answer")
                                    .disabled(self.busy),
                            )
                            .child(self.render_files(cx))
                    }),
            )
            .when_some(self.error.clone(), |panel, error| {
                panel.child(
                    div()
                        .id("agent-question-error")
                        .role(Role::Alert)
                        .text_color(theme::lychee_300())
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.action(
                        "agent-question-skip",
                        "Skip",
                        WebButtonVariant::Ghost,
                        self.busy,
                        cx,
                        |this, cx| this.respond(true, cx),
                    ))
                    .child(div().flex_1())
                    .when(total > 1, |row| {
                        row.child(self.action(
                            "agent-question-back",
                            "Back",
                            WebButtonVariant::Ghost,
                            self.busy || position == 0,
                            cx,
                            |this, cx| {
                                this.auto_advance = None;
                                this.position = this.position.saturating_sub(1);
                                cx.notify();
                            },
                        ))
                    })
                    .when(position + 1 < total, |row| {
                        row.child(self.action(
                            "agent-question-next",
                            "Next",
                            WebButtonVariant::Outline,
                            self.busy || !self.can_advance(),
                            cx,
                            Self::advance,
                        ))
                    })
                    .when(position + 1 == total, |row| {
                        row.child(self.action(
                            "agent-question-send",
                            "Send answers",
                            WebButtonVariant::Primary,
                            self.busy || !self.can_send(),
                            cx,
                            Self::advance,
                        ))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use crate::views::chat::{behavior_tests::chat_window, AgentUiEvent, DesktopPermissionRequest};
    use bruhs_models::logic::user_question::CHOICE_ADVANCE_DELAY_MS;
    use bruhs_ui::zoom::px;
    use gpui::{size, Modifiers, TestAppContext, VisualTestContext};
    use serde_json::json;
    use std::time::Duration;

    #[gpui::test]
    fn question_draft_survives_navigation_and_stays_out_of_composer(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        cx.simulate_resize(size(px(640.0), px(480.0)));
        chat.update_in(cx, |chat, window, cx| {
            chat.input_state.update(cx, |input, cx| {
                input.set_value("Keep this draft", window, cx);
            });
            chat.apply_agent_event(AgentUiEvent::PermissionRequest(DesktopPermissionRequest {
                id: "question-fixture".into(),
                tool: bruhs_models::logic::user_question::TOOL_NAME.into(),
                input: json!({"questions": [
                    {"question": "Format?", "options": [{"label": "Concise"}, {"label": "Detailed"}]},
                    {"question": "Sections?", "multiSelect": true, "options": [{"label": "Setup"}, {"label": "Examples"}]},
                ]}),
                can_answer: true,
            }));
            cx.notify();
        });
        cx.run_until_parked();

        let click = |cx: &mut VisualTestContext, selector: &'static str| {
            let button = cx.debug_bounds(selector).expect(selector);
            assert!(button.left() >= px(0.0));
            assert!(button.right() <= px(640.0));
            assert!(button.top() >= px(0.0));
            assert!(button.bottom() <= px(480.0));
            cx.simulate_click(button.center(), Modifiers::default());
            cx.run_until_parked();
        };
        click(cx, "agent-question-option-0");
        click(cx, "agent-question-option-1");
        click(cx, "agent-question-next");
        click(cx, "agent-question-option-0");
        click(cx, "agent-question-option-1");
        let prompt = chat.read_with(cx, |chat, _| chat.question_prompt.clone().unwrap());
        prompt.update_in(cx, |prompt, window, cx| {
            assert_eq!(prompt.position, 1);
            prompt.fields[1]
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.focus(window, cx));
        });
        cx.simulate_input("Include current examples");
        cx.run_until_parked();
        click(cx, "agent-question-back");
        click(cx, "agent-question-next");

        prompt.read_with(cx, |prompt, _| {
            assert_eq!(prompt.answers[0].selected, vec![1]);
            assert_eq!(prompt.answers[1].selected, vec![0, 1]);
            assert_eq!(prompt.answers[1].text, "Include current examples");
        });
        chat.read_with(cx, |chat, cx| {
            assert_eq!(
                chat.input_state.read(cx).value().as_str(),
                "Keep this draft"
            );
        });
        chat.update(cx, |chat, cx| {
            chat.apply_agent_event(AgentUiEvent::PermissionCancelled("question-fixture".into()));
            cx.notify();
        });
        cx.run_until_parked();
        assert!(chat.read_with(cx, |chat, _| chat.question_prompt.is_none()));
    }

    #[gpui::test]
    fn question_shortcuts_preserve_text_and_collapsed_answers(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        cx.simulate_resize(size(px(640.0), px(480.0)));
        chat.update(cx, |chat, cx| {
            chat.apply_agent_event(AgentUiEvent::PermissionRequest(DesktopPermissionRequest {
                id: "shortcut-question-fixture".into(),
                tool: bruhs_models::logic::user_question::TOOL_NAME.into(),
                input: json!({"questions": [{
                    "header": "Sections",
                    "question": "Which sections should I include?",
                    "multiSelect": true,
                    "options": [{"label": "Setup"}, {"label": "Examples"}]
                }]}),
                can_answer: true,
            }));
            cx.notify();
        });
        cx.run_until_parked();
        let prompt = chat.read_with(cx, |chat, _| chat.question_prompt.clone().unwrap());

        chat.update_in(cx, |chat, window, cx| chat.chat_focus.focus(window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("2 9 shift-1");
        cx.run_until_parked();
        assert_eq!(
            prompt.read_with(cx, |prompt, _| prompt.answers[0].selected.clone()),
            vec![1]
        );

        let toggle = cx.debug_bounds("agent-question-toggle").unwrap();
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("agent-question-option-0").is_none());
        cx.simulate_keystrokes("1");
        prompt.update_in(cx, |prompt, window, cx| {
            assert_eq!(prompt.answers[0].selected, vec![1]);
            prompt.fields[0]
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.focus(window, cx));
        });
        cx.simulate_input("1");
        cx.run_until_parked();

        let toggle = cx.debug_bounds("agent-question-toggle").unwrap();
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("agent-question-option-0").is_some());
        prompt.update_in(cx, |prompt, window, cx| {
            prompt.fields[0]
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.focus(window, cx));
        });
        cx.simulate_input("2");
        cx.run_until_parked();
        prompt.read_with(cx, |prompt, _| {
            assert_eq!(prompt.answers[0].selected, vec![1]);
            assert_eq!(prompt.answers[0].text, "12");
        });

        chat.update_in(cx, |chat, window, cx| chat.focus_composer(window, cx));
        cx.simulate_input("2");
        cx.run_until_parked();
        assert_eq!(
            prompt.read_with(cx, |prompt, _| prompt.answers[0].selected.clone()),
            vec![1]
        );
        chat.read_with(cx, |chat, cx| {
            assert_eq!(chat.input_state.read(cx).value().as_str(), "2")
        });
    }

    #[gpui::test]
    fn single_choices_advance_after_the_last_selection_and_preserve_mixed_answers(
        cx: &mut TestAppContext,
    ) {
        let (chat, cx) = chat_window(cx);
        cx.simulate_resize(size(px(640.0), px(480.0)));
        chat.update(cx, |chat, cx| {
            chat.apply_agent_event(AgentUiEvent::PermissionRequest(DesktopPermissionRequest {
                id: "single-choice-fixture".into(),
                tool: bruhs_models::logic::user_question::TOOL_NAME.into(),
                input: json!({"questions": [
                    {"question":"Format?", "options":[
                        {"label":"Brief", "preview":"**One paragraph**"},
                        {"label":"Detailed"}
                    ]},
                    {"question":"Sections?", "multiSelect":true, "options":[{"label":"Examples"}]}
                ]}),
                can_answer: true,
            }));
            cx.notify();
        });
        cx.run_until_parked();
        let prompt = chat.read_with(cx, |chat, _| chat.question_prompt.clone().unwrap());
        let click = |cx: &mut VisualTestContext, selector| {
            let bounds = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.run_until_parked();
        };

        click(cx, "agent-question-preview-0");
        click(cx, "agent-question-next");
        assert!(prompt.read_with(cx, |prompt, _| prompt.answers[0].selected.is_empty()));
        assert_eq!(prompt.read_with(cx, |prompt, _| prompt.position), 0);

        let half_delay = Duration::from_millis(u64::from(CHOICE_ADVANCE_DELAY_MS / 2));
        click(cx, "agent-question-option-0");
        cx.executor().advance_clock(half_delay);
        click(cx, "agent-question-option-1");
        cx.executor().advance_clock(half_delay);
        cx.run_until_parked();
        assert_eq!(prompt.read_with(cx, |prompt, _| prompt.position), 0);
        cx.executor().advance_clock(half_delay);
        cx.run_until_parked();
        prompt.read_with(cx, |prompt, _| {
            assert_eq!(prompt.position, 1);
            assert_eq!(prompt.answers[0].selected, vec![1]);
        });

        click(cx, "agent-question-back");
        prompt.update_in(cx, |prompt, window, cx| {
            prompt.fields[0]
                .as_ref()
                .unwrap()
                .update(cx, |field, cx| field.focus(window, cx));
        });
        cx.simulate_input("Mention the fixes");
        cx.run_until_parked();
        click(cx, "agent-question-option-0");
        cx.executor().advance_clock(half_delay * 2);
        cx.run_until_parked();
        prompt.read_with(cx, |prompt, _| {
            assert_eq!(prompt.position, 0);
            assert_eq!(prompt.answers[0].selected, vec![0]);
            assert_eq!(prompt.answers[0].text, "Mention the fixes");
        });
        click(cx, "agent-question-next");
        assert_eq!(prompt.read_with(cx, |prompt, _| prompt.position), 1);
    }

    #[gpui::test]
    fn native_files_stay_on_their_question_in_a_compact_window(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        cx.simulate_resize(size(px(640.0), px(480.0)));
        chat.update(cx, |chat, cx| {
            chat.apply_agent_event(AgentUiEvent::PermissionRequest(DesktopPermissionRequest {
                id: "file-question-fixture".into(),
                tool: bruhs_models::logic::user_question::TOOL_NAME.into(),
                input: json!({"questions": [
                    {"question":"Share release notes?"},
                    {"question":"Format?", "allowCustom":false, "options":[{"label":"Concise"}]}
                ]}),
                can_answer: true,
            }));
            cx.notify();
        });
        cx.run_until_parked();
        let prompt = chat.read_with(cx, |chat, _| chat.question_prompt.clone().unwrap());
        prompt.update(cx, |prompt, cx| {
            prompt.record_fixture_upload(0);
            cx.notify();
        });
        cx.run_until_parked();
        let attach = cx
            .debug_bounds("agent-question-attach")
            .expect("attach files");
        assert!(attach.bottom() < px(480.0));
        assert!(cx.debug_bounds("agent-question-file-0").is_some());
        let next = cx.debug_bounds("agent-question-next").unwrap();
        cx.simulate_click(next.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("agent-question-attach").is_none());
        assert!(cx.debug_bounds("agent-question-file-0").is_none());
        let back = cx.debug_bounds("agent-question-back").unwrap();
        cx.simulate_click(back.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("agent-question-file-0").is_some());
        prompt.read_with(cx, |prompt, _| {
            assert_eq!(prompt.answers[0].attachments.len(), 1);
            assert!(prompt.answers[1].attachments.is_empty());
        });
    }
}
