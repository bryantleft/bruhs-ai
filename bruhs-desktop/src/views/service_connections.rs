//! Native account controls, shared by settings and private operation review.
use super::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::{
    app_state::{spawn_tokio, DesktopState},
    theme,
};
use bruhs_core::secrets::SecretManager;
use bruhs_models::{connections::*, ids::ThreadId};
use gpui::{prelude::*, *};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    Idle,
    Loading,
    SignIn(ServiceProvider),
    Disconnect(ServiceProvider),
}

pub(crate) struct ServiceConnections {
    providers: Vec<ServiceProvider>,
    rows: Vec<ServiceConnection>,
    scope: Option<(ThreadId, String)>,
    work: Work,
    error: Option<String>,
    task: Option<Task<()>>,
}

struct LoginGuard {
    manager: SecretManager,
    id: String,
}
impl Drop for LoginGuard {
    fn drop(&mut self) {
        self.manager.connections().cancel(&self.id);
    }
}

impl ServiceConnections {
    pub fn new(
        provider: Option<ServiceProvider>,
        scope: Option<(ThreadId, String)>,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let mut this = Self {
            providers: provider.map_or_else(|| ServiceProvider::ALL.to_vec(), |p| vec![p]),
            rows: Vec::new(),
            scope,
            work: Work::Idle,
            error: None,
            task: None,
        };
        this.refresh(cx);
        this
    }

    pub fn current(&self, provider: ServiceProvider) -> Option<&ServiceConnection> {
        self.rows.iter().find(|row| row.provider == provider)
    }

    pub fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        if self.work != Work::Idle {
            return;
        }
        self.work = Work::Loading;
        let manager = cx
            .global::<DesktopState>()
            .app
            .secrets()
            .connections()
            .clone();
        let operation = spawn_tokio(cx, async move { manager.list().await });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.work = Work::Idle;
                match result {
                    Ok(rows) => {
                        this.rows = rows;
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
    }

    fn connect(&mut self, provider: ServiceProvider, cx: &mut Context<'_, Self>) {
        if self.work != Work::Idle {
            return;
        }
        self.work = Work::SignIn(provider);
        self.error = None;
        let manager = cx.global::<DesktopState>().app.secrets().clone();
        let scope = self.scope.clone();
        let operation = spawn_tokio(cx, async move {
            let started = if let Some((thread, request)) = scope {
                manager.connect_request(&thread, &request, provider).await
            } else {
                manager
                    .connections()
                    .start(provider)
                    .await
                    .map_err(bruhs_core::secrets::SecretError::from)
            }?;
            let guard = LoginGuard {
                manager,
                id: started.id.clone(),
            };
            Ok::<_, bruhs_core::secrets::SecretError>((started, guard))
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let (started, guard) = match operation.await {
                Ok(result) => result,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.work = Work::Idle;
                        this.error = Some(error.to_string());
                        cx.notify();
                    });
                    return;
                }
            };
            if this
                .update(cx, |_, cx| cx.open_url(&started.auth_url))
                .is_err()
            {
                return;
            }
            loop {
                let status = guard.manager.connections().attempt(&started.id);
                match status {
                    Ok(status) if status.state == AttemptState::Pending => {}
                    result => {
                        let _ = this.update(cx, |this, cx| {
                            this.work = Work::Idle;
                            this.error = match result {
                                Ok(status) => status.message,
                                Err(error) => Some(error.to_string()),
                            };
                            let manager = guard.manager.connections().clone();
                            let operation = spawn_tokio(cx, async move { manager.list().await });
                            this.task = Some(cx.spawn(async move |this, cx| {
                                let result = operation.await;
                                let _ = this.update(cx, |this, cx| {
                                    match result {
                                        Ok(rows) => this.rows = rows,
                                        Err(error) => this.error = Some(error.to_string()),
                                    };
                                    cx.notify();
                                });
                            }));
                            cx.notify();
                        });
                        break;
                    }
                }
                // Same polling cadence as the existing Calendar connect UI.
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
            }
        }));
        cx.notify();
    }

    fn disconnect(&mut self, provider: ServiceProvider, cx: &mut Context<'_, Self>) {
        if self.work != Work::Idle {
            return;
        }
        self.work = Work::Disconnect(provider);
        self.error = None;
        let manager = cx
            .global::<DesktopState>()
            .app
            .secrets()
            .connections()
            .clone();
        let operation = spawn_tokio(cx, async move {
            manager.disconnect(provider).await?;
            manager.list().await
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.work = Work::Idle;
                match result {
                    Ok(rows) => this.rows = rows,
                    Err(error) => this.error = Some(error.to_string()),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn button(
        &self,
        provider: ServiceProvider,
        action: &'static str,
        label: &'static str,
        disabled: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        web_button(
            SharedString::from(format!("service-{}-{action}", provider.id())),
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(disabled),
        )
        .min_h(px(40.0))
        .aria_label(format!("{label} {}", provider.name()))
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| match action {
            "connect" => this.connect(provider, cx),
            "disconnect" => this.disconnect(provider, cx),
            "cancel" => {
                this.cancel(cx);
            }
            _ => this.refresh(cx),
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                match action {
                    "connect" => this.connect(provider, cx),
                    "disconnect" => this.disconnect(provider, cx),
                    "cancel" => {
                        this.cancel(cx);
                    }
                    _ => this.refresh(cx),
                }
            }
        }))
    }

    fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(self.work, Work::SignIn(_)) {
            self.task = None;
            self.work = Work::Idle;
            self.error = None;
            cx.notify();
        }
    }
}

impl Render for ServiceConnections {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .providers
            .iter()
            .map(|provider| {
                let provider = *provider;
                let row = self.current(provider);
                let configured = row.is_some_and(|r| r.configured);
                let connected = row.is_some_and(|r| r.id.is_some());
                let status = if self.work == Work::Disconnect(provider) {
                    "Disconnecting…"
                } else if self.work == Work::SignIn(provider) {
                    "Waiting for browser sign-in…"
                } else {
                    match row {
                        Some(row) if !row.configured => {
                            "This service is not set up on this device."
                        }
                        Some(row) => match row.state {
                            ConnectionState::Connected => "Connected",
                            ConnectionState::Disconnected => "Not connected",
                            ConnectionState::NeedsSignIn => "Needs sign-in",
                            ConnectionState::Unavailable => "Keychain needs attention",
                        },
                        None => "Loading account…",
                    }
                };
                div()
                    .id(SharedString::from(format!("service-{}", provider.id())))
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().font_weight(FontWeight::MEDIUM).child(provider.name()))
                    .child(
                        div()
                            .text_color(theme::rambutan_400())
                            .child(format!("Requested access: {}", provider.access_summary())),
                    )
                    .child(
                        div().text_color(theme::rambutan_400()).child(
                            row.and_then(|r| r.account.clone())
                                .unwrap_or_else(|| "No account connected".into()),
                        ),
                    )
                    .child(
                        div()
                            .id("service-account-status")
                            .role(Role::Status)
                            .child(status),
                    )
                    .when_some(row.map(|r| r.scopes.join(", ")), |element, scopes| {
                        element.child(
                            div()
                                .text_color(theme::rambutan_400())
                                .child(format!("Permissions: {scopes}")),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(self.button(
                                provider,
                                "connect",
                                if connected { "Reconnect" } else { "Connect" },
                                self.work != Work::Idle || !configured,
                                cx,
                            ))
                            .when(self.work == Work::SignIn(provider), |row| {
                                row.child(self.button(provider, "cancel", "Cancel", false, cx))
                            })
                            .when(connected, |row| {
                                row.child(self.button(
                                    provider,
                                    "disconnect",
                                    "Disconnect",
                                    self.work != Work::Idle,
                                    cx,
                                ))
                            })
                            .child(self.button(
                                provider,
                                "refresh",
                                "Refresh",
                                self.work != Work::Idle,
                                cx,
                            )),
                    )
            })
            .collect();
        div().id("service-connections").font_family(theme::UI_SMALL_FONT_FAMILY).text_size(px(12.0)).text_color(theme::rambutan_200()).flex().flex_col().gap_2()
            .child(div().font_weight(FontWeight::MEDIUM).child("Connected accounts"))
            .child("Connect in your browser to reuse an existing sign-in. Your provider may still require consent or MFA. Each agent operation needs approval.")
            .children(rows)
            .when_some(self.error.clone(), |element, error| element.child(div().id("service-connection-error").role(Role::Alert).text_color(theme::lychee_300()).child(error)))
            .child(div().text_color(theme::rambutan_400()).child("Disconnect removes this local connection. It keeps your browser signed in and does not revoke access at the service."))
    }
}
