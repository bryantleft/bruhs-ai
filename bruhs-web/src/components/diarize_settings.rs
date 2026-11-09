//! Speaker rows: sherpa-onnx sub-speaker clustering with a speaker cap,
//! Vision OCR of participant names, and cross-meeting voice linking with
//! its similarity threshold.

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use leptos::prelude::*;

use crate::components::settings_page::{SettingsRow, SettingsRowBody};
use crate::state::{DiarizeSettingsSignal, SetDiarizeSettingsSignal};
use crate::ui::{Slider, Switch};

#[component]
pub fn DiarizeSettingsPanel() -> impl IntoView {
    let DiarizeSettingsSignal(settings) = expect_context::<DiarizeSettingsSignal>();
    let SetDiarizeSettingsSignal(set_settings) = expect_context::<SetDiarizeSettingsSignal>();

    let on_subspeaker_toggle = Callback::new(move |checked: bool| {
        set_settings.update(|s| s.subspeaker_enabled = checked);
    });
    let on_auto_link_toggle = Callback::new(move |checked: bool| {
        set_settings.update(|s| s.cross_meeting_linking_auto = checked);
    });
    let on_video_ocr_toggle = Callback::new(move |checked: bool| {
        set_settings.update(|s| s.video_ocr_enabled = checked);
    });
    let on_threshold_change = Callback::new(move |v: f64| {
        set_settings.update(|s| s.cross_meeting_linking_threshold = v.to_f32_lossy());
    });
    let on_max_speakers_change = Callback::new(move |v: f64| {
        set_settings.update(|s| s.max_speakers = v.to_u32_saturating().clamp(2, 20));
    });

    view! {
        <SettingsRow
            title="Distinguish multiple speakers"
            description="Clusters voice embeddings on the system-audio channel so remote speakers become Speaker 1, Speaker 2, and so on. ~30 MB model, downloaded on first use."
        >
            <Switch
                checked=Signal::derive(move || settings.get().subspeaker_enabled)
                on_change=on_subspeaker_toggle
                label="Distinguish multiple speakers"
            />
        </SettingsRow>
        <Show when=move || settings.get().subspeaker_enabled>
            <SettingsRow
                title="Max speakers"
                description="Higher resolves more people on large calls; lower is faster with fewer false splits."
            >
                <span class="text-code-sm tabular-nums text-rambutan-200">
                    {move || settings.get().max_speakers.to_string()}
                </span>
            </SettingsRow>
            <SettingsRowBody>
                <Slider
                    min=2.0
                    max=20.0
                    step=1.0
                    value=Signal::derive(move || f64::from(settings.get().max_speakers))
                    on_input=on_max_speakers_change
                />
                <div class="mt-1 flex justify-between text-label-sm text-rambutan-500">
                    <span>"2 · faster"</span>
                    <span>"20 · more resolution"</span>
                </div>
            </SettingsRowBody>
        </Show>
        <SettingsRow
            title="Read participant names from video"
            description="Reads on-screen name hints locally at 2 fps. Visible names do not prove who is speaking and will not rename transcript speakers. Frames stay in memory; name observations are saved on this device. Requires Screen Recording permission."
        >
            <Switch
                checked=Signal::derive(move || settings.get().video_ocr_enabled)
                on_change=on_video_ocr_toggle
                label="Read participant names from video"
            />
        </SettingsRow>
        <SettingsRow
            title="Link voices across meetings"
            description="Match new clusters against known speaker profiles by voice similarity. Off makes every cluster a fresh profile you can link by hand."
        >
            <Switch
                checked=Signal::derive(move || settings.get().cross_meeting_linking_auto)
                on_change=on_auto_link_toggle
                label="Auto-link voices across meetings"
            />
        </SettingsRow>
        <Show when=move || settings.get().cross_meeting_linking_auto>
            <SettingsRow
                title="Linking threshold"
                description="Higher means fewer false matches, lower means better recall. Default 0.75."
            >
                <span class="text-code-sm tabular-nums text-rambutan-200">
                    {move || format!("{:.2}", settings.get().cross_meeting_linking_threshold)}
                </span>
            </SettingsRow>
            <SettingsRowBody>
                <Slider
                    min=0.5
                    max=0.95
                    step=0.05
                    value=Signal::derive(move || f64::from(settings.get().cross_meeting_linking_threshold))
                    on_input=on_threshold_change
                />
                <div class="mt-1 flex justify-between text-label-sm text-rambutan-500">
                    <span>"0.50 · looser"</span>
                    <span>"0.95 · stricter"</span>
                </div>
            </SettingsRowBody>
        </Show>
    }
}
