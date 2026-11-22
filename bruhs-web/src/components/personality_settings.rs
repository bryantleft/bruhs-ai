//! People & observations: per-person observation extraction toggle.

use leptos::prelude::*;

use crate::components::settings_page::SettingsRow;
use crate::state::{PersonalitySettingsSignal, SetPersonalitySettingsSignal};
use crate::ui::Switch;

#[component]
pub fn PersonalitySettings() -> impl IntoView {
    let PersonalitySettingsSignal(settings) = expect_context::<PersonalitySettingsSignal>();
    let SetPersonalitySettingsSignal(set_settings) =
        expect_context::<SetPersonalitySettingsSignal>();

    let on_toggle =
        Callback::new(move |c: bool| set_settings.update(|s| s.extract_observations = c));

    view! {
        <SettingsRow
            title="Person observations"
            description="Capture topics, decisions, and quotes per speaker."
        >
            <Switch
                checked=Signal::derive(move || settings.get().extract_observations)
                on_change=on_toggle
                label="Extract observations about each person during summaries"
            />
        </SettingsRow>
    }
}
