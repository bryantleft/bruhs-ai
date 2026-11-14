//! Saved data and assertions for the normal native-platform performance binary.
use super::*;
use serde_json::{json, Value};
use sha2::Digest as _;

#[derive(Clone, Copy, Debug)]
pub(crate) enum NativeAction {
    Redraw,
    SearchHit,
    SearchMiss,
    ClearSearch,
    SelectLast,
    HideSidebar,
    ShowSidebar,
}

impl NativeAction {
    pub(crate) const ALL: [Self; 7] = [
        Self::Redraw,
        Self::SearchHit,
        Self::SearchMiss,
        Self::ClearSearch,
        Self::SelectLast,
        Self::HideSidebar,
        Self::ShowSidebar,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Redraw => "redraw",
            Self::SearchHit => "search-hit",
            Self::SearchMiss => "search-miss",
            Self::ClearSearch => "clear-search",
            Self::SelectLast => "previous-thread-wrap",
            Self::HideSidebar => "hide-sidebar",
            Self::ShowSidebar => "show-sidebar",
        }
    }
}

pub(crate) fn saved_bytes(
    threads: usize,
    project_path: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut state = PersistedWorkspaceState {
        current_model: Some(DEFAULT_AGENT_MODEL.into()),
        update_checks: Some(false),
        calendar_anchor: Some("2026-05-12".into()),
        permission_mode: Some(PermissionMode::Default),
        ..Default::default()
    };
    for (index, rule) in state
        .scheduling_preferences
        .availability
        .iter_mut()
        .enumerate()
    {
        rule.id = format!("00000000-0000-4000-8000-{index:012}");
    }
    for index in 0..threads {
        state.threads.push(serde_json::from_value(json!({
            "id":format!("native-thread-{index:06}"),
            "title":if index+1==threads {format!("Native quartz café 👩🏽‍💻 {index}")} else {format!("Native saved thread café {index}")},
            "project_name":"","project_path":project_path,
            "model_id":DEFAULT_AGENT_MODEL,"permission_mode":"default",
            "updated_at_ms":4_102_444_800_000_i64,"pinned":true,"pin_rank":index,
            "messages":[],
        }))?);
    }
    serde_json::to_vec(&state)
}

impl Workspace {
    pub(crate) fn prepare_native_action(
        &mut self,
        action: NativeAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.sidebar_visible = !matches!(action, NativeAction::ShowSidebar);
        self.sidebar_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(id) = self.sidebar_thread_ids(cx).first().cloned() {
            if self.active_thread_id.as_ref() != Some(&id) {
                self.select_thread_by_id(&id, cx);
            }
        }
        if matches!(action, NativeAction::ClearSearch) {
            self.sidebar_search
                .update(cx, |input, cx| input.set_value("quartz", window, cx));
        }
        self.workspace_focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn dispatch_native_action(
        &mut self,
        action: NativeAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match action {
            NativeAction::Redraw => cx.notify(),
            NativeAction::SearchHit | NativeAction::SearchMiss | NativeAction::ClearSearch => {
                let query = match action {
                    NativeAction::SearchHit => "quartz",
                    NativeAction::SearchMiss => "no-native-title-matches-this-query",
                    _ => "",
                };
                self.sidebar_search
                    .update(cx, |input, cx| input.set_value(query, window, cx));
            }
            NativeAction::SelectLast => window.dispatch_action(Box::new(PrevThread), cx),
            NativeAction::HideSidebar | NativeAction::ShowSidebar => {
                window.dispatch_action(Box::new(ToggleSidebar), cx)
            }
        }
    }

    pub(crate) fn verify_native_action(
        &self,
        action: NativeAction,
        initial: &Value,
        cx: &App,
    ) -> Value {
        assert!(self.threads.iter().all(|t| {
            let chat = t.chat.read(cx);
            !chat.is_streaming
                && chat.prompt_queue.is_empty()
                && chat.messages.is_empty()
                && !chat.project_scoped
                && chat.session_id.is_none()
        }));
        assert!(
            !self.diff_panel_visible && !self.files_panel_visible && !self.preview_panel_visible
        );
        let mut expected = initial.clone();
        expected["sidebar_visible"] = json!(!matches!(action, NativeAction::HideSidebar));
        let ids = self.sidebar_thread_ids(cx);
        match action {
            NativeAction::SearchHit => {
                assert_eq!(self.sidebar_query(cx), "quartz");
                assert_eq!(ids.len(), 1);
            }
            NativeAction::SearchMiss => {
                assert_eq!(self.sidebar_query(cx), "no-native-title-matches-this-query");
                assert!(ids.is_empty());
            }
            _ => assert!(self.sidebar_query(cx).is_empty()),
        }
        if matches!(action, NativeAction::SelectLast) {
            expected["active_thread_id"] = json!(ids.last());
        }
        if self.sidebar_visible {
            let viewport = self.sidebar_viewport.borrow();
            assert_eq!(
                viewport.list.item_count(),
                ids.len(),
                "all synthetic matches remain in the list"
            );
            if !ids.is_empty() {
                assert_eq!(viewport.list.logical_scroll_top().item_ix, 0);
                let first = viewport.first_row_bounds.expect("first result is laid out");
                assert!(first.size.width > px(0.0) && first.size.height > px(0.0));
                assert!(viewport.list.viewport_bounds().size.height > px(0.0));
            }
        }
        let actual = serde_json::to_value(self.snapshot(cx)).expect("synthetic snapshot");
        assert_eq!(actual, expected, "native {} state", action.label());
        json!({"snapshot_sha256":format!("{:x}",sha2::Sha256::digest(serde_json::to_vec(&actual).expect("snapshot bytes"))),"sidebar_order_ids":ids.len(),"active_thread_id":self.active_thread_id,"sidebar_visible":self.sidebar_visible})
    }

    pub(crate) fn native_snapshot(&self, cx: &App) -> Value {
        serde_json::to_value(self.snapshot(cx)).expect("synthetic snapshot")
    }

    pub(crate) fn native_save_settled(&self) -> bool {
        matches!(self.save_status, SaveStatus::Saved)
    }

    pub(crate) fn native_verify_startup_auth(&self, expected: usize, cx: &App) -> Value {
        assert_eq!(self.threads.len(), expected);
        for (index, thread) in self.threads.iter().enumerate() {
            assert_eq!(thread.id, format!("native-thread-{index:06}"));
            assert_eq!(
                thread.chat.read(cx).native_fixture_auth_user(),
                Some("fixture@example.invalid")
            );
        }
        let snapshot = self.native_snapshot(cx);
        self.verify_native_action(NativeAction::Redraw, &snapshot, cx);
        snapshot
    }

    pub(crate) fn native_request_fixture_save(&mut self, cx: &mut Context<'_, Self>) {
        self.persist(cx);
    }
}
