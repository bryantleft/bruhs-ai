//! Agent workflows' cards, drawn like the web's (`components/mcp_review.rs`,
//! `components/automations_page.rs`, `components/workflow_payload.rs`): an
//! MCP client's undo request and every workflow waiting in preview get a
//! card of their own above Local suggestions, the page's own dry run is
//! reviewed in Preview changes, then the jobs and the audit timeline. Which
//! workflows wait, the grants they need and every word are shared
//! (`bruhs_models::logic::workflow_review`).

use super::*;
use bruhs_models::logic::workflow_history::UNDO;
use bruhs_models::logic::workflow_review as review;
use gpui_component::zoom::px;

/// Web `rounded-grape`.
pub(in crate::views::workspace) const CARD_RADIUS_PX: f32 = 8.0;
/// Web `text-h4`.
const TITLE_PX: f32 = 16.0;
/// Web `text-body-sm`.
const BODY_SM_PX: f32 = 13.0;
/// Web `text-label`.
const LABEL_PX: f32 = 13.0;
/// Web `text-label-sm`.
const LABEL_SM_PX: f32 = 12.0;

/// Web `rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-…` with
/// `ring` as the ring.
pub(in crate::views::workspace) fn web_card(ring: Hsla) -> Div {
    div()
        .min_w_0()
        .rounded(px(CARD_RADIUS_PX))
        .bg(theme::longan_900().opacity(0.7))
        .border_1()
        .border_color(ring)
        // The web's ring is drawn inside its 16px padding and takes no
        // room; this 1px border does, so the padding gives it back.
        .p(px(15.0))
}

/// Web `inset-ring-longan-700/50`, the cards' usual ring.
pub(in crate::views::workspace) fn neutral_ring() -> Hsla {
    theme::longan_700().opacity(0.5)
}

/// Web `text-h4 text-rambutan-100`, a card's title.
pub(in crate::views::workspace) fn card_title(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .font_family(theme::HEADING_FONT_FAMILY)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(TITLE_PX))
        .text_color(theme::rambutan_100())
        .child(text.into())
}

/// Web `text-body-sm` in `color`.
pub(in crate::views::workspace) fn body_sm(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .min_w_0()
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(BODY_SM_PX))
        .text_color(color)
        .child(text.into())
}

/// Web `text-label` in `color`.
pub(in crate::views::workspace) fn label(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .min_w_0()
        .font_family(theme::UI_FONT_FAMILY)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(LABEL_PX))
        .text_color(color)
        .child(text.into())
}

/// Web `text-label-sm` in `color`.
pub(in crate::views::workspace) fn label_sm(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .min_w_0()
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(LABEL_SM_PX))
        .text_color(color)
        .child(text.into())
}

/// Web `text-label-sm uppercase tracking-wide text-rambutan-400`, the label
/// over the Safety contract and Workflow history cards.
pub(in crate::views::workspace) fn caps_label(text: &'static str) -> Stateful<Div> {
    label_sm(text.to_uppercase(), theme::rambutan_400())
        .id(text)
        .aria_label(text)
}

/// A payload instant as both apps print it: ISO 8601 in UTC and the raw
/// milliseconds.
fn review_instant(millis: f64) -> String {
    if !millis.is_finite() || millis.fract() != 0.0 {
        return format!("Invalid timestamp ({millis})");
    }
    chrono::DateTime::from_timestamp_millis(EpochMs::saturating_from_f64(millis).as_millis())
        .map_or_else(
            || format!("Invalid timestamp ({millis})"),
            |date| {
                format!(
                    "{} ({millis} ms since Unix epoch)",
                    date.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                )
            },
        )
}

/// Web `WorkflowPayload`: every operation of a workflow with all its fields.
pub(super) fn workflow_payload(operations: &[WorkflowOperation]) -> Stateful<Div> {
    div()
        .id("workflow-payload")
        .min_w_0()
        .role(Role::Group)
        .aria_label("Workflow payload")
        .child(label_sm(review::PAYLOAD_NOTE, theme::rambutan_400()).mt_2())
        .child(
            div()
                .mt_3()
                .flex()
                .flex_col()
                .gap_2()
                .children(operations.iter().enumerate().map(|(index, operation)| {
                    div()
                        .min_w_0()
                        .rounded(px(CARD_RADIUS_PX))
                        .bg(theme::longan_950().opacity(0.6))
                        .px_3()
                        .py_2()
                        .child(label(
                            format!("{}, {}", index + 1, operation.domain().label()),
                            theme::rambutan_200(),
                        ))
                        .child(div().mt_2().flex().flex_col().gap_2().children(
                            operation.review_fields(review_instant).into_iter().map(
                                |(name, value)| {
                                    div()
                                        .min_w_0()
                                        .child(label_sm(name, theme::rambutan_500()))
                                        .child(body_sm(value, theme::rambutan_300()))
                                },
                            ),
                        ))
                })),
        )
}

impl Workspace {
    fn workflow_operations(&self, id: &str) -> Vec<WorkflowOperation> {
        self.automation_state
            .workflows
            .iter()
            .find(|workflow| workflow.id == id)
            .map(|workflow| {
                workflow
                    .steps
                    .iter()
                    .map(|step| step.operation.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Web `WorkflowReview`: "Review workflow" shows the payload under it.
    /// `key` tells apart the places one workflow can be reviewed.
    pub(super) fn review_disclosure(
        &self,
        key: String,
        workflow_id: &str,
        cx: &Context<'_, Self>,
    ) -> Div {
        let open = self.workflow_disclosures.contains(&key);
        let operations = open.then(|| self.workflow_operations(workflow_id));
        div()
            .w_full()
            .min_w_0()
            .child(
                web_button(
                    SharedString::from(format!("review-workflow-{key}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector({
                    let key = key.clone();
                    move || format!("desktop-review-workflow-{key}")
                })
                .aria_label(review::REVIEW_WORKFLOW)
                .aria_expanded(open)
                // Web `<summary>`: the row's full width, its words at the left.
                .w_full()
                .justify_start()
                .px_0()
                .text_color(theme::rambutan_400())
                .child(review::REVIEW_WORKFLOW)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.workflow_disclosures.remove(&key) {
                        this.workflow_disclosures.insert(key.clone());
                    }
                    cx.notify();
                })),
            )
            .when_some(operations, |el, operations| {
                el.child(workflow_payload(&operations))
            })
    }

    /// The page's own dry run while it waits, reviewed in Preview changes.
    pub(in crate::views::workspace) fn shown_dry_run(&self) -> Option<&str> {
        let id = self.dry_run_workflow_id.as_deref()?;
        self.automation_state
            .workflows
            .iter()
            .any(|workflow| workflow.id == id && workflow.status == WorkflowStatus::Previewed)
            .then_some(id)
    }

    /// Web `WorkflowReviews`: each undo request, then each workflow waiting
    /// in preview, newest first.
    pub(super) fn render_workflow_reviews(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let undo = self
            .mcp_bridge
            .rollback_requests()
            .into_iter()
            .filter(|id| {
                self.automation_state.workflows.iter().any(|workflow| {
                    &workflow.id == id && workflow.status != WorkflowStatus::RolledBack
                })
            })
            .map(|id| self.render_undo_request(id, cx));
        let awaiting = review::awaiting_review(&self.automation_state, self.shown_dry_run())
            .into_iter()
            .enumerate()
            .map(|(index, id)| self.render_awaiting_review(index, id, cx));
        undo.chain(awaiting).collect()
    }

    fn render_undo_request(&self, id: String, cx: &Context<'_, Self>) -> AnyElement {
        let saved = self.save_status.is_saved();
        let keep_id = id.clone();
        let undo_id = id.clone();
        web_card(theme::lychee_400().opacity(0.3))
            .id(SharedString::from(format!("mcp-rollback-review-{id}")))
            .debug_selector(|| "desktop-mcp-rollback-review".into())
            .role(Role::Group)
            .aria_label(bruhs_models::workspace_bridge::ROLLBACK_REVIEW_NOTE)
            .child(card_title(review::UNDO_REQUESTED))
            .child(
                body_sm(
                    bruhs_models::workspace_bridge::ROLLBACK_REVIEW_NOTE,
                    theme::lychee_300(),
                )
                .mt_1(),
            )
            .child(self.review_disclosure(format!("undo:{id}"), &id, cx))
            .child(
                div()
                    .mt_3()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        web_button(
                            SharedString::from(format!("mcp-decline-rollback-{id}")),
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-decline-mcp-rollback".into())
                        .aria_label(review::KEEP_CHANGES)
                        .child(review::KEEP_CHANGES)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.mcp_bridge.decline_rollback(&keep_id) {
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        web_button(
                            SharedString::from(format!("mcp-accept-rollback-{id}")),
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(!saved),
                        )
                        .debug_selector(|| "desktop-accept-mcp-rollback".into())
                        .aria_label(UNDO)
                        .child(UNDO)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.rollback_reviewed_automation(&undo_id, cx)
                        })),
                    ),
            )
            .into_any_element()
    }

    fn render_awaiting_review(
        &self,
        index: usize,
        id: String,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let Some(workflow) = self
            .automation_state
            .workflows
            .iter()
            .find(|workflow| workflow.id == id)
        else {
            return div().into_any_element();
        };
        let saved = self.save_status.is_saved();
        let blocked = review::blocked_domains(&self.automation_state, &id);
        let operations: Vec<_> = workflow
            .steps
            .iter()
            .map(|step| step.operation.clone())
            .collect();
        let origin = review::origin(self.mcp_bridge.owns(&id));
        // The newest card's controls keep the names the tests and the
        // smoke scenes use; later cards number theirs.
        let selector = move |name: &'static str| {
            move || {
                if index == 0 {
                    format!("desktop-{name}")
                } else {
                    format!("desktop-{name}-{index}")
                }
            }
        };
        let (recheck_id, start_id, decline_id) = (id.clone(), id.clone(), id.clone());
        web_card(theme::blueberry_400().opacity(0.3))
            .id(SharedString::from(format!("workflow-awaiting-review-{id}")))
            .debug_selector(selector("workflow-awaiting-review"))
            .role(Role::Group)
            .aria_label(review::AWAITING_REVIEW_LABEL)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .child(card_title(workflow.title.clone()))
                            .child(body_sm(origin, theme::rambutan_500())),
                    )
                    .child(
                        web_button(
                            SharedString::from(format!("automation-recheck-{id}")),
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(!saved),
                        )
                        .debug_selector(selector("recheck-workflow"))
                        .aria_label(review::RECHECK_GRANTS)
                        .child(review::RECHECK_GRANTS)
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.recheck_workflow(&recheck_id, cx)
                            }),
                        ),
                    ),
            )
            .child(workflow_payload(&operations))
            .when_some(review::grant_write_for(&blocked), |el, message| {
                el.child(
                    body_sm(message, theme::durian_300())
                        .id("grant-write-for")
                        .mt_3()
                        .role(Role::Alert),
                )
            })
            .child(
                div()
                    .mt_3()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        web_button(
                            SharedString::from(format!("automation-start-{id}")),
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(!blocked.is_empty() || !saved),
                        )
                        .debug_selector(selector("start-workflow"))
                        .aria_label(review::START_TRANSACTION)
                        .child(review::START_TRANSACTION)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_reviewed_automation(&start_id, cx)
                        })),
                    )
                    .child(
                        web_button(
                            SharedString::from(format!("automation-decline-{id}")),
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(!saved),
                        )
                        .debug_selector(selector("decline-workflow"))
                        .aria_label(review::DECLINE_PROPOSAL)
                        .child(review::DECLINE_PROPOSAL)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.decline_reviewed_workflow(&decline_id, cx)
                        })),
                    ),
            )
            .into_any_element()
    }

    /// Web "Preview changes": the page's own dry run, while it waits.
    pub(super) fn render_preview_changes(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let id = self.shown_dry_run()?.to_owned();
        let saved = self.save_status.is_saved();
        let blocked = review::blocked_domains(&self.automation_state, &id);
        let operations = self.workflow_operations(&id);
        let (recheck_id, start_id, decline_id) = (id.clone(), id.clone(), id);
        Some(
            web_card(neutral_ring())
                .id("automation-dry-run")
                .debug_selector(|| "desktop-preview-changes".into())
                .role(Role::Group)
                .aria_label(review::PREVIEW_CHANGES)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .min_w_0()
                                .child(card_title(review::PREVIEW_CHANGES))
                                .child(body_sm(review::NOTHING_RUN_YET, theme::rambutan_500())),
                        )
                        .child(
                            web_button(
                                "automation-dry-run-recheck",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::disabled_if(!saved),
                            )
                            .debug_selector(|| "desktop-recheck-dry-run".into())
                            .aria_label(review::RECHECK_GRANTS)
                            .child(review::RECHECK_GRANTS)
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.recheck_workflow(&recheck_id, cx),
                            )),
                        ),
                )
                .child(workflow_payload(&operations))
                .when_some(review::grant_write_for(&blocked), |el, message| {
                    el.child(
                        body_sm(message, theme::durian_300())
                            .id("grant-write-for")
                            .mt_3()
                            .role(Role::Alert),
                    )
                })
                .child(
                    div()
                        .mt_3()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            web_button(
                                "automation-dry-run-start",
                                WebButtonVariant::Primary,
                                WebButtonSize::Small,
                                ButtonState::disabled_if(!blocked.is_empty() || !saved),
                            )
                            // Painted after the awaiting cards, so this one
                            // answers to the name when both are shown.
                            .debug_selector(|| "desktop-start-workflow".into())
                            .aria_label(review::START_TRANSACTION)
                            .child(review::START_TRANSACTION)
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.start_reviewed_automation(&start_id, cx),
                            )),
                        )
                        .child(
                            web_button(
                                "automation-dry-run-decline",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::disabled_if(!saved),
                            )
                            .debug_selector(|| "desktop-decline-dry-run".into())
                            .aria_label(review::DECLINE_PROPOSAL)
                            .child(review::DECLINE_PROPOSAL)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.decline_reviewed_workflow(&decline_id, cx)
                                },
                            )),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Previews a waiting workflow again against the current grants.
    fn recheck_workflow(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if !self.save_status.is_saved() {
            return;
        }
        self.automation_error = self
            .automation_state
            .preview(id, EpochMs::now().as_millis())
            .is_none()
            .then(|| "This workflow can no longer be previewed. No records were changed.".into());
        self.persist_and_notify(cx);
    }

    fn decline_reviewed_workflow(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if self.save_status.is_saved()
            && self
                .automation_state
                .decline_workflow(id, EpochMs::now().as_millis())
        {
            self.persist_and_notify(cx);
        }
    }

    /// Web "Background jobs": the newest jobs, each with its progress and
    /// payload. Their controls are on the workflow on show in the history.
    pub(super) fn render_background_jobs(&self, cx: &Context<'_, Self>) -> Div {
        let jobs = self
            .automation_state
            .jobs
            .iter()
            .rev()
            .take(review::JOBS_SHOWN)
            .map(|job| {
                let total = self
                    .automation_state
                    .workflows
                    .iter()
                    .find(|workflow| workflow.id == job.workflow_id)
                    .map_or(0, |workflow| workflow.steps.len());
                let active = matches!(
                    job.status,
                    BackgroundJobStatus::Running | BackgroundJobStatus::Paused
                );
                div()
                    .id(SharedString::from(format!("automation-job-{}", job.id)))
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .rounded(px(CARD_RADIUS_PX))
                    .bg(theme::longan_950().opacity(0.6))
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .min_w_0()
                            .child(label(
                                review::job_progress(job.next_step, total),
                                theme::rambutan_200(),
                            ))
                            .child(label_sm(format!("{:?}", job.status), theme::rambutan_500())),
                    )
                    .when(!active, |row| {
                        row.child(label_sm(review::CHECKPOINTED, theme::pandan_300()))
                    })
                    .child(self.review_disclosure(format!("job:{}", job.id), &job.workflow_id, cx))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        web_card(neutral_ring())
            .debug_selector(|| "desktop-background-jobs".into())
            .child(card_title(review::BACKGROUND_JOBS))
            .child(body_sm(review::BACKGROUND_JOBS_NOTE, theme::rambutan_500()))
            .child(div().mt_3().flex().flex_col().gap_2().children(jobs))
    }

    /// Web "Audit timeline": the newest audit events.
    pub(super) fn render_audit_timeline(&self) -> Div {
        web_card(neutral_ring())
            .debug_selector(|| "desktop-audit-timeline".into())
            .child(card_title(review::AUDIT_TIMELINE))
            .child(
                div().mt_3().flex().flex_col().gap_2().children(
                    self.automation_state
                        .audit
                        .iter()
                        .rev()
                        .take(review::AUDIT_SHOWN)
                        .map(|event| {
                            div()
                                .min_w_0()
                                .border_l_1()
                                .border_color(theme::blueberry_500().opacity(0.3))
                                .pl_3()
                                .child(label_sm(
                                    format!("{:?}", event.kind),
                                    theme::blueberry_300(),
                                ))
                                .child(body_sm(event.message.clone(), theme::rambutan_300()))
                        }),
                ),
            )
    }

    /// Web "Safety contract": the rules, the tool and workflow count and the
    /// local records the workflow domains hold.
    pub(super) fn render_safety_contract(&self) -> Div {
        let records = self.work_items.len()
            + self.calendar_events.len()
            + self.knowledge_documents.len()
            + self.inbox_conversations.len()
            + self.captures.len()
            + self.threads.len()
            + self.meeting_notes.len()
            + self.life_context.finance.transactions.len()
            + self.life_context.health.workouts.len();
        web_card(neutral_ring())
            .debug_selector(|| "desktop-safety-contract".into())
            .child(caps_label(review::SAFETY_CONTRACT))
            .child(body_sm(review::SAFETY_CONTRACT_NOTE, theme::rambutan_300()).mt_2())
            .child(
                label_sm(
                    review::tools_and_workflows(
                        agent_tool_definitions().len(),
                        self.automation_state.workflows.len(),
                    ),
                    theme::pandan_300(),
                )
                .mt_2(),
            )
            .child(label_sm(review::local_records(records), theme::rambutan_400()).mt_1())
    }
}
