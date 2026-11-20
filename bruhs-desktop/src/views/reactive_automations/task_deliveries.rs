//! Inspect the selected task's authenticated webhook requests on its host.
use super::*;

pub(super) struct DeliveryHistory {
    task: String,
    title: String,
    agent: String,
    source: String,
    request: uuid::Uuid,
    list: Option<Result<Vec<WebhookDeliverySummary>, String>>,
    selected: Option<String>,
    detail: Option<Result<WebhookDelivery, String>>,
    pub(super) initial_focus: FocusHandle,
    back_focus: FocusHandle,
    row_focus: FocusHandle,
    last_row: Option<String>,
    list_scroll: Option<ScrollHandle>,
}

impl ReactiveAutomations {
    #[cfg(test)]
    pub(crate) fn set_delivery_test_client(&mut self, client: ServerClient) {
        self.client = client;
    }

    pub(super) fn open_task_deliveries(
        &mut self,
        task: String,
        title: String,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(sub) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.subscriptions.iter().find(|sub| sub.id == task))
        else {
            return;
        };
        self.task_dialog.history = Some(DeliveryHistory {
            agent: sub.agent_id.clone(),
            source: sub.source_id.clone(),
            task,
            title,
            request: uuid::Uuid::nil(),
            list: None,
            selected: None,
            detail: None,
            initial_focus: cx.focus_handle().tab_stop(true),
            back_focus: cx.focus_handle().tab_stop(true),
            row_focus: cx.focus_handle().tab_stop(true),
            last_row: None,
            list_scroll: None,
        });
        self.refresh_task_deliveries(cx);
    }

    pub(super) fn reconcile_task_deliveries(&mut self, snapshot: &ReactiveSnapshot) {
        let Some(history) = &mut self.task_dialog.history else {
            return;
        };
        let subscription = snapshot.subscriptions.iter().find(|sub| {
            sub.id == history.task
                && sub.agent_id == history.agent
                && sub.source_id == history.source
        });
        if snapshot
            .sources
            .iter()
            .any(|s| s.id == history.source && s.kind == SourceKind::Webhook)
        {
            if let (Some(sub), Some(agent)) = (
                subscription,
                snapshot.agents.iter().find(|a| a.id == history.agent),
            ) {
                sub.title(agent).clone_into(&mut history.title);
                return;
            }
        }
        self.task_dialog.history = None;
    }

    fn refresh_task_deliveries(&mut self, cx: &mut Context<'_, Self>) {
        let Some(history) = &mut self.task_dialog.history else {
            return;
        };
        let request = uuid::Uuid::new_v4();
        history.request = request;
        history.list = None;
        let task = history.task.clone();
        let client = self.client.clone();
        let result = spawn_tokio(cx, async move { client.webhook_deliveries(&task).await });
        cx.spawn(async move |this, cx| {
            let rows = result.await.map_err(reactive_error);
            let _ = this.update(cx, |this, cx| {
                if let Some(history) = this
                    .task_dialog
                    .history
                    .as_mut()
                    .filter(|h| h.request == request)
                {
                    history.list = Some(rows);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn view_task_delivery(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(history) = &mut self.task_dialog.history else {
            return;
        };
        let request = uuid::Uuid::new_v4();
        history.request = request;
        history.selected = Some(id.clone());
        history.last_row = Some(id.clone());
        history.detail = None;
        history.back_focus.focus(window, cx);
        history.list_scroll = Some(std::mem::take(&mut self.task_dialog.scroll));
        let task = history.task.clone();
        let client = self.client.clone();
        let result = spawn_tokio(cx, async move { client.webhook_delivery(&task, &id).await });
        cx.spawn(async move |this, cx| {
            let result = result.await.map_err(reactive_error);
            let _ = this.update(cx, |this, cx| {
                if let Some(history) = this
                    .task_dialog
                    .history
                    .as_mut()
                    .filter(|h| h.request == request && h.selected.is_some())
                {
                    history.detail = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_task_deliveries(
        &self,
        history: &DeliveryHistory,
        cx: &Context<'_, Self>,
    ) -> (String, &'static str, AnyElement, Div) {
        let mut content = div().flex().flex_col().gap_2().min_w_0();
        let mut footer = scheduled_tasks::task_footer();
        if history.selected.is_some() {
            content = match &history.detail {
                None => content.child(muted("Loading delivery…")),
                Some(Err(error)) => content.child(
                    muted(error.clone())
                        .id("delivery-read-error")
                        .role(Role::Alert)
                        .aria_label(error.clone()),
                ),
                Some(Ok(detail)) => content
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(delivery_heading(&detail.summary))
                            .when(detail.signature_verified, |row| {
                                row.child(
                                    muted("Signature verified")
                                        .debug_selector(|| "delivery-signature-verified".into()),
                                )
                            }),
                    )
                    .when_some(detail.error.as_ref(), |el, error| {
                        el.child(muted(error.clone()).when(
                            matches!(
                                detail.summary.outcome,
                                WebhookOutcome::Rejected
                                    | WebhookOutcome::RejectedSignature
                                    | WebhookOutcome::Expired
                            ),
                            |line| line.text_color(theme::lychee_300()),
                        ))
                    })
                    .child(section(
                        if detail.summary.outcome == WebhookOutcome::WindowOpened {
                            "Scheduled run"
                        } else {
                            "Task prompt"
                        },
                        detail.prompt.as_deref().unwrap_or(
                            if detail.summary.outcome == WebhookOutcome::WindowOpened {
                                "The run is scheduled when the window closes."
                            } else {
                                "No run was started for this request."
                            },
                        ),
                    ))
                    .when(!detail.missing_fields.is_empty(), |c| {
                        c.child(muted(format!(
                            "Empty placeholders · {}",
                            detail.missing_fields.join(", ")
                        )))
                    })
                    .child(section("Headers", &detail.headers))
                    .when(!detail.query.is_empty(), |c| {
                        c.child(section("Query", &detail.query))
                    })
                    .child(section(
                        if detail.body_truncated {
                            "Body (truncated)"
                        } else {
                            "Body"
                        },
                        &detail.body,
                    )),
            };
            footer = footer.child(
                ghost_control("delivery-back", "Back", false)
                    .track_focus(&history.back_focus)
                    .on_activate(cx, |this, window, cx| {
                        if let Some(history) = &mut this.task_dialog.history {
                            history.selected = None;
                            history.detail = None;
                            if let Some(scroll) = history.list_scroll.take() {
                                this.task_dialog.scroll = scroll;
                            }
                            history.row_focus.focus(window, cx);
                            cx.notify();
                        }
                    }),
            );
        } else {
            match &history.list {
                None => content = content.child(muted("Loading deliveries…")),
                Some(Err(error)) => {
                    content = content.child(
                        muted(error.clone())
                            .id("delivery-read-error")
                            .role(Role::Alert)
                            .aria_label(error.clone()),
                    )
                }
                Some(Ok(rows)) if rows.is_empty() => {
                    content = content.child(
                        muted("No recorded requests yet.")
                            .id("delivery-empty")
                            .debug_selector(|| "delivery-empty".into())
                            .role(Role::Status)
                            .aria_label("No recorded requests yet."),
                    )
                }
                Some(Ok(rows)) => {
                    for row in rows {
                        let id = row.id.clone();
                        let label = delivery_label(row);
                        content = content.child(
                            web_button(
                                SharedString::from(format!("delivery-{id}")),
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .aria_label(label)
                            .debug_selector({
                                let id = id.clone();
                                move || format!("delivery-row-{id}")
                            })
                            .w_full()
                            .justify_start()
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(delivery_heading(row))
                                    .when_some(row.filter_reason.as_ref(), |el, reason| {
                                        el.child(muted(reason.clone()))
                                    })
                                    .when(row.missing_field_count > 0, |el| {
                                        el.child(muted(missing_label(row.missing_field_count)))
                                    }),
                            )
                            .when(history.last_row.as_ref() == Some(&id), |b| {
                                b.track_focus(&history.row_focus)
                            })
                            .on_activate(
                                cx,
                                move |this, window, cx| {
                                    this.view_task_delivery(id.clone(), window, cx)
                                },
                            ),
                        );
                    }
                }
            }
            footer = footer.child(
                ghost_control("deliveries-refresh", "Refresh", history.list.is_none())
                    .on_activate(cx, |this, _, cx| this.refresh_task_deliveries(cx)),
            );
        }
        footer = footer.child(
            control_as("deliveries-done", "Done", false, true)
                .track_focus(&history.initial_focus)
                .on_activate(cx, |this, _, cx| this.dismiss_scheduled_task(cx)),
        );
        (
            format!("Deliveries · {}", history.title),
            "Recent requests to this task’s webhook.",
            content.into_any_element(),
            footer,
        )
    }
}

fn missing_label(count: u32) -> String {
    format!(
        "{count} empty placeholder{}",
        if count == 1 { "" } else { "s" }
    )
}

fn delivery_label(row: &WebhookDeliverySummary) -> String {
    let mut parts = vec![
        row.outcome.label().to_owned(),
        "POST".into(),
        local_time(row.received_at),
    ];
    parts.extend(row.filter_reason.iter().cloned());
    if row.missing_field_count > 0 {
        parts.push(missing_label(row.missing_field_count));
    }
    parts.join(" · ")
}

fn delivery_heading(row: &WebhookDeliverySummary) -> Div {
    use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
    let color = match row.outcome {
        WebhookOutcome::Queued => theme::pandan_400(),
        WebhookOutcome::AwaitingReview => theme::blueberry_400(),
        WebhookOutcome::WindowOpened
        | WebhookOutcome::Duplicate
        | WebhookOutcome::Throttled
        | WebhookOutcome::Expired
        | WebhookOutcome::Paused
        | WebhookOutcome::Coalesced => theme::durian_400(),
        WebhookOutcome::Rejected | WebhookOutcome::RejectedSignature => theme::lychee_400(),
        WebhookOutcome::Filtered => theme::rambutan_400(),
    };
    let badge = updates::badge_plain(row.outcome.label())
        .bg(color.opacity(0.14))
        .text_color(color);
    let age = format_relative_ms(
        row.received_at,
        bruhs_models::EpochMs::now(),
        RelativeTimeStyle::Ago,
    );
    div()
        .flex()
        .items_center()
        .flex_wrap()
        .gap_2()
        .child(badge)
        .child(muted("POST"))
        .child(muted(age))
}

fn section(label: &'static str, text: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            heading(label)
                .id(SharedString::from(format!("delivery-section-{label}")))
                .debug_selector(move || format!("delivery-section-{label}"))
                .role(Role::Heading)
                .aria_level(3)
                .aria_label(label),
        )
        .child(
            updates::selectable_text(label, if text.is_empty() { "(empty)" } else { text })
                .font_family(theme::CODE_FONT_FAMILY)
                .p_2()
                .rounded(px(6.))
                .bg(theme::longan_900()),
        )
}
