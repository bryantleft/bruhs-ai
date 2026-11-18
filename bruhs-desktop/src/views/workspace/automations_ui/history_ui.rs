use super::*;
use bruhs_models::logic::workflow_history::{
    self, NEXT_WORKFLOW, NEXT_WORKFLOW_LABEL, NO_WORKFLOW_YET, PREVIOUS_WORKFLOW,
    PREVIOUS_WORKFLOW_LABEL,
};
use bruhs_models::logic::workflow_review as review;

impl Workspace {
    #[cfg(test)]
    pub(super) fn reviewed_automation(&self) -> Option<&bruhs_models::Workflow> {
        workflow_history::reviewed(&self.automation_state, self.reviewed_workflow_id.as_deref())
    }

    fn review_automation(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if self
            .automation_state
            .workflows
            .iter()
            .any(|workflow| workflow.id == id)
        {
            self.reviewed_workflow_id = Some(id.into());
            cx.notify();
        }
    }

    /// Web `WorkflowHistory`: every workflow, one at a time, with the
    /// controls of the one on show and its payload under Review workflow.
    pub(super) fn render_workflow_history(&self, cx: &Context<'_, Self>) -> Div {
        let card = review_ui::web_card(review_ui::neutral_ring())
            .debug_selector(|| "desktop-workflow-history".into())
            .child(review_ui::caps_label(review::WORKFLOW_HISTORY));
        let Some(page) =
            workflow_history::page(&self.automation_state, self.reviewed_workflow_id.as_deref())
        else {
            return card.child(review_ui::body_sm(NO_WORKFLOW_YET, theme::rambutan_500()).mt_1());
        };
        let saved = self.save_status.is_saved();
        let previous = page.previous.map(str::to_owned);
        let next = page.next.map(str::to_owned);
        let workflow_id = page.workflow.id.clone();
        let controls = workflow_history::controls(&self.automation_state, page.workflow);
        let running = controls.running.map(|job| job.id.clone());
        let paused = controls.paused.map(|job| job.id.clone());
        let in_background = controls.in_background();
        let undo = controls.undo.then(|| workflow_id.clone());
        let job_status = controls.job_status();
        let has_controls = running.is_some() || paused.is_some() || undo.is_some();
        card.child(
            div()
                .id("automation-history")
                .debug_selector(|| "desktop-workflow-review".into())
                .role(Role::Group)
                .aria_label(page.label())
                .min_w_0()
                .child(review_ui::label(page.workflow.title.clone(), theme::rambutan_200()).mt_1())
                .child(review_ui::body_sm(page.status(), theme::rambutan_500()))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .mt_3()
                        .child(
                            web_button(
                                "automation-previous",
                                WebButtonVariant::Outline,
                                WebButtonSize::Small,
                                ButtonState::disabled_if(previous.is_none()),
                            )
                            .debug_selector(|| "desktop-previous-workflow".into())
                            .aria_label(PREVIOUS_WORKFLOW_LABEL)
                            .child(PREVIOUS_WORKFLOW)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(id) = &previous {
                                        this.review_automation(id, cx);
                                    }
                                },
                            )),
                        )
                        .child(
                            web_button(
                                "automation-next",
                                WebButtonVariant::Outline,
                                WebButtonSize::Small,
                                ButtonState::disabled_if(next.is_none()),
                            )
                            .debug_selector(|| "desktop-next-workflow".into())
                            .aria_label(NEXT_WORKFLOW_LABEL)
                            .child(NEXT_WORKFLOW)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(id) = &next {
                                        this.review_automation(id, cx);
                                    }
                                },
                            )),
                        ),
                )
                // The job's progress, as the web reads it out.
                .when_some(job_status, |el, status| {
                    el.child(
                        review_ui::body_sm(status, theme::rambutan_400())
                            .mt_3()
                            .debug_selector(|| "desktop-workflow-job-status".into()),
                    )
                })
                .when(has_controls, |el| {
                    el.child(
                        div()
                            .mt_3()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .when_some(running.clone(), |row, job_id| {
                                let background_id = job_id.clone();
                                let pause_id = job_id.clone();
                                row.child(
                                    web_button(
                                        "automation-run-next",
                                        WebButtonVariant::Outline,
                                        WebButtonSize::Small,
                                        ButtonState::disabled_if(!saved),
                                    )
                                    .debug_selector(|| "desktop-run-workflow-step".into())
                                    .aria_label(workflow_history::RUN_NEXT_STEP)
                                    .child(workflow_history::RUN_NEXT_STEP)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| this.run_automation_step(&job_id, cx),
                                    )),
                                )
                                .child(
                                    web_button(
                                        "automation-enable-background",
                                        WebButtonVariant::Outline,
                                        WebButtonSize::Small,
                                        ButtonState::disabled_if(!saved || in_background),
                                    )
                                    .debug_selector(|| "desktop-enable-workflow-background".into())
                                    .aria_label(workflow_history::RUN_IN_BACKGROUND)
                                    .child(workflow_history::RUN_IN_BACKGROUND)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.enable_background_automation(&background_id, cx)
                                        },
                                    )),
                                )
                                .child(
                                    // Not gated on the save: the desktop saves after
                                    // every step, so a running workflow is never
                                    // "saved" for long, and the next save writes
                                    // the pause. (The web's Pause waits instead.)
                                    web_button(
                                        "automation-pause",
                                        WebButtonVariant::Ghost,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .debug_selector(|| "desktop-pause-workflow".into())
                                    .aria_label(workflow_history::PAUSE_LABEL)
                                    .child(workflow_history::PAUSE)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if this
                                                .automation_state
                                                .pause_job(&pause_id, EpochMs::now().as_millis())
                                            {
                                                this.persist_and_notify(cx);
                                            }
                                        },
                                    )),
                                )
                            })
                            .when_some(paused.clone(), |row, job_id| {
                                row.child(
                                    web_button(
                                        "automation-resume",
                                        WebButtonVariant::Outline,
                                        WebButtonSize::Small,
                                        ButtonState::disabled_if(!saved),
                                    )
                                    .debug_selector(|| "desktop-resume-workflow".into())
                                    .aria_label(workflow_history::RESUME_LABEL)
                                    .child(workflow_history::RESUME)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| this.resume_automation(&job_id, cx),
                                    )),
                                )
                            })
                            .when_some(undo.clone(), |row, undo_id| {
                                row.child(
                                    web_button(
                                        "automation-rollback",
                                        WebButtonVariant::Destructive,
                                        WebButtonSize::Small,
                                        ButtonState::disabled_if(!saved),
                                    )
                                    .debug_selector(|| "desktop-undo-workflow".into())
                                    .aria_label(workflow_history::UNDO)
                                    .child(workflow_history::UNDO)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.rollback_reviewed_automation(&undo_id, cx)
                                        },
                                    )),
                                )
                            }),
                    )
                })
                .child(self.review_disclosure(format!("history:{workflow_id}"), &workflow_id, cx)),
        )
    }

    // Existing executor tests use one-workflow convenience methods; production
    // controls always capture an explicit workflow or job id at render time.
    #[cfg(test)]
    pub(super) fn enable_latest_background_automation(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(id) = self
            .automation_state
            .jobs
            .iter()
            .rev()
            .find(|job| job.status == BackgroundJobStatus::Running)
            .map(|job| job.id.clone())
        {
            self.enable_background_automation(&id, cx);
        }
    }

    #[cfg(test)]
    pub(super) fn resume_latest_automation(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(id) = self
            .automation_state
            .jobs
            .iter()
            .rev()
            .find(|job| job.status == BackgroundJobStatus::Paused)
            .map(|job| job.id.clone())
        {
            self.resume_automation(&id, cx);
        }
    }

    #[cfg(test)]
    pub(super) fn rollback_latest_automation(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(id) = self
            .automation_state
            .workflows
            .iter()
            .rev()
            .find(|workflow| workflow.can_rollback())
            .map(|workflow| workflow.id.clone())
        {
            self.rollback_reviewed_automation(&id, cx);
        }
    }
}
