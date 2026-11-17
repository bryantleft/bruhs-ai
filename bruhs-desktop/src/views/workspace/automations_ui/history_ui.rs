use super::*;
use bruhs_models::logic::workflow_history::{
    self, NEXT_WORKFLOW, NEXT_WORKFLOW_LABEL, NO_WORKFLOW_YET, PREVIOUS_WORKFLOW,
    PREVIOUS_WORKFLOW_LABEL,
};
use gpui_component::zoom::px;

impl Workspace {
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

    pub(super) fn render_automation_history(&self, cx: &Context<'_, Self>) -> AnyElement {
        let Some(page) =
            workflow_history::page(&self.automation_state, self.reviewed_workflow_id.as_deref())
        else {
            return div().child(NO_WORKFLOW_YET).into_any_element();
        };
        let previous = page.previous.map(str::to_owned);
        let next = page.next.map(str::to_owned);
        let label = page.label();
        div()
            .id("automation-history")
            .debug_selector(|| "desktop-workflow-review".into())
            .role(Role::Group)
            .aria_label(label.clone())
            .min_w_0()
            .mt_2()
            .text_size(px(11.0))
            .text_color(theme::rambutan_300())
            .child(label)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .mt_2()
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
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(id) = &previous {
                                this.review_automation(id, cx);
                            }
                        })),
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
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(id) = &next {
                                this.review_automation(id, cx);
                            }
                        })),
                    ),
            )
            .into_any_element()
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
