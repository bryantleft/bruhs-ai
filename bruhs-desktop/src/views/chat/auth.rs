//! Claude CLI sign-in: status checks, the login flow with polling, and the banner.

use super::*;
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
use std::borrow::Cow;
pub(super) const AUTH_BANNER_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const AUTH_BANNER_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const AUTH_BANNER_RADIUS_PX: f32 = 8.0;
pub(super) const AUTH_BANNER_X_MARGIN_PX: f32 = 16.0;
pub(super) const AUTH_BANNER_Y_MARGIN_PX: f32 = 12.0;
pub(super) const AUTH_BANNER_PADDING_PX: f32 = 12.0;
pub(super) const AUTH_BANNER_CONTENT_GAP_PX: f32 = 12.0;
pub(super) const AUTH_BANNER_TITLE_BOTTOM_MARGIN_PX: f32 = 4.0;
pub(super) const AUTH_BANNER_MESSAGE_BOTTOM_MARGIN_PX: f32 = 8.0;
pub(super) const AUTH_BANNER_ACTION_GAP_PX: f32 = 8.0;
pub(super) const AUTH_BANNER_ACTION_TOP_MARGIN_PX: f32 = 4.0;
pub(super) const AUTH_BANNER_BORDER_OPACITY: f32 = 0.30;
pub(super) const AUTH_BANNER_BG_OPACITY: f32 = 0.10;
pub(super) const AUTH_BANNER_MESSAGE_OPACITY: f32 = 0.80;
pub(super) const AUTH_BANNER_ICON_SIZE_PX: f32 = 16.0;
pub(super) const AUTH_BANNER_ICON_TOP_OFFSET_PX: f32 = 2.0;
/// Web's wording: `cli_login::sign_in_banner_title("Claude", None)`.
pub(super) const CLAUDE_AUTH_NOT_AUTHENTICATED_LABEL: &str = "Claude is not authenticated.";
pub(super) const CLAUDE_LOGIN_ACTION_LABEL: &str = "Log in with Claude";
pub(super) const CLAUDE_LOGIN_WAITING_LABEL: &str = "Waiting…";
pub(super) const CLAUDE_LOGIN_WAITING_MESSAGE: &str =
    "Waiting for Claude login… complete the flow in the terminal where bruhs-server is running.";
pub(super) const CLAUDE_AUTH_STATUS_ARGS: &[&str] = &["auth", "status"];
pub(super) const CLAUDE_AUTH_POLL_INTERVAL_MS: u64 = 2_000;
pub(super) const CLAUDE_AUTH_POLL_ATTEMPTS: usize = 30;

impl Default for DesktopAuthStatus {
    fn default() -> Self {
        Self::SignedOut(SignedOut::default())
    }
}

impl DesktopAuthStatus {
    pub(super) fn is_authenticated(&self) -> bool {
        matches!(self, Self::SignedIn(_))
    }

    pub(super) fn user(&self) -> Option<&str> {
        match self {
            Self::SignedIn(account) => account.user.as_deref(),
            Self::SignedOut(_) => None,
        }
    }

    pub(super) fn message(&self) -> Option<&str> {
        match self {
            Self::SignedIn(_) => None,
            Self::SignedOut(signed_out) => signed_out.message.as_deref(),
        }
    }

    #[cfg(all(test, unix))]
    pub(super) fn login_in_flight(&self) -> bool {
        matches!(
            self,
            Self::SignedOut(SignedOut {
                login_in_flight: true,
                ..
            })
        )
    }
}

#[cfg(test)]
pub(super) fn auth_banner_visible_like_web(model_id: &str, authenticated: bool) -> bool {
    AgentProvider::for_model(model_id) == AgentProvider::Claude && !authenticated
}

/// The `claude` program to run (tests point this at a fixture script).
pub(super) const CLAUDE_PROGRAM: &str = "claude";
/// `claude auth status` normally answers at once; a hung one is abandoned.
pub(super) const CLAUDE_AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the interactive `claude auth login` may run.
pub(super) const CLAUDE_LOGIN_TIMEOUT: Duration = Duration::from_secs(60);

impl Default for LoginTiming {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(CLAUDE_AUTH_POLL_INTERVAL_MS),
            poll_attempts: CLAUDE_AUTH_POLL_ATTEMPTS,
            login_timeout: CLAUDE_LOGIN_TIMEOUT,
        }
    }
}

pub(super) async fn fetch_auth_status_with(program: &str) -> DesktopAuthStatus {
    let output = TokioCommand::new(program)
        .args(CLAUDE_AUTH_STATUS_ARGS)
        .kill_on_drop(true)
        .output();
    match timeout(CLAUDE_AUTH_STATUS_TIMEOUT, output).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            parse_desktop_auth_status(&stdout, &stderr, None)
        }
        Ok(Err(error)) => {
            let message = normalize_claude_command_error(&error);
            parse_desktop_auth_status("", "", Some(&message))
        }
        Err(_) => parse_desktop_auth_status("", "", Some("`claude auth status` timed out.")),
    }
}

/// A fresh check keeps the login guidance still on screen while it says
/// signed out.
pub(super) fn desktop_auth_status_after_check(
    mut status: DesktopAuthStatus,
    previous_message: Option<String>,
) -> DesktopAuthStatus {
    if let DesktopAuthStatus::SignedOut(signed_out) = &mut status {
        if signed_out.message.is_none() {
            signed_out.message = previous_message;
        }
    }
    status
}

pub(super) async fn poll_auth_until_authenticated(
    program: &str,
    timing: LoginTiming,
) -> Option<DesktopAuthStatus> {
    for _ in 0..timing.poll_attempts {
        sleep(timing.poll_interval).await;
        let status = fetch_auth_status_with(program).await;
        if status.is_authenticated() {
            return Some(status);
        }
    }
    None
}

pub(super) async fn run_claude_login_and_fetch_status() -> DesktopAuthStatus {
    run_login_and_fetch_status_with(CLAUDE_PROGRAM, LoginTiming::default()).await
}

/// Run `<program> auth login` while polling the auth status. A successful
/// poll ends the wait at once and kills the login process (it is
/// `kill_on_drop`), so a login finished in the browser does not leave the
/// CLI waiting.
pub(super) async fn run_login_and_fetch_status_with(
    program: &str,
    timing: LoginTiming,
) -> DesktopAuthStatus {
    let login = timeout(
        timing.login_timeout,
        TokioCommand::new(program)
            .args(["auth", "login"])
            .kill_on_drop(true)
            .output(),
    );
    tokio::pin!(login);
    let login_result = tokio::select! {
        result = &mut login => result,
        authed = poll_auth_until_authenticated(program, timing) => {
            if let Some(status) = authed {
                return status;
            }
            // Polling gave up first; the login's own outcome explains why.
            login.await
        }
    };

    let status = fetch_auth_status_with(program).await;
    let DesktopAuthStatus::SignedOut(mut signed_out) = status else {
        return status;
    };
    signed_out.message = Some(match login_result {
        Ok(Ok(output)) if output.status.success() => {
            "Still not authenticated. Run `claude auth login` in a terminal, then click Check again."
                .to_string()
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                "Claude login failed. Run `claude auth login` in a terminal, then click Check again."
                    .to_string()
            } else {
                format!("Login failed: {stderr}")
            }
        }
        Ok(Err(error)) => format!("Login failed: {}", normalize_claude_command_error(&error)),
        Err(_) => {
            "Claude login timed out. Run `claude auth login` in a terminal, then click Check again."
                .to_string()
        }
    });
    DesktopAuthStatus::SignedOut(signed_out)
}

pub(super) fn parse_desktop_auth_status(
    stdout: &str,
    stderr: &str,
    command_error: Option<&str>,
) -> DesktopAuthStatus {
    if let Some(error) = command_error {
        return DesktopAuthStatus::SignedOut(SignedOut {
            error: Some(error.to_string()),
            ..SignedOut::default()
        });
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout.trim()) {
        let explicit = ["authenticated", "loggedIn"].iter().find_map(|key| value[key].as_bool());
        let authenticated = explicit.unwrap_or_else(|| match value["status"].as_str() {
            Some("authenticated" | "active") => true,
            Some("unauthenticated" | "signed_out" | "inactive") => false,
            _ => ["account","email","user"].iter().any(|key| {
                let field = &value[key];
                field.as_str().is_some_and(|text| !text.trim().is_empty())
                    || field.as_object().is_some_and(|account| !account.is_empty())
            })
        });

        if !authenticated {
            return DesktopAuthStatus::default();
        }
        return DesktopAuthStatus::SignedIn(ClaudeAccount {
            user: value
                .get("user")
                .or_else(|| value.get("email"))
                .or_else(|| value.get("account"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            subscription_type: value
                .get("subscription_type")
                .or_else(|| value.get("subscriptionType"))
                .or_else(|| value.get("plan"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            auth_method: value
                .get("auth_method")
                .or_else(|| value.get("authMethod"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        });
    }

    let combined = format!("{stdout}\n{stderr}");
    let lower = combined.to_lowercase();
    let is_unauth = lower.contains("not logged in")
        || lower.contains("unauthenticated")
        || lower.contains("login required");
    if !is_unauth {
        return DesktopAuthStatus::SignedIn(ClaudeAccount::default());
    }
    DesktopAuthStatus::SignedOut(SignedOut {
        error: (!stderr.trim().is_empty()).then(|| stderr.trim().to_string()),
        ..SignedOut::default()
    })
}

pub(super) fn normalize_claude_command_error(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        "Claude Code CLI not found. Install it first.".to_string()
    } else {
        error.to_string()
    }
}

/// What a CLI's sign-in banner says and which actions it offers.
struct CliAuthBanner<'a> {
    provider: AgentProvider,
    title: String,
    signed_out: &'a SignedOut,
    login_label: &'static str,
    login_id: &'static str,
    recheck_id: &'static str,
    recheck_label: &'static str,
}

#[derive(Default)]
struct InitialAuthChecks {
    // Keep the flight open until foreground delivery, including synchronous restore.
    pending: Option<Vec<WeakEntity<ChatView>>>,
}

impl Global for InitialAuthChecks {}

pub(super) fn auth_check_program(cx: &App, enabled: bool) -> Option<Cow<'static, str>> {
    if !enabled { return None; }
    #[cfg(feature = "native-performance")]
    if let Some(program) = crate::native_performance::auth_fixture_program(cx) {
        return Some(Cow::Owned(program));
    }
    #[cfg(feature = "native-performance")]
    if crate::native_performance::block_service(cx, crate::native_performance::Service::Auth) {
        return None;
    }
    #[cfg(not(test))]
    {
        let _ = cx;
        Some(Cow::Borrowed(CLAUDE_PROGRAM))
    }
    #[cfg(all(test, unix))]
    {
        probe_tests::program(cx).map(Cow::Owned)
    }
    #[cfg(all(test, not(unix)))]
    {
        let _ = cx;
        None
    }
}

fn spawn_auth_check(program: Cow<'static, str>, cx: &App) -> UnboundedReceiver<DesktopAuthStatus> {
    #[cfg(feature = "native-performance")]
    crate::native_performance::auth_probe_started(cx);
    let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
    let (tx, rx) = unbounded_channel();
    #[cfg(all(test, unix))]
    let completed = probe_tests::completion(cx);
    runtime.spawn(async move {
        let _ = tx.send(fetch_auth_status_with(&program).await);
        #[cfg(all(test, unix))]
        if let Some(completed) = completed {
            let _ = completed.send(());
        }
    });
    rx
}

impl ChatView {
    #[cfg(all(test, unix))]
    pub(crate) fn install_auth_probe_for_test(
        cx: &mut App,
        program: String,
    ) -> std::sync::mpsc::Receiver<()> {
        probe_tests::install(cx, program)
    }

    #[cfg(all(test, unix))]
    pub(crate) fn clear_auth_probe_for_test(cx: &mut App) {
        probe_tests::clear(cx);
    }

    pub(super) fn start_initial_claude_auth_check(&mut self, cx: &mut Context<'_, Self>) {
        let Some(program) = auth_check_program(cx, self.provider_enabled_for_chat(AgentProvider::Claude)) else {
            cx.notify();
            return;
        };
        #[cfg(all(test, unix))]
        if probe_tests::use_original_checks(cx) {
            self.start_auth_check_with(program, cx);
            return;
        }
        #[cfg(feature = "native-performance")]
        if crate::native_performance::auth_fixture_original(cx) {
            self.start_auth_check_with(program, cx);
            return;
        }

        self.initial_auth_pending = true;
        let entity = cx.weak_entity();
        let pending = &mut cx.default_global::<InitialAuthChecks>().pending;
        if let Some(waiters) = pending {
            waiters.push(entity);
            cx.notify();
            return;
        }
        *pending = Some(vec![entity]);
        let mut rx = spawn_auth_check(program, cx);
        // Delivery belongs to the app: closing the first chat must not strand others.
        cx.spawn(async move |_, cx| {
            let status = rx.recv().await.unwrap_or_else(|| {
                parse_desktop_auth_status(
                    "",
                    "",
                    Some("Claude auth status check stopped. Check again to retry."),
                )
            });
            cx.update(|cx| {
                let waiters = cx
                    .global_mut::<InitialAuthChecks>()
                    .pending
                    .take()
                    .expect("An initial auth completion must own the pending flight");
                for entity in waiters {
                    entity
                        .update(cx, |view, cx| {
                            if !view.initial_auth_pending {
                                return;
                            }
                            view.initial_auth_pending = false;
                            let previous = view.auth_status.message().map(str::to_string);
                            view.auth_status =
                                desktop_auth_status_after_check(status.clone(), previous);
                            cx.notify();
                            #[cfg(feature = "native-performance")]
                            crate::native_performance::auth_status_delivered(cx);
                        })
                        .ok();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn start_claude_auth_check(&mut self, cx: &mut Context<'_, Self>) {
        self.initial_auth_pending = false;
        if let Some(program) = auth_check_program(cx, self.provider_enabled_for_chat(AgentProvider::Claude)) {
            self.start_auth_check_with(program, cx);
        } else {
            cx.notify();
        }
    }

    fn start_auth_check_with(&mut self, program: Cow<'static, str>, cx: &mut Context<'_, Self>) {
        let previous_message = self.auth_status.message().map(str::to_string);
        let mut rx = spawn_auth_check(program, cx);
        cx.spawn(async move |entity, cx| {
            if let Some(status) = rx.recv().await {
                entity
                    .update(cx, |view, cx| {
                        view.auth_status =
                            desktop_auth_status_after_check(status, previous_message);
                        cx.notify();
                        #[cfg(feature = "native-performance")]
                        crate::native_performance::auth_status_delivered(cx);
                    })
                    .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn prepare_claude_login(&mut self) -> bool {
        // The login button only shows while signed out.
        let DesktopAuthStatus::SignedOut(signed_out) = &mut self.auth_status else {
            return false;
        };
        if signed_out.login_in_flight {
            return false;
        }
        self.initial_auth_pending = false;
        signed_out.login_in_flight = true;
        signed_out.message = Some(CLAUDE_LOGIN_WAITING_MESSAGE.to_string());
        true
    }

    #[cfg(all(feature = "native-performance", target_os = "macos"))]
    pub(crate) fn native_fixture_auth_user(&self) -> Option<&str> {
        if self.initial_auth_pending {
            return None;
        }
        self.auth_status.user()
    }

    pub(super) fn start_claude_login(&mut self, cx: &mut Context<'_, Self>) {
        if !self.prepare_claude_login() {
            return;
        }
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let (tx, mut rx) = unbounded_channel();
        runtime.spawn(async move {
            let _ = tx.send(run_claude_login_and_fetch_status().await);
        });
        cx.spawn(async move |entity, cx| {
            if let Some(status) = rx.recv().await {
                entity
                    .update(cx, |view, cx| {
                        view.auth_status = status;
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
        cx.notify();
    }

    /// The sign-in banner for the selected model's CLI, as on the web: Claude
    /// and Codex offer a login here; other runtimes manage it in their CLI.
    pub(super) fn render_auth_banner(&self, cx: &Context<'_, Self>) -> AnyElement {
        match AgentProvider::for_model(&self.model_id) {
            AgentProvider::Claude => match &self.auth_status {
                DesktopAuthStatus::SignedOut(signed_out) => self.render_cli_auth_banner(
                    CliAuthBanner {
                        provider: AgentProvider::Claude,
                        title: signed_out.error.as_ref().map_or_else(
                            || CLAUDE_AUTH_NOT_AUTHENTICATED_LABEL.to_string(),
                            |error| {
                                bruhs_models::logic::cli_login::sign_in_banner_title(
                                    "Claude",
                                    Some(error),
                                )
                            },
                        ),
                        signed_out,
                        login_label: CLAUDE_LOGIN_ACTION_LABEL,
                        login_id: "desktop-claude-login",
                        recheck_id: "desktop-claude-auth-recheck",
                        recheck_label: "Check Claude authentication again",
                    },
                    cx,
                ),
                DesktopAuthStatus::SignedIn(_) => div().into_any_element(),
            },
            AgentProvider::Codex => match &self.codex_auth {
                Some(DesktopAuthStatus::SignedOut(signed_out)) => self.render_cli_auth_banner(
                    CliAuthBanner {
                        provider: AgentProvider::Codex,
                        title: signed_out.error.as_ref().map_or_else(
                            || codex_auth::CODEX_AUTH_NOT_AUTHENTICATED_LABEL.to_string(),
                            |error| {
                                bruhs_models::logic::cli_login::sign_in_banner_title(
                                    "Codex",
                                    Some(error),
                                )
                            },
                        ),
                        signed_out,
                        login_label: codex_auth::CODEX_LOGIN_ACTION_LABEL,
                        login_id: "desktop-codex-login",
                        recheck_id: "desktop-codex-auth-recheck",
                        recheck_label: "Check Codex authentication again",
                    },
                    cx,
                ),
                _ => div().into_any_element(),
            },
            _ => div().into_any_element(),
        }
    }

    fn start_cli_login(&mut self, provider: AgentProvider, cx: &mut Context<'_, Self>) {
        match provider {
            AgentProvider::Codex => self.start_codex_login(cx),
            _ => self.start_claude_login(cx),
        }
    }

    fn start_cli_auth_check(&mut self, provider: AgentProvider, cx: &mut Context<'_, Self>) {
        match provider {
            AgentProvider::Codex => self.start_codex_auth_check(cx),
            _ => self.start_claude_auth_check(cx),
        }
    }

    fn render_cli_auth_banner(
        &self,
        banner: CliAuthBanner<'_>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let CliAuthBanner {
            provider,
            title,
            signed_out,
            login_label,
            login_id,
            recheck_id,
            recheck_label,
        } = banner;
        let message = signed_out.message.clone();
        let login_in_flight = signed_out.login_in_flight;
        let login_label = if login_in_flight {
            CLAUDE_LOGIN_WAITING_LABEL
        } else {
            login_label
        };

        div()
            .debug_selector(|| "desktop-auth-banner".to_string())
            .mx(px(AUTH_BANNER_X_MARGIN_PX))
            .my(px(AUTH_BANNER_Y_MARGIN_PX))
            .rounded(px(AUTH_BANNER_RADIUS_PX))
            .border_1()
            .border_color(theme::durian_400().opacity(AUTH_BANNER_BORDER_OPACITY))
            .bg(theme::durian_500().opacity(AUTH_BANNER_BG_OPACITY))
            .p(px(AUTH_BANNER_PADDING_PX))
            .font_family(crate::theme::FontFace::Code.with_default(AUTH_BANNER_FONT_FAMILY))
            .text_size(px(AUTH_BANNER_TEXT_SIZE_PX))
            .text_color(theme::durian_100())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(AUTH_BANNER_CONTENT_GAP_PX))
                    .child(
                        svg()
                            .path(assets::ALERT_TRIANGLE_ICON_PATH)
                            .size(px(AUTH_BANNER_ICON_SIZE_PX))
                            .mt(px(AUTH_BANNER_ICON_TOP_OFFSET_PX))
                            .text_color(theme::durian_300())
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::durian_100())
                                    .mb(px(AUTH_BANNER_TITLE_BOTTOM_MARGIN_PX))
                                    .child(title),
                            )
                            .when_some(message, |el, message| {
                                el.child(
                                    div()
                                        .mb(px(AUTH_BANNER_MESSAGE_BOTTOM_MARGIN_PX))
                                        .text_size(px(AUTH_BANNER_TEXT_SIZE_PX))
                                        .text_color(
                                            theme::durian_200()
                                                .opacity(AUTH_BANNER_MESSAGE_OPACITY),
                                        )
                                        .child(message),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(AUTH_BANNER_ACTION_GAP_PX))
                                    .mt(px(AUTH_BANNER_ACTION_TOP_MARGIN_PX))
                                    .child(
                                        web_button(
                                            login_id,
                                            WebButtonVariant::Primary,
                                            WebButtonSize::Small,
                                            ButtonState::disabled_if(login_in_flight),
                                        )
                                        .aria_label(login_label)
                                        .child(login_label)
                                        .when(
                                            !login_in_flight,
                                            |el| {
                                                el.tab_index(0)
                                                    .role(Role::Button)
                                                    .focus_visible(|el| {
                                                        el.border_color(theme::persimmon_400())
                                                    })
                                                    .map(|el| {
                                                        on_press_view(
                                                            el,
                                                            cx,
                                                            move |this, _window, cx| {
                                                                this.start_cli_login(provider, cx);
                                                            },
                                                        )
                                                    })
                                            },
                                        ),
                                    )
                                    .child(
                                        web_button(
                                            recheck_id,
                                            WebButtonVariant::Outline,
                                            WebButtonSize::Small,
                                            ButtonState::Enabled,
                                        )
                                        .aria_label(recheck_label)
                                        .child("Check again")
                                        .tab_index(0)
                                        .role(Role::Button)
                                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                        .map(|el| {
                                            on_press_view(el, cx, move |this, _window, cx| {
                                                this.start_cli_auth_check(provider, cx);
                                            })
                                        }),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn is_claude_model(&self) -> bool {
        AgentProvider::for_model(&self.model_id) == AgentProvider::Claude
    }
}

#[cfg(all(test, unix))]
mod probe_tests;
