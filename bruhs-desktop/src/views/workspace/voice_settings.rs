//! Voice preferences apply to composer dictation independently of meeting recordings.
use super::*;
use crate::voice_capture::Destination;

mod microphones;

impl Workspace {
    pub(super) fn refresh_voice_devices(&mut self, cx: &mut Context<'_, Self>) {
        self.voice_device_generation = self.voice_device_generation.wrapping_add(1);
        let generation = self.voice_device_generation;
        let devices = cx.background_spawn(async { crate::voice_capture::microphone_names() });
        cx.spawn(async move |this, cx| {
            let devices = devices.await;
            let _ = this.update(cx, |this, cx| {
                if this.voice_device_generation == generation {
                    if let Ok(names) = &devices {
                        if this.dictation_prefs.remember_microphones(names) {
                            this.stamp_run_prefs_on_threads(cx);
                            this.persist_and_notify(cx);
                        }
                    }
                    this.voice_devices = Some(devices);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn test_voice_model(&mut self, cx: &mut Context<'_, Self>) {
        if self.voice_testing || self.microphone_test.busy() {
            return;
        }
        if let Err(error) = self.dictation_prefs.selected_service() {
            self.voice_test_result = Some(error.into());
            cx.notify();
            return;
        }
        self.voice_testing = true;
        self.voice_test_result = Some("Recording for two seconds…".into());
        let prefs = self.dictation_prefs.clone();
        let project = self.active_settings_project(cx);
        let project_name = project.as_deref().and_then(|path| {
            self.threads
                .iter()
                .find(|thread| thread.project_path == path)
                .map(|thread| thread.project_name.clone())
        });
        let cleanup_model = self.effective_text_model(project.as_deref(), &self.current_model);
        let microphone = prefs.ordered_microphones();
        let capture = cx.background_spawn(async move {
            crate::voice_capture::record_microphone_sample(
                &microphone,
                std::time::Duration::from_secs(2),
            )
        });
        cx.spawn(async move |this, cx| {
            let result = match capture.await {
                Ok(wav) => {
                    let request = this.update(cx, |this, cx| {
                        this.voice_test_result = Some("Transcribing sample…".into());
                        cx.notify();
                        crate::app_state::spawn_tokio(cx, async move {
                            let reply = prefs
                                .transcribe(
                                    wav,
                                    project.as_deref(),
                                    project_name.as_deref(),
                                    &cleanup_model,
                                    Default::default(),
                                    None,
                                )
                                .await?;
                            Ok::<_, String>(if reply.text.trim().is_empty() {
                                "No speech detected.".to_owned()
                            } else if let Some(warning) = reply.warning {
                                format!("{} · {warning}", reply.text)
                            } else {
                                reply.text
                            })
                        })
                    });
                    let Ok(request) = request else {
                        return;
                    };
                    request.await
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |this, cx| {
                this.voice_testing = false;
                this.voice_test_result =
                    Some(result.unwrap_or_else(|error| format!("Voice test failed. {error}")));
                cx.notify();
            });
        })
        .detach();
    }

    fn voice_language_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let prefs = &self.dictation_prefs;
        let saved = prefs.language.as_deref().unwrap_or("auto");
        let name = |code: &str| {
            LANGUAGES
                .iter()
                .find(|(value, _)| *value == code)
                .map_or_else(|| code.to_owned(), |(_, name)| name.to_string())
        };
        let effective = prefs.options().language;
        let unavailable = saved != "auto" && Some(saved) != effective.as_deref();
        let effective_label = effective
            .as_deref()
            .map_or_else(|| "Auto-detect".into(), name);
        let label = if unavailable {
            format!("{effective_label} ({} unavailable)", name(saved))
        } else if saved == "auto" && effective.is_some() {
            format!("Auto ({effective_label})")
        } else {
            effective_label
        };
        let mut options = vec![DropdownOption {
            value: "auto".into(),
            label: "Auto".into(),
            description: None,
        }];
        if unavailable {
            options.push(DropdownOption {
                value: "unavailable".into(),
                label: format!("{} unavailable", name(saved)),
                description: None,
            });
        }
        options.extend(
            LANGUAGES
                .iter()
                .skip(1)
                .filter(|(code, _)| match prefs.destination {
                    Destination::Cloud(_) => bruhs_models::cloud_speech::LANGUAGES.contains(code),
                    Destination::Local if !prefs.model.descriptor().multilingual => *code == "en",
                    _ => true,
                })
                .map(|&(value, label)| DropdownOption {
                    value: value.into(),
                    label: label.into(),
                    description: None,
                }),
        );
        let id = "settings-voice-language";
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let unavailable = unavailable.then_some("unavailable");
        let picker = bruhs_ui::dropdown::Dropdown::new(
            id,
            "Transcription language",
            unavailable.unwrap_or(saved),
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            self.settings_listbox(id),
            toggle,
            select,
        )
        .selection_label(label, unavailable)
        .into_any_element();
        if unavailable.is_some() {
            div()
                .w(px(320.))
                .max_w_full()
                .min_w_0()
                .child(picker)
                .into_any_element()
        } else {
            settings_ui::control_column(picker)
        }
    }

    pub(super) fn render_voice_settings(&self, cx: &Context<'_, Self>) -> AnyElement {
        let model_id = "settings-voice-model";
        let (toggle, select) = self.settings_dropdown_handlers(model_id, cx);
        let model = settings_ui::control_column(dropdown(
            model_id,
            "Transcription model",
            self.dictation_prefs.model.key(),
            settings_ui::whisper_options(),
            self.settings_listbox(model_id),
            toggle,
            select,
        ));
        let destination = self.dictation_prefs.destination;
        let model_test = web_button(
            "voice-test-model",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(
                self.voice_testing
                    || self.microphone_test.busy()
                    || destination == Destination::Off,
            ),
        )
        .child("Test model");
        let shortcut = self
            .keybindings
            .iter()
            .position(|rule| rule.command == "voice.dictate");
        let mut input = self.microphone_rows(cx);
        let mut transcription = vec![self.settings_row(
            "settings-row-voice-destination",
            "Transcribe with",
            "Whisper runs locally; cloud providers receive audio on stop.",
            settings_ui::control_column(self.choice_control(
                "settings-voice-destination",
                "Transcribe with",
                destination.key(),
                &[
                    ("off", "Off"),
                    ("local", "Local Whisper"),
                    ("openai", "OpenAI"),
                    ("xai", "xAI"),
                    ("openrouter", "OpenRouter"),
                    ("compatible", "Compatible server"),
                ],
                cx,
            )),
        )];
        if destination != Destination::Off {
            let mut env_options = vec![DropdownOption {
                value: "local".into(),
                label: "This computer".into(),
                description: None,
            }];
            for (id, name, connected) in self.linked_projects.host_summaries() {
                env_options.push(DropdownOption {
                    value: id.into(),
                    label: name,
                    description: Some(if connected { "Connected" } else { "Offline" }.into()),
                });
            }
            let current_env = self
                .dictation_prefs
                .transcription_environment
                .as_deref()
                .unwrap_or("local");
            let env_id = "settings-voice-environment";
            let (toggle, select) = self.settings_dropdown_handlers(env_id, cx);
            transcription.push(
                self.settings_row(
                    "settings-row-voice-environment",
                    "Transcription environment",
                    "Choose which machine runs speech transcription for composer dictation.",
                    settings_ui::control_column(
                        bruhs_ui::dropdown::Dropdown::new(
                            env_id,
                            "Transcription environment",
                            current_env,
                            vec![DropdownGroup {
                                label: None,
                                mark: None,
                                options: env_options,
                            }],
                            self.settings_listbox(env_id),
                            toggle,
                            select,
                        )
                        .into_any_element(),
                    ),
                ),
            );
        }
        transcription.extend(self.voice_cloud_rows(cx));
        if destination == Destination::Local {
            transcription.push(self.settings_row(
                "desktop-settings-row-voice-model",
                "Transcription model",
                "Downloaded on first use.",
                model,
            ));
            transcription.extend(self.voice_model_rows(cx));
        } else if destination == Destination::Compatible {
            transcription.push(
                self.settings_row(
                    "settings-row-voice-server-model",
                    "Server transcription model",
                    self.compatible_description(),
                    settings_ui::control_column(
                        Input::new(&self.voice_model_input)
                            .aria_label("Dictation server model")
                            .into_any_element(),
                    ),
                ),
            );
        }
        if destination != Destination::Off {
            transcription.push(self.settings_row(
                "desktop-settings-row-voice-language",
                "Transcription language",
                "",
                self.voice_language_control(cx),
            ));
            transcription.push(self.settings_row(
                "desktop-settings-row-voice-silence",
                "Silence filtering",
                if matches!(
                    destination,
                    Destination::Cloud(bruhs_models::cloud_speech::Provider::OpenAi)
                ) {
                    "Handled automatically by OpenAI."
                } else {
                    "Skip quiet recordings and trim silence."
                },
                settings_ui::switch(
                    "settings-voice-silence",
                    self.dictation_prefs.silence_filtering,
                    matches!(
                        destination,
                        Destination::Cloud(bruhs_models::cloud_speech::Provider::OpenAi)
                    ),
                    "Silence filtering",
                    cx,
                    |this, _, cx| {
                        this.dictation_prefs.silence_filtering =
                            !this.dictation_prefs.silence_filtering;
                        this.stamp_run_prefs_on_threads(cx);
                        this.persist_and_notify(cx);
                    },
                ),
            ));
        }
        let mut cleanup = vec![self.settings_row(
            "desktop-settings-row-voice-fillers",
            "Remove filler words",
            "Strip spoken hesitations during cleanup.",
            settings_ui::switch(
                "settings-voice-fillers",
                self.dictation_prefs.remove_filler_words,
                false,
                "Remove filler words",
                cx,
                |this, _, cx| {
                    this.dictation_prefs.remove_filler_words =
                        !this.dictation_prefs.remove_filler_words;
                    this.stamp_run_prefs_on_threads(cx);
                    this.persist_and_notify(cx);
                },
            ),
        )];
        cleanup.extend(self.cleanup_rows(cx));
        let mode = self.dictation_prefs.shortcut_mode;
        input.push(self.settings_row(
            "desktop-settings-row-dictation-shortcut-mode",
            "Dictation shortcut mode",
            mode.hint(),
            settings_ui::control_column(self.choice_control(
                "settings-voice-shortcut-mode",
                "Dictation shortcut mode",
                mode.label(),
                &[("Auto", "Auto"), ("Hold", "Hold"), ("Toggle", "Toggle")],
                cx,
            )),
        ));
        if let Some(index) = shortcut {
            let key = &self.keybindings[index].key;
            input.push(
                self.settings_row(
                    "desktop-settings-row-voice-shortcut",
                    "Dictation shortcut",
                    "Start or stop recording in chat.",
                    web_button(
                        "voice-edit-shortcut",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .child(if key.is_empty() {
                        "Set shortcut".into()
                    } else {
                        key.clone()
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_settings_section(SettingsSection::Keyboard, cx);
                        this.keybinding_filter
                            .update(cx, |field, cx| field.set_value("voice.dictate", window, cx));
                        this.select_keybinding(index, window, cx);
                    }))
                    .into_any_element(),
                ),
            );
        }
        if destination == Destination::Local {
            transcription.push(self.settings_row("voice-row-streaming","Streaming transcription","Show provisional words in the composer as you speak. Stopping transcribes the complete recording.",
                settings_ui::switch("voice-streaming",self.dictation_prefs.streaming,false,"Streaming transcription",cx,|this,_,cx|{
                    this.dictation_prefs.streaming = !this.dictation_prefs.streaming;this.stamp_run_prefs_on_threads(cx);this.persist_and_notify(cx);
                })));
            transcription.extend(self.voice_runtime_rows(cx));
        }
        input.push(self.microphone_test_row(cx));
        transcription.push(
            self.settings_row(
                "desktop-settings-row-voice-test-model",
                "Test model",
                "Transcribe a two-second sample without inserting it in a chat.",
                on_press_view(model_test, cx, |this, _, cx| this.test_voice_model(cx))
                    .into_any_element(),
            ),
        );
        if let Some(result) = &self.voice_test_result {
            transcription.push(
                div()
                    .p_3()
                    .id("voice-test-result")
                    .role(Role::Status)
                    .child(result.clone())
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(self.settings_card("Input", "", input))
            .child(self.settings_card("Transcription", "", transcription))
            .child(self.settings_card("Dictionary", match destination {
                Destination::Cloud(provider) if provider.accepts_vocabulary() =>
                    "Preferred terms from the active dictionaries are sent to the transcription provider. Aliases are applied to the returned text.",
                _ => "",
            }, self.voice_dictionary_rows(cx)))
            .child(self.settings_card("Cleanup", "", cleanup))
            .into_any_element()
    }
}

// Language codes accepted by the bundled Whisper decoder.
pub(super) const LANGUAGES: &[(&str, &str)] = &[
    ("auto", "Auto detect"),
    ("af", "Afrikaans"),
    ("sq", "Albanian"),
    ("am", "Amharic"),
    ("ar", "Arabic"),
    ("hy", "Armenian"),
    ("as", "Assamese"),
    ("az", "Azerbaijani"),
    ("ba", "Bashkir"),
    ("eu", "Basque"),
    ("be", "Belarusian"),
    ("bn", "Bengali"),
    ("bs", "Bosnian"),
    ("br", "Breton"),
    ("bg", "Bulgarian"),
    ("yue", "Cantonese"),
    ("ca", "Catalan"),
    ("zh", "Chinese"),
    ("hr", "Croatian"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("nl", "Dutch"),
    ("en", "English"),
    ("et", "Estonian"),
    ("fo", "Faroese"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("gl", "Galician"),
    ("ka", "Georgian"),
    ("de", "German"),
    ("el", "Greek"),
    ("gu", "Gujarati"),
    ("ht", "Haitian Creole"),
    ("ha", "Hausa"),
    ("haw", "Hawaiian"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hu", "Hungarian"),
    ("is", "Icelandic"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("jw", "Javanese"),
    ("kn", "Kannada"),
    ("kk", "Kazakh"),
    ("km", "Khmer"),
    ("ko", "Korean"),
    ("lo", "Lao"),
    ("la", "Latin"),
    ("lv", "Latvian"),
    ("ln", "Lingala"),
    ("lt", "Lithuanian"),
    ("lb", "Luxembourgish"),
    ("mk", "Macedonian"),
    ("mg", "Malagasy"),
    ("ms", "Malay"),
    ("ml", "Malayalam"),
    ("mt", "Maltese"),
    ("mi", "Maori"),
    ("mr", "Marathi"),
    ("mn", "Mongolian"),
    ("my", "Myanmar"),
    ("ne", "Nepali"),
    ("no", "Norwegian"),
    ("nn", "Nynorsk"),
    ("oc", "Occitan"),
    ("ps", "Pashto"),
    ("fa", "Persian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("pa", "Punjabi"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sa", "Sanskrit"),
    ("sr", "Serbian"),
    ("sn", "Shona"),
    ("sd", "Sindhi"),
    ("si", "Sinhala"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("so", "Somali"),
    ("es", "Spanish"),
    ("su", "Sundanese"),
    ("sw", "Swahili"),
    ("sv", "Swedish"),
    ("tl", "Tagalog"),
    ("tg", "Tajik"),
    ("ta", "Tamil"),
    ("tt", "Tatar"),
    ("te", "Telugu"),
    ("th", "Thai"),
    ("bo", "Tibetan"),
    ("tr", "Turkish"),
    ("tk", "Turkmen"),
    ("uk", "Ukrainian"),
    ("ur", "Urdu"),
    ("uz", "Uzbek"),
    ("vi", "Vietnamese"),
    ("cy", "Welsh"),
    ("yi", "Yiddish"),
    ("yo", "Yoruba"),
];
