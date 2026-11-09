//! Launch checks driven by environment variables (debug builds only).
//!
//! - `BRUHS_DESKTOP_WINDOW_SMOKE=1`: print `ready`, then quit after a beat.
//! - `BRUHS_DESKTOP_TERMINAL_SMOKE=1`: open the ⌘J drawer, type into the
//!   real shell, print what the emulator holds, optionally load a preview
//!   URL, open settings or a named scene, hold for a screenshot, then quit.
//!
//! - `BRUHS_DESKTOP_COMMANDS_SMOKE=1`: in a new project-less chat, send a
//!   real prompt, then `/second-opinion` and `/handoff`, print what each put
//!   in the composer, then quit. Spends real tokens; point
//!   `BRUHS_DESKTOP_STATE_FILE` at a scratch file so no threads are kept.
//!
//! None of this is compiled into a release build: a shipped app must not be
//! drivable (or made to auto-submit prompts) through its environment.

use crate::{app_state, views};
use gpui::*;
use gpui_component::Root;
use std::{env, time::Duration};

pub(crate) const DESKTOP_WINDOW_SMOKE_ENV: &str = "BRUHS_DESKTOP_WINDOW_SMOKE";
pub(crate) const DESKTOP_WINDOW_SMOKE_READY: &str = "BRUHS_DESKTOP_WINDOW_SMOKE ready";
pub(crate) const DESKTOP_WINDOW_SMOKE_DONE: &str = "BRUHS_DESKTOP_WINDOW_SMOKE done";
const DESKTOP_WINDOW_SMOKE_DELAY_MS: u64 = 500;
/// Opens the ⌘J drawer, types an echo into the real shell and prints what
/// the emulator holds, then quits. A launch check for the terminal pipeline.
const DESKTOP_TERMINAL_SMOKE_ENV: &str = "BRUHS_DESKTOP_TERMINAL_SMOKE";
const DESKTOP_TERMINAL_SMOKE_OPEN_DELAY_MS: u64 = 800;
const DESKTOP_TERMINAL_SMOKE_SHELL_DELAY_MS: u64 = 2500;
const DESKTOP_TERMINAL_SMOKE_ECHO_DELAY_MS: u64 = 1500;
const DESKTOP_TERMINAL_SMOKE_PREVIEW_DELAY_MS: u64 = 2500;
/// Local page the smoke loads into the Preview surface.
const DESKTOP_TERMINAL_SMOKE_PREVIEW_URL_ENV: &str = "BRUHS_DESKTOP_SMOKE_PREVIEW_URL";
/// Extra time to keep the window up (for a screenshot) before quitting.
const DESKTOP_SMOKE_HOLD_ENV: &str = "BRUHS_DESKTOP_SMOKE_HOLD_MS";
/// Leave the ⌘J drawer closed (screenshots of the bare chat surface).
const DESKTOP_SMOKE_SKIP_TERMINAL_ENV: &str = "BRUHS_DESKTOP_SMOKE_SKIP_TERMINAL";

/// Named window state for a web-vs-desktop comparison screenshot (`Workspace::smoke_scene`).
const DESKTOP_SMOKE_SCENE_ENV: &str = "BRUHS_DESKTOP_SMOKE_SCENE";

pub(crate) fn desktop_window_smoke_enabled() -> bool {
    env::var(DESKTOP_WINDOW_SMOKE_ENV)
        .ok()
        .is_some_and(|value| desktop_window_smoke_value_enabled(&value))
}

pub(crate) fn desktop_window_smoke_value_enabled(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && !matches!(value, "0" | "false" | "False" | "FALSE")
}

pub(crate) fn desktop_terminal_smoke_enabled() -> bool {
    env::var(DESKTOP_TERMINAL_SMOKE_ENV)
        .ok()
        .is_some_and(|value| desktop_window_smoke_value_enabled(&value))
}

pub(crate) fn schedule_desktop_terminal_smoke(
    window: WindowHandle<Root>,
    workspace: Entity<views::workspace::Workspace>,
    cx: &mut App,
) {
    eprintln!("{DESKTOP_TERMINAL_SMOKE_ENV} ready");
    cx.spawn(async move |cx: &mut AsyncApp| {
        let timer =
            |ms: u64, cx: &AsyncApp| cx.background_executor().timer(Duration::from_millis(ms));
        timer(DESKTOP_TERMINAL_SMOKE_OPEN_DELAY_MS, cx).await;
        if env::var(DESKTOP_SMOKE_SKIP_TERMINAL_ENV).is_err() {
            let toggle_workspace = workspace.clone();
            let _ = window.update(cx, move |_, window, cx| {
                toggle_workspace.update(cx, |workspace, cx| {
                    workspace.smoke_toggle_terminal(window, cx);
                });
            });
            timer(DESKTOP_TERMINAL_SMOKE_SHELL_DELAY_MS, cx).await;
            cx.update(|cx| {
                if let Some(id) = workspace.read(cx).terminal_active_id(cx) {
                    let result = cx
                        .global::<app_state::DesktopState>()
                        .app
                        .terminals()
                        .write(&id, b"echo desktop-smoke-$((40+2))\n");
                    eprintln!("{DESKTOP_TERMINAL_SMOKE_ENV} wrote={:?}", result.is_ok());
                } else {
                    eprintln!("{DESKTOP_TERMINAL_SMOKE_ENV} no active session");
                }
            });
            timer(DESKTOP_TERMINAL_SMOKE_ECHO_DELAY_MS, cx).await;
        }
        cx.update(|cx| {
            eprintln!(
                "{DESKTOP_TERMINAL_SMOKE_ENV} {}",
                workspace.read(cx).terminal_smoke_report(cx)
            );
        });
        if let Ok(url) = env::var(DESKTOP_TERMINAL_SMOKE_PREVIEW_URL_ENV) {
            let preview_workspace = workspace.clone();
            let _ = window.update(cx, move |_, window, cx| {
                preview_workspace.update(cx, |workspace, cx| {
                    workspace.preview_smoke_go(&url, window, cx);
                });
            });
            timer(DESKTOP_TERMINAL_SMOKE_PREVIEW_DELAY_MS, cx).await;
            let place_workspace = workspace.clone();
            let placed = window.update(cx, move |_, window, cx| {
                place_workspace.update(cx, |workspace, cx| {
                    workspace.preview_smoke_place(window, cx)
                })
            });
            eprintln!("{DESKTOP_TERMINAL_SMOKE_ENV} preview place={placed:?}");
            timer(DESKTOP_TERMINAL_SMOKE_ECHO_DELAY_MS, cx).await;
            cx.update(|cx| {
                eprintln!(
                    "{DESKTOP_TERMINAL_SMOKE_ENV} preview {}",
                    workspace.read(cx).preview_smoke_report(cx)
                );
            });
        }
        if env::var("BRUHS_DESKTOP_SMOKE_SETTINGS").is_ok() {
            let settings_workspace = workspace.clone();
            let _ = window.update(cx, move |_, window, cx| {
                settings_workspace.update(cx, |workspace, cx| {
                    workspace.smoke_open_settings(window, cx);
                });
            });
        }
        if let Ok(scene) = env::var(DESKTOP_SMOKE_SCENE_ENV) {
            let scene_workspace = workspace.clone();
            let known = window.update(cx, move |_, window, cx| {
                scene_workspace.update(cx, |workspace, cx| {
                    workspace.smoke_scene(&scene, window, cx)
                })
            });
            eprintln!("{DESKTOP_TERMINAL_SMOKE_ENV} scene known={known:?}");
        }
        let hold_ms = env::var(DESKTOP_SMOKE_HOLD_ENV)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        if hold_ms > 0 {
            timer(hold_ms, cx).await;
        }
        let cleanup_workspace = workspace.clone();
        let _ = window.update(cx, move |_, _window, cx| {
            cleanup_workspace.update(cx, |workspace, cx| workspace.smoke_cleanup(cx));
        });
        cx.update(|cx| {
            eprintln!("{DESKTOP_TERMINAL_SMOKE_ENV} done");
            cx.quit();
        });
    })
    .detach();
}

pub(crate) fn schedule_desktop_window_smoke_quit(cx: &mut App) {
    eprintln!("{DESKTOP_WINDOW_SMOKE_READY}");
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor()
            .timer(Duration::from_millis(DESKTOP_WINDOW_SMOKE_DELAY_MS))
            .await;
        cx.update(|cx: &mut App| {
            eprintln!("{DESKTOP_WINDOW_SMOKE_DONE}");
            cx.quit();
        });
    })
    .detach();
}

const DESKTOP_COMMANDS_SMOKE_ENV: &str = "BRUHS_DESKTOP_COMMANDS_SMOKE";
/// Longest wait for one agent step (a turn, a review, a brief).
const DESKTOP_COMMANDS_SMOKE_STEP_SECS: u64 = 300;

pub(crate) fn desktop_commands_smoke_enabled() -> bool {
    env::var(DESKTOP_COMMANDS_SMOKE_ENV)
        .ok()
        .is_some_and(|value| desktop_window_smoke_value_enabled(&value))
}

/// `/second-opinion` and `/handoff` driven through the desktop's own
/// composer, the way the web check drives the browser.
pub(crate) fn schedule_desktop_commands_smoke(
    window: WindowHandle<Root>,
    workspace: Entity<views::workspace::Workspace>,
    cx: &mut App,
) {
    let tag = DESKTOP_COMMANDS_SMOKE_ENV;
    eprintln!("{tag} ready");
    cx.spawn(async move |cx: &mut AsyncApp| {
        let tick = |cx: &AsyncApp| cx.background_executor().timer(Duration::from_secs(1));
        tick(cx).await;
        let new_workspace = workspace.clone();
        let Ok(Some(chat)) = window.update(cx, move |_, window, cx| {
            new_workspace.update(cx, |workspace, cx| workspace.smoke_new_chat(window, cx))
        }) else {
            eprintln!("{tag} no chat");
            cx.update(|cx| cx.quit());
            return;
        };
        let enter = |text: &'static str, chat: Entity<views::chat::ChatView>, cx: &mut AsyncApp| {
            let _ = window.update(cx, move |_, window, cx| {
                chat.update(cx, |chat, cx| chat.smoke_enter(text, window, cx));
            });
        };
        // Poll until `done` says so or the step times out.
        async fn wait(cx: &mut AsyncApp, done: impl Fn(&mut AsyncApp) -> bool) -> bool {
            for _ in 0..DESKTOP_COMMANDS_SMOKE_STEP_SECS {
                if done(cx) {
                    return true;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
            false
        }

        enter(
            "Is 221 a prime number? Answer yes or no, then one short sentence why.",
            chat.clone(),
            cx,
        );
        let watched = chat.clone();
        let answered = wait(cx, move |cx| {
            cx.update(|cx| {
                let chat = watched.read(cx);
                !chat.is_streaming && !chat.smoke_last_reply().trim().is_empty()
            })
        })
        .await;
        let reply = cx.update(|cx| chat.read(cx).smoke_last_reply());
        eprintln!("{tag} turn answered={answered} reply={reply:?}");

        enter("/second-opinion", chat.clone(), cx);
        let watched = chat.clone();
        let reviewed = wait(cx, move |cx| {
            cx.update(|cx| {
                watched
                    .read(cx)
                    .smoke_composer(cx)
                    .starts_with("A second reviewer")
            })
        })
        .await;
        let draft = cx.update(|cx| chat.read(cx).smoke_composer(cx));
        eprintln!("{tag} second-opinion drafted={reviewed} draft={draft:?}");

        let source = cx.update(|cx| workspace.read(cx).smoke_active_id());
        enter(
            "/handoff keep checking which numbers are prime",
            chat.clone(),
            cx,
        );
        let watched = workspace.clone();
        let source_id = source.clone();
        let handed = wait(cx, move |cx| {
            cx.update(|cx| {
                let workspace = watched.read(cx);
                workspace.smoke_active_id() != source_id
                    && workspace
                        .smoke_active_chat()
                        .is_some_and(|chat| chat.read(cx).smoke_composer(cx).len() > 40)
            })
        })
        .await;
        let brief = cx.update(|cx| {
            workspace
                .read(cx)
                .smoke_active_chat()
                .map(|chat| chat.read(cx).smoke_composer(cx))
                .unwrap_or_default()
        });
        let moved = cx.update(|cx| workspace.read(cx).smoke_active_id() != source);
        eprintln!("{tag} handoff opened={handed} new_thread={moved} brief={brief:?}");
        eprintln!("{tag} done");
        cx.update(|cx| cx.quit());
    })
    .detach();
}
