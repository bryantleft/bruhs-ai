use super::*;
fn file_bytes(bytes: u64) -> String {
    format!("{bytes} {}", if bytes == 1 { "byte" } else { "bytes" })
}
#[track_caller]
fn meta(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(12.0))
        .line_height(relative(1.5))
        .text_color(theme::rambutan_400())
        .child(gpui::Text::new(
            ElementId::CodeLocation(*std::panic::Location::caller()),
            text.into(),
        ))
}
pub(super) fn status_text(job: &Job) -> &'static str {
    if job.selecting_files {
        return "Reading the selected files…";
    }
    match job.phase {
        Phase::Local => "Saving on this device…",
        Phase::Server => "Saving on the mail server…",
        Phase::Upload => "Uploading the selected file…",
        Phase::Review => "Preparing the saved email for review…",
        Phase::Send => "Sending the reviewed email. You can leave Email.",
        Phase::Status => "Checking saved send status…",
        Phase::Reconcile => "Checking Sent for the reviewed email. Nothing will be resent.",
        Phase::SentUnconfirmed => {
            "No matching Sent copy was verified. Sending remains unconfirmed; nothing was resent."
        }
        Phase::SentError => {
            "Sent could not be checked. Check saved send status or try Check Sent again."
        }
        Phase::Error => "Your draft remains here. Resolve the error before continuing.",
        Phase::Ready if job.saved != job.sequence || !job.added.is_empty() => {
            "Waiting to save on this device…"
        }
        Phase::Ready if job.value.consumed() => {
            "The provider accepted this email. Recipient delivery is not confirmed."
        }
        Phase::Ready if job.value.pending_send.is_none() && job.value.latest_send.as_ref().is_some_and(|status| status.state == MailDispatchState::Accepted && (status.draft_revision != job.value.server_revision || job.value.dirty)) => {
            "The reviewed version was accepted. Newer edits remain a draft; recipient delivery is not confirmed."
        }
        Phase::Ready if job.value.pending_send.is_some() => {
            "The send needs checking. It will not be sent again automatically."
        }
        Phase::Ready if job.value.dirty => "Saved on this device. Waiting for the mail server.",
        Phase::Ready => "Saved on this device and the mail server.",
    }
}
impl MailComposer {
    fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        action: Action,
        enabled: bool,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let id = id.into();
        let debug = id.to_string();
        let label = label.into();
        let keyboard = action.clone();
        web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            if enabled {
                ButtonState::Enabled
            } else {
                ButtonState::Disabled
            },
        )
        .debug_selector(move || debug)
        .aria_label(label.clone())
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| {
            if enabled {
                this.action(action.clone(), window, cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if enabled
                && is_web_button_activation_key(&event.keystroke.key)
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt
            {
                window.prevent_default();
                cx.stop_propagation();
                this.action(keyboard.clone(), window, cx);
            }
        }))
    }
    fn list(&self, cx: &Context<'_, Self>) -> Div {
        let mut rows = self
            .local_rows
            .iter()
            .map(|row| {
                (
                    row.client_id.clone(),
                    row.subject.clone(),
                    row.recipients.clone(),
                )
            })
            .collect::<Vec<_>>();
        if let Some((scope, _)) = &self.list_scope {
            for job in self
                .jobs
                .values()
                .filter(|job| &job.value.scope == scope && job.value.listed())
            {
                rows.retain(|row| row.0 != job.value.client_id);
                rows.insert(
                    0,
                    (
                        job.value.client_id.clone(),
                        job.value.content.subject.chars().take(500).collect(),
                        job.value.content.to.chars().take(500).collect(),
                    ),
                );
            }
        }
        if let Some(page) = &self.server_rows {
            for row in &page.rows {
                if !rows.iter().any(|old| old.0 == row.client_id) {
                    rows.push((
                        row.client_id.clone(),
                        row.subject.clone(),
                        row.recipients.clone(),
                    ));
                }
            }
        }
        let more = self.local_rows.len() == 50
            || self.server_rows.as_ref().is_some_and(|page| {
                page.page.saturating_add(1).saturating_mul(50) < page.total_matches
            });
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when(self.loading, |el| el.child(meta("Loading saved drafts…")))
            .when(rows.is_empty() && !self.loading, |el| {
                el.child(meta("No saved drafts in this view."))
            })
            .children(rows.into_iter().map(|(id, subject, recipients)| {
                self.button(
                    ("device-draft", element_key(&id)),
                    if subject.trim().is_empty() {
                        "No subject".into()
                    } else {
                        subject
                    },
                    Action::Open(id),
                    !self.loading,
                    cx,
                )
                .w_full()
                .flex_col()
                .items_start()
                .child(meta(recipients).truncate())
            }))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .child(self.button(
                        "desktop-compose-previous",
                        "Previous",
                        Action::Page(self.list_page.saturating_sub(1)),
                        self.list_page > 0 && !self.loading,
                        cx,
                    ))
                    .child(meta(format!("Page {}", self.list_page.saturating_add(1))))
                    .child(self.button(
                        "desktop-compose-next",
                        "Next",
                        Action::Page(self.list_page.saturating_add(1)),
                        more && !self.loading,
                        cx,
                    )),
            )
    }
    fn editor(&self, cx: &Context<'_, Self>) -> Div {
        let Some(job) = self.active() else {
            return div().child(meta("Open a saved draft or start a new email."));
        };
        let locked = job.locked();
        let files_locked = locked || job.phase == Phase::Upload;
        div().flex().flex_col().gap_3()
            .child(meta(format!("From {}",job.value.provider.name())))
            .children(["To","Cc","Bcc","Subject","Message"].into_iter().enumerate().map(|(index,label)|div().id(("compose-field",index)).debug_selector(move ||format!("desktop-compose-field-{index}")).min_w_0().flex().flex_col().gap_1().child(meta(label)).child(Input::new(&self.inputs[index]).aria_label(format!("Email {label}")).disabled(locked || (index==3&&job.value.reply.is_some())).when(index==4,|input|input.h(px(160.0))))))
            .children(job.value.files.iter().map(|file|div().id(("compose-file-row",element_key(&file.id))).role(Role::Group).aria_label("Attached file").min_w_0().flex().items_center().justify_between().gap_2().p_3().rounded(px(8.0)).bg(theme::longan_900())
                .child(div().min_w_0().flex_1().child(div().whitespace_normal().child(gpui::Text::new(("compose-file-name",element_key(&file.id)).into(),file.file_name.clone().into()))).child(meta(format!("{}  {}",file_bytes(file.size_bytes),if file.server.is_some(){"Uploaded"}else{"Waiting to upload"}))))
                .child(self.button(("remove-compose-file",element_key(&file.id)),"Remove",Action::Remove(file.id.clone()),!files_locked,cx))))
            .when_some(job.value.latest_send.as_ref(),|el,status|el.child(meta(match status.state{
                MailDispatchState::Accepted if status.draft_revision != job.value.server_revision || job.value.dirty => "The reviewed version was accepted. Newer edits remain a draft; recipient delivery is not confirmed.",
                MailDispatchState::Accepted=>"The provider accepted this email. Recipient delivery is not confirmed.",
                MailDispatchState::Uncertain=>"Sending could not be confirmed. Check Sent mail before starting another send.",
                MailDispatchState::Queued=>"Email is queued.",MailDispatchState::Dispatching=>"Sending the reviewed email.",MailDispatchState::Cancelled=>"The send was cancelled. Your draft remains saved.",MailDispatchState::NeedsReview=>"The draft changed. Review the current version.",MailDispatchState::Rejected=>"The provider did not accept this email. Resolve the issue and review again.",
            })))
    }
    fn preview(&self, review: &MailSendReview) -> Div {
        fn addresses(values: &[MailAddress]) -> String {
            values
                .iter()
                .map(|value| {
                    if value.name.is_empty() {
                        value.address.clone()
                    } else {
                        format!("{} <{}>", value.name, value.address)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(18.0)).child("Review email"))
            .children(
                [
                    ("From", review.from.address.clone()),
                    ("To", addresses(&review.recipients.to)),
                    ("Cc", addresses(&review.recipients.cc)),
                    ("Bcc", addresses(&review.recipients.bcc)),
                    ("Subject", review.content.subject.clone()),
                ]
                .into_iter()
                .map(|(label, text)| {
                    div()
                        .id(format!("compose-review-row-{label}"))
                        .role(Role::Group)
                        .aria_label(label)
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(meta(label))
                        .child(gpui::Text::new(
                            format!("compose-review-{label}").into(),
                            text.into(),
                        ))
                }),
            )
            .child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .p_4()
                    .bg(theme::longan_900())
                    .rounded(px(8.0))
                    .child(gpui::Text::new(
                        "desktop-compose-review-body".into(),
                        review.content.body.clone().into(),
                    )),
            )
            .children(review.files.iter().map(|file| {
                let text = format!("{}  {}", file.file_name, file_bytes(file.size_bytes));
                meta(text)
                    .id(("compose-reviewed-file", element_key(&file.id)))
                    .role(Role::Group)
                    .aria_label("Reviewed file")
            }))
    }
}
impl Render for MailComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if self.focus_review {
            self.focus_review = false;
            self.surface_focus.focus(window, cx);
        }
        if let Some((key, generation)) = self.focus_sent_feedback.take() {
            if self.generation == generation && self.active.as_deref() == Some(key.as_str()) {
                self.feedback_focus.focus(window, cx);
            }
        }
        let job = self.active();
        let busy = job.is_some_and(|job| {
            job.running
                || matches!(
                    job.phase,
                    Phase::Review | Phase::Send | Phase::Status | Phase::Reconcile
                )
        });
        let review = job.and_then(|job| job.review.as_ref());
        let locked = job.is_none_or(Job::locked);
        let can_review = job.is_some_and(|job| {
            !job.locked() && !job.running && !job.value.dirty && job.value.server_revision > 0
        });
        let saved = job.is_some_and(|job| job.saved == job.sequence && job.added.is_empty());
        let pending = job.is_some_and(|job| job.value.pending_send.is_some());
        let uncertain = job
            .and_then(|job| job.value.latest_send.as_ref())
            .is_some_and(|status| status.state == MailDispatchState::Uncertain);
        let queued = job
            .and_then(|job| job.value.latest_send.as_ref())
            .is_some_and(|status| status.state == MailDispatchState::Queued);
        let error = job
            .and_then(|job| {
                job.error.clone().or_else(|| {
                    if job.review.is_none() && job.phase == Phase::Ready {
                        job.value
                            .latest_send
                            .as_ref()
                            .and_then(|status| status.problem.as_ref().map(ToString::to_string))
                    } else {
                        None
                    }
                })
            })
            .or_else(|| self.store_error.clone());
        div()
            .id("desktop-mail-composer")
            .debug_selector(|| "desktop-mail-composer".into())
            .track_focus(&self.surface_focus)
            .tab_index(0)
            .role(Role::Group)
            .aria_label("Email composer")
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|element| element.border_color(theme::persimmon_400()))
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(theme::longan_950())
            .text_color(theme::rambutan_200())
            .font_family(theme::UI_FONT_FAMILY)
            .text_size(px(13.0))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.is_held {
                    return;
                }
                if event.keystroke.key == "escape" {
                    window.prevent_default();
                    cx.stop_propagation();
                    if this.active().is_some_and(|job| job.review.is_some()) {
                        this.action(Action::EditReview, window, cx);
                    } else {
                        this.action(Action::Close, window, cx);
                    }
                } else if event.keystroke.key == "enter"
                    && (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    if this.active().is_some_and(|job| job.review.is_some()) {
                        this.action(Action::Send, window, cx);
                    } else {
                        this.action(Action::Review, window, cx);
                    }
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .child(div().text_size(px(18.0)).child(if self.listing {
                        "Email drafts"
                    } else {
                        "Compose email"
                    }))
                    .child(self.button(
                        "desktop-compose-close",
                        if self.listing {
                            "Back to email"
                        } else {
                            "Close editor"
                        },
                        Action::Close,
                        true,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("desktop-compose-feedback")
                    .debug_selector(|| "desktop-compose-feedback".into())
                    .role(Role::Group)
                    .aria_label("Draft status")
                    .track_focus(&self.feedback_focus)
                    .tab_index(0)
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|element| element.border_color(theme::persimmon_400()))
                    .max_h(px(128.0))
                    .flex_shrink_0()
                    .track_scroll(&self.feedback_scroll)
                    .overflow_y_scrollbar()
                    .px_4()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when_some(job, |el, job| {
                        el.child(
                            meta(status_text(job))
                                .id("desktop-compose-status")
                                .role(Role::Status),
                        )
                    })
                    .when_some(error.clone(), |el, error| {
                        el.child(
                            div()
                                .id("desktop-compose-error")
                                .role(Role::Alert)
                                .aria_label(error.clone())
                                .whitespace_normal()
                                .text_color(theme::lychee_400())
                                .child(error),
                        )
                    })
                    .when(
                        error.is_some()
                            && !busy
                            && job.is_none_or(|job| job.phase != Phase::SentError),
                        |el| {
                            el.child(self.button(
                                "desktop-compose-retry",
                                "Retry saving",
                                Action::Retry,
                                true,
                                cx,
                            ))
                        },
                    )
                    .when(pending && !busy, |el| {
                        el.child(self.button(
                            "desktop-compose-check",
                            "Check send status",
                            Action::Status,
                            true,
                            cx,
                        ))
                    })
                    .when(uncertain, |el| {
                        el.child(self.button(
                            "desktop-compose-check-sent",
                            "Check Sent",
                            Action::CheckSent,
                            !busy && !self.loading && job.is_some_and(sent::available),
                            cx,
                        ))
                    })
                    .when(queued && !busy, |el| {
                        el.child(self.button(
                            "desktop-compose-cancel",
                            "Cancel queued send",
                            Action::Cancel,
                            true,
                            cx,
                        ))
                    }),
            )
            .child(
                div()
                    .id("desktop-compose-scroll")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scrollbar()
                    .p_4()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(896.0))
                            .mx_auto()
                            .child(if self.listing {
                                self.list(cx)
                            } else if let Some(review) = review {
                                self.preview(review)
                            } else {
                                self.editor(cx)
                            }),
                    ),
            )
            .when(!self.listing, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .flex_shrink_0()
                        .px_4()
                        .py_3()
                        .border_t_1()
                        .border_color(theme::longan_800())
                        .child(self.button(
                            "desktop-compose-attach",
                            "Attach files",
                            Action::Attach,
                            !locked && !job.is_some_and(|job| job.phase == Phase::Upload),
                            cx,
                        ))
                        .child(if review.is_some() {
                            div()
                                .flex()
                                .gap_2()
                                .child(self.button(
                                    "desktop-compose-edit",
                                    "Edit",
                                    Action::EditReview,
                                    !busy,
                                    cx,
                                ))
                                .child(self.button(
                                    "desktop-compose-send",
                                    "Send email",
                                    Action::Send,
                                    !busy && saved,
                                    cx,
                                ))
                        } else {
                            div().child(self.button(
                                "desktop-compose-review",
                                "Review email",
                                Action::Review,
                                can_review,
                                cx,
                            ))
                        }),
                )
            })
    }
}
