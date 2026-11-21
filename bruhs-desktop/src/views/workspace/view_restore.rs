//! Reopening the view the app closed on, as the web does from its URL. Each
//! detail view goes through its own opener so its fields load, and a record
//! that is gone falls back to its list.

use super::*;

impl Workspace {
    pub(super) fn reopen_view_after_start(
        &mut self,
        view: AppView,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Somewhere else was opened first, such as a smoke scene or a link.
        if self.active_view != AppView::Chat {
            return;
        }
        // Settings is a dialog on desktop, and Chat is where it starts.
        if matches!(view, AppView::Settings | AppView::Chat) {
            return;
        }
        let thread = self.task_thread_view.take();
        self.open_routed_view(view, window, cx);
        self.task_thread_view = thread.filter(|t| matches!(&self.active_view, AppView::Bruh { agent_id } if agent_id == &t.agent_id));
    }

    /// Open `view` the way its own control does, for a `bruhs://` link or
    /// the view the app closed on.
    pub(super) fn open_routed_view(
        &mut self,
        view: AppView,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match view {
            AppView::DocumentDetail { document_id } => {
                if self
                    .knowledge_documents
                    .iter()
                    .any(|doc| doc.id == document_id)
                {
                    self.select_knowledge_document(&document_id, window, cx);
                } else {
                    self.activate_sidebar_view(AppView::Knowledge, window, cx);
                }
            }
            AppView::ClipDetail { clip_id } => {
                if self.source_clips.iter().any(|clip| clip.id == clip_id) {
                    self.select_source_clip(&clip_id, window, cx);
                } else {
                    self.activate_sidebar_view(AppView::Knowledge, window, cx);
                }
            }
            AppView::ConversationDetail { conversation_id } => {
                if self
                    .inbox_conversations
                    .iter()
                    .any(|conversation| conversation.id == conversation_id)
                {
                    self.select_inbox_conversation(&conversation_id, window, cx);
                } else {
                    self.activate_sidebar_view(AppView::Inbox, window, cx);
                }
            }
            AppView::MeetingNoteDetail { note_id } => {
                self.open_local_meeting_note(note_id, window, cx);
            }
            AppView::MeetingDetail { meeting_id } => self.open_meeting_detail(meeting_id, cx),
            AppView::Booking { slug } => self.open_booking_page(slug, window, cx),
            AppView::Settings => self.open_settings(&OpenSettings, window, cx),
            view => self.activate_sidebar_view(view, window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DiskVersion, PersistedWorkspaceState, RestoredRecords, Workspace};
    use crate::{app_state::DesktopState, theme};
    use bruhs_models::{AppView, InboxConversation, InboxMessage, InboxSource};
    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn restarted(
        cx: &mut TestAppContext,
        state: PersistedWorkspaceState,
    ) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let entity = Rc::new(RefCell::new(None));
        let captured = entity.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| {
                Workspace::from_loaded_state(
                    (Some(state), None, DiskVersion::Missing),
                    window,
                    cx,
                    RestoredRecords::take_from_persisted,
                    Workspace::restore_threads,
                )
            });
            *captured.borrow_mut() = Some(workspace.clone());
            bruhs_ui::Root::new(workspace, window, cx)
        });
        let workspace = entity.borrow().clone().expect("workspace");
        cx.run_until_parked();
        (workspace, cx)
    }

    #[gpui::test]
    fn a_restart_reopens_the_last_conversation_with_its_draft(cx: &mut TestAppContext) {
        let mut conversation = InboxConversation::new(InboxSource::Email, "Plans", Vec::new(), 1);
        conversation.receive(InboxMessage::incoming("Ada", "Lunch?", 1));
        conversation.upsert_draft("Friday works", 2);
        let view = AppView::ConversationDetail {
            conversation_id: conversation.id.clone(),
        };
        let state: PersistedWorkspaceState = serde_json::from_value(serde_json::json!({
            "active_view": view.route(),
            "inbox_conversations": [conversation],
        }))
        .expect("state");
        let (workspace, cx) = restarted(cx, state);
        workspace.read_with(cx, |workspace, cx| {
            assert_eq!(workspace.active_view, view);
            assert_eq!(
                workspace.inbox_reply_input.read(cx).value().as_ref(),
                "Friday works"
            );
        });
    }

    #[gpui::test]
    fn a_restart_falls_back_to_the_list_when_the_record_is_gone(cx: &mut TestAppContext) {
        let state: PersistedWorkspaceState = serde_json::from_value(serde_json::json!({
            "active_view": "/knowledge/documents/gone",
        }))
        .expect("state");
        let (workspace, cx) = restarted(cx, state);
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.active_view, AppView::Knowledge);
        });
    }

    #[gpui::test]
    fn a_restart_reopens_workbench_and_a_name_saved_by_an_earlier_build(cx: &mut TestAppContext) {
        let state: PersistedWorkspaceState =
            serde_json::from_value(serde_json::json!({ "active_view": "/workbench" }))
                .expect("state");
        let (workspace, cx) = restarted(cx, state);
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.active_view, AppView::Workbench);
        });
        let legacy: PersistedWorkspaceState =
            serde_json::from_value(serde_json::json!({ "active_view": "calendar" }))
                .expect("state");
        assert_eq!(legacy.active_view, Some(AppView::Calendar));
    }
}
