use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::KeyboardEvent;

use crate::components::search_page::{preview, scroll_result_into_view, LocalSearch};
use crate::components::sidebar::project_names;
use crate::components::terminal_panel::open_terminal_session;
use crate::state::{
    new_thread_or_pick, toggle_pinned, toggle_settled, ActiveProjectSignal, ActiveTerminalId,
    ActiveView, AppView, BranchPullSignal, ChipRetargetGuard, CommandBarOpen, ComposerChips,
    ComposerDraft, EditorSignal, InboxCreateRequest, KnowledgeCreateClipRequest, PanelSurface,
    PlanDayRequest, PromptStashSignal, RightPanelSignal, SchedulingOpenRequest, SetActiveView,
    SetCaptureEditId, SetCaptureOpen, SetCommandBarOpen, SetKnowledgeDocumentsSignal,
    SetModelSelectOpen, SetProjectPickerOpen, SetSidebarOpen, SetTaskCreateOpen, StashMenuOpen,
    TerminalOpen, TerminalSessions, ThreadNotice, UnreadThreads,
};
use crate::ui::{copy_to_clipboard, Dialog};

/// Every command the palette can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActionId {
    NavToday,
    NavChat,
    NavInbox,
    NavKnowledge,
    NavTasks,
    NavCalendar,
    NavLife,
    NavAutomations,
    NavMeetings,
    NavPeople,
    NavSearch,
    NavTerminal,
    OpenProject,
    NewThread,
    ImportThread,
    PinThread,
    SettleThread,
    CopyThreadLink,
    CopyResumeCommand,
    CopyThreadMarkdown,
    ContinueInT3,
    CopyPullLink,
    CopyPullNumber,
    NewTask,
    PlanDay,
    SmartScheduling,
    NewDocument,
    ImportConversation,
    NewWebClip,
    Capture,
    NewMeetingNote,
    ToggleSidebar,
    ToggleDiff,
    TogglePanel,
    ToggleTasks,
    ToggleTerminal,
    NewTerminal,
    TogglePreview,
    ToggleFiles,
    StashPrompt,
    OpenInEditor,
    ModelSelect,
    Settings,
}

/// One row in the command palette: stable id, group, label, hint (small text on
/// the right). The group is used to render uppercase headings.
#[derive(PartialEq, Eq)]
struct Action {
    id: ActionId,
    group: &'static str,
    label: &'static str,
    hint: &'static str,
}

const ACTIONS: &[Action] = &[
    Action {
        id: ActionId::NavToday,
        group: "Navigate",
        label: "Today",
        hint: "plan & focus",
    },
    Action {
        id: ActionId::NavChat,
        group: "Navigate",
        label: "Chat",
        hint: "conversations",
    },
    Action {
        id: ActionId::NavInbox,
        group: "Navigate",
        label: "Inbox",
        hint: "messages & follow-ups",
    },
    Action {
        id: ActionId::NavKnowledge,
        group: "Navigate",
        label: "Knowledge",
        hint: "documents & sources",
    },
    Action {
        id: ActionId::NavTasks,
        group: "Navigate",
        label: "Tasks",
        hint: "projects & issues",
    },
    Action {
        id: ActionId::NavCalendar,
        group: "Navigate",
        label: "Calendar",
        hint: "schedule & events",
    },
    Action {
        id: ActionId::NavLife,
        group: "Navigate",
        label: "Life",
        hint: "finance, health & habits",
    },
    Action {
        id: ActionId::NavAutomations,
        group: "Navigate",
        label: "Agent workflows",
        hint: "permissions, jobs & audit",
    },
    Action {
        id: ActionId::NavMeetings,
        group: "Navigate",
        label: "Meetings",
        hint: "recorded conversations",
    },
    Action {
        id: ActionId::NavPeople,
        group: "Navigate",
        label: "People",
        hint: "meeting profiles",
    },
    Action {
        id: ActionId::NavSearch,
        group: "Navigate",
        label: "Search",
        hint: "everything, instantly",
    },
    Action {
        id: ActionId::NavTerminal,
        group: "Navigate",
        label: "Terminal",
        hint: "shells, no thread needed",
    },
    Action {
        id: ActionId::OpenProject,
        group: "Agent",
        label: "Open project...",
        hint: "coding agent",
    },
    Action {
        id: ActionId::NewThread,
        group: "Agent",
        label: "New thread",
        hint: "⌘⇧O",
    },
    Action {
        id: ActionId::ImportThread,
        group: "Agent",
        label: "Import thread...",
        hint: "Claude Code, Codex, T3, Cursor",
    },
    Action {
        id: ActionId::PinThread,
        group: "Actions",
        label: "Pin / unpin thread",
        hint: "⌘⇧P",
    },
    Action {
        id: ActionId::SettleThread,
        group: "Actions",
        label: "Settle / unsettle thread",
        hint: "⌘⇧S",
    },
    Action {
        id: ActionId::CopyThreadLink,
        group: "Actions",
        label: "Copy thread link",
        hint: "⌘⇧C",
    },
    Action {
        id: ActionId::CopyResumeCommand,
        group: "Actions",
        label: "Copy resume command",
        hint: "Claude Code / Codex",
    },
    Action {
        id: ActionId::CopyThreadMarkdown,
        group: "Actions",
        label: "Copy thread as Markdown",
        hint: "for Cursor and others",
    },
    Action {
        id: ActionId::ContinueInT3,
        group: "Actions",
        label: "Continue in T3 Code",
        hint: "new T3 thread",
    },
    Action {
        id: ActionId::CopyPullLink,
        group: "Actions",
        label: "Copy pull request link",
        hint: "open pull request",
    },
    Action {
        id: ActionId::CopyPullNumber,
        group: "Actions",
        label: "Copy pull request number",
        hint: "# and the number",
    },
    Action {
        id: ActionId::NewTask,
        group: "Tasks",
        label: "New task",
        hint: "create",
    },
    Action {
        id: ActionId::PlanDay,
        group: "Planning",
        label: "Plan my day",
        hint: "review focus blocks",
    },
    Action {
        id: ActionId::SmartScheduling,
        group: "Planning",
        label: "Smart scheduling",
        hint: "availability, links & conflicts",
    },
    Action {
        id: ActionId::NewDocument,
        group: "Knowledge",
        label: "New document",
        hint: "Markdown note",
    },
    Action {
        id: ActionId::ImportConversation,
        group: "Inbox",
        label: "Import conversation",
        hint: "email, Slack, Discord, iMessage, X",
    },
    Action {
        id: ActionId::NewWebClip,
        group: "Knowledge",
        label: "Save web clip",
        hint: "source & excerpt",
    },
    Action {
        id: ActionId::Capture,
        group: "Actions",
        label: "Capture anything",
        hint: "⌘⇧Space",
    },
    Action {
        id: ActionId::NewMeetingNote,
        group: "Meetings",
        label: "New meeting note",
        hint: "local text",
    },
    // ⌘B is context-aware: toggles the chat sidebar in Chat view, the
    // project sidebar in Tasks view. One shortcut, one mental model.
    Action {
        id: ActionId::ToggleSidebar,
        group: "View",
        label: "Toggle sidebar",
        hint: "⌘B",
    },
    Action {
        id: ActionId::ToggleDiff,
        group: "View",
        label: "Toggle diff panel",
        hint: "⌘D",
    },
    Action {
        id: ActionId::TogglePanel,
        group: "View",
        label: "Toggle right panel",
        hint: "⌘⌥B",
    },
    Action {
        id: ActionId::ToggleTasks,
        group: "View",
        label: "Toggle tasks panel",
        hint: "⌘⇧T",
    },
    Action {
        id: ActionId::ToggleTerminal,
        group: "View",
        label: "Toggle terminal",
        hint: "⌘J",
    },
    Action {
        id: ActionId::NewTerminal,
        group: "View",
        label: "New terminal",
        hint: "project or home",
    },
    Action {
        id: ActionId::TogglePreview,
        group: "View",
        label: "Toggle preview",
        hint: "⌘⇧J",
    },
    Action {
        id: ActionId::ToggleFiles,
        group: "View",
        label: "Toggle files panel",
        hint: "project tree",
    },
    Action {
        id: ActionId::StashPrompt,
        group: "Actions",
        label: "Stash prompt",
        hint: "⌘S",
    },
    Action {
        id: ActionId::OpenInEditor,
        group: "Actions",
        label: "Open project in editor",
        hint: "⌘⌥O",
    },
    Action {
        id: ActionId::ModelSelect,
        group: "Actions",
        label: "Model",
        hint: "switch provider",
    },
    Action {
        id: ActionId::Settings,
        group: "Actions",
        label: "Settings",
        hint: "preferences",
    },
];

fn action_matches_query(action: &Action, query: &str) -> bool {
    query.is_empty()
        || action.label.to_lowercase().contains(query)
        || action.group.to_lowercase().contains(query)
        || action.hint.to_lowercase().contains(query)
}

fn filtered_actions_for_query(query: &str) -> Vec<&'static Action> {
    let query = query.to_lowercase();
    ACTIONS
        .iter()
        .filter(|action| action_matches_query(action, &query))
        .collect()
}

fn grouped_actions_for_query(query: &str) -> Vec<&'static Action> {
    let mut actions = filtered_actions_for_query(query);
    let mut groups = Vec::new();
    for action in &actions {
        if !groups.contains(&action.group) {
            groups.push(action.group);
        }
    }
    // Keyboard indices must follow the same first-seen groups as the renderer.
    actions.sort_by_key(|action| groups.iter().position(|group| *group == action.group));
    actions
}

fn clamp_selected_index(index: usize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        index.min(count - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_action_filter_should_match_desktop_raw_query_behavior() {
        let matching_labels = filtered_actions_for_query(" ")
            .into_iter()
            .map(|action| action.label)
            .collect::<Vec<_>>();

        // "Settings" no longer matches a raw " " query: its hint changed from
        // "model & auth" to the single word "preferences" when the model
        // picker became its own "Model" action.
        assert_eq!(
            matching_labels,
            vec![
                "Today",
                "Inbox",
                "Knowledge",
                "Tasks",
                "Calendar",
                "Life",
                "Agent workflows",
                "Meetings",
                "People",
                "Search",
                "Terminal",
                "Open project...",
                "New thread",
                "Import thread...",
                "Pin / unpin thread",
                "Settle / unsettle thread",
                "Copy thread link",
                "Copy resume command",
                "Copy thread as Markdown",
                "Continue in T3 Code",
                "Copy pull request link",
                "Copy pull request number",
                "New task",
                "Plan my day",
                "Smart scheduling",
                "New document",
                "Import conversation",
                "Save web clip",
                "Capture anything",
                "New meeting note",
                "Toggle sidebar",
                "Toggle diff panel",
                "Toggle right panel",
                "Toggle tasks panel",
                "Toggle terminal",
                "New terminal",
                "Toggle preview",
                "Toggle files panel",
                "Stash prompt",
                "Open project in editor",
                "Model",
            ]
        );
    }

    #[test]
    fn command_bar_selection_should_wrap_for_arrow_keys() {
        assert_eq!(move_selected_index(0, 7, "ArrowDown"), Some(1));
        assert_eq!(move_selected_index(6, 7, "ArrowDown"), Some(0));
        assert_eq!(move_selected_index(0, 7, "ArrowUp"), Some(6));
        assert_eq!(move_selected_index(9, 7, "ArrowUp"), Some(5));
        assert_eq!(move_selected_index(0, 0, "ArrowDown"), Some(0));
        assert_eq!(move_selected_index(0, 7, "Enter"), None);
        assert_eq!(move_selected_index(0, 0, "Backspace"), None);
        assert_eq!(move_selected_index(0, 0, "Escape"), None);
    }

    #[test]
    fn command_bar_selection_should_clamp_for_enter() {
        assert_eq!(clamp_selected_index(0, 0), 0);
        assert_eq!(clamp_selected_index(12, 7), 6);
        assert_eq!(
            filtered_actions_for_query("model")[0].id,
            ActionId::ModelSelect
        );
    }

    #[test]
    fn command_selection_order_should_follow_visible_groups() {
        let actions = grouped_actions_for_query("");
        let mut seen = std::collections::HashSet::new();
        let mut previous = None;
        for action in &actions {
            if previous != Some(action.group) {
                assert!(seen.insert(action.group), "group must be contiguous");
                previous = Some(action.group);
            }
        }
        assert_eq!(actions.len(), ACTIONS.len());
    }
}

/// The three ways to take the active thread elsewhere, from the palette
/// or from a keybinding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadExport {
    ResumeCommand,
    Markdown,
    ContinueInT3,
}

impl ThreadExport {
    /// The export a keybinding command runs.
    pub fn from_command(command: &str) -> Option<Self> {
        match command {
            "thread.copyResumeCommand" => Some(Self::ResumeCommand),
            "thread.copyMarkdown" => Some(Self::Markdown),
            "thread.continueInT3" => Some(Self::ContinueInT3),
            _ => None,
        }
    }
}

/// What exporting the active thread reads, taken from context when a
/// component mounts (an event handler has no context to read).
#[derive(Clone, Copy)]
pub struct ActiveThreadExport {
    threads: ReadSignal<Vec<crate::state::Thread>>,
    active_thread_id: ReadSignal<Option<String>>,
    messages: ReadSignal<Vec<crate::state::AgentMessage>>,
    active_project: ReadSignal<Option<crate::state::ActiveProject>>,
    notice: ThreadNotice,
}

impl ActiveThreadExport {
    pub fn from_context() -> Self {
        Self {
            threads: expect_context::<crate::state::ThreadsSignal>().0,
            active_thread_id: expect_context::<crate::state::ActiveThreadId>().0,
            messages: expect_context::<crate::state::MessagesSignal>().0,
            active_project: expect_context::<ActiveProjectSignal>().0,
            notice: expect_context::<ThreadNotice>(),
        }
    }

    /// Run `export` on the active thread, if there is one.
    pub fn run(self, export: ThreadExport) {
        let thread = self.active_thread_id.get_untracked().and_then(|id| {
            self.threads
                .with_untracked(|ts| ts.iter().find(|t| t.id == id).cloned())
        });
        let Some(thread) = thread else {
            return;
        };
        // Where its turns run (chat.rs: the thread's project, else the
        // active one).
        let run_dir = thread
            .project_path
            .clone()
            .or_else(|| self.active_project.get_untracked().map(|p| p.path));
        export_thread(
            export,
            &thread,
            run_dir,
            || {
                self.messages.with_untracked(|ms| {
                    crate::components::import_thread_dialog::portable_messages(ms)
                })
            },
            self.notice,
        );
    }
}

/// `export` for `thread`. `run_dir` is where the thread's turns run (its
/// project, else the active one; none for a home-directory chat).
fn export_thread(
    export: ThreadExport,
    thread: &crate::state::Thread,
    run_dir: Option<String>,
    messages: impl FnOnce() -> Vec<bruhs_models::logic::portable::PortableMessage>,
    notice: ThreadNotice,
) {
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::logic::portable::{
        resume_command, transcript_markdown, ExportT3Request, IMPORT_CONTEXT_MAX_CHARS,
    };
    // Resume commands and missing-project notices need only thread metadata.
    // Project the transcript only when an export will actually consume it.
    let markdown = || {
        let messages = messages();
        transcript_markdown(&thread.title, "bruhs", &messages, IMPORT_CONTEXT_MAX_CHARS)
    };
    match export {
        ThreadExport::ResumeCommand => match thread.session_id.as_deref() {
            Some(session) => {
                let provider =
                    AgentProvider::for_model(thread.model.as_deref().unwrap_or_default());
                copy_to_clipboard(&resume_command(provider, session, run_dir.as_deref()));
                notice.flash("Resume command copied");
            }
            None => notice.flash("This thread has no session to resume yet."),
        },
        ThreadExport::Markdown => {
            copy_to_clipboard(&markdown());
            notice.flash("Thread copied as Markdown");
        }
        ThreadExport::ContinueInT3 => {
            let Some(project_path) = run_dir else {
                notice.flash("Continue in T3 Code needs a thread with a project.");
                return;
            };
            let request = ExportT3Request {
                project_path,
                title: thread.title.clone(),
                model: thread
                    .model
                    .clone()
                    .unwrap_or_else(|| bruhs_models::agent::DEFAULT_AGENT_MODEL.to_string()),
                markdown: markdown(),
            };
            leptos::task::spawn_local(async move {
                match crate::api::export_to_t3(&request).await {
                    Ok(_) => notice.flash("Started a T3 thread"),
                    Err(error) => notice.flash(format!("T3: {error}")),
                }
            });
        }
    }
}

pub(super) fn move_selected_index(index: usize, count: usize, key: &str) -> Option<usize> {
    if key != "ArrowDown" && key != "ArrowUp" {
        return None;
    }
    if count == 0 {
        return Some(0);
    }

    let index = clamp_selected_index(index, count);
    match key {
        "ArrowDown" => Some((index + 1) % count),
        "ArrowUp" => Some(if index == 0 { count - 1 } else { index - 1 }),
        _ => None,
    }
}

#[component]
pub fn CommandBar() -> impl IntoView {
    let local_search = expect_context::<LocalSearch>();
    let CommandBarOpen(command_bar_open) = expect_context::<CommandBarOpen>();
    let SetCommandBarOpen(set_command_bar_open) = expect_context::<SetCommandBarOpen>();
    let SetSidebarOpen(set_sidebar_open) = expect_context::<SetSidebarOpen>();
    let RightPanelSignal(right_panel) = expect_context::<RightPanelSignal>();
    let BranchPullSignal(branch_pull) = expect_context::<BranchPullSignal>();
    let TerminalOpen(terminal_open) = expect_context::<TerminalOpen>();
    let ComposerDraft(composer_draft) = expect_context::<ComposerDraft>();
    let ComposerChips(composer_chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(chip_guard) = expect_context::<ChipRetargetGuard>();
    let PromptStashSignal(prompt_stash) = expect_context::<PromptStashSignal>();
    let StashMenuOpen(stash_menu_open) = expect_context::<StashMenuOpen>();
    let EditorSignal(editor_id) = expect_context::<EditorSignal>();
    let ActiveProjectSignal(active_project) = expect_context::<ActiveProjectSignal>();
    let thread_notice = expect_context::<ThreadNotice>();
    let TerminalSessions(terminal_sessions) = expect_context::<TerminalSessions>();
    let ActiveTerminalId(active_terminal_id) = expect_context::<ActiveTerminalId>();
    let ActiveView(active_view) = expect_context::<ActiveView>();
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let set_threads = expect_context::<crate::state::SetThreadsSignal>().0;
    let set_active_thread_id = expect_context::<crate::state::SetActiveThreadId>().0;
    let active_thread_id = expect_context::<crate::state::ActiveThreadId>().0;
    let UnreadThreads(unread_threads) = expect_context::<UnreadThreads>();
    let set_messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let crate::state::ImportThreadOpen(import_open) =
        expect_context::<crate::state::ImportThreadOpen>();
    let thread_export = ActiveThreadExport::from_context();
    let SetTaskCreateOpen(set_task_create_open) = expect_context::<SetTaskCreateOpen>();
    let SetModelSelectOpen(set_model_select_open) = expect_context::<SetModelSelectOpen>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let SetProjectPickerOpen(set_project_picker_open) = expect_context::<SetProjectPickerOpen>();
    let SetCaptureOpen(set_capture_open) = expect_context::<SetCaptureOpen>();
    let note_executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let SetCaptureEditId(set_capture_edit_id) = expect_context::<SetCaptureEditId>();
    let PlanDayRequest(plan_day_request) = expect_context::<PlanDayRequest>();
    let SchedulingOpenRequest(scheduling_open_request) = expect_context::<SchedulingOpenRequest>();
    let KnowledgeCreateClipRequest(create_clip_request) =
        expect_context::<KnowledgeCreateClipRequest>();
    let InboxCreateRequest(inbox_create_request) = expect_context::<InboxCreateRequest>();
    let SetKnowledgeDocumentsSignal(set_knowledge_documents) =
        expect_context::<SetKnowledgeDocumentsSignal>();

    let (query, set_query) = signal(String::new());
    // What the keyboard / pointer last picked; `selected_index` is that,
    // clamped to the current results.
    let (picked_index, set_selected_index) = signal(0usize);
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    // Reset state and refocus the input every time the dialog opens, so typing
    // from an arbitrary prior state can't produce stale filter results.
    Effect::new(move |_| {
        if command_bar_open.get() {
            local_search.load_history.run(());
            set_query.set(String::new());
            set_selected_index.set(0);
            if let Some(el) = input_ref.get() {
                let _ = el.focus();
            }
        }
    });

    // Filter actions case-insensitively against label + group + hint.
    let filtered = Memo::new(move |_| {
        let query = query.get();
        grouped_actions_for_query(&query)
    });
    let records = Memo::new(move |_| {
        if !command_bar_open.get() || query.with(|value| value.trim().is_empty()) {
            return Vec::new();
        }
        query.with(|query| local_search.index.with(|index| index.search(query, 20)))
    });
    let result_count = move || filtered.with(Vec::len) + records.with(Vec::len);
    let selected_index =
        Memo::new(move |_| clamp_selected_index(picked_index.get(), result_count()));

    let close = move || {
        set_command_bar_open.set(false);
        set_query.set(String::new());
        set_selected_index.set(0);
    };

    let on_close_cb = Callback::new(move |_: ()| close());

    let catalog = expect_context::<crate::thread_catalog::ThreadCatalog>();
    let execute_action = move |id: ActionId| {
        match id {
            ActionId::NavToday => set_active_view.set(AppView::Today),
            ActionId::NavInbox => set_active_view.set(AppView::Inbox),
            ActionId::NavKnowledge => set_active_view.set(AppView::Knowledge),
            ActionId::NavChat => set_active_view.set(AppView::Chat),
            ActionId::NavTasks => set_active_view.set(AppView::Tasks),
            ActionId::NavCalendar => set_active_view.set(AppView::Calendar),
            ActionId::NavLife => set_active_view.set(AppView::Life),
            ActionId::NavAutomations => set_active_view.set(AppView::Automations),
            ActionId::NavMeetings => set_active_view.set(AppView::Meetings),
            ActionId::NavPeople => set_active_view.set(AppView::People),
            ActionId::NavSearch => set_active_view.set(AppView::Search),
            ActionId::NavTerminal => set_active_view.set(AppView::Terminal),
            ActionId::NewTerminal => {
                let ctx = active_thread_id
                    .get_untracked()
                    .and_then(|id| {
                        threads.with_untracked(|ts| {
                            ts.iter().find(|t| t.id == id).and_then(|t| {
                                t.project_path.clone().map(|p| (t.project_name.clone(), p))
                            })
                        })
                    })
                    .or_else(|| active_project.get_untracked().map(|p| (p.name, p.path)));
                let (name, path) = match ctx {
                    Some((n, p)) => (Some(n), Some(p)),
                    None => (None, None),
                };
                open_terminal_session(terminal_sessions, active_terminal_id, path, name);
                if active_view.get_untracked() == AppView::Chat {
                    terminal_open.set(true);
                } else {
                    set_active_view.set(AppView::Terminal);
                }
            }
            ActionId::OpenProject => set_project_picker_open.set(true),
            ActionId::ImportThread => import_open.set(true),
            ActionId::CopyResumeCommand => thread_export.run(ThreadExport::ResumeCommand),
            ActionId::CopyThreadMarkdown => thread_export.run(ThreadExport::Markdown),
            ActionId::ContinueInT3 => thread_export.run(ThreadExport::ContinueInT3),
            ActionId::PinThread => {
                if let Some(id) = active_thread_id.get_untracked() {
                    toggle_pinned(catalog, set_threads, &id);
                }
            }
            ActionId::SettleThread => {
                if let Some(id) = active_thread_id.get_untracked() {
                    toggle_settled(catalog, set_threads, unread_threads, &id);
                }
            }
            ActionId::CopyThreadLink => {
                if let Some(id) = active_thread_id.get_untracked() {
                    let origin = web_sys::window()
                        .and_then(|w| js_sys::Reflect::get(&w, &"location".into()).ok())
                        .and_then(|l| js_sys::Reflect::get(&l, &"origin".into()).ok())
                        .and_then(|o| o.as_string())
                        .unwrap_or_default();
                    copy_to_clipboard(&format!("{origin}/#thread/{id}"));
                }
            }
            ActionId::CopyPullLink | ActionId::CopyPullNumber => {
                let panel = right_panel.get_untracked();
                let shown = branch_pull.get_untracked();
                let open = panel.open && panel.surface == Some(PanelSurface::Pull);
                let link = id == ActionId::CopyPullLink;
                match bruhs_models::logic::branch_pull::pull_copy_text(
                    open,
                    shown.as_ref().map(|pull| pull.number),
                    shown.as_ref().map(|pull| pull.url.as_str()),
                    link,
                ) {
                    Ok(text) => {
                        copy_to_clipboard(&text);
                        thread_notice.flash(if link {
                            "Pull request link copied"
                        } else {
                            "Pull request number copied"
                        });
                    }
                    Err(message) => thread_notice.flash(message),
                }
            }
            ActionId::NewThread => {
                set_active_view.set(AppView::Chat);
                let has_projects = threads.with_untracked(|ts| !project_names(ts).is_empty());
                new_thread_or_pick(
                    catalog,
                    has_projects,
                    set_threads,
                    set_active_thread_id,
                    set_messages,
                    set_project_picker_open,
                );
            }
            ActionId::ToggleDiff => {
                set_active_view.set(AppView::Chat);
                right_panel.update(|p| p.toggle_surface(PanelSurface::Diff));
            }
            ActionId::TogglePanel => {
                set_active_view.set(AppView::Chat);
                right_panel.update(|p| p.toggle());
            }
            ActionId::ToggleTasks => {
                set_active_view.set(AppView::Chat);
                right_panel.update(|p| p.toggle_surface(PanelSurface::Tasks));
            }
            ActionId::ToggleTerminal => {
                set_active_view.set(AppView::Chat);
                terminal_open.update(|v| *v = !*v);
            }
            ActionId::TogglePreview => {
                set_active_view.set(AppView::Chat);
                right_panel.update(|p| p.toggle_surface(PanelSurface::Preview));
            }
            ActionId::ToggleFiles => {
                set_active_view.set(AppView::Chat);
                right_panel.update(|p| p.toggle_surface(PanelSurface::Files));
            }
            ActionId::OpenInEditor => {
                set_active_view.set(AppView::Chat);
                let path = active_thread_id
                    .get_untracked()
                    .and_then(|id| {
                        threads.with_untracked(|ts| {
                            ts.iter()
                                .find(|t| t.id == id)
                                .and_then(|t| t.project_path.clone())
                        })
                    })
                    .or_else(|| active_project.get_untracked().map(|p| p.path));
                if let Some(path) = path {
                    let editor = editor_id.get_untracked();
                    wasm_bindgen_futures::spawn_local(async move {
                        let msg = match crate::api::open_in_editor(&path, &editor, None).await {
                            Ok(label) => format!("Opened in {label}"),
                            Err(e) => e.to_string(),
                        };
                        thread_notice.flash(msg);
                    });
                } else {
                    thread_notice.set(Some("No project to open".to_string()));
                }
            }
            ActionId::StashPrompt => {
                set_active_view.set(AppView::Chat);
                let text = composer_draft.get_untracked();
                if text.trim().is_empty() {
                    stash_menu_open.set(true);
                } else {
                    let chips = composer_chips.get_untracked();
                    prompt_stash.update(|s| {
                        s.push_with_chips(&text, &chips, js_sys::Date::now());
                    });
                    chip_guard.set_value(true);
                    composer_chips.set(Vec::new());
                    composer_draft.set(String::new());
                }
            }
            ActionId::NewTask => {
                // Switch to Tasks view first so the dialog mounts behind
                // the right surface, then open the create dialog.
                set_active_view.set(AppView::Tasks);
                set_task_create_open.set(true);
            }
            ActionId::PlanDay => {
                if active_view.get_untracked() != AppView::Today {
                    set_active_view.set(AppView::Today);
                }
                plan_day_request.request();
            }
            ActionId::SmartScheduling => {
                set_active_view.set(AppView::Calendar);
                scheduling_open_request.request();
            }
            ActionId::NewDocument => {
                let document =
                    bruhs_models::KnowledgeDocument::new("Untitled", "", crate::clock::now_ms());
                let id = document.id.clone();
                set_knowledge_documents.update(|items| items.insert(0, document.clone()));
                crate::storage::persist_knowledge_document(None, document);
                set_active_view.set(AppView::DocumentDetail { document_id: id });
            }
            ActionId::NewWebClip => {
                if active_view.get_untracked() != AppView::Knowledge {
                    set_active_view.set(AppView::Knowledge);
                }
                create_clip_request.request();
            }
            ActionId::ImportConversation => {
                if active_view.get_untracked() != AppView::Inbox {
                    set_active_view.set(AppView::Inbox);
                }
                inbox_create_request.request();
            }
            ActionId::Capture => {
                set_capture_edit_id.set(None);
                set_capture_open.set(true);
            }
            ActionId::NewMeetingNote => {
                note_executor.note_editor.begin(None);
                set_active_view.set(AppView::Meetings);
            }
            // Single sidebar_open signal gates both the chat sidebar (Chat
            // view) and the project sidebar (Tasks view). ⌘B does the right
            // thing depending on which view is active.
            ActionId::ToggleSidebar => set_sidebar_open.update(|v| *v = !*v),
            ActionId::ModelSelect => set_model_select_open.set(true),
            ActionId::Settings => set_active_view.set(AppView::Settings),
        }
        close();
    };

    let on_keydown = move |ev: KeyboardEvent| {
        if ev.is_composing() {
            return;
        }
        let actions = filtered.get();
        let key = ev.key();

        if let Some(next) =
            move_selected_index(selected_index.get_untracked(), result_count(), &key)
        {
            ev.prevent_default();
            set_selected_index.set(next);
            scroll_result_into_view(&format!("command-result-{next}"));
            return;
        }

        if key == "Enter" {
            let selected = selected_index.get_untracked();
            if let Some(action) = actions.get(selected) {
                ev.prevent_default();
                execute_action(action.id);
            } else if let Some(hit) =
                records.with(|hits| hits.get(selected - actions.len()).cloned())
            {
                ev.prevent_default();
                close();
                local_search.open.run(hit);
            }
        }
    };

    view! {
        <Dialog
            open=Signal::derive(move || command_bar_open.get())
            on_close=on_close_cb
            label="Command bar"
        >
            // Announces the filtered result count to screen readers as the
            // query changes (WCAG 4.1.3); visually hidden.
            <span class="sr-only" aria-live="polite" aria-atomic="true">
                {move || {
                    let n = result_count();
                    format!("{n} result{}", if n == 1 { "" } else { "s" })
                }}
            </span>
            // Search input — controlled, single full-width row with an
            // underlined divider (matches TS Command.Input pattern).
            <input
                node_ref=input_ref
                type="text"
                role="combobox"
                aria-label="Search commands and records"
                aria-controls="command-results"
                aria-expanded="true"
                aria-autocomplete="list"
                aria-activedescendant=move || (result_count() > 0).then(|| format!("command-result-{}", selected_index.get()))
                prop:value=move || query.get()
                placeholder="What do you need?"
                class="w-full border-b border-longan-800 bg-transparent px-5 py-4 text-body-sm text-rambutan-200 placeholder:text-rambutan-300 focus:outline-none"
                on:input=move |ev: web_sys::Event| {
                    if let Some(t) = ev.target() {
                        if let Ok(el) = t.dyn_into::<web_sys::HtmlInputElement>() {
                            set_query.set(el.value());
                            set_selected_index.set(0);
                        }
                    }
                }
                on:keydown=on_keydown
            />

            // Grouped action list. Each group renders its uppercase heading
            // followed by its filtered items (TS Command.Group pattern).
            // Keep the selected row visible within the viewport-bound scroller.
            <div id="command-results" role="listbox" aria-label="Commands and records" class="max-h-[min(36rem,calc(100dvh-16rem))] overflow-y-auto px-2 py-2">
                {move || {
                    let actions = filtered.get();
                    if actions.is_empty() && records.with(Vec::is_empty) {
                        return view! {
                            <div class="px-4 py-8 text-center text-label-sm text-rambutan-300">
                                "nothing found"
                            </div>
                        }.into_any();
                    }
                    // Group preserving first-seen order
                    let mut groups: Vec<(&'static str, Vec<(usize, &Action)>)> = Vec::new();
                    for (idx, a) in actions.into_iter().enumerate() {
                        if let Some((_, items)) = groups.iter_mut().find(|(g, _)| *g == a.group) {
                            items.push((idx, a));
                        } else {
                            groups.push((a.group, vec![(idx, a)]));
                        }
                    }
                    let rendered: Vec<_> = groups
                        .into_iter()
                        .map(|(group, items)| {
                            let item_views: Vec<_> = items
                                .into_iter()
                                .map(|(idx, a)| {
                                    let id = a.id;
                                    let label = a.label;
                                    let hint = a.hint;
                                    let action_class = move || {
                                        let base = "flex w-full items-center justify-between rounded-grape px-3 py-2.5 text-label-sm transition-colors cursor-pointer";
                                        if selected_index.get() == idx {
                                            format!("{base} bg-longan-700/70 text-rambutan-100")
                                        } else {
                                            format!("{base} text-rambutan-200 hover:bg-longan-700/70 hover:text-rambutan-100")
                                        }
                                    };
                                    view! {
                                        <button
                                            type="button"
                                            role="option"
                                            id=format!("command-result-{idx}")
                                            on:click=move |_| execute_action(id)
                                            on:mousemove=move |event: web_sys::MouseEvent| {
                                                if event.movement_x() != 0 || event.movement_y() != 0 {
                                                    set_selected_index.set(idx);
                                                }
                                            }
                                            aria-selected=move || (selected_index.get() == idx).to_string()
                                            class=action_class
                                        >
                                            <span>{label}</span>
                                            <span class="text-label-sm text-rambutan-300">{hint}</span>
                                        </button>
                                    }
                                })
                                .collect();
                            view! {
                                <div class="mb-1">
                                    <div class="px-3 pb-1 text-label-sm uppercase tracking-widest text-rambutan-400">
                                        {group}
                                    </div>
                                    {item_views}
                                </div>
                            }
                        })
                        .collect();
                    view! { <div>{rendered}</div> }.into_any()
                }}
                {move || {
                    let offset = filtered.with(Vec::len);
                    records.get().into_iter().enumerate().map(|(position, hit)| {
                        let idx = offset + position;
                        let label = format!("{}: {}", hit.document.kind.label(), hit.document.title);
                        let aria_label = label.clone();
                        let subtitle = preview(&hit.document.subtitle);
                        view! {
                            <button
                                type="button"
                                role="option"
                                id=format!("command-result-{idx}")
                                aria-label=aria_label
                                aria-selected=move || (selected_index.get() == idx).to_string()
                                class="block w-full min-w-0 rounded-grape px-3 py-2.5 text-left text-label-sm text-rambutan-200 hover:bg-longan-700/70 cursor-pointer"
                                class:bg-longan-700=move || selected_index.get() == idx
                                on:mousemove=move |event: web_sys::MouseEvent| {
                                    if event.movement_x() != 0 || event.movement_y() != 0 {
                                        set_selected_index.set(idx);
                                    }
                                }
                                on:click=move |_| { close(); local_search.open.run(hit.clone()); }
                            >
                                <span class="block truncate">{label}</span>
                                <span class="block truncate text-rambutan-400">{subtitle}</span>
                            </button>
                        }
                    }).collect_view()
                }}
            </div>

            // Footer hint row (TS pattern).
            <div class="flex items-center justify-between border-t border-longan-800/50 px-5 py-2.5 text-label-sm text-rambutan-300">
                <span>
                    <kbd class="text-rambutan-200">"↑↓"</kbd>
                    " navigate · "
                    <kbd class="text-rambutan-200">"↵"</kbd>
                    " select"
                </span>
                <span>
                    <kbd class="text-rambutan-200">"esc"</kbd>
                    " close"
                </span>
            </div>
        </Dialog>
    }
}
