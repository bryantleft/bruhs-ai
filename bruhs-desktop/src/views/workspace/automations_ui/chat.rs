use super::*;

impl Workspace {
    pub(super) fn schedule_chat_workflow_step(
        &mut self,
        job_id: String,
        cx: &mut Context<'_, Self>,
    ) {
        self.automation_pending = true;
        let window = self.workflow_window;
        cx.spawn(async move |workspace, cx| {
            let result = window.update(cx, |_, window, cx| {
                workspace.update(cx, |this, cx| {
                    this.automation_pending = false;
                    this.run_chat_workflow_step(&job_id, window, cx);
                })
            });
            if result.is_err() {
                // Fails only when the workspace itself is gone: nothing to report to.
                let _ = workspace.update(cx, |this, cx| {
                    this.automation_pending = false;
                    this.automation_error =
                        Some("The workspace window is unavailable. No chat was created.".into());
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn run_chat_workflow_step(
        &mut self,
        job_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.require_saved_workflow_state(cx) {
            return;
        }
        // Scheduling is not authorization: payload, lifecycle and grants can
        // change before the window-bound callback gets its turn.
        let Some(WorkflowOperation::CreateChatThread { title }) =
            self.automation_state.pending_operation(job_id).cloned()
        else {
            self.automation_error = Some(
                "The reviewed chat step is no longer authorized. No records were changed.".into(),
            );
            cx.notify();
            return;
        };
        let thread = Self::create_thread_with_title(
            String::new(),
            current_dir_string(),
            self.current_model.clone(),
            self.permission_mode,
            &title,
            window,
            cx,
        );
        self.stamp_chat_run_prefs(&thread.chat, cx);
        let snapshot = match serde_json::to_string(&thread.snapshot(cx)) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                self.automation_error = Some(
                    "The chat checkpoint could not be encoded. No records were changed.".into(),
                );
                cx.notify();
                return;
            }
        };
        if !self.automation_state.complete_step_with_snapshot(
            job_id,
            thread.id.clone(),
            snapshot,
            EpochMs::now().as_millis(),
        ) {
            self.automation_error =
                Some("The job no longer permits this chat step. No records were changed.".into());
            cx.notify();
            return;
        }
        self.threads.push(thread);
        self.automation_error = None;
        self.persist_and_notify(cx);
    }
}
