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
//! - `BRUHS_DESKTOP_THREADS_SMOKE=1`: in a new project-less chat, have the
//!   agent start a thread with `message_thread` and wait for that thread's
//!   agent to answer back, then start threads with `wait` and with a role,
//!   then set and remove a role on the first thread with `/role`, print
//!   what each thread received, then quit.
//!   Spends real tokens; use a scratch `BRUHS_DESKTOP_STATE_FILE`.
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

/// Has one thread's agent message another and get an answer back.
const DESKTOP_THREADS_SMOKE_ENV: &str = "BRUHS_DESKTOP_THREADS_SMOKE";
/// The word the second thread must send back.
const DESKTOP_THREADS_SMOKE_WORD: &str = "SCOUT-OK";
/// The model of the role the smoke adds: not the default, so a thread that
/// runs on it shows the role was applied.
const DESKTOP_THREADS_SMOKE_ROLE_MODEL: &str = "claude-haiku-4-5";
/// The word the third thread must answer inside the waiting call.
const DESKTOP_THREADS_SMOKE_WAIT_WORD: &str = "RELAY-OK";

pub(crate) fn desktop_threads_smoke_enabled() -> bool {
    env::var(DESKTOP_THREADS_SMOKE_ENV)
        .ok()
        .is_some_and(|value| desktop_window_smoke_value_enabled(&value))
}

/// Messages between threads, driven through the desktop's own composer.
/// First the agent starts a second thread whose agent answers back with a
/// message of its own. Then it starts a third and waits, inside the tool
/// call, for that thread's reply.
pub(crate) fn schedule_desktop_threads_smoke(
    window: WindowHandle<Root>,
    workspace: Entity<views::workspace::Workspace>,
    cx: &mut App,
) {
    let tag = DESKTOP_THREADS_SMOKE_ENV;
    eprintln!("{tag} ready");
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        let new_workspace = workspace.clone();
        let Ok(Some(chat)) = window.update(cx, move |_, window, cx| {
            new_workspace.update(cx, |workspace, cx| {
                workspace.smoke_bypass_permissions(cx);
                workspace.smoke_new_chat(window, cx)
            })
        }) else {
            eprintln!("{tag} no chat");
            cx.update(|cx| cx.quit());
            return;
        };
        let enter = |text: &'static str, cx: &mut AsyncApp| {
            let sender = chat.clone();
            let _ = window.update(cx, move |_, window, cx| {
                sender.update(cx, |chat, cx| chat.smoke_enter(text, window, cx));
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
        let last = |title: &'static str, cx: &mut AsyncApp| {
            cx.update(|cx| {
                workspace
                    .read(cx)
                    .smoke_chat_titled(title, cx)
                    .map(|chat| {
                        let chat = chat.read(cx);
                        (chat.smoke_last_prompt(), chat.smoke_last_reply())
                    })
                    .unwrap_or_default()
            })
        };

        enter(
            "This is a harness check of the bruhs_threads MCP tools. Call message_thread once with no thread_id, the title \"Scout\", and exactly this message: \"Harness check. Call message_thread once, addressed to the thread id named in the header of this message, with exactly the message: SCOUT-OK. Then stop.\" Do not set wait. Do not use any other tool. Then answer in one short line.",
            cx,
        );
        // The answer is a new prompt in the first thread, and its turn ends.
        let watched = chat.clone();
        let answered = wait(cx, move |cx| {
            cx.update(|cx| {
                let chat = watched.read(cx);
                let prompt = chat.smoke_last_prompt();
                !chat.is_streaming
                    && bruhs_models::thread_link::prompt_hops(&prompt) == 2
                    && prompt.contains(DESKTOP_THREADS_SMOKE_WORD)
            })
        })
        .await;
        let (scout_prompt, scout_reply) = last("Scout", cx);
        let stayed = cx.update(|cx| {
            workspace
                .read(cx)
                .smoke_active_chat()
                .map(|active| active.entity_id())
                == Some(chat.entity_id())
        });
        let (answer_prompt, final_reply) = cx.update(|cx| {
            let chat = chat.read(cx);
            (chat.smoke_last_prompt(), chat.smoke_last_reply())
        });
        eprintln!(
            "{tag} started={} scout_hops={} scout_prompt={:?} scout_reply={scout_reply:?}",
            !scout_prompt.is_empty(),
            bruhs_models::thread_link::prompt_hops(&scout_prompt),
            scout_prompt.lines().next().unwrap_or_default(),
        );
        eprintln!(
            "{tag} answered={answered} view_stayed={stayed} answer_prompt={answer_prompt:?} final_reply={final_reply:?}"
        );

        // A waiting call: the third thread's reply comes back as the tool
        // result, inside this one turn, and no message is sent back.
        enter(
            "Second harness check. Call message_thread once with no thread_id, the title \"Relay\", wait set to true, and exactly this message: \"Harness check. Reply with exactly the word RELAY-OK and nothing else. Do not call any tool.\" Do not use any other tool. Then tell me, in one short line, the exact text that tool call returned.",
            cx,
        );
        let watched = chat.clone();
        let waited = wait(cx, move |cx| {
            cx.update(|cx| {
                let chat = watched.read(cx);
                !chat.is_streaming
                    && chat.smoke_last_prompt().starts_with("Second harness check")
                    && chat.smoke_last_reply().contains(DESKTOP_THREADS_SMOKE_WAIT_WORD)
            })
        })
        .await;
        let (relay_prompt, relay_reply) = last("Relay", cx);
        let (last_prompt, waited_reply) = cx.update(|cx| {
            let chat = chat.read(cx);
            (chat.smoke_last_prompt(), chat.smoke_last_reply())
        });
        eprintln!(
            "{tag} waited={waited} relay_awaited={} relay_reply={relay_reply:?} sender_got_no_message_back={} sender_reply={waited_reply:?}",
            bruhs_models::thread_link::parse_delivery(&relay_prompt)
                .is_some_and(|delivery| delivery.awaited),
            last_prompt.starts_with("Second harness check"),
        );
        // A role: the agent starts a thread with it and waits. The thread
        // runs on the role's model and follows the role's instructions.
        let roles_workspace = workspace.clone();
        cx.update(|cx| {
            roles_workspace.update(cx, |workspace, cx| {
                workspace.smoke_add_role(
                    "Echo",
                    DESKTOP_THREADS_SMOKE_ROLE_MODEL,
                    "End every reply with the token ROLE-OK.",
                    cx,
                );
            });
        });
        enter(
            "Third harness check. Call message_thread once with no thread_id, the title \"Echoer\", the role \"Echo\", wait set to true, and exactly this message: \"Harness check. Reply with one short sentence about tea. Do not call any tool.\" Do not use any other tool. Then tell me, in one short line, the exact text that tool call returned.",
            cx,
        );
        let watched = chat.clone();
        let role_done = wait(cx, move |cx| {
            cx.update(|cx| {
                let chat = watched.read(cx);
                !chat.is_streaming && chat.smoke_last_prompt().starts_with("Third harness check")
            })
        })
        .await;
        let (_, echo_reply) = last("Echoer", cx);
        let (echo_model, echo_role) = cx.update(|cx| {
            workspace
                .read(cx)
                .smoke_chat_titled("Echoer", cx)
                .map(|chat| chat.read(cx).smoke_model_and_role())
                .unwrap_or_default()
        });
        let sender_model = cx.update(|cx| chat.read(cx).smoke_model_and_role().0);
        eprintln!(
            "{tag} role_done={role_done} role={echo_role:?} role_model={echo_model:?} sender_model={sender_model:?} followed_instructions={} echo_reply={echo_reply:?}",
            echo_reply.contains("ROLE-OK"),
        );

        // `/role` by hand: this thread's session started without the role's
        // instructions, so its next turn starts a new session that holds
        // them. `/role none` then drops them again.
        enter("/role echo", cx);
        enter(
            "Fourth harness check. Reply with one short sentence about rain. Do not call any tool.",
            cx,
        );
        let watched = chat.clone();
        let own_done = wait(cx, move |cx| {
            cx.update(|cx| {
                let chat = watched.read(cx);
                !chat.is_streaming
                    && chat.smoke_last_prompt().starts_with("Fourth harness check")
                    && !chat.smoke_last_reply().trim().is_empty()
            })
        })
        .await;
        let ((own_model, own_role), own_reply) = cx.update(|cx| {
            let chat = chat.read(cx);
            (chat.smoke_model_and_role(), chat.smoke_last_reply())
        });
        eprintln!(
            "{tag} own_role_done={own_done} own_role={own_role:?} own_model={own_model:?} followed_instructions={} own_reply={own_reply:?}",
            own_reply.contains("ROLE-OK"),
        );
        enter("/role none", cx);
        enter(
            "Fifth harness check. Reply with the single word PLAIN-OK. Do not call any tool.",
            cx,
        );
        let watched = chat.clone();
        let plain_done = wait(cx, move |cx| {
            cx.update(|cx| {
                let chat = watched.read(cx);
                !chat.is_streaming
                    && chat.smoke_last_prompt().starts_with("Fifth harness check")
                    && !chat.smoke_last_reply().trim().is_empty()
            })
        })
        .await;
        let ((_, plain_role), plain_reply) = cx.update(|cx| {
            let chat = chat.read(cx);
            (chat.smoke_model_and_role(), chat.smoke_last_reply())
        });
        eprintln!(
            "{tag} role_removed_done={plain_done} role={plain_role:?} still_followed={} reply={plain_reply:?}",
            plain_reply.contains("ROLE-OK"),
        );
        eprintln!("{tag} done");
        cx.update(|cx| cx.quit());
    })
    .detach();
}
