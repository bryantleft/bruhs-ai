// bruhs-desktop — native desktop client using GPUI and the bruhs-ui toolkit.

mod app_state;
mod assets;
mod clock_format;
mod helpers;
#[cfg(unix)]
mod instance_lock;
mod launcher;
mod logging;
mod motion;
#[cfg(feature = "native-performance")]
mod native_performance;
mod paths;
mod restore_relaunch;
mod server_client;
#[cfg(target_os = "macos")]
mod shell_path;
#[cfg(debug_assertions)]
mod smoke;
#[cfg(debug_assertions)]
mod synthetic_dock;
mod theme;
mod views;
mod voice_capture;
mod webview_platform;
#[cfg(feature = "whiteboard-benchmark")]
mod whiteboard_performance;
#[cfg(target_os = "macos")]
mod window_chrome;
mod zoom;

use bruhs_ui::zoom::px;
use bruhs_ui::{Root, TitleBar};
use gpui::*;
use std::cell::RefCell;
use std::env;
use std::rc::Rc;
use views::chat::FindInThread;
use views::diff_panel::FindInDiff;
use views::workspace::{
    CloseOverlays, ClosePanel, CopyThreadLink, FocusChatInput, FocusCommandBar, JumpThread,
    MentionFile, NavigateBack, NavigateForward, NewThread, NextThread, OpenCapture, OpenInboxView,
    OpenKnowledgeView, OpenSettings, OpenTaskCreateShortcut, OpenTodayView, PinThread, PrevThread,
    ResetZoom, SaveKnowledgeDocument, SearchThreads, SettleThread, ToggleBranchMenu, ToggleDiff,
    ToggleEffortMenu, ToggleModelPicker, TogglePanel, TogglePreview, ToggleSidebar, ToggleTasks,
    ToggleTerminal, ToggleWorkspaceMenu, UsePreviousWorktree, ZoomIn, ZoomOut,
};

const DESKTOP_GLOBAL_SHORTCUTS: &[&str] = &[
    "cmd-shift-t",
    "cmd-b",
    "cmd-k",
    "/",
    "c",
    "escape",
    "cmd-shift-p",
    "cmd-shift-s",
    "cmd-shift-c",
    "cmd-d",
    "cmd-alt-b",
    "cmd-w",
    "cmd-shift-m",
    "cmd-n",
    "cmd-shift-o",
    "cmd-shift-n",
    "cmd-shift-[",
    "cmd-shift-]",
    "cmd-/",
    "cmd-p",
    "cmd-j",
    "cmd-shift-j",
    "cmd-shift-f",
    "cmd-shift-space",
    "cmd-shift-y",
    "cmd-shift-k",
    "cmd-shift-i",
    "cmd-shift-g",
    "cmd-shift-x",
    "cmd-shift-e",
    "cmd-shift-l",
    "cmd-1",
    "cmd-2",
    "cmd-3",
    "cmd-4",
    "cmd-5",
    "cmd-6",
    "cmd-7",
    "cmd-8",
    "cmd-9",
];
/// Offset of `cmd-1` in `DESKTOP_GLOBAL_SHORTCUTS`.
const JUMP_THREAD_FIRST_INDEX: usize = 31;
/// Window title, matching bruhs-web's `<title>`.
const DESKTOP_WINDOW_TITLE: &str = "bruhs.ai";

actions!(bruhs, [Quit]);

fn main() {
    if let Some(code) = launcher::cli() {
        std::process::exit(code);
    }
    if bruhs_core::workbench_tools::is_mcp_helper() {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(bruhs_core::workbench_tools::run_mcp_helper()));
        if let Err(error) = result {
            eprintln!("Workbench helper failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if bruhs_core::api_client_tools::is_mcp_helper() {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(bruhs_core::api_client_tools::run_mcp_helper()));
        if let Err(error) = result {
            eprintln!("API client helper failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if bruhs_core::database_tools::is_mcp_helper() {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(bruhs_core::database_tools::run_mcp_helper()));
        if let Err(error) = result {
            eprintln!("Database helper failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if bruhs_core::website_reader::is_mcp_helper() {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(bruhs_core::website_reader::run_mcp_helper()));
        if let Err(error) = result {
            eprintln!("Local website reader failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if bruhs_core::artifact_tools::is_mcp_helper() {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(bruhs_core::artifact_tools::run_mcp_helper()));
        if let Err(error) = result {
            eprintln!("Local artifact helper failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if bruhs_core::reactive_tools::is_mcp_helper() {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(bruhs_core::reactive_tools::run_mcp_helper()));
        if let Err(error) = result {
            eprintln!("Reactive automation helper failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(feature = "whiteboard-benchmark")]
    if whiteboard_performance::is_executable() {
        if let Err(error) = whiteboard_performance::run() {
            eprintln!("WHITEBOARD_PERFORMANCE_ERROR {error}");
            std::process::exit(2);
        }
        return;
    }
    #[cfg(feature = "native-performance")]
    if native_performance::is_executable() {
        if let Err(error) = native_performance::run() {
            eprintln!("NATIVE_PERFORMANCE_ERROR {error}");
            std::process::exit(2);
        }
        return;
    }
    if bruhs_core::secrets::is_mcp_helper() {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(bruhs_core::secrets::run_mcp_helper()));
        if result.is_err() {
            std::process::exit(1);
        }
        return;
    }
    if bruhs_core::thread_link::is_mcp_helper() {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(bruhs_core::thread_link::run_mcp_helper()));
        if result.is_err() {
            std::process::exit(1);
        }
        return;
    }
    webview_platform::configure_startup();
    // Bruhs.app opened from Finder has launchd's bare PATH; adopt the login
    // shell's so providers, git and the helpers resolve. Before any thread.
    #[cfg(target_os = "macos")]
    if bruhs_core::updates::release::current_app_bundle().is_some() {
        shell_path::adopt_login_path();
    }
    let log_file = logging::init();
    if let Some(path) = &log_file {
        tracing::info!("bruhs-desktop starting; logging to {}", path.display());
    }
    #[cfg(unix)]
    let _instance = instance_lock::apply_restore_at_start();
    if env::args().any(|arg| arg == "--synthetic-meeting-dock") {
        #[cfg(debug_assertions)]
        synthetic_dock::run();
        #[cfg(not(debug_assertions))]
        tracing::error!("Synthetic meeting dock acceptance requires a debug build.");
        return;
    }
    // `bruhs://` links from the OS. One that launches the app waits in the
    // channel until the workspace is open.
    let (link_tx, link_rx) = async_channel::unbounded::<String>();
    gpui_platform::application()
        .with_assets(assets::DesktopAssets)
        .with_app_links(link_tx)
        .run(move |cx: &mut App| {
            cx.on_action(|_: &Quit, cx| cx.quit());
            // The workspace handles the configurable keyboard quit gesture.
            // ---- bruhs-ui setup: the theme and the root, text field and
            // text selection key bindings ----
            bruhs_ui::init(cx);
            // Native selectors route editing to an embedded WebKit first
            // responder; gpui-component handles the same actions in its inputs.
            #[cfg(target_os = "macos")]
            let window_menu = window_chrome::window_menu(cx);
            cx.set_menus([
                Menu::new("bruhs.ai").items([
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::separator(),
                    MenuItem::action("Quit bruhs.ai", Quit),
                ]),
                Menu::new("Edit").items([
                    MenuItem::os_action("Undo", bruhs_ui::input::Undo, OsAction::Undo),
                    MenuItem::os_action("Redo", bruhs_ui::input::Redo, OsAction::Redo),
                    MenuItem::separator(),
                    MenuItem::os_action("Cut", bruhs_ui::input::Cut, OsAction::Cut),
                    MenuItem::os_action("Copy", bruhs_ui::input::Copy, OsAction::Copy),
                    MenuItem::os_action("Paste", bruhs_ui::input::Paste, OsAction::Paste),
                    MenuItem::separator(),
                    MenuItem::os_action(
                        "Select All",
                        bruhs_ui::input::SelectAll,
                        OsAction::SelectAll,
                    ),
                ]),
                // The web app's browser zoom, as a browser's View menu offers it.
                Menu::new("View").items([
                    MenuItem::action("Zoom In", ZoomIn),
                    MenuItem::action("Zoom Out", ZoomOut),
                    MenuItem::action("Actual Size", ResetZoom),
                ]),
                #[cfg(target_os = "macos")]
                window_menu,
            ]);
            motion::init(cx);

            // The bruhs type stack is all Monaspace. The web loads it as a
            // webfont; the desktop embeds the same variable faces so both
            // surfaces render identical glyphs without a system install.
            if let Err(err) = cx.text_system().add_fonts(assets::embedded_fonts()) {
                tracing::error!("could not load embedded fonts: {err}");
            }

            // Switch to dark theme + apply bruhs design tokens on top.
            bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
            theme::apply_bruhs_theme(cx);

            // ---- App state ----
            let state = match app_state::DesktopState::new() {
                Ok(state) => state,
                Err(err) => {
                    tracing::error!("failed to start desktop async runtime: {err}");
                    cx.quit();
                    return;
                }
            };
            cx.set_global(state);
            // Bruhs.app: start the bundled server and capture daemon.
            helpers::start(cx);

            // Register global keyboard shortcuts
            let mut bindings = vec![
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[0], ToggleTasks, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[1], ToggleSidebar, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[2], FocusCommandBar, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[3], FocusChatInput, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[4], OpenTaskCreateShortcut, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[5], CloseOverlays, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[6], PinThread, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[7], SettleThread, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[8], CopyThreadLink, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[9], ToggleDiff, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[10], TogglePanel, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[11], ClosePanel, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[12], ToggleModelPicker, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[13], NewThread, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[14], NewThread, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[15], NewThread, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[16], PrevThread, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[17], NextThread, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[18], OpenSettings, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[19], MentionFile, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[20], ToggleTerminal, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[21], TogglePreview, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[22], SearchThreads, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[23], OpenCapture, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[24], OpenTodayView, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[25], OpenKnowledgeView, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[26], OpenInboxView, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[27], ToggleBranchMenu, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[28], ToggleWorkspaceMenu, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[29], ToggleEffortMenu, None),
                KeyBinding::new(DESKTOP_GLOBAL_SHORTCUTS[30], UsePreviousWorktree, None),
            ];
            // Chat-only: ⌘S parks the draft (web prompt stash).
            bindings.push(KeyBinding::new(
                "cmd-s",
                views::chat::StashDraft,
                Some("Chat"),
            ));
            // Cmd+F: chat searches the open thread. A focused diff
            // searches that diff. The terminal keeps the key (its own context).
            bindings.push(KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-shift-enter"
                } else {
                    "ctrl-shift-enter"
                },
                bruhs_ui::input::Enter {
                    secondary: true,
                    shift: true,
                },
                Some("ChatComposer"),
            ));
            bindings.push(KeyBinding::new("cmd-f", FindInThread, Some("Chat")));
            bindings.push(KeyBinding::new("cmd-f", FindInDiff, Some("Diff")));
            bindings.push(KeyBinding::new(
                "cmd-s",
                SaveKnowledgeDocument,
                Some("Workspace"),
            ));
            // Page history. The terminal keeps these; the action propagates
            // while it is focused. They stay out of DESKTOP_GLOBAL_SHORTCUTS
            // so the jump-thread index does not move.
            bindings.push(KeyBinding::new("cmd-[", NavigateBack, None));
            bindings.push(KeyBinding::new("cmd-]", NavigateForward, None));
            // Zoom, as a browser zooms the web app: ⌘+ (or ⌘=) in, ⌘− out,
            // ⌘0 back to 100%. Also out of DESKTOP_GLOBAL_SHORTCUTS.
            bindings.push(KeyBinding::new("cmd-=", ZoomIn, None));
            bindings.push(KeyBinding::new("cmd-+", ZoomIn, None));
            bindings.push(KeyBinding::new("cmd--", ZoomOut, None));
            bindings.push(KeyBinding::new("cmd-0", ResetZoom, None));
            for (offset, key) in DESKTOP_GLOBAL_SHORTCUTS[JUMP_THREAD_FIRST_INDEX..]
                .iter()
                .enumerate()
            {
                bindings.push(KeyBinding::new(key, JumpThread(offset), None));
            }
            cx.bind_keys(bindings);

            let window_options = WindowOptions {
                titlebar: Some(TitleBar::title_bar_options()),
                window_min_size: Some(size(px(640.0), px(480.0))),
                // Same-size comparisons with the web (debug builds only).
                #[cfg(debug_assertions)]
                window_bounds: smoke::window_size().map(|size| {
                    WindowBounds::Windowed(match smoke::window_origin() {
                        Some(origin) => Bounds { origin, size },
                        None => Bounds::centered(None, size, cx),
                    })
                }),
                ..Default::default()
            };
            let link_workspace: Rc<RefCell<Option<Entity<views::workspace::Workspace>>>> =
                Rc::new(RefCell::new(None));
            let link_workspace_for_window = link_workspace.clone();
            // The smoke hooks drive the workspace after the window opens.
            #[cfg(debug_assertions)]
            let workspace_cell: Rc<
                RefCell<Option<Entity<views::workspace::Workspace>>>,
            > = Rc::new(RefCell::new(None));
            #[cfg(debug_assertions)]
            let workspace_for_window = workspace_cell.clone();
            // Bring the window to the front even when launched from a shell;
            // an unfocused, off-screen window never draws a frame. A capture
            // run can ask to stay in the background, at the screen's edge, so
            // it never takes the keyboard or a click from the person at the
            // machine.
            #[cfg(debug_assertions)]
            let in_background = smoke::window_in_background();
            #[cfg(not(debug_assertions))]
            let in_background = false;
            if !in_background {
                cx.activate(true);
            }
            let opened = cx.open_window(window_options, |window, cx| {
                // The first level on the window MUST be a bruhs_ui::Root —
                // it provides the dialog/sheet/notification overlays and applies
                // the theme background + font family to the whole window.
                // Matches the web's `<title>`. gpui sets the platform title and
                // the AccessKit window title together; without it VoiceOver
                // announces an unnamed window.
                window.set_window_title(DESKTOP_WINDOW_TITLE);
                #[cfg(target_os = "windows")]
                window.on_window_should_close(cx, |window, _| {
                    // Let GPUI release children and own the HWND destruction.
                    // Default WM_CLOSE would destroy it before that cleanup.
                    window.remove_window();
                    false
                });
                #[cfg(target_os = "macos")]
                window_chrome::hide_buttons(window);
                let workspace = cx.new(|cx| {
                    #[cfg(target_os = "macos")]
                    cx.observe_window_bounds(window, |_, window, _| {
                        window_chrome::hide_buttons(window)
                    })
                    .detach();
                    views::workspace::Workspace::new(window, cx)
                });
                *link_workspace_for_window.borrow_mut() = Some(workspace.clone());
                #[cfg(debug_assertions)]
                {
                    *workspace_for_window.borrow_mut() = Some(workspace.clone());
                }
                cx.new(|cx| Root::new(workspace, window, cx))
            });
            if let (Ok(window), Some(workspace)) = (&opened, link_workspace.borrow().clone()) {
                open_app_links(*window, workspace, link_rx, cx);
            }
            match opened {
                // Launch checks driven by environment variables: debug
                // builds only, so a release app cannot be auto-driven.
                #[cfg(debug_assertions)]
                Ok(window) if smoke::desktop_terminal_smoke_enabled() => {
                    if let Some(workspace) = workspace_cell.borrow().clone() {
                        smoke::schedule_desktop_terminal_smoke(window, workspace, cx);
                    }
                }
                #[cfg(debug_assertions)]
                Ok(window) if smoke::desktop_commands_smoke_enabled() => {
                    if let Some(workspace) = workspace_cell.borrow().clone() {
                        smoke::schedule_desktop_commands_smoke(window, workspace, cx);
                    }
                }
                #[cfg(debug_assertions)]
                Ok(window) if smoke::desktop_threads_smoke_enabled() => {
                    if let Some(workspace) = workspace_cell.borrow().clone() {
                        smoke::schedule_desktop_threads_smoke(window, workspace, cx);
                    }
                }
                #[cfg(debug_assertions)]
                Ok(_) if smoke::desktop_window_smoke_enabled() => {
                    smoke::schedule_desktop_window_smoke_quit(cx)
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::error!("failed to open window: {err}");
                    cx.quit();
                }
            }
        });
}

/// Queues the `bruhs://` links the OS hands the app.
trait AppLinks {
    fn with_app_links(self, links: async_channel::Sender<String>) -> Self;
}

impl AppLinks for Application {
    fn with_app_links(self, links: async_channel::Sender<String>) -> Self {
        self.on_open_urls(move |urls| {
            for url in urls {
                let _ = links.try_send(url);
            }
        });
        self
    }
}

/// Open each `bruhs://` link in the workspace and bring the app forward.
fn open_app_links(
    window: WindowHandle<Root>,
    workspace: Entity<views::workspace::Workspace>,
    links: async_channel::Receiver<String>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Ok(link) = links.recv().await {
            let workspace = workspace.clone();
            let opened = window.update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.open_app_link(&link, window, cx)
                });
                window.activate_window();
            });
            if opened.is_err() {
                break;
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::DESKTOP_WINDOW_TITLE;

    /// The web's `<title>`; VoiceOver reads it as the window name.
    #[test]
    fn window_title_should_match_web_document_title() {
        let index = include_str!("../../bruhs-web/index.html");
        let web_title = index
            .split_once("<title>")
            .and_then(|(_, rest)| rest.split_once("</title>"))
            .map(|(title, _)| title.trim())
            .expect("bruhs-web/index.html should have a <title>");
        assert_eq!(DESKTOP_WINDOW_TITLE, web_title);
    }

    #[cfg(debug_assertions)]
    use super::smoke::{
        desktop_window_smoke_value_enabled, DESKTOP_WINDOW_SMOKE_DONE, DESKTOP_WINDOW_SMOKE_ENV,
        DESKTOP_WINDOW_SMOKE_READY,
    };
    use super::{DESKTOP_GLOBAL_SHORTCUTS, JUMP_THREAD_FIRST_INDEX};

    #[cfg(debug_assertions)]
    #[test]
    fn window_size_hook_reads_points_and_keeps_the_minimum() {
        use gpui::{px, size};
        assert_eq!(
            super::smoke::parse_window_size("1280x800"),
            Some(size(px(1280.0), px(800.0)))
        );
        assert_eq!(
            super::smoke::parse_window_size(" 1440 x 900 "),
            Some(size(px(1440.0), px(900.0)))
        );
        for bad in ["", "1280", "320x200", "wide x tall", "NaNx800"] {
            assert_eq!(super::smoke::parse_window_size(bad), None, "{bad}");
        }
    }

    #[test]
    fn desktop_global_shortcuts_should_match_web_shell_reference() {
        assert_eq!(
            DESKTOP_GLOBAL_SHORTCUTS,
            [
                "cmd-shift-t",
                "cmd-b",
                "cmd-k",
                "/",
                "c",
                "escape",
                "cmd-shift-p",
                "cmd-shift-s",
                "cmd-shift-c",
                "cmd-d",
                "cmd-alt-b",
                "cmd-w",
                "cmd-shift-m",
                "cmd-n",
                "cmd-shift-o",
                "cmd-shift-n",
                "cmd-shift-[",
                "cmd-shift-]",
                "cmd-/",
                "cmd-p",
                "cmd-j",
                "cmd-shift-j",
                "cmd-shift-f",
                "cmd-shift-space",
                "cmd-shift-y",
                "cmd-shift-k",
                "cmd-shift-i",
                "cmd-shift-g",
                "cmd-shift-x",
                "cmd-shift-e",
                "cmd-shift-l",
                "cmd-1",
                "cmd-2",
                "cmd-3",
                "cmd-4",
                "cmd-5",
                "cmd-6",
                "cmd-7",
                "cmd-8",
                "cmd-9",
            ]
        );
        assert_eq!(DESKTOP_GLOBAL_SHORTCUTS[JUMP_THREAD_FIRST_INDEX], "cmd-1");
        assert_eq!(
            DESKTOP_GLOBAL_SHORTCUTS[JUMP_THREAD_FIRST_INDEX..].len(),
            9,
            "⌘1–⌘9 jump bindings"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    fn desktop_window_smoke_env_should_be_opt_in() {
        assert_eq!(DESKTOP_WINDOW_SMOKE_ENV, "BRUHS_DESKTOP_WINDOW_SMOKE");
        assert_eq!(
            DESKTOP_WINDOW_SMOKE_READY,
            "BRUHS_DESKTOP_WINDOW_SMOKE ready"
        );
        assert_eq!(DESKTOP_WINDOW_SMOKE_DONE, "BRUHS_DESKTOP_WINDOW_SMOKE done");

        assert!(!desktop_window_smoke_value_enabled(""));
        assert!(!desktop_window_smoke_value_enabled("  "));
        assert!(!desktop_window_smoke_value_enabled("0"));
        assert!(!desktop_window_smoke_value_enabled("false"));
        assert!(desktop_window_smoke_value_enabled("1"));
        assert!(desktop_window_smoke_value_enabled("true"));
    }
}
