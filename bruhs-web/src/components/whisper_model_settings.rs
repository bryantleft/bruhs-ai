//! Whisper model rows. Writes to WhisperModelSignal (final pass) and
//! LiveWhisperModelSignal (live preview); persistence is handled by Effects
//! in state.rs.

use leptos::prelude::*;

use crate::components::settings_page::SettingsRow;
use crate::state::{
    LiveWhisperModelSignal, SetLiveWhisperModelSignal, SetWhisperModelSignal, WhisperModel,
    WhisperModelSignal,
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
        <SettingsRow
            title="Live preview model"
            description="Used for the live transcript while recording. Smaller is faster; the final pass still uses the model above."
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
