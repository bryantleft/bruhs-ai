//! Hardware and model residency are owned by the capture host.
use super::*;
use bruhs_models::dictation_runtime::{Change, Hardware, Status, Unload};

#[derive(Default)]
pub(super) struct Runtime {
    status: Option<Status>,
    busy: bool,
    error: Option<String>,
}
const HARDWARE: &str = "settings-voice-hardware";
const UNLOAD: &str = "settings-voice-unload";

impl Workspace {
    pub(super) fn refresh_voice_runtime(
        &mut self,
        change: Option<Change>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.voice_runtime.busy {
            return;
        }
        self.voice_runtime.busy = true;
        self.voice_runtime.error = None;
        let request = crate::app_state::spawn_tokio(cx, async move {
            bruhs_core::capture_client::dictation_runtime(change).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.voice_runtime;
                state.busy = false;
                match result {
                    Ok(status) => {
                        state.status = Some(status);
                        state.error = None;
                    }
                    Err(error) => {
                        let friendly = settings_ui::friendly_server_error(&error);
                        state.error = Some(if friendly == "Local server unavailable." {
                            "Capture host unavailable.".to_owned()
                        } else {
                            format!("Capture host unavailable. {friendly}")
                        });
                        state.status = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn select_voice_runtime(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let change = match id {
            HARDWARE => match value {
                "Auto" => Change::Hardware(Hardware::Auto),
                "CPU" => Change::Hardware(Hardware::Cpu),
                "GPU" => Change::Hardware(Hardware::Gpu),
                _ => return false,
            },
            UNLOAD => {
                let Some((policy, _)) = Unload::CHOICES.iter().find(|(_, label)| *label == value)
                else {
                    return false;
                };
                Change::Unload(*policy)
            }
            _ => return false,
        };
        self.refresh_voice_runtime(Some(change), cx);
        true
    }
    pub(super) fn compatible_description(&self) -> String {
        match &self.voice_runtime.status {
            Some(status) => status.compatible_origin.as_ref().map_or_else(
                || "Configure BRUHS_TRANSCRIPTION_BASE_URL and BRUHS_TRANSCRIPTION_TIMEOUT_SECS on the capture host. Set BRUHS_TRANSCRIPTION_API_KEY if the service needs a key.".into(),
                |origin| format!("Recordings are sent to {origin}. Enter a model ID supported by that server."),
            ),
            None=>self.voice_runtime.error.clone().unwrap_or_else(|| "Reading the capture host’s transcription server…".into()),
        }
    }

    pub(super) fn voice_runtime_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let state = &self.voice_runtime;
        let Some(status) = &state.status else {
            let description = state
                .error
                .clone()
                .unwrap_or_else(|| "Reading capture host settings…".into());
            let button = web_button(
                "voice-runtime-retry",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(state.busy),
            )
            .aria_label("Refresh transcription hardware")
            .child("Refresh");
            return vec![
                self.settings_row(
                    "desktop-settings-row-voice-hardware",
                    "Transcription hardware",
                    description,
                    on_press_view(button, cx, |this, _, cx| {
                        this.refresh_voice_runtime(None, cx)
                    })
                    .into_any_element(),
                ),
                self.settings_row_opts(
                    "desktop-settings-row-voice-unload",
                    "Unload model",
                    "Unload the dictation model after idle time.",
                    None,
                    true,
                    div()
                        .text_xs()
                        .child("Capture host required")
                        .into_any_element(),
                ),
            ];
        };
        let mut choices = vec![
            DropdownOption {
                value: "Auto".into(),
                label: "Auto".into(),
                description: None,
            },
            DropdownOption {
                value: "CPU".into(),
                label: "CPU".into(),
                description: None,
            },
        ];
        if let Some(name) = &status.gpu {
            choices.push(DropdownOption {
                value: "GPU".into(),
                label: name.clone(),
                description: None,
            });
        }
        let (toggle, select) = self.settings_dropdown_handlers(HARDWARE, cx);
        let hardware = match status.settings.hardware {
            Hardware::Auto => "Auto",
            Hardware::Cpu => "CPU",
            Hardware::Gpu => "GPU",
        };
        let policy = Unload::CHOICES
            .iter()
            .find(|(policy, _)| *policy == status.settings.unload)
            .map_or("Immediately", |(_, name)| *name);
        let unload: Vec<_> = Unload::CHOICES
            .iter()
            .map(|(_, name)| (*name, *name))
            .collect();
        let control = |element: AnyElement| {
            if state.busy {
                div().text_xs().child("Saving…").into_any_element()
            } else {
                settings_ui::control_column(element)
            }
        };
        vec![
            self.settings_row(
                "desktop-settings-row-voice-hardware",
                "Transcription hardware",
                "Processor used for local dictation.",
                control(dropdown(
                    HARDWARE,
                    "Transcription hardware",
                    hardware,
                    vec![DropdownGroup {
                        label: None,
                        mark: None,
                        options: choices,
                    }],
                    self.settings_listbox(HARDWARE),
                    toggle,
                    select,
                )),
            ),
            self.settings_row(
                "desktop-settings-row-voice-unload",
                "Unload model",
                "Unload the dictation model after idle time.",
                control(self.choice_control(UNLOAD, "Unload model", policy, &unload, cx)),
            ),
        ]
    }
}
