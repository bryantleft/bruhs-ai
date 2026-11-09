//! Claude CLI sign-in: status checks, the login flow with polling, and the banner.

use super::*;
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
pub(super) const CLAUDE_AUTH_NOT_AUTHENTICATED_LABEL: &str = "Claude Code not authenticated.";
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

    #[cfg(test)]
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

#[cfg(not(test))]
pub(super) async fn fetch_desktop_auth_status() -> DesktopAuthStatus {
    fetch_auth_status_with(CLAUDE_PROGRAM).await
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
            "Still not authenticated. Run `claude auth login` in a terminal, then click Re-check."
                .to_string()
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                "Claude login failed. Run `claude auth login` in a terminal, then click Re-check."
                    .to_string()
            } else {
                format!("Login failed: {stderr}")
            }
        }
        Ok(Err(error)) => format!("Login failed: {}", normalize_claude_command_error(&error)),
        Err(_) => {
            "Claude login timed out. Run `claude auth login` in a terminal, then click Re-check."
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
        let authenticated = value
            .get("authenticated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            || value
                .get("loggedIn")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            || matches!(
                value.get("status").and_then(serde_json::Value::as_str),
                Some("authenticated") | Some("active")
            )
            || value.get("account").is_some()
            || value.get("email").is_some()
            || value.get("user").is_some();

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

impl ChatView {
    #[cfg(not(test))]
    pub(super) fn start_claude_auth_check(&mut self, cx: &mut Context<'_, Self>) {
        let previous_message = self.auth_status.message().map(str::to_string);
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let (tx, mut rx) = unbounded_channel();
        runtime.spawn(async move {
            let _ = tx.send(fetch_desktop_auth_status().await);
        });
        cx.spawn(async move |entity, cx| {
            if let Some(status) = rx.recv().await {
                entity
                    .update(cx, |view, cx| {
                        view.auth_status =
                            desktop_auth_status_after_check(status, previous_message);
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn start_claude_auth_check(&mut self, cx: &mut Context<'_, Self>) {
        cx.notify();
    }

    pub(super) fn start_claude_login(&mut self, cx: &mut Context<'_, Self>) {
        // The login button only shows while signed out.
        let DesktopAuthStatus::SignedOut(signed_out) = &mut self.auth_status else {
            return;
        };
        if signed_out.login_in_flight {
            return;
        }
        signed_out.login_in_flight = true;
        signed_out.message = Some(CLAUDE_LOGIN_WAITING_MESSAGE.to_string());
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

    pub(super) fn render_auth_banner(&self, cx: &Context<'_, Self>) -> AnyElement {
        let DesktopAuthStatus::SignedOut(signed_out) = &self.auth_status else {
            return div().into_any_element();
        };
        if !auth_banner_visible_like_web(&self.model_id, false) {
            return div().into_any_element();
        }

        let title = if let Some(error) = &signed_out.error {
            format!("Could not check auth: {error}")
        } else {
            CLAUDE_AUTH_NOT_AUTHENTICATED_LABEL.to_string()
        };
        let message = signed_out.message.clone();
        let login_in_flight = signed_out.login_in_flight;
        let login_label = if login_in_flight {
            CLAUDE_LOGIN_WAITING_LABEL
        } else {
            CLAUDE_LOGIN_ACTION_LABEL
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
            .font_family(AUTH_BANNER_FONT_FAMILY)
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
                                            "desktop-claude-login",
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
                                                        el.border_2()
                                                            .border_color(theme::persimmon_400())
                                                    })
                                                    .on_click(cx.listener(
                                                        |this, _ev, _window, cx| {
                                                            this.start_claude_login(cx);
                                                        },
                                                    ))
                                                    .on_key_down(cx.listener(
                                                        |this, event: &KeyDownEvent, window, cx| {
                                                            if is_web_button_activation_key(
                                                                event.keystroke.key.as_str(),
                                                            ) {
                                                                window.prevent_default();
                                                                cx.stop_propagation();
                                                                this.start_claude_login(cx);
                                                            }
                                                        },
                                                    ))
                                            },
                                        ),
                                    )
                                    .child(
                                        web_button(
                                            "desktop-claude-auth-recheck",
                                            WebButtonVariant::Outline,
                                            WebButtonSize::Small,
                                            ButtonState::Enabled,
                                        )
                                        .aria_label("Re-check Claude authentication")
                                        .child("Re-check")
                                        .tab_index(0)
                                        .role(Role::Button)
                                        .focus_visible(|el| {
                                            el.border_2().border_color(theme::persimmon_400())
                                        })
                                        .on_click(cx.listener(|this, _ev, _window, cx| {
                                            this.start_claude_auth_check(cx);
                                        }))
                                        .on_key_down(
                                            cx.listener(
                                                |this, event: &KeyDownEvent, window, cx| {
                                                    if is_web_button_activation_key(
                                                        event.keystroke.key.as_str(),
                                                    ) {
                                                        window.prevent_default();
                                                        cx.stop_propagation();
                                                        this.start_claude_auth_check(cx);
                                                    }
                                                },
                                            ),
                                        ),
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
