//! Calendar-driven auto-record rows: master switch, attendee threshold,
//! stop delay, prompt / audio sub-toggles, and the always-skip list.

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use leptos::prelude::*;

use crate::components::settings_page::{SettingsRow, SettingsRowBody};
use crate::state::{
    AutoRecordSettingsSignal, SetAutoRecordSettingsSignal, SetSuppressedEventsSignal,
    SuppressedEventsSignal,
};
use crate::ui::{Stepper, Switch};

#[component]
pub fn AutoRecordSettingsPanel() -> impl IntoView {
    let AutoRecordSettingsSignal(settings) = expect_context::<AutoRecordSettingsSignal>();
    let SetAutoRecordSettingsSignal(set_settings) = expect_context::<SetAutoRecordSettingsSignal>();
    let SuppressedEventsSignal(suppressed) = expect_context::<SuppressedEventsSignal>();
    let SetSuppressedEventsSignal(set_suppressed) = expect_context::<SetSuppressedEventsSignal>();

    let crate::activity_monitor::MonitorFeedback(monitor_feedback) = expect_context();
    let on_toggle = Callback::new(move |c: bool| {
        set_settings.update(|s| {
            s.enabled = c;
            if !c {
                s.audio_driven_enabled = false;
            }
        })
    });
    let on_min_attendees = Callback::new(move |v: f64| {
        set_settings.update(|s| s.min_attendees = v.to_u32_saturating())
    });
    let on_stop_delay = Callback::new(move |v: f64| {
        set_settings.update(|s| s.stop_delay_secs = v.to_i64_saturating())
    });
    let on_prompt_toggle =
        Callback::new(move |c: bool| set_settings.update(|s| s.prompt_before_starting = c));
    let on_audio_driven_toggle =
        Callback::new(move |c: bool| set_settings.update(|s| s.audio_driven_enabled = c));
    let on_remove = move |key: String| {
        set_suppressed.update(|s| {
            s.remove(&key);
        });
    };

    let disabled = Signal::derive(move || !settings.get().enabled);
    let skipped = Memo::new(move |_| {
        let mut v: Vec<String> = suppressed.get().into_iter().collect();
        v.sort();
        v
    });

    view! {
        <SettingsRow
            title="Auto-record calendar meetings"
            description="Start recording when a Google Calendar event begins. Needs the bruhs-capture daemon running and a calendar connected."
        >
            <Switch
                checked=Signal::derive(move || settings.get().enabled)
                on_change=on_toggle
                label="Auto-record calendar meetings"
            />
        </SettingsRow>
        <SettingsRow
            title="Minimum attendees"
            description="Only events with at least this many attendees are recorded."
            disabled=disabled
        >
            <Stepper
                min=0.0
                max=50.0
                value=Signal::derive(move || f64::from(settings.get().min_attendees))
                on_change=on_min_attendees
                disabled=disabled
                label="Minimum attendees"
            />
        </SettingsRow>
        <SettingsRow
            title="Stop after the event ends"
            description="Grace period before the recording stops."
            disabled=disabled
        >
            <Stepper
                min=0.0
                max=900.0
                step=5.0
                value=Signal::derive(move || settings.get().stop_delay_secs.to_f64_lossy())
                on_change=on_stop_delay
                disabled=disabled
                suffix="s"
                label="Seconds after the event ends"
            />
        </SettingsRow>
        <SettingsRow
            title="Prompt before starting"
            description="Off means recordings start silently."
            disabled=disabled
        >
            <Switch
                checked=Signal::derive(move || settings.get().prompt_before_starting)
                on_change=on_prompt_toggle
                disabled=disabled
                label="Prompt before starting"
            />
        </SettingsRow>
        <SettingsRow
            title="Also detect from audio activity"
            description=bruhs_models::ACTIVITY_MONITOR_HELP
            disabled=disabled
        >
            <Switch
                checked=Signal::derive(move || settings.get().audio_driven_enabled)
                on_change=on_audio_driven_toggle
                disabled=disabled
                label="Detect meetings from audio activity"
            />
        </SettingsRow>
        <SettingsRowBody>
            <p role="status" aria-live="polite" class="text-label-sm text-rambutan-400" data-testid="activity-monitor-status">
                {move || monitor_feedback.get()}
            </p>
        </SettingsRowBody>
        <SettingsRow
            title="Always-skip events"
            description="Events you told the prompt to never record."
        >
            <span class="text-label-sm tabular-nums text-rambutan-500">
                {move || {
                    let n = skipped.with(|s| s.len());
                    if n == 0 { "None".to_string() } else { format!("{n}") }
                }}
            </span>
        </SettingsRow>
        {move || (!skipped.with(|s| s.is_empty())).then(|| view! {
            <SettingsRowBody>
                <ul role="list" class="flex flex-col gap-px">
                    <For each=move || skipped.get() key=|k| k.clone() let:key>
                        {
                            let key_for_button = key.clone();
                            view! {
                                <li class="flex items-center justify-between gap-2 rounded-grape px-2 py-1 text-code-sm text-rambutan-300 hover:bg-longan-800/40">
                                    <span class="truncate">{key}</span>
                                    <button
                                        type="button"
                                        class="shrink-0 rounded-seed px-1.5 text-label-sm text-rambutan-400 transition-colors hover:text-lychee-300"
                                        on:click=move |_| on_remove(key_for_button.clone())
                                    >
                                        "Remove"
                                    </button>
                                </li>
                            }
                        }
                    </For>
                </ul>
            </SettingsRowBody>
        })}
    }
}
