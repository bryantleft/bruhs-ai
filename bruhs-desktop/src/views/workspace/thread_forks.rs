//! Start an independent desktop conversation at a selected reply.
use super::*;
use crate::views::chat::ForkContext;
use bruhs_core::thread_title::{conversation, ForkSource, TitleTurn};
use bruhs_models::logic::portable::PortablePart;

impl ForkContext {
    fn through(parent: String, view: &ChatView, messages: &[Message]) -> Self {
        let mut history = view
            .fork
            .as_ref()
            .map(|fork| fork.history.clone())
            .unwrap_or_default();
        history.extend(
            messages
                .iter()
                .filter(|message| {
                    message.role != MessageRole::Tool && !message.state.is_streaming()
                })
                .map(|m| {
                    let turn = TitleTurn {
                        role: m.role,
                        text: m.text_for_title(),
                        files: &m.files,
                    };
                    import_ui::portable_message(m, &turn.text_with_attachment_names())
                }),
        );
        Self {
            parent,
            title: view.thread_title.clone(),
            history,
        }
    }

    pub(super) fn title_source(&self, current_parent_title: Option<&str>) -> ForkSource {
        let context = conversation(self.history.iter().map(|m| {
            TitleTurn {
                role: m.role,
                files: &[],
                text: m
                    .parts
                    .iter()
                    .find_map(|p| match p {
                        PortablePart::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .unwrap_or(""),
            }
        }));
        ForkSource {
            title: current_parent_title.unwrap_or(&self.title).to_owned(),
            history: context.text,
        }
    }

    pub(super) fn placeholder(&self) -> String {
        format!("{} fork", self.title)
    }
}

impl Workspace {
    pub(super) fn fork_from_reply(
        &mut self,
        chat: &Entity<ChatView>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(parent) = self.active_thread().filter(|t| t.chat == *chat) else {
            return;
        };
        let view = chat.read(cx);
        if !view.can_fork_response(index) {
            return;
        }
        let messages = &view.messages[..=index];
        let fork = ForkContext::through(parent.id.clone(), view, messages);
        let project = parent.project_name.clone();
        let root = view.settings_project_path().to_owned();
        let model = view.model_id.clone();
        let permissions = view.permission_mode;
        let scoped_permissions = view.project_permission_override;
        let role = view.role.clone();
        let effort = view.effort.clone();
        let model_options=view.model_options.clone();
        let worktree = view.worktree_path.clone();
        let branch = parent.branch.clone();
        let mut child = Self::create_thread_with_title(
            project,
            root,
            model,
            permissions,
            &fork.placeholder(),
            window,
            cx,
        );
        child.branch = branch.clone();
        child.chat.update(cx, |view, cx| {
            view.project_permission_override = scoped_permissions;
            view.role = role;
            view.model_options=model_options;
            view.effort = effort;
            view.fresh_session = true;
            view.fork = Some(std::sync::Arc::new(fork));
            view.tasks_panel_visible = self.tasks_panel_visible;
            view.diff_panel_visible = self.diff_panel_visible;
            view.lock_place(worktree, branch, cx);
            view.focus_composer(window, cx);
        });
        self.stamp_chat_run_prefs(&child.chat, cx);
        let id = child.id.clone();
        self.threads.push(child);
        self.select_thread_by_id(&id, cx);
    }
}
