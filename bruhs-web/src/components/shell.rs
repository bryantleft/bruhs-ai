//! App shell: sidebar · main view · right panel, plus the global keyboard
//! shortcuts, in three columns.

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::KeyboardEvent;

use crate::components::automations_page::AutomationsPage;
use crate::components::booking_page::BookingPage;
use crate::components::calendar_page::CalendarPage;
use crate::components::chat::ChatPage;
use crate::components::command_bar::{ActiveThreadExport, CommandBar, ThreadExport};
use crate::components::find_bar::{focus_within, FindCommands};
use crate::components::import_thread_dialog::ImportThreadDialog;
use crate::components::inbox_page::InboxPage;
use crate::components::knowledge_page::KnowledgePage;
use crate::components::life_page::LifePage;
use crate::components::meeting_detail::MeetingDetailPage;
use crate::components::meeting_live::MeetingLive;
use crate::components::meetings_page::MeetingsPage;
use crate::components::people_page::PeoplePage;
use crate::components::project_picker_dialog::ProjectPickerDialog;
use crate::components::right_panel::{BranchPullWatch, RightPanel};
use crate::components::search_page::SearchPage;
use crate::components::settings_page::SettingsPage;
use crate::components::sidebar::{sidebar_order, Sidebar};
use crate::components::tasks_page::TasksPage;
use crate::components::terminal_page::TerminalPage;
use crate::components::terminal_panel::open_terminal_session;
use crate::components::today_page::TodayPage;
use crate::state::{
    activate_thread, create_thread, new_thread_or_pick, toggle_pinned, toggle_settled,
    ActiveProjectSignal, ActiveTerminalId, ActiveView, AppView, BranchMenuOpen, BranchPullSignal,
    ChipRetargetGuard, ComposerChips, ComposerDraft, EditorSignal, EffortMenuOpen,
    KeybindingsSignal, KnowledgeSaveRequest, ModelSelectOpen, PanelSurface,
    PendingPermissionsSignal, PreviousWorktreeSignal, PromptStashSignal, RightPanelSignal,
    RunChoiceSignal, SetActiveView, SetCaptureEditId, SetCaptureOpen, SetCommandBarOpen,
    SetModelSelectOpen, SetProjectPickerOpen, SetSidebarOpen, SetTaskCreateOpen, SidebarChrome,
    SidebarOpen, SidebarUndo, StashMenuOpen, StreamBufferSignal, StreamingThreadId, TerminalOpen,
    TerminalSessions, ThreadNotice, UndoRestore, UnreadThreads, WorkspaceMenuOpen,
};
use crate::ui::copy_to_clipboard;

/// `mod` is Command on Apple platforms and Control everywhere else.
fn host_mod_is_command() -> bool {
    let platform = web_sys::window()
        .and_then(|window| window.navigator().platform().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    ["mac", "iphone", "ipad"]
        .iter()
        .any(|needle| platform.contains(needle))
}

#[component]
pub fn Shell() -> impl IntoView {
    let find_thread = RwSignal::new(0u32);
    let find_diff = RwSignal::new(0u32);
    let find_thread_open = RwSignal::new(false);
    let find_diff_open = RwSignal::new(false);
    provide_context(FindCommands {
        thread: find_thread,
        diff: find_diff,
        thread_open: find_thread_open,
        diff_open: find_diff_open,
    });
    let catalog = expect_context::<crate::thread_catalog::ThreadCatalog>();
    let history = expect_context::<crate::thread_history::ThreadHistory>();
    crate::components::search_page::provide_local_search();
    let SidebarOpen(sidebar_open) = expect_context::<SidebarOpen>();
    let ActiveView(active_view) = expect_context::<ActiveView>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    // Same-value sets still notify and would remount the chat page.
    let ensure_chat = move || {
        if active_view.get_untracked() != AppView::Chat {
            set_active_view.set(AppView::Chat);
        }
    };
    let SetCommandBarOpen(set_command_bar_open) = expect_context::<SetCommandBarOpen>();
    let SetSidebarOpen(set_sidebar_open) = expect_context::<SetSidebarOpen>();
    let SetModelSelectOpen(set_model_select_open) = expect_context::<SetModelSelectOpen>();
    let SetTaskCreateOpen(set_task_create_open) = expect_context::<SetTaskCreateOpen>();
    let SetProjectPickerOpen(set_project_picker_open) = expect_context::<SetProjectPickerOpen>();
    let SetCaptureOpen(set_capture_open) = expect_context::<SetCaptureOpen>();
    let SetCaptureEditId(set_capture_edit_id) = expect_context::<SetCaptureEditId>();
    let RightPanelSignal(right_panel) = expect_context::<RightPanelSignal>();
    let BranchPullSignal(branch_pull) = expect_context::<BranchPullSignal>();
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let set_threads = expect_context::<crate::state::SetThreadsSignal>().0;
    let active_thread_id = expect_context::<crate::state::ActiveThreadId>().0;
    let set_active_thread_id = expect_context::<crate::state::SetActiveThreadId>().0;
    let crate::state::WorkspaceMode(workspace_mode) =
        expect_context::<crate::state::WorkspaceMode>();
    let crate::state::SidebarProjectFilter(project_filter) =
        expect_context::<crate::state::SidebarProjectFilter>();
    // Only a different thread/project changes mode; streaming and renaming do not.
    let active_project_identity = Memo::new(move |_| {
        active_thread_id.get().and_then(|id| {
            threads.with(|rows| {
                rows.iter()
                    .find(|thread| thread.id == id)
                    .map(|thread| (id, thread.project_name.clone()))
            })
        })
    });
    let initial_thread_seen = StoredValue::new(false);
    Effect::new(move |_| {
        if let Some((_, project)) = active_project_identity.get() {
            // Hydration restores the saved mode; only explicit subsequent
            // thread navigation should follow the destination's mode.
            if !initial_thread_seen.get_value() {
                initial_thread_seen.set_value(true);
                return;
            }
            workspace_mode.set(bruhs_models::SidebarTab::for_project(&project));
            if !project.is_empty()
                && project_filter
                    .get_untracked()
                    .as_ref()
                    .is_some_and(|filter| filter != &project)
            {
                project_filter.set(None);
            }
        }
    });
    let set_messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let StreamBufferSignal(stream_buffer) = expect_context::<StreamBufferSignal>();
    let UnreadThreads(unread_threads) = expect_context::<UnreadThreads>();

    let StreamingThreadId(streaming_thread_id) = expect_context::<StreamingThreadId>();
    let ModelSelectOpen(model_select_open) = expect_context::<ModelSelectOpen>();
    let BranchMenuOpen(branch_menu_open) = expect_context::<BranchMenuOpen>();
    let WorkspaceMenuOpen(workspace_menu_open) = expect_context::<WorkspaceMenuOpen>();
    let EffortMenuOpen(effort_menu_open) = expect_context::<EffortMenuOpen>();
    let KeybindingsSignal(keybindings) = expect_context::<KeybindingsSignal>();
    let RunChoiceSignal(run_choice) = expect_context::<RunChoiceSignal>();
    let PreviousWorktreeSignal(previous_worktree) = expect_context::<PreviousWorktreeSignal>();
    let thread_notice = expect_context::<ThreadNotice>();
    let crate::state::ImportThreadOpen(import_open) =
        expect_context::<crate::state::ImportThreadOpen>();
    let thread_export = ActiveThreadExport::from_context();
    let TerminalOpen(terminal_open) = expect_context::<TerminalOpen>();
    let ComposerDraft(composer_draft) = expect_context::<ComposerDraft>();
    let ComposerChips(composer_chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(chip_guard) = expect_context::<ChipRetargetGuard>();
    let PromptStashSignal(prompt_stash) = expect_context::<PromptStashSignal>();
    let StashMenuOpen(stash_menu_open) = expect_context::<StashMenuOpen>();
    let EditorSignal(editor_id) = expect_context::<EditorSignal>();
    let TerminalSessions(terminal_sessions) = expect_context::<TerminalSessions>();
    let ActiveTerminalId(active_terminal_id) = expect_context::<ActiveTerminalId>();
    let ActiveProjectSignal(active_project) = expect_context::<ActiveProjectSignal>();
    let KnowledgeSaveRequest(knowledge_save_request) = expect_context::<KnowledgeSaveRequest>();
    let PendingPermissionsSignal(pending_permissions) =
        expect_context::<PendingPermissionsSignal>();
    let sidebar_chrome = expect_context::<SidebarChrome>();
    let sidebar_undo = expect_context::<SidebarUndo>();

    // Sidebar order: pinned, fresh, then settled.
    let thread_order = move || {
        let now = js_sys::Date::now();
        let active = active_thread_id.get_untracked();
        let working = streaming_thread_id.get_untracked();
        unread_threads.with_untracked(|unread| {
            threads.with_untracked(|ts| {
                sidebar_order(
                    ts,
                    &crate::components::sidebar::OrderQuery {
                        now_ms: now,
                        active: active.as_deref(),
                        working: working.as_deref(),
                        needs_you: working.as_deref().filter(|_| {
                            pending_permissions.with_untracked(|pending| !pending.is_empty())
                        }),
                        unread,
                        working_section: sidebar_chrome.working_section.get_untracked(),
                        folds: bruhs_models::logic::sidebar::FoldOpen {
                            working: sidebar_chrome.working_open.get_untracked(),
                            snoozed: sidebar_chrome.snoozed_open.get_untracked(),
                            settled: sidebar_chrome.settled_open.get_untracked(),
                        },
                    },
                )
                .into_iter()
                .filter(|thread| {
                    workspace_mode
                        .get_untracked()
                        .matches_project(&thread.project_name)
                })
                .filter(|thread| {
                    workspace_mode.get_untracked() == bruhs_models::SidebarTab::Chat
                        || project_filter.with_untracked(|filter| {
                            filter
                                .as_deref()
                                .filter(|name| !name.is_empty())
                                .is_none_or(|name| thread.project_name == name)
                        })
                })
                .collect::<Vec<_>>()
            })
        })
    };
    let open_thread = move |id: String| {
        ensure_chat();
        activate_thread(
            history,
            active_thread_id,
            id,
            set_active_thread_id,
            set_messages,
            stream_buffer,
            unread_threads,
        );
    };
    // ⌘⇧[ ] / ⌘⌥↑↓: previous / next thread in sidebar order.
    let step_thread = move |delta: i32| {
        let order = thread_order();
        if order.is_empty() {
            return;
        }
        let current = active_thread_id.get_untracked();
        let idx = current
            .as_ref()
            .and_then(|id| order.iter().position(|t| &t.id == id));
        let len = order.len();
        let next = match idx {
            // `(i + delta).rem_euclid(len)` in unsigned arithmetic.
            Some(i) => {
                let step = usize::try_from(delta.unsigned_abs()).unwrap_or(0) % len;
                if delta >= 0 {
                    (i + step) % len
                } else {
                    (i + len - step) % len
                }
            }
            None => 0,
        };
        open_thread(order[next].id.clone());
    };
    // ⌘1–9: the Nth thread in sidebar order.
    let jump_thread = move |n: usize| {
        if let Some(t) = thread_order().get(n) {
            open_thread(t.id.clone());
        }
    };
    // Two-second confirmation under the transcript.
    let flash = move |msg: &'static str| thread_notice.flash(msg);
    let pin_active = move || {
        if let Some(id) = active_thread_id.get_untracked() {
            let previous =
                threads.with_untracked(|rows| rows.iter().find(|row| row.id == id).cloned());
            match toggle_pinned(catalog, set_threads, &id) {
                Some(true) => flash("Thread pinned"),
                Some(false) => {
                    if let Some(previous) = previous {
                        sidebar_undo.push(crate::state::UndoEntry {
                            message: "Thread unpinned".into(),
                            restore: UndoRestore::Thread(Box::new(previous)),
                        });
                    }
                }
                None => {}
            }
        }
    };
    let settle_active = move || {
        if let Some(id) = active_thread_id.get_untracked() {
            let previous =
                threads.with_untracked(|rows| rows.iter().find(|row| row.id == id).cloned());
            match toggle_settled(catalog, set_threads, unread_threads, &id) {
                Some(true) => {
                    if let Some(previous) = previous {
                        sidebar_undo.push(crate::state::UndoEntry {
                            message: "Thread settled".into(),
                            restore: UndoRestore::Thread(Box::new(previous)),
                        });
                    }
                }
                Some(false) => flash("Thread unsettled"),
                None => {}
            }
        }
    };
    let copy_thread_link = move || {
        if let Some(id) = active_thread_id.get_untracked() {
            let origin = web_sys::window()
                .and_then(|w| js_sys::Reflect::get(&w, &"location".into()).ok())
                .and_then(|l| js_sys::Reflect::get(&l, &"origin".into()).ok())
                .and_then(|o| o.as_string())
                .unwrap_or_default();
            copy_to_clipboard(&format!("{origin}/#thread/{id}"));
            flash("Thread link copied");
        }
    };
    let copy_open_pull = move |link: bool| {
        let panel = right_panel.get_untracked();
        let shown = branch_pull.get_untracked();
        let open = panel.open && panel.surface == Some(PanelSurface::Pull);
        match bruhs_models::logic::branch_pull::pull_copy_text(
            open,
            shown.as_ref().map(|pull| pull.number),
            shown.as_ref().map(|pull| pull.url.as_str()),
            link,
        ) {
            Ok(text) => {
                copy_to_clipboard(&text);
                flash(if link {
                    "Pull request link copied"
                } else {
                    "Pull request number copied"
                });
            }
            Err(message) => flash(message),
        }
    };
    let flash_text = move |msg: String| thread_notice.flash(msg);
    // ⌘O: the thread's project in the editor
    // chosen in Settings → General.
    let open_project_in_editor = move || {
        ensure_chat();
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
        let Some(path) = path else {
            flash("No project to open");
            return;
        };
        let editor = editor_id.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::open_in_editor(&path, &editor, None).await {
                Ok(label) => flash_text(format!("Opened in {label}")),
                Err(e) => flash_text(e.to_string()),
            }
        });
    };
    // ⌘S: park the draft; with nothing typed, open
    // the stash menu instead.
    let stash_draft = move || {
        ensure_chat();
        let text = composer_draft.get_untracked();
        if text.trim().is_empty() {
            stash_menu_open.set(true);
            return;
        }
        let chips = composer_chips.get_untracked();
        prompt_stash.update(|s| {
            s.push_with_chips(&text, &chips, js_sys::Date::now());
        });
        chip_guard.set_value(true);
        composer_chips.set(Vec::new());
        composer_draft.set(String::new());
        flash("Prompt stashed");
    };
    // ⌘P: focus the composer and open the @-file picker.
    let mention_file = move || {
        ensure_chat();
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Some(el) = doc.get_element_by_id("chat-input") else {
            return;
        };
        if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            let _ = ta.focus();
            let v = ta.value();
            let text = if v.is_empty() || v.ends_with(char::is_whitespace) {
                "@"
            } else {
                " @"
            };
            // `execCommand` is not in this web-sys feature set; call it by name.
            if let Ok(f) = js_sys::Reflect::get(&doc, &"execCommand".into()) {
                if let Ok(f) = f.dyn_into::<js_sys::Function>() {
                    let _ = f.call3(&doc, &"insertText".into(), &false.into(), &text.into());
                }
            }
        }
    };

    // A new thread lands in the project you are in; the picker is
    // for choosing a different one (sidebar folder-plus, command bar).
    let new_thread = move || {
        ensure_chat();
        if workspace_mode.get_untracked() == bruhs_models::SidebarTab::Chat {
            create_thread(
                catalog,
                set_threads,
                set_active_thread_id,
                set_messages,
                String::new(),
                None,
            );
            return;
        }
        let current = active_thread_id.get_untracked().and_then(|id| {
            threads.with_untracked(|ts| {
                ts.iter()
                    .find(|t| t.id == id && !t.project_name.is_empty())
                    .map(|t| (t.project_name.clone(), t.project_path.clone()))
            })
        });
        match current {
            Some((name, path)) => {
                create_thread(
                    catalog,
                    set_threads,
                    set_active_thread_id,
                    set_messages,
                    name,
                    path,
                );
            }
            None => {
                new_thread_or_pick(
                    catalog,
                    true,
                    set_threads,
                    set_active_thread_id,
                    set_messages,
                    set_project_picker_open,
                );
            }
        }
    };

    // One `hashchange` listener for the page: `#thread/<id>` links (⌘⇧C
    // copies one) open that thread; routed hashes (back/forward) switch the
    // view. Startup handles the initial hash in app.rs.
    let hash_listener = window_event_listener(leptos::ev::hashchange, move |_| {
        let hash = web_sys::window()
            .and_then(|w| w.location().hash().ok())
            .unwrap_or_default();
        if let Some(id) = hash.strip_prefix("#thread/") {
            if threads.with_untracked(|ts| ts.iter().any(|t| t.id == id)) {
                open_thread(id.to_string());
            }
            return;
        }
        // Our own URL update also emits hashchange. Reapplying the current
        // view remounts its page and discards focus/drafts.
        if let Some(view) = crate::app::parse_hash(&hash) {
            if active_view.get_untracked() != view {
                set_active_view.set(view);
            }
        }
    });
    on_cleanup(move || hash_listener.remove());

    // Global keyboard shortcuts, removed with the Shell.
    {
        // True if the active element is an <input>/<textarea>/contenteditable,
        // used to gate non-modifier shortcuts so plain keys don't fire while typing.
        let target_is_input = || -> bool {
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.active_element())
                .is_some_and(|el| {
                    let tag = el.tag_name().to_lowercase();
                    if matches!(tag.as_str(), "input" | "textarea") {
                        return true;
                    }
                    el.dyn_ref::<web_sys::HtmlElement>()
                        .is_some_and(|html_el| html_el.is_content_editable())
                })
        };

        let keydown = window_event_listener(leptos::ev::keydown, move |ev: KeyboardEvent| {
            use bruhs_models::logic::keybindings::{chord_from_event, resolve, KeyContext};
            let chord = chord_from_event(
                &ev.code(),
                &ev.key(),
                ev.meta_key(),
                ev.ctrl_key(),
                ev.alt_key(),
                ev.shift_key(),
            );
            let composer_focus = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.active_element())
                .is_some_and(|element| element.id() == "chat-input");
            let panel = right_panel.get_untracked();
            let ctx = KeyContext {
                terminal_focus: focus_within("[data-terminal-panel]"),
                terminal_open: terminal_open.get_untracked(),
                preview_focus: focus_within("[data-preview-panel]"),
                preview_open: panel.open && panel.surface == Some(PanelSurface::Preview),
                model_picker_open: model_select_open.get_untracked(),
                composer_focus,
                composer_draft: !composer_draft.get_untracked().trim().is_empty(),
                turn_running: streaming_thread_id.get_untracked().is_some(),
                editable_focus: target_is_input(),
                is_web: true,
                ..KeyContext::default()
            };
            let rules = keybindings.get_untracked();
            let Some(command) = resolve(&rules, &chord, &ctx, host_mod_is_command()) else {
                return;
            };
            match command {
                "thread.undo" => {
                    // Without a pending undo the textarea keeps its own undo.
                    if sidebar_undo.current().is_none() {
                        return;
                    }
                    ev.prevent_default();
                    if let Some(entry) = sidebar_undo.take() {
                        match entry.restore {
                            UndoRestore::Draft { text, chips } => {
                                chip_guard.set_value(true);
                                composer_chips.set(chips);
                                composer_draft.set(text);
                            }
                            UndoRestore::Thread(previous) => {
                                let previous = *previous;
                                let catalog = catalog;
                                set_threads.update(|threads| {
                                    if let Some(thread) =
                                        threads.iter_mut().find(|thread| thread.id == previous.id)
                                    {
                                        let current = thread.clone();
                                        *thread = previous.clone();
                                        catalog.persist(Some(current), previous);
                                    }
                                });
                            }
                        }
                    }
                }
                "chat.find" => {
                    if focus_within("[data-diff-root]") {
                        ev.prevent_default();
                        find_diff.update(|n| *n = n.saturating_add(1));
                    } else if active_view.get_untracked() == AppView::Chat {
                        ev.prevent_default();
                        find_thread.update(|n| *n = n.saturating_add(1));
                    }
                }
                "capture.open" => {
                    ev.prevent_default();
                    set_capture_edit_id.set(None);
                    set_capture_open.set(true);
                }
                "view.today" => {
                    ev.prevent_default();
                    set_active_view.set(AppView::Today);
                }
                "view.knowledge" => {
                    ev.prevent_default();
                    set_active_view.set(AppView::Knowledge);
                }
                "view.inbox" => {
                    ev.prevent_default();
                    set_active_view.set(AppView::Inbox);
                }
                "command.palette" => {
                    ev.prevent_default();
                    set_command_bar_open.update(|open| *open = !*open);
                }
                "sidebar.toggle" => {
                    ev.prevent_default();
                    set_sidebar_open.update(|open| *open = !*open);
                }
                "rightPanel.toggle" => {
                    ev.prevent_default();
                    ensure_chat();
                    right_panel.update(|panel| panel.toggle());
                }
                "diff.toggle" => {
                    ev.prevent_default();
                    ensure_chat();
                    right_panel.update(|panel| panel.toggle_surface(PanelSurface::Diff));
                }
                "terminal.toggle" => {
                    ev.prevent_default();
                    if active_view.get_untracked() == AppView::Terminal {
                        return;
                    }
                    ensure_chat();
                    let opening = !terminal_open.get_untracked();
                    if opening && terminal_sessions.with_untracked(|sessions| sessions.is_empty()) {
                        let place = active_thread_id
                            .get_untracked()
                            .and_then(|id| {
                                threads.with_untracked(|rows| {
                                    rows.iter()
                                        .find(|thread| thread.id == id)
                                        .and_then(|thread| {
                                            thread
                                                .project_path
                                                .clone()
                                                .map(|path| (thread.project_name.clone(), path))
                                        })
                                })
                            })
                            .or_else(|| {
                                active_project
                                    .get_untracked()
                                    .map(|project| (project.name, project.path))
                            });
                        let (name, path) = match place {
                            Some((name, path)) => (Some(name), Some(path)),
                            None => (None, None),
                        };
                        open_terminal_session(terminal_sessions, active_terminal_id, path, name);
                    }
                    terminal_open.update(|open| *open = !*open);
                }
                "preview.toggle" => {
                    ev.prevent_default();
                    ensure_chat();
                    right_panel.update(|panel| panel.toggle_surface(PanelSurface::Preview));
                }
                "tasks.toggle" => {
                    ev.prevent_default();
                    ensure_chat();
                    right_panel.update(|panel| panel.toggle_surface(PanelSurface::Tasks));
                }
                "chat.new" => {
                    ev.prevent_default();
                    new_thread();
                }
                "model.picker" => {
                    ev.prevent_default();
                    branch_menu_open.set(false);
                    workspace_menu_open.set(false);
                    effort_menu_open.set(false);
                    set_model_select_open.update(|open| *open = !*open);
                }
                "branch.menu" => {
                    ev.prevent_default();
                    let open = !branch_menu_open.get_untracked();
                    workspace_menu_open.set(false);
                    effort_menu_open.set(false);
                    set_model_select_open.set(false);
                    branch_menu_open.set(open);
                }
                "workspace.menu" => {
                    ev.prevent_default();
                    let open = !workspace_menu_open.get_untracked();
                    branch_menu_open.set(false);
                    effort_menu_open.set(false);
                    set_model_select_open.set(false);
                    workspace_menu_open.set(open);
                }
                "effort.menu" => {
                    ev.prevent_default();
                    let open = !effort_menu_open.get_untracked();
                    branch_menu_open.set(false);
                    workspace_menu_open.set(false);
                    set_model_select_open.set(false);
                    effort_menu_open.set(open);
                }
                "worktree.previous" => {
                    ev.prevent_default();
                    if previous_worktree.get_untracked().is_some() {
                        run_choice
                            .set(bruhs_models::logic::run_place::WorkspaceChoice::PreviousWorktree);
                        thread_notice.flash("Previous worktree");
                    } else {
                        thread_notice.flash("No previous worktree");
                    }
                }
                "thread.previous" => {
                    ev.prevent_default();
                    step_thread(-1);
                }
                "thread.next" => {
                    ev.prevent_default();
                    step_thread(1);
                }
                "mention.file" => {
                    ev.prevent_default();
                    mention_file();
                }
                "search.open" => {
                    ev.prevent_default();
                    set_active_view.set(AppView::Search);
                }
                "rightPanel.close" => {
                    if right_panel.with_untracked(|panel| panel.open) {
                        ev.prevent_default();
                        right_panel.update(|panel| panel.open = false);
                    }
                }
                "thread.copyLink" => {
                    ev.prevent_default();
                    copy_thread_link();
                }
                "pull.copyLink" => {
                    ev.prevent_default();
                    copy_open_pull(true);
                }
                "thread.import" => {
                    ev.prevent_default();
                    import_open.set(true);
                }
                command if ThreadExport::from_command(command).is_some() => {
                    ev.prevent_default();
                    if let Some(export) = ThreadExport::from_command(command) {
                        thread_export.run(export);
                    }
                }
                "pull.copyNumber" => {
                    ev.prevent_default();
                    copy_open_pull(false);
                }
                "navigation.back" => {
                    ev.prevent_default();
                    if let Some(history) =
                        web_sys::window().and_then(|window| window.history().ok())
                    {
                        let _ = history.back();
                    }
                }
                "navigation.forward" => {
                    ev.prevent_default();
                    if let Some(history) =
                        web_sys::window().and_then(|window| window.history().ok())
                    {
                        let _ = history.forward();
                    }
                }
                "thread.settle" => {
                    ev.prevent_default();
                    settle_active();
                }
                "draft.stash" => {
                    ev.prevent_default();
                    if matches!(active_view.get_untracked(), AppView::DocumentDetail { .. }) {
                        knowledge_save_request.request();
                    } else {
                        stash_draft();
                    }
                }
                "project.openInEditor" => {
                    ev.prevent_default();
                    open_project_in_editor();
                }
                "thread.pin" => {
                    ev.prevent_default();
                    pin_active();
                }
                "settings.open" => {
                    ev.prevent_default();
                    set_active_view.set(AppView::Settings);
                }
                "dialog.dismiss" => {
                    if !ctx.terminal_focus && !ctx.preview_focus {
                        if focus_within("[data-diff-root]") && find_diff_open.get_untracked() {
                            ev.prevent_default();
                            find_diff_open.set(false);
                            return;
                        }
                        if find_thread_open.get_untracked() {
                            ev.prevent_default();
                            find_thread_open.set(false);
                            return;
                        }
                        if find_diff_open.get_untracked() {
                            ev.prevent_default();
                            find_diff_open.set(false);
                            return;
                        }
                    }
                    set_command_bar_open.set(false);
                    set_model_select_open.set(false);
                    branch_menu_open.set(false);
                    workspace_menu_open.set(false);
                    effort_menu_open.set(false);
                    set_task_create_open.set(false);
                    set_project_picker_open.set(false);
                    set_capture_open.set(false);
                    set_capture_edit_id.set(None);
                }
                "composer.focus" => {
                    ev.prevent_default();
                    if let Some(doc) = web_sys::window().and_then(|window| window.document()) {
                        if let Some(element) = doc.get_element_by_id("chat-input") {
                            if let Some(html) = element.dyn_ref::<web_sys::HtmlElement>() {
                                let _ = html.focus();
                            }
                        }
                    }
                }
                "task.new" => {
                    if active_view.get_untracked() == AppView::Tasks {
                        ev.prevent_default();
                        set_task_create_open.set(true);
                    }
                }
                other => {
                    if let Some(digit) = other.strip_prefix("thread.jump.") {
                        if let Ok(n) = digit.parse::<usize>() {
                            if (1..=9).contains(&n) {
                                ev.prevent_default();
                                jump_thread(n - 1);
                            }
                        }
                    }
                }
            }
        });
        on_cleanup(move || keydown.remove());
    }

    view! {
        <div class="flex h-full isolate bg-longan-950 text-rambutan-300">
            <a
                href="#app-main"
                class="sr-only focus:not-sr-only focus:absolute focus:left-3 focus:top-3 focus:z-[80] focus:rounded-grape focus:bg-longan-800 focus:px-3 focus:py-2 focus:text-label focus:text-rambutan-100 focus:outline focus:outline-2 focus:outline-offset-2 focus:outline-persimmon-400"
                on:click=move |ev: web_sys::MouseEvent| {
                    // The app routes on the URL hash (`#/chat`). Following
                    // this link would replace that route, so move focus
                    // without changing the address.
                    ev.prevent_default();
                    let Some(doc) = web_sys::window().and_then(|window| window.document()) else {
                        return;
                    };
                    if let Some(main) = doc
                        .get_element_by_id("app-main")
                        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
                    {
                        let _ = main.focus();
                    }
                }
            >
                "Skip to content"
            </a>
            // Left: thread sidebar on every view, so the
            // footer view icons and thread list are always one click away.
            // Below `sm` it is a fixed overlay drawer over a tap-to-dismiss scrim.
            <Show
                // Settings is a full page with its own section nav;
                // the thread sidebar stays out of it.
                when=move || sidebar_open.get() && !matches!(active_view.get(), AppView::Settings | AppView::Booking { .. })
                fallback=|| view! { <></> }
            >
                <div
                    aria-hidden="true"
                    class="fixed inset-0 z-30 bg-longan-950/70 sm:hidden"
                    on:click=move |_| set_sidebar_open.set(false)
                ></div>
                <aside
                    aria-label="Sidebar navigation"
                    class="fixed inset-y-0 left-0 z-40 sm:contents"
                >
                    <Sidebar />
                </aside>
            </Show>

            // Center: the active view.
            <main id="app-main" tabindex="-1" class="flex min-w-0 flex-1 flex-col overflow-hidden md:min-w-[360px] focus:outline-none">
                <Show when=move || !matches!(active_view.get(), AppView::Settings | AppView::Booking { .. })>
                    <super::workspace_modes::WorkspaceModes />
                </Show>
                <super::workflow_status::WorkflowStatus />
                <crate::thread_catalog::ThreadCatalogStatus />
                <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
                {move || match active_view.get() {
                    AppView::Today => view! { <TodayPage /> }.into_any(),
                    AppView::Inbox => view! { <InboxPage /> }.into_any(),
                    AppView::Knowledge => view! { <KnowledgePage /> }.into_any(),
                    AppView::Chat => view! { <ChatPage /> }.into_any(),
                    AppView::Tasks => view! { <TasksPage /> }.into_any(),
                    AppView::Calendar => view! { <CalendarPage /> }.into_any(),
                    AppView::Life => view! { <LifePage /> }.into_any(),
                    AppView::Automations => view! { <AutomationsPage /> }.into_any(),
                    AppView::Meetings => view! { <MeetingsPage /> }.into_any(),
                    AppView::People => view! { <PeoplePage /> }.into_any(),
                    AppView::Settings => view! { <SettingsPage /> }.into_any(),
                    AppView::MeetingDetail { meeting_id } => view! { <MeetingDetailPage meeting_id=meeting_id /> }.into_any(),
                    AppView::MeetingNoteDetail { note_id } => view! { <super::meeting_note_detail::MeetingNoteDetail note_id=note_id /> }.into_any(),
                    AppView::DocumentDetail { document_id } => view! { <KnowledgePage document_id=document_id /> }.into_any(),
                    AppView::ClipDetail { clip_id } => view! { <KnowledgePage clip_id=clip_id /> }.into_any(),
                    AppView::ConversationDetail { conversation_id } => view! { <InboxPage conversation_id=conversation_id /> }.into_any(),
                    AppView::Booking { slug } => view! { <BookingPage slug=slug /> }.into_any(),
                    AppView::Search => view! { <SearchPage /> }.into_any(),
                    AppView::Terminal => view! { <TerminalPage /> }.into_any(),
                }}
                </div>
                // Notifications reserve their own bounded space, so failures
                // cannot cover a page header, focused input, or touch navigation.
                <super::banner::BannerStack />
                <crate::task_saves::TaskSaveNotice />
            </main>

            // Right: Diff / Tasks panel — below `md` it overlays as a drawer.
            <Show
                when=move || right_panel.get().open && active_view.get() == AppView::Chat
                fallback=|| view! { <></> }
            >
                <div
                    aria-hidden="true"
                    class="fixed inset-0 z-30 bg-longan-950/70 md:hidden"
                    on:click=move |_| right_panel.update(|p| p.open = false)
                ></div>
                <div class="fixed inset-y-0 right-0 z-40 w-[min(88vw,24rem)] md:static md:z-auto md:min-w-0 md:w-auto md:shrink">
                    <RightPanel />
                </div>
            </Show>

            <BranchPullWatch />
            <CommandBar />
            <ProjectPickerDialog />
            <ImportThreadDialog />
            <MeetingLive />
        </div>
    }
}
