use super::*;
use crate::views::whiteboard::catalog::{WhiteboardEvent, WhiteboardLibrary};
use bruhs_models::whiteboard::BoardLink;
impl Workspace {
    pub(super) fn whiteboard_request_project(
        &self,
        chat: &Entity<ChatView>,
        cx: &App,
    ) -> Option<String> {
        let thread = self
            .threads
            .iter()
            .find(|thread| thread.chat.entity_id() == chat.entity_id())?;
        let chat = chat.read(cx);
        if !chat.project_scoped && thread.project_name.is_empty() {
            return None;
        }
        chat.worktree_path
            .clone()
            .filter(|path| !path.is_empty())
            .or_else(|| (!chat.project_path.is_empty()).then(|| chat.project_path.clone()))
    }
    pub(super) fn whiteboard_project(&self, cx: &App) -> String {
        self.active_thread()
            .and_then(|thread| self.whiteboard_request_project(&thread.chat, cx))
            .or_else(|| self.recent_projects.first().map(|p| p.path.clone()))
            .unwrap_or_default()
    }

    pub(super) fn whiteboard_library(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<WhiteboardLibrary> {
        if let Some(panel) = self.whiteboard_panels.get(&project) {
            return panel.clone();
        }
        let panel = cx.new(|cx| WhiteboardLibrary::new(project.clone(), None, window, cx));
        self._subscriptions.push(
            cx.subscribe_in(&panel, window, |this, _, event, window, cx| match event {
                WhiteboardEvent::OpenLink(link, project) => match link {
                    BoardLink::Thread { id } => {
                        this.select_thread_by_id(id, cx);
                        this.activate_sidebar_view(AppView::Chat, window, cx);
                    }
                    BoardLink::File {
                        path,
                        start_line,
                        end_line,
                    } => this.open_attached_file(
                        path.clone(),
                        format!("{path}:{start_line}–{end_line}"),
                        Some(project.clone()),
                        true,
                        cx,
                    ),
                    BoardLink::Task { id } => {
                        this.selected_work_item_id = Some(id.clone());
                        this.activate_sidebar_view(AppView::Tasks, window, cx);
                    }
                    BoardLink::Meeting { id } => this.activate_sidebar_view(
                        AppView::MeetingDetail {
                            meeting_id: id.clone(),
                        },
                        window,
                        cx,
                    ),
                    BoardLink::Person { id } => {
                        this.expanded_person_id = Some(id.clone());
                        this.activate_sidebar_view(AppView::People, window, cx);
                    }
                    BoardLink::Document { id } => this.activate_sidebar_view(
                        AppView::DocumentDetail {
                            document_id: id.clone(),
                        },
                        window,
                        cx,
                    ),
                    BoardLink::PullRequest { url } => cx.open_url(url),
                },
            }),
        );
        self.whiteboard_panels.insert(project, panel.clone());
        panel
    }
    pub(super) fn show_whiteboard(
        &mut self,
        create: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let project = self.whiteboard_project(cx);
        let panel = self.whiteboard_library(project, window, cx);
        if create {
            panel.update(cx, |panel, cx| panel.new_board(window, cx));
        }
        self.whiteboard_panel = Some(panel);
        self.activate_sidebar_view(AppView::Whiteboards { board_id: None }, window, cx);
    }
}
