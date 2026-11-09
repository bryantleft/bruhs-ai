//! Final-provider and model preferences, plus the on-device live preview.

use bruhs_models::TranscriptionProvider;
use leptos::prelude::*;

use crate::components::settings_page::SettingsRow;
use crate::state::{
    LiveWhisperModelSignal, SetLiveWhisperModelSignal, SetWhisperModelSignal,
    TranscriptionSettingsSignal, WhisperModel, WhisperModelSignal,
};
use crate::ui::{Dropdown, DropdownOption};

fn model_for_key(key: &str) -> Option<WhisperModel> {
    WhisperModel::all().iter().find(|m| m.key() == key).copied()
}

fn options() -> Vec<DropdownOption> {
    WhisperModel::all()
        .iter()
        .map(|m| {
            let d = m.descriptor();
            DropdownOption::new(m.key(), format!("{} ({} MB)", d.label, d.size_mb))
        })
        .collect()
}

#[component]
pub fn WhisperModelSettings() -> impl IntoView {
    let TranscriptionSettingsSignal(transcription) = expect_context();
    let WhisperModelSignal(model) = expect_context::<WhisperModelSignal>();
    let SetWhisperModelSignal(set_model) = expect_context::<SetWhisperModelSignal>();
    let LiveWhisperModelSignal(live_model) = expect_context::<LiveWhisperModelSignal>();
    let SetLiveWhisperModelSignal(set_live_model) = expect_context::<SetLiveWhisperModelSignal>();

    let on_final_change = Callback::new(move |v: String| {
        if let Some(m) = model_for_key(&v) {
            set_model.set(m);
        }
    });
    let on_live_change = Callback::new(move |v: String| {
        if let Some(m) = model_for_key(&v) {
            set_live_model.set(m);
        }
    });

    view! {
        <SettingsRow
            title="Transcription provider"
            description="Applies to the final transcript on your next recording. On-device Whisper keeps audio here. A compatible server receives recorded audio after Stop and may charge for usage. Live previews always stay on-device."
        >
            <Dropdown
                options={TranscriptionProvider::ALL.into_iter().map(|provider| DropdownOption::new(provider.key(), provider.label())).collect::<Vec<_>>()}
                value=Signal::derive(move || transcription.get().provider.key().to_owned())
                on_change=Callback::new(move |value: String| {
                    if let Some(provider) = TranscriptionProvider::for_key(&value) {
                        transcription.update(|settings| settings.provider = provider);
                    }
                })
                label="Transcription provider"
                class="w-full sm:w-80"
                align_end=true
            />
        </SettingsRow>
        <Show when=move || transcription.get().provider == TranscriptionProvider::OpenAiCompatible>
            <SettingsRow
                title="Server transcription model"
                description="Enter a model ID supported by your configured server. It must return timestamped segments in verbose_json format. Configure the server URL, key, and request deadline in the capture daemon environment."
            >
                <input
                    type="text"
                    aria-label="Server transcription model"
                    autocomplete="off"
                    spellcheck="false"
                    placeholder="Model ID from your server"
                    prop:value=move || transcription.get().remote_model
                    on:input=move |event| transcription.update(|settings| settings.remote_model = event_target_value(&event))
                    class="min-h-10 w-full min-w-0 rounded-seed bg-longan-950 px-3 text-body-sm inset-ring inset-ring-longan-700 focus:outline focus:outline-persimmon-400 sm:w-80"
                />
            </SettingsRow>
            {move || transcription.get().validate().err().map(|error| view! {
                <p role="alert" class="px-4 pb-3 text-label-sm text-lychee-300">{error.to_string()}</p>
            })}
        </Show>
        <Show when=move || transcription.get().provider == TranscriptionProvider::LocalWhisper>
        <SettingsRow
            title="Transcription model"
            description="Downloaded on first use to ~/Library/Application Support/dev.bruhs.bruhs-ai/models. Takes effect on the next recording."
        >
            <Dropdown
                options=options()
                value=Signal::derive(move || model.get().key().to_string())
                on_change=on_final_change
                label="Transcription model"
                class="w-full sm:w-80"
                align_end=true
            />
        </SettingsRow>
        </Show>
        <SettingsRow
            title="Live preview model"
            description="Used for the live transcript while recording. Smaller is faster; the final pass uses your selected provider and model."
        >
            <Dropdown
                options=options()
                value=Signal::derive(move || live_model.get().key().to_string())
                on_change=on_live_change
                label="Live preview model"
                class="w-full sm:w-80"
                align_end=true
            />
        </SettingsRow>
    }
}
