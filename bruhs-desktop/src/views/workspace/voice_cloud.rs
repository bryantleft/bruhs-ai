//! Explicitly select the service that receives a stopped dictation recording.
use super::*;
use crate::voice_capture::Destination;
use bruhs_models::cloud_speech::{KeyChange, Provider, SpeechModel};

pub(super) fn provider(id: &str) -> Option<Provider> {
    [Provider::OpenAi, Provider::Xai, Provider::OpenRouter]
        .into_iter()
        .find(|p| p.id() == id)
}
pub(super) struct Editor {
    provider: Option<Provider>,
    key: Option<Entity<InputState>>,
    query: Entity<InputState>,
    configured: bool,
    models: Vec<SpeechModel>,
    selected: String,
    busy: bool,
    error: Option<String>,
    revision: u64,
    removing: bool,
}
impl Editor {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
        subs: &mut Vec<Subscription>,
    ) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search transcription models"));
        subs.push(cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        Self {
            provider: None,
            key: None,
            query,
            configured: false,
            models: vec![],
            selected: String::new(),
            busy: false,
            error: None,
            revision: 0,
            removing: false,
        }
    }
    pub(super) fn dismiss(&mut self) {
        self.provider = None;
        self.key = None;
        self.revision = self.revision.wrapping_add(1);
        self.busy = false;
        self.removing = false;
    }
    pub(super) fn close(&mut self, window: &mut Window, cx: &mut App) {
        self.dismiss();
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
    }
}
impl Workspace {
    pub(super) fn open_voice_cloud(
        &mut self,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.voice_cloud.close(window, cx);
        let editor = &mut self.voice_cloud;
        editor.provider = Some(provider);
        editor.key = Some(cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("API key")
        }));
        editor.configured = false;
        editor.selected = self.dictation_prefs.openrouter_model.clone();
        editor.models.clear();
        editor.error = None;
        editor.busy = true;
        let revision = editor.revision;
        let request = crate::app_state::spawn_tokio(cx, async move {
            let key = bruhs_core::capture_client::speech_key(provider, None);
            let models = async move {
                if provider == Provider::OpenRouter {
                    bruhs_core::capture_client::cloud_speech_models().await
                } else {
                    Ok(Vec::new())
                }
            };
            tokio::join!(key, models)
        });
        cx.spawn(async move |this, cx| {
            let (key, models) = request.await;
            let _ = this.update(cx, |this, cx| {
                let editor = &mut this.voice_cloud;
                if editor.revision != revision {
                    return;
                }
                editor.busy = false;
                match key {
                    Ok(status) => editor.configured = status.configured,
                    Err(e) => editor.error = Some(e.to_string()),
                }
                match models {
                    Ok(models) => editor.models = models,
                    Err(e) => editor.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn apply_voice_cloud(&mut self, remove: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let editor = &mut self.voice_cloud;
        let Some(provider) = editor.provider else {
            return;
        };
        if editor.busy {
            return;
        }
        let selected = editor.selected.clone();
        if !remove
            && provider == Provider::OpenRouter
            && !editor.models.iter().any(|m| m.id == selected)
        {
            editor.error = Some("Select an available transcription model.".into());
            cx.notify();
            return;
        }
        let change = if remove {
            Some(KeyChange::Remove)
        } else {
            let value = editor
                .key
                .as_ref()
                .map(|input| input.read(cx).value().trim().to_string())
                .unwrap_or_default();
            if value.is_empty() && !editor.configured {
                editor.error = Some("Enter an API key to use this provider.".into());
                cx.notify();
                return;
            }
            (!value.is_empty()).then_some(KeyChange::Save(value))
        };
        editor.busy = true;
        editor.error = None;
        if let Some(key) = &editor.key {
            key.update(cx, |input, cx| input.set_value("", window, cx));
        }
        let revision = editor.revision;
        let req = crate::app_state::spawn_tokio(cx, async move {
            bruhs_core::capture_client::speech_key(provider, change).await
        });
        cx.spawn(async move |this, cx| {
            let result = req.await;
            let _ = this.update(cx, |this, cx| {
                if this.voice_cloud.revision != revision {
                    return;
                }
                this.voice_cloud.busy = false;
                match result {
                    Ok(status) if status.configured && !remove => {
                        this.dictation_prefs.destination = Destination::Cloud(provider);
                        if provider == Provider::OpenRouter {
                            this.dictation_prefs.openrouter_model = selected;
                        }
                        this.voice_cloud.dismiss();
                        this.stamp_run_prefs_on_threads(cx);
                        this.persist_and_notify(cx);
                    }
                    Ok(_) if remove => {
                        if this.dictation_prefs.destination == Destination::Cloud(provider) {
                            this.dictation_prefs.destination = Destination::Off;
                        }
                        this.voice_cloud.dismiss();
                        this.stamp_run_prefs_on_threads(cx);
                        this.persist_and_notify(cx);
                    }
                    Ok(_) => {
                        this.voice_cloud.error =
                            Some("The saved key is unavailable. Enter a new key.".into())
                    }
                    Err(e) => this.voice_cloud.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn voice_cloud_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let state = &self.voice_cloud;
        let Some(provider) = state.provider else {
            let Destination::Cloud(provider) = self.dictation_prefs.destination else {
                return vec![];
            };
            let button = web_button(
                "voice-manage-cloud",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .aria_label("Manage cloud dictation")
            .child("Manage");
            return vec![self.settings_row(
                "voice-cloud-active",
                "Cloud dictation",
                format!(
                    "{} · {}",
                    provider.label(),
                    provider
                        .fixed_model()
                        .unwrap_or(&self.dictation_prefs.openrouter_model)
                ),
                on_press_view(button, cx, move |this, window, cx| {
                    this.open_voice_cloud(provider, window, cx)
                })
                .into_any_element(),
            )];
        };
        let mut body=div().mx_3().mb_3().flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(theme::longan_700())
            .child(format!("Use {} for dictation",provider.label()))
            .child(div().text_xs().text_color(theme::rambutan_400()).child(format!("{} receives the recording when you stop. Your key is stored in the capture host’s credential store.",provider.label())));
        if let Some(key) = &state.key {
            body = body.child(
                Input::new(key)
                    .aria_label(format!("{} API key", provider.label()))
                    .disabled(state.busy),
            );
        }
        if state.configured {
            body = body.child(
                div()
                    .text_xs()
                    .child("A key is saved. Leave the field empty to keep it."),
            );
        }
        if provider == Provider::OpenRouter {
            body = body.child(Input::new(&state.query).aria_label("Search OpenRouter models"));
            let query = state.query.read(cx).value().trim().to_lowercase();
            let mut list = div()
                .id("voice-cloud-models")
                .overflow_y_scroll()
                .max_h(px(155.))
                .flex()
                .flex_col()
                .gap_1();
            let mut matches = false;
            for model in state.models.iter().filter(|m| {
                m.name.to_lowercase().contains(&query) || m.id.to_lowercase().contains(&query)
            }) {
                matches = true;
                let id = model.id.clone();
                let choice = web_button(
                    SharedString::from(format!("dictate-{id}")),
                    if id == state.selected {
                        WebButtonVariant::Outline
                    } else {
                        WebButtonVariant::Ghost
                    },
                    WebButtonSize::Small,
                    ButtonState::disabled_if(state.busy),
                )
                .aria_label(model.name.clone())
                .child(model.name.clone());
                list = list.child(on_press_view(choice, cx, move |this, _, cx| {
                    this.voice_cloud.selected = id.clone();
                    cx.notify();
                }));
            }
            if !matches && !state.busy {
                list = list.child("No transcription models match.");
            }
            body = body.child(list);
        }
        if let Some(error) = &state.error {
            body = body.child(div().text_xs().child(error.clone()));
        }
        let mut buttons = div().flex().gap_2().flex_wrap();
        for (id, label) in [
            ("voice-cloud-use", "Use provider"),
            ("voice-cloud-cancel", "Cancel"),
            (
                "voice-cloud-remove",
                if state.removing {
                    "Confirm removal"
                } else {
                    "Remove key"
                },
            ),
            ("voice-cloud-retry", "Refresh"),
        ] {
            if id == "voice-cloud-remove" && !state.configured {
                continue;
            }
            if id == "voice-cloud-retry" && state.error.is_none() {
                continue;
            }
            let button = web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(state.busy && id != "voice-cloud-cancel"),
            )
            .aria_label(label)
            .child(label);
            buttons = buttons.child(on_press_view(
                button,
                cx,
                move |this, window, cx| match id {
                    "voice-cloud-use" => this.apply_voice_cloud(false, window, cx),
                    "voice-cloud-remove" if this.voice_cloud.removing => {
                        this.apply_voice_cloud(true, window, cx)
                    }
                    "voice-cloud-remove" => {
                        this.voice_cloud.removing = true;
                        cx.notify();
                    }
                    "voice-cloud-retry" => this.open_voice_cloud(provider, window, cx),
                    _ => {
                        this.voice_cloud.close(window, cx);
                        cx.notify();
                    }
                },
            ));
        }
        vec![body.child(buttons).into_any_element()]
    }
}
