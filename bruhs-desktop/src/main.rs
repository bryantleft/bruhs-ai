// bruhs-desktop — native desktop client using GPUI + gpui-component primitives.

mod app_state;
mod assets;
mod helpers;
mod logging;
mod motion;
mod paths;
mod server_client;
mod shell_path;
#[cfg(debug_assertions)]
mod smoke;
#[cfg(debug_assertions)]
mod synthetic_dock;
mod theme;
mod views;
mod voice_capture;

use gpui::*;
use gpui_component::{Root, TitleBar};
#[cfg(debug_assertions)]
use std::cell::RefCell;
use std::env;
#[cfg(debug_assertions)]
use std::rc::Rc;
use views::chat::FindInThread;
use views::diff_panel::FindInDiff;
use views::workspace::{
    CloseOverlays, ClosePanel, CopyThreadLink, FocusChatInput, FocusCommandBar, JumpThread,
    MentionFile, NavigateBack, NavigateForward, NewThread, NextThread, OpenCapture, OpenInboxView,
    OpenKnowledgeView, OpenSettings, OpenTaskCreateShortcut, OpenTodayView, PinThread, PrevThread,
    SaveKnowledgeDocument, SearchThreads, SettleThread, ToggleBranchMenu, ToggleDiff,
    ToggleEffortMenu, ToggleModelPicker, TogglePanel, TogglePreview, ToggleSidebar, ToggleTasks,
    ToggleTerminal, ToggleWorkspaceMenu, UsePreviousWorktree,
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

fn main() {
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
    // Bruhs.app opened from Finder has launchd's bare PATH; adopt the login
    // shell's so providers, git and the helpers resolve. Before any thread.
    if bruhs_core::updates::release::current_app_bundle().is_some() {
        shell_path::adopt_login_path();
    }
    let log_file = logging::init();
    if let Some(path) = &log_file {
        tracing::info!("bruhs-desktop starting; logging to {}", path.display());
    }
    if env::args().any(|arg| arg == "--synthetic-meeting-dock") {
        #[cfg(debug_assertions)]
        synthetic_dock::run();
        #[cfg(not(debug_assertions))]
        tracing::error!("Synthetic meeting dock acceptance requires a debug build.");
        return;
    }
    gpui_platform::application()
        .with_assets(assets::DesktopAssets)
        .run(|cx: &mut App| {
            // ---- gpui-component setup ----
            // Initializes Theme, Root, dialog/sheet/notification machinery, input system, etc.
            gpui_component::init(cx);
            motion::init(cx);

            // The bruhs type stack is all Monaspace. The web loads it as a
            // webfont; the desktop embeds the same variable faces so both
            // surfaces render identical glyphs without a system install.
            if let Err(err) = cx.text_system().add_fonts(assets::embedded_fonts()) {
                tracing::error!("could not load embedded fonts: {err}");
            }

            // Switch to dark theme + apply bruhs design tokens on top.
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
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
                ..Default::default()
            };
            // The smoke hooks drive the workspace after the window opens.
            #[cfg(debug_assertions)]
            let workspace_cell: Rc<
                RefCell<Option<Entity<views::workspace::Workspace>>>,
            > = Rc::new(RefCell::new(None));
            #[cfg(debug_assertions)]
            let workspace_for_window = workspace_cell.clone();
            // Bring the window to the front even when launched from a shell;
            // an unfocused, off-screen window never draws a frame.
            cx.activate(true);
            match cx.open_window(window_options, |window, cx| {
                // The first level on the window MUST be a gpui_component::Root —
                // it provides the dialog/sheet/notification overlays and applies
                // the theme background + font family to the whole window.
                // Matches the web's `<title>`. gpui sets the platform title and
                // the AccessKit window title together; without it VoiceOver
                // announces an unnamed window.
                window.set_window_title(DESKTOP_WINDOW_TITLE);
                let workspace = cx.new(|cx| views::workspace::Workspace::new(window, cx));
                #[cfg(debug_assertions)]
                {
                    *workspace_for_window.borrow_mut() = Some(workspace.clone());
                }
                cx.new(|cx| Root::new(workspace, window, cx))
            }) {
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

    use super::smoke::{
        desktop_window_smoke_value_enabled, DESKTOP_WINDOW_SMOKE_DONE, DESKTOP_WINDOW_SMOKE_ENV,
        DESKTOP_WINDOW_SMOKE_READY,
    };
    use super::{DESKTOP_GLOBAL_SHORTCUTS, JUMP_THREAD_FIRST_INDEX};

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
