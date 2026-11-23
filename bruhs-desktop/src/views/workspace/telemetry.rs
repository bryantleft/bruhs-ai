//! Edit receiver URLs on the server that actually exports each signal.
use super::*;
use crate::{app_state::spawn_tokio, server_client::ServerClient};
use bruhs_models::telemetry::{ExportSettings, SetEndpoint, Signal};

pub(super) struct Editor {
    fields: [Entity<InputState>; 3],
    settings: Option<ExportSettings>,
    messages: [Option<String>; 3],
    working: [bool; 3],
    loading: bool,
    error: Option<String>,
}
impl Editor {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, Workspace>) -> Self {
        Self {
            fields: Signal::ALL.map(|s| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(format!("http://localhost:4318/{}", s.path()))
                })
            }),
            settings: None,
            messages: Default::default(),
            working: [false; 3],
            loading: false,
            error: None,
        }
    }
}
impl Workspace {
    pub(super) fn read_export_settings(&mut self, cx: &mut Context<'_, Self>) {
        if self.telemetry.loading || self.telemetry.working.iter().any(|busy| *busy) {
            return;
        }
        self.telemetry.loading = true;
        let before = self
            .telemetry
            .fields
            .each_ref()
            .map(|f| f.read(cx).value().to_string());
        let request = spawn_tokio(cx, async {
            ServerClient::workbench_client().export_settings(None).await
        });
        let window = self.workflow_window;
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = window.update(cx, |_, window, cx| {
                this.update(cx, |this, cx| {
                    let editor = &mut this.telemetry;
                    editor.loading = false;
                    match result {
                        Ok(settings) => {
                            for s in Signal::ALL {
                                let i = s.index();
                                let value = &settings.signals[i].url;
                                // A refresh may finish after the owner starts typing.
                                let untouched = editor.fields[i].read(cx).value().as_ref()
                                    == before[i]
                                    && editor
                                        .settings
                                        .as_ref()
                                        .is_none_or(|old| old.signals[i].url == before[i]);
                                if untouched || settings.signals[i].environment.is_some() {
                                    editor.fields[i]
                                        .update(cx, |f, cx| f.set_value(value.clone(), window, cx));
                                }
                            }
                            editor.settings = Some(settings);
                            editor.error = None;
                        }
                        Err(e) => editor.error = Some(settings_ui::friendly_server_error(&e)),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }
    fn run_export_action(&mut self, signal: Signal, check: bool, cx: &mut Context<'_, Self>) {
        let i = signal.index();
        let editor = &mut self.telemetry;
        if editor.working[i] || editor.loading {
            return;
        }
        let Some(settings) = &editor.settings else {
            return;
        };
        if !check && settings.signals[i].environment.is_some() {
            return;
        }
        let url = editor.fields[i].read(cx).value().trim().to_owned();
        if !(url.is_empty() && !check)
            && !reqwest::Url::parse(&url).is_ok_and(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u.host().is_some()
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.fragment().is_none()
            })
        {
            editor.messages[i] = Some("Enter an HTTP or HTTPS receiver URL.".into());
            cx.notify();
            return;
        }
        editor.working[i] = true;
        editor.messages[i] = None;
        let submitted = url.clone();
        let request = spawn_tokio(cx, async move {
            let c = ServerClient::workbench_client();
            let change = SetEndpoint { signal, url };
            if check {
                c.check_export_endpoint(&change).await.map(|_| None)
            } else {
                c.export_settings(Some(&change)).await.map(Some)
            }
        });
        let window = self.workflow_window;
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = window.update(cx, |_, w, cx| {
                this.update(cx, |this, cx| {
                    let editor = &mut this.telemetry;
                    editor.working[i] = false;
                    let unchanged = editor.fields[i].read(cx).value().trim() == submitted;
                    let message = match result {
                        Ok(Some(saved)) => {
                            let endpoint = saved.signals[i].clone();
                            if unchanged {
                                editor.fields[i].update(cx, |field, cx| {
                                    field.set_value(endpoint.url.clone(), w, cx)
                                });
                            }
                            if let Some(current) = &mut editor.settings {
                                current.signals[i] = endpoint;
                            }
                            if submitted.is_empty() {
                                "Export disabled.".into()
                            } else {
                                "Saved on this host.".into()
                            }
                        }
                        Ok(None) => "Receiver accepted the check.".into(),
                        Err(e) => e
                            .conflict_message()
                            .unwrap_or_else(|| settings_ui::friendly_server_error(&e)),
                    };
                    if unchanged {
                        editor.messages[i] = Some(message);
                    }
                    cx.notify();
                })
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_export_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let editor = &self.telemetry;
        let mut rows = vec![self.settings_row(
            "settings-diagnostic-export",
            "OpenTelemetry export",
            "Send host traces, metrics, and logs to OTLP HTTP receivers.",
            on_press_view(
                web_button(
                    "diagnostic-export-refresh",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(editor.loading),
                )
                .aria_label("Refresh export settings")
                .child("Refresh"),
                cx,
                |this, _, cx| this.read_export_settings(cx),
            )
            .into_any_element(),
        )];
        if let Some(error) = &editor.error {
            rows.push(div().px_3().py_2().child(error.clone()).into_any_element());
        }
        for signal in Signal::ALL {
            let i = signal.index();
            let configured = editor
                .settings
                .as_ref()
                .map(|settings| &settings.signals[i]);
            let env = configured.and_then(|e| e.environment.as_deref());
            let waiting = editor.working[i] || configured.is_none() || editor.loading;
            let form = div()
                .min_w_0()
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    Input::new(&editor.fields[i])
                        .aria_label(format!("{} receiver URL", signal.label()))
                        .disabled(env.is_some() || configured.is_none()),
                )
                .child(
                    div().flex().flex_wrap().gap_2().children(
                        [(true, "Check"), (false, "Save")]
                            .into_iter()
                            .map(|(check, label)| {
                                let button = web_button(
                                    format!("diagnostic-export-{i}-{label}"),
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::disabled_if(waiting || (!check && env.is_some())),
                                )
                                .aria_label(format!("{label} {} receiver", signal.label()))
                                .child(label);
                                on_press_view(button, cx, move |this, _, cx| {
                                    this.run_export_action(signal, check, cx)
                                })
                            }),
                    ),
                )
                .children(editor.messages[i].clone());
            let description =
                env.map_or_else(|| "Receiver URL".to_owned(), |key| format!("Set by {key}"));
            rows.push(self.settings_row(
                [
                    "settings-export-traces",
                    "settings-export-metrics",
                    "settings-export-logs",
                ][i],
                signal.label(),
                description,
                form.into_any_element(),
            ));
        }
        rows
    }
}
