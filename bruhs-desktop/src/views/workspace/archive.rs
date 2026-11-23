//! Archived threads leave the sidebar and the agent thread catalog. Restore
//! reuses their chat in this session or loads the saved history after restart.

use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ArchivedRecord {
    thread: PersistedThread,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived_at: Option<EpochMs>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    draft: String,
}

pub(super) enum ArchivedThread {
    Live(Box<DesktopThread>, EpochMs),
    Stored(Box<ArchivedRecord>),
}

impl ArchivedThread {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Live(thread, _) => &thread.id,
            Self::Stored(record) => &record.thread.id,
        }
    }

    pub(super) fn project_root<'a>(&'a self, cx: &'a App) -> Option<&'a str> {
        match self {
            Self::Live(t, _) => {
                let chat = t.chat.read(cx);
                chat.project_scoped.then(|| chat.settings_project_path())
            }
            Self::Stored(r) if !r.thread.project_name.is_empty() => r
                .thread
                .project_settings_root
                .as_deref()
                .or(r.thread.project_path.as_deref()),
            Self::Stored(_) => None,
        }
    }

    pub(super) fn checkout_paths<'a>(&'a self, cx: &'a App) -> impl Iterator<Item = &'a str> {
        let paths = match self {
            Self::Live(t, _) => {
                let chat = t.chat.read(cx);
                [
                    Some(chat.project_path.as_str()),
                    chat.worktree_path.as_deref(),
                ]
            }
            Self::Stored(r) => [
                r.thread.project_path.as_deref(),
                r.thread.worktree_path.as_deref(),
            ],
        };
        paths.into_iter().flatten()
    }

    pub(super) fn snapshot(&self, cx: &App) -> ArchivedRecord {
        match self {
            Self::Stored(record) => (**record).clone(),
            Self::Live(thread, at) => ArchivedRecord {
                archived_at: Some(*at),
                thread: thread.snapshot(cx),
                draft: thread
                    .chat
                    .read(cx)
                    .input_state
                    .read(cx)
                    .value()
                    .to_string(),
            },
        }
    }
}

impl Workspace {
    pub(super) fn request_thread_archive(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            return;
        };
        let chat = thread.chat.read(cx);
        if chat.is_streaming {
            return;
        }
        if !self.confirm_thread_archive {
            self.archive_thread(id, cx);
            return;
        }
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Archive “{}”?", chat.thread_title),
            Some("Restore this thread from Archived threads in Settings."),
            &[
                PromptButton::Cancel("Cancel".into()),
                PromptButton::Ok("Archive".into()),
            ],
            cx,
        );
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| this.archive_thread(&id, cx));
            }
        })
        .detach();
    }

    fn archive_thread(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let Some(index) = self
            .threads
            .iter()
            .position(|thread| thread.id == id && !thread.chat.read(cx).is_streaming)
        else {
            return;
        };
        self.end_thread_waits_on(id, cx);
        self.title_requests.remove(id);
        self.cancel_quota_recovery(id);
        self.unread_thread_ids.remove(id);
        self.active_thread_id =
            threads_ui::thread_selection_after_delete(self.active_thread_id.take(), id);
        self.archived_threads.insert(
            0,
            ArchivedThread::Live(Box::new(self.threads.remove(index)), EpochMs::now()),
        );
        self.sync_thread_catalog(cx);
        self.persist_and_notify(cx);
    }

    fn restore_archived_thread(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(index) = self
            .archived_threads
            .iter()
            .position(|thread| thread.id() == id)
        else {
            return;
        };
        let thread = match self.archived_threads.remove(index) {
            ArchivedThread::Live(thread, _) => *thread,
            ArchivedThread::Stored(record) => {
                let ArchivedRecord {
                    thread: persisted,
                    draft,
                    ..
                } = *record;
                let thread = Self::create_thread_from_persisted(
                    persisted,
                    &self.current_model,
                    self.permission_mode,
                    window,
                    cx,
                );
                if !draft.is_empty() {
                    thread.chat.update(cx, |chat, cx| {
                        chat.input_state.update(cx, |input, cx| {
                            let end = draft.len();
                            input.set_value(draft, window, cx);
                            input.set_selected_range(end..end, cx);
                        });
                    });
                }
                thread
            }
        };
        self.stamp_chat_run_prefs(&thread.chat, cx);
        self.threads.insert(0, thread);
        self.sync_thread_catalog(cx);
        self.persist_and_notify(cx);
    }

    fn request_archived_delete(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(index) = self.archived_threads.iter().position(|t| t.id() == id) else {
            return;
        };
        if !self.confirm_thread_delete {
            self.delete_archive(id, cx);
            return;
        }
        let title = match &self.archived_threads[index] {
            ArchivedThread::Live(t, _) => &t.chat.read(cx).thread_title,
            ArchivedThread::Stored(r) => &r.thread.title,
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Delete “{title}”?"),
            Some("This deletes the archived thread, its draft and its chat history."),
            &[
                PromptButton::Cancel("Cancel".into()),
                PromptButton::Ok("Delete".into()),
            ],
            cx,
        );
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| this.delete_archive(&id, cx));
            }
        })
        .detach();
    }

    fn delete_archive(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let Some(index) = self.archived_threads.iter().position(|t| t.id() == id) else {
            return;
        };
        let checkout = match self.archived_threads.remove(index) {
            ArchivedThread::Live(t, _) => self.deleted_checkout(&t, cx),
            ArchivedThread::Stored(r) => {
                let project = r
                    .thread
                    .project_settings_root
                    .as_deref()
                    .or(r.thread.project_path.as_deref())
                    .unwrap_or_default();
                let (enabled, keep) = self.cleanup_rules(project);
                if enabled {
                    worktree_storage::CleanupTarget::new(
                        &r.thread.id,
                        r.thread.worktree_path,
                        r.thread.branch,
                        keep,
                    )
                } else {
                    None
                }
            }
        };
        if let Some(checkout) = checkout {
            self.cleanup_deleted_checkout(checkout, cx);
        }
        self.details.hidden_inline.remove(id);
        self.thread_origins
            .retain(|child, origin| child != id && origin.parent() != id);
        self.known_thread_pulls.remove(id);
        self.pull_check_times.remove(id);
        self.persist_and_notify(cx);
    }
    pub(super) fn render_archived_threads(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        if self.archived_threads.is_empty() {
            return vec![div()
                .p_4()
                .text_color(theme::rambutan_400())
                .child("No archived threads")
                .into_any_element()];
        }
        self.archived_threads
            .iter()
            .map(|thread| {
                let (title, project, archived_at, updated_at) = match thread {
                    ArchivedThread::Live(thread, at) => (
                        &thread.chat.read(cx).thread_title,
                        &thread.project_name,
                        Some(*at),
                        thread.updated_at_ms,
                    ),
                    ArchivedThread::Stored(record) => (
                        &record.thread.title,
                        &record.thread.project_name,
                        record.archived_at,
                        record.thread.updated_at_ms,
                    ),
                };
                let id = thread.id().to_owned();
                let label = format!("Restore {title}");
                let selector = format!("settings-restore-thread-{id}");
                let deleting = id.clone();
                let delete = web_button(
                    SharedString::from(format!("archive-delete-{id}")),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(format!("Delete archived thread {title}"))
                .child("Delete");
                let delete = on_press_view(delete, cx, move |this, window, cx| {
                    this.request_archived_delete(&deleting, window, cx)
                });
                let restore = web_button(
                    SharedString::from(format!("settings-restore-thread-{id}")),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(move || selector)
                .aria_label(label)
                .child("Restore");
                let restore = on_press_view(restore, cx, move |this, window, cx| {
                    this.restore_archived_thread(&id, window, cx)
                });
                let age = relative_age(archived_at.unwrap_or(updated_at), EpochMs::now());
                let timing = if archived_at.is_some() {
                    format!("Archived {age}")
                } else {
                    format!("Last active {age}")
                };
                div()
                    .p_3()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().child(title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::rambutan_400())
                                    .truncate()
                                    .child(if project.is_empty() {
                                        timing
                                    } else {
                                        format!("{project} · {timing}")
                                    }),
                            ),
                    )
                    .child(restore)
                    .child(delete)
                    .into_any_element()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #[gpui::test]
    fn deleting_one_archive_honors_confirmation_and_preserves_restored_threads(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx, first, second) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |w, window, cx| {
            w.archive_thread(&first, cx);
            w.archive_thread(&second, cx);
            w.confirm_thread_delete = true;
            w.request_archived_delete(&first, window, cx);
        });
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        workspace.update_in(cx, |w, window, cx| {
            assert_eq!(w.snapshot(cx).archived_threads.len(), 2);
            w.request_archived_delete(&first, window, cx);
            w.restore_archived_thread(&first, window, cx);
        });
        cx.simulate_prompt_answer("Delete");
        cx.run_until_parked();
        workspace.update_in(cx, |w, window, cx| {
            assert!(w.threads.iter().any(|t| t.id == first));
            w.request_archived_delete(&second, window, cx);
        });
        cx.simulate_prompt_answer("Delete");
        cx.run_until_parked();
        workspace.update_in(cx, |w, _, cx| {
            assert!(w.snapshot(cx).archived_threads.is_empty())
        });
    }
}
