//! Inspect the selected task's authenticated webhook requests on its host.
use super::*;
use crate::views::workspace::LinkedDeliveries;

enum DeliveryOrigin {
    Local { agent: String, source: String },
    Linked(LinkedDeliveries),
}

pub(super) struct DeliveryHistory {
    task: String,
    title: String,
    origin: DeliveryOrigin,
    pub(super) return_focus: Option<FocusHandle>,
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
        let origin = DeliveryOrigin::Local {
            agent: sub.agent_id.clone(),
            source: sub.source_id.clone(),
        };
        self.open_delivery_history(task, title, origin, None, cx);
    }

    pub(crate) fn open_linked_task_deliveries(
        &mut self,
        task: String,
        title: String,
        host: LinkedDeliveries,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.task_dialog.focus_on_open = true;
        self.task_dialog.scroll = ScrollHandle::new();
        self.open_delivery_history(
            task,
            title,
            DeliveryOrigin::Linked(host),
            window.focused(cx),
            cx,
        );
    }

    fn open_delivery_history(
        &mut self,
        task: String,
        title: String,
        origin: DeliveryOrigin,
        return_focus: Option<FocusHandle>,
        cx: &mut Context<'_, Self>,
    ) {
        self.task_dialog.history = Some(DeliveryHistory {
            origin,
            return_focus,
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
        let DeliveryOrigin::Local { agent, source } = &history.origin else {
            return;
        };
        let subscription = snapshot.subscriptions.iter().find(|sub| {
            sub.id == history.task && &sub.agent_id == agent && &sub.source_id == source
        });
        if snapshot
            .sources
            .iter()
            .any(|s| &s.id == source && s.kind == SourceKind::Webhook)
        {
            if let (Some(sub), Some(agent)) = (
                subscription,
                snapshot.agents.iter().find(|a| &a.id == agent),
            ) {
                sub.title(agent).clone_into(&mut history.title);
                return;
            }
        }
        self.task_dialog.history = None;
    }

    pub(crate) fn linked_delivery_origin(&self) -> Option<(&LinkedDeliveries, &str, &str)> {
        let history = self.task_dialog.history.as_ref()?;
        let DeliveryOrigin::Linked(reader) = &history.origin else {
            return None;
        };
        Some((reader, &history.task, &history.title))
    }

    pub(crate) fn reconcile_linked_deliveries(
        &mut self,
        title: Option<&str>,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(title) = title {
            if let Some(history) = &mut self.task_dialog.history {
                title.clone_into(&mut history.title);
            }
        } else if let Some(history) = self.task_dialog.history.take() {
            if visible {
                if let Some(focus) = history.return_focus {
                    focus.focus(window, cx);
                }
            }
        }
        cx.notify();
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
        let result = match &history.origin {
            DeliveryOrigin::Linked(host) => {
                let host = host.clone();
                spawn_tokio(cx, async move { host.read(&task, None).await })
            }
            DeliveryOrigin::Local { .. } => spawn_tokio(cx, async move {
                client
                    .webhook_deliveries(&task)
                    .await
                    .map_err(reactive_error)
            }),
        };
        cx.spawn(async move |this, cx| {
            let rows = result.await;
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
        let result = match &history.origin {
            DeliveryOrigin::Linked(host) => {
                let host = host.clone();
                spawn_tokio(cx, async move { host.read(&task, Some(&id)).await })
            }
            DeliveryOrigin::Local { .. } => spawn_tokio(cx, async move {
                client
                    .webhook_delivery(&task, &id)
                    .await
                    .map_err(reactive_error)
            }),
        };
        cx.spawn(async move |this, cx| {
            let result = result.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(history) = this
                    .task_dialog
                    .history
                    .as_mut()
                    .filter(|h| h.request == request && h.selected.is_some())
                {
                    if let (DeliveryOrigin::Linked(_), Err(error)) = (&history.origin, &result) {
                        history.list = Some(Err(error.clone()));
                    }
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
                control_as("delivery-back", "Back", false, false)
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
                    content = content.gap_0();
                    for (index, row) in rows.iter().enumerate() {
                        if index > 0 {
                            content =
                                content.child(div().h_px().flex_shrink_0().bg(theme::ring_modal()));
                        }
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
                            .py_2()
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
                control_as(
                    "deliveries-refresh",
                    "Refresh",
                    history.list.is_none(),
                    false,
                )
                .on_activate(cx, |this, _, cx| this.refresh_task_deliveries(cx)),
            );
        }
        footer = footer.child(
            control_as("deliveries-done", "Done", false, true)
                .track_focus(&history.initial_focus)
                .on_activate(cx, |this, window, cx| {
                    this.dismiss_scheduled_task(window, cx)
                }),
        );
        (
            match &history.origin {
                DeliveryOrigin::Local { .. } => format!("Deliveries · {}", history.title),
                DeliveryOrigin::Linked(host) => {
                    format!("Deliveries · {} · {}", history.title, host.name())
                }
            },
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
    use bruhs_models::time::{RelativeTimeStyle, format_relative_ms};
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
            div()
                .id(SharedString::from(format!("delivery-preview-{label}")))
                .max_h(px(256.))
                .overflow_y_scroll()
                .child(
                    updates::selectable_text(label, if text.is_empty() { "(empty)" } else { text })
                        .font_family(theme::CODE_FONT_FAMILY),
                )
                .p_2()
                .rounded(px(6.))
                .bg(theme::longan_900()),
        )
}
