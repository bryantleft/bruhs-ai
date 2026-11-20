//! Codex CLI sign-in, as on the web's auth banner: a status check when a
//! thread switches to a Codex model or the model picker opens, the banner
//! while signed out, and `codex login` while the status is polled.

use super::auth::{desktop_auth_status_after_check, CLAUDE_AUTH_STATUS_TIMEOUT};
use super::*;
use bruhs_models::logic::cli_login::{codex_login_status, CliLogin};

/// The `codex` program to run; tests install a fixture instead.
#[cfg(not(test))]
const CODEX_PROGRAM: &str = "codex";
const CODEX_STATUS_ARGS: &[&str] = &["login", "status"];
const CODEX_LOGIN_ARGS: &[&str] = &["login"];
pub(super) const CODEX_AUTH_NOT_AUTHENTICATED_LABEL: &str = "Codex is not authenticated.";
pub(super) const CODEX_LOGIN_ACTION_LABEL: &str = "Log in with Codex";
pub(super) const CODEX_LOGIN_WAITING_MESSAGE: &str =
    "Waiting for Codex login… complete the flow in the terminal where bruhs-server is running.";

/// The desktop's status for what `codex login status` said.
fn desktop_codex_status(login: CliLogin) -> DesktopAuthStatus {
    match login {
        CliLogin::SignedIn { method } => DesktopAuthStatus::SignedIn(ClaudeAccount {
            auth_method: method,
            ..ClaudeAccount::default()
        }),
        CliLogin::SignedOut => DesktopAuthStatus::SignedOut(SignedOut::default()),
        CliLogin::Unknown(error) => DesktopAuthStatus::SignedOut(SignedOut {
            error: Some(error),
            ..SignedOut::default()
        }),
    }
}

fn codex_command_error(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        "Codex CLI not found. Install it first.".to_string()
    } else {
        error.to_string()
    }
}

pub(super) async fn fetch_codex_status_with(program: &str) -> DesktopAuthStatus {
    let output = TokioCommand::new(program)
        .args(CODEX_STATUS_ARGS)
        .kill_on_drop(true)
        .output();
    desktop_codex_status(match timeout(CLAUDE_AUTH_STATUS_TIMEOUT, output).await {
        Ok(Ok(output)) => codex_login_status(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Ok(Err(error)) => CliLogin::Unknown(codex_command_error(&error)),
        Err(_) => CliLogin::Unknown("`codex login status` timed out.".to_string()),
    })
}

async fn poll_codex_until_signed_in(
    program: &str,
    timing: LoginTiming,
) -> Option<DesktopAuthStatus> {
    for _ in 0..timing.poll_attempts {
        sleep(timing.poll_interval).await;
        let status = fetch_codex_status_with(program).await;
        if status.is_authenticated() {
            return Some(status);
        }
    }
    None
}

/// Run `codex login` while polling its status. A poll that finds the login
/// done ends the wait and kills the login process (`kill_on_drop`).
pub(super) async fn run_codex_login_with(program: &str, timing: LoginTiming) -> DesktopAuthStatus {
    let login = timeout(
        timing.login_timeout,
        TokioCommand::new(program)
            .args(CODEX_LOGIN_ARGS)
            .kill_on_drop(true)
            .output(),
    );
    tokio::pin!(login);
    let login_result = tokio::select! {
        result = &mut login => result,
        signed_in = poll_codex_until_signed_in(program, timing) => {
            if let Some(status) = signed_in {
                return status;
            }
            login.await
        }
    };
    let status = fetch_codex_status_with(program).await;
    let DesktopAuthStatus::SignedOut(mut signed_out) = status else {
        return status;
    };
    signed_out.message = Some(match login_result {
        Ok(Ok(output)) if output.status.success() => {
            "Still not authenticated. Run `codex login` in a terminal, then click Check again."
                .to_string()
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                "Codex login failed. Run `codex login` in a terminal, then click Check again."
                    .to_string()
            } else {
                format!("Login failed: {stderr}")
            }
        }
        Ok(Err(error)) => format!("Login failed: {}", codex_command_error(&error)),
        Err(_) => "Codex login timed out. Run `codex login` in a terminal, then click Check again."
            .to_string(),
    });
    DesktopAuthStatus::SignedOut(signed_out)
}

/// The program to check with: `codex`, a fixture under test, or nothing
/// when a test installed none or a performance run blocks auth.
pub(super) fn codex_program(cx: &App) -> Option<String> {
    #[cfg(feature = "native-performance")]
    if crate::native_performance::block_service(cx, crate::native_performance::Service::Auth) {
        return None;
    }
    #[cfg(not(test))]
    {
        let _ = cx;
        Some(CODEX_PROGRAM.to_string())
    }
    #[cfg(test)]
    {
        cx.try_global::<CodexProbe>()
            .and_then(|probe| probe.0.clone())
    }
}

/// Under test, the fixture that stands in for `codex`.
#[cfg(test)]
pub(crate) struct CodexProbe(pub(crate) Option<String>);

#[cfg(test)]
impl Global for CodexProbe {}

/// Chats restored with a Codex model at launch share one status check (web
/// checks Codex once at launch), so many Codex threads start one process.
#[derive(Default)]
struct InitialCodexChecks {
    pending: Option<Vec<WeakEntity<ChatView>>>,
}

impl Global for InitialCodexChecks {}

impl ChatView {
    /// A chat made with a Codex model learns the sign-in like the web does
    /// at launch, sharing the check with every other such chat.
    pub(super) fn start_initial_codex_auth_check(&mut self, cx: &mut Context<'_, Self>) {
        let Some(program) = codex_program(cx) else {
            return;
        };
        let entity = cx.weak_entity();
        let pending = &mut cx.default_global::<InitialCodexChecks>().pending;
        if let Some(waiters) = pending {
            waiters.push(entity);
            return;
        }
        *pending = Some(vec![entity]);
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let (tx, mut rx) = unbounded_channel();
        runtime.spawn(async move {
            let _ = tx.send(fetch_codex_status_with(&program).await);
        });
        cx.spawn(async move |_, cx| {
            let status = rx.recv().await;
            cx.update(|cx| {
                let waiters = cx
                    .global_mut::<InitialCodexChecks>()
                    .pending
                    .take()
                    .unwrap_or_default();
                let Some(status) = status else {
                    return;
                };
                for entity in waiters {
                    entity
                        .update(cx, |view, cx| {
                            // A check the chat started since then is newer.
                            if view.codex_auth.is_none() {
                                view.codex_auth = Some(status.clone());
                                cx.notify();
                            }
                        })
                        .ok();
                }
            });
        })
        .detach();
    }

    pub(super) fn start_codex_auth_check(&mut self, cx: &mut Context<'_, Self>) {
        let Some(program) = codex_program(cx) else {
            return;
        };
        let previous_message = self
            .codex_auth
            .as_ref()
            .and_then(|status| status.message().map(str::to_string));
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let (tx, mut rx) = unbounded_channel();
        runtime.spawn(async move {
            let _ = tx.send(fetch_codex_status_with(&program).await);
        });
        cx.spawn(async move |entity, cx| {
            if let Some(status) = rx.recv().await {
                entity
                    .update(cx, |view, cx| {
                        view.codex_auth =
                            Some(desktop_auth_status_after_check(status, previous_message));
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
    }

    pub(super) fn start_codex_login(&mut self, cx: &mut Context<'_, Self>) {
        let Some(program) = codex_program(cx) else {
            return;
        };
        // The login button only shows while signed out.
        let Some(DesktopAuthStatus::SignedOut(signed_out)) = &mut self.codex_auth else {
            return;
        };
        if signed_out.login_in_flight {
            return;
        }
        signed_out.login_in_flight = true;
        signed_out.message = Some(CODEX_LOGIN_WAITING_MESSAGE.to_string());
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let (tx, mut rx) = unbounded_channel();
        runtime.spawn(async move {
            let _ = tx.send(run_codex_login_with(&program, LoginTiming::default()).await);
        });
        cx.spawn(async move |entity, cx| {
            if let Some(status) = rx.recv().await {
                entity
                    .update(cx, |view, cx| {
                        view.codex_auth = Some(status);
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
        cx.notify();
    }
}

impl ChatView {
    /// `BRUHS_DESKTOP_SMOKE_SCENE=codex-login`: the banner's Log in, for a
    /// real sign-in check in a debug build.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_start_codex_login(&mut self, cx: &mut Context<'_, Self>) {
        self.start_codex_login(cx);
    }

    /// What the Codex banner knows, for the same check.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_codex_auth(&self) -> String {
        format!("{:?}", self.codex_auth)
    }
}

#[cfg(all(test, unix))]
mod tests {
    // Named imports: `use super::*` here recurses through `#[gpui::test]`.
    use super::super::{ChatView, DesktopAuthStatus};
    use super::{desktop_codex_status, CodexProbe, CODEX_LOGIN_WAITING_MESSAGE};
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::logic::cli_login::CliLogin;
    use gpui::{AppContext as _, Entity, TestAppContext};
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    /// A `codex` stand-in: signed out until `codex login` runs. Each status
    /// check adds a line to `calls`.
    fn fixture(dir: &std::path::Path) -> String {
        let state = dir.join("signed-in");
        let calls = dir.join("calls");
        let script = dir.join("codex");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \"$1 $2\" in\n'login status') echo . >> '{calls}'; if [ -f '{state}' ]; then echo 'Logged in using ChatGPT'; else echo 'Not logged in'; exit 1; fi ;;\n'login ') /usr/bin/touch '{state}' ;;\n*) exit 99 ;;\nesac\n",
                state = state.display(),
                calls = calls.display()
            ),
        )
        .expect("write the codex fixture");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make the codex fixture runnable");
        script.to_string_lossy().into_owned()
    }

    /// The fixture answers on the tokio runtime; wait for its delivery.
    fn wait_for(
        visual: &mut gpui::VisualTestContext,
        chat: &Entity<ChatView>,
        done: impl Fn(&ChatView) -> bool,
    ) {
        for _ in 0..500 {
            visual.run_until_parked();
            if chat.read_with(visual, |chat, _| done(chat)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the codex fixture did not answer");
    }

    #[gpui::test]
    fn a_codex_thread_shows_the_web_sign_in_banner_until_codex_login(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().expect("fixture dir");
        let program = fixture(dir.path());
        let (chat, visual) = super::super::behavior_tests::chat_window(cx);
        visual.update(|_, cx| cx.set_global(CodexProbe(Some(program))));

        chat.update(visual, |chat, cx| chat.set_model("gpt-5.5", cx));
        wait_for(visual, &chat, |chat| chat.codex_auth.is_some());
        chat.read_with(visual, |chat, _| {
            assert!(!chat.codex_auth.as_ref().unwrap().is_authenticated());
            assert_eq!(
                chat.model_picker_status_line(AgentProvider::Codex),
                bruhs_models::logic::cli_login::CODEX_SIGNED_OUT_LINE
            );
        });
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        assert!(visual.debug_bounds("desktop-auth-banner").is_some());

        // Log in: the poll sees the fixture signed in and the banner goes.
        chat.update(visual, |chat, cx| chat.start_codex_login(cx));
        chat.read_with(visual, |chat, _| {
            assert_eq!(
                chat.codex_auth
                    .as_ref()
                    .and_then(DesktopAuthStatus::message),
                Some(CODEX_LOGIN_WAITING_MESSAGE)
            );
        });
        wait_for(visual, &chat, |chat| {
            chat.codex_auth
                .as_ref()
                .is_some_and(DesktopAuthStatus::is_authenticated)
        });
        chat.read_with(visual, |chat, _| {
            assert_eq!(
                chat.model_picker_status_line(AgentProvider::Codex),
                bruhs_models::logic::cli_login::CODEX_SIGNED_IN_LINE
            );
        });
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        assert!(visual.debug_bounds("desktop-auth-banner").is_none());
    }

    /// Like the web's launch check, chats made with a Codex model, as at a
    /// restore, learn the sign-in, and they share one `codex login status`.
    #[gpui::test]
    fn restored_codex_chats_share_one_sign_in_check(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().expect("fixture dir");
        let program = fixture(dir.path());
        cx.update(|cx| {
            bruhs_ui::init(cx);
            bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
            crate::theme::apply_bruhs_theme(cx);
            cx.set_global(crate::app_state::DesktopState::new().expect("desktop state"));
            cx.set_global(CodexProbe(Some(program)));
        });
        let project = dir.path().to_string_lossy().into_owned();
        let made = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let capture = made.clone();
        let (_root, visual) = cx.add_window_view(|window, cx| {
            let chats: Vec<Entity<ChatView>> = (0..2)
                .map(|_| {
                    cx.new(|cx| {
                        ChatView::new_with_model_and_project_path(
                            "gpt-5.5".into(),
                            project.clone(),
                            window,
                            cx,
                        )
                    })
                })
                .collect();
            let first = chats[0].clone();
            *capture.borrow_mut() = chats;
            bruhs_ui::Root::new(first, window, cx)
        });
        let chats = made.borrow().clone();
        for chat in &chats {
            wait_for(visual, chat, |chat| chat.codex_auth.is_some());
            chat.read_with(visual, |chat, _| {
                assert!(!chat.codex_auth.as_ref().unwrap().is_authenticated());
            });
        }
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap_or_default();
        assert_eq!(calls.lines().count(), 1, "both chats share one check");
    }

    #[test]
    fn codex_status_maps_to_the_desktop_banner_state() {
        assert!(desktop_codex_status(CliLogin::SignedIn { method: None }).is_authenticated());
        let unknown = desktop_codex_status(CliLogin::Unknown("Codex reported: boom".into()));
        assert!(!unknown.is_authenticated());
        let DesktopAuthStatus::SignedOut(signed_out) = unknown else {
            unreachable!()
        };
        assert_eq!(signed_out.error.as_deref(), Some("Codex reported: boom"));
    }
}
