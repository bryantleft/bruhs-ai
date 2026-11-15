//! Calendar schedules, local time zones and readable times for always-on agents.
use bruhs_models::reactive::{calendar_shape_error, SourceKind, WEEKDAY_NAMES};
use bruhs_models::EpochMs;
use leptos::prelude::*;

/// The browser's IANA time zone, such as `Europe/Paris`; `UTC` when unknown.
pub(super) fn local_time_zone() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let options =
            js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new())
                .resolved_options();
        if let Some(zone) = js_sys::Reflect::get(&options, &"timeZone".into())
            .ok()
            .and_then(|zone| zone.as_string())
            .filter(|zone| !zone.trim().is_empty())
        {
            return zone;
        }
    }
    "UTC".into()
}

/// A local date and time for an instant, such as "Nov 14, 2026, 9:05 AM".
pub(super) fn local_time(time: EpochMs) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(time.as_f64()));
        let options = js_sys::Object::new();
        for (key, value) in [("dateStyle", "medium"), ("timeStyle", "short")] {
            let _ = js_sys::Reflect::set(&options, &key.into(), &value.into());
        }
        date.to_locale_string("en-US", &options).into()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        format!("{} ms since Unix epoch", time.as_millis())
    }
}

/// Times typed as `08:00, 17:30`; blanks between commas are ignored.
pub(super) fn parse_times(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|time| !time.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A calendar source from the editor fields, checked before saving.
pub(super) fn calendar_source(
    times: &str,
    weekdays: &[u8],
    time_zone: &str,
) -> Result<SourceKind, String> {
    let times = parse_times(times);
    let mut weekdays = weekdays.to_vec();
    weekdays.sort_unstable();
    weekdays.dedup();
    let time_zone = time_zone.trim().to_owned();
    if let Some(error) = calendar_shape_error(&times, &weekdays, &time_zone) {
        return Err(error);
    }
    Ok(SourceKind::Calendar {
        times,
        weekdays,
        time_zone,
    })
}

/// Toggles Monday to Sunday; none selected means every day.
#[component]
pub(super) fn WeekdayToggles(value: RwSignal<Vec<u8>>) -> impl IntoView {
    view! {
        <fieldset class="m-0 min-w-0 border-0 p-0">
            <legend class="mb-1 text-label-sm text-rambutan-300">"Days"</legend>
            <div class="flex flex-wrap gap-1">
                {(1_u8..=7).zip(WEEKDAY_NAMES).map(|(day, name)| {
                    let on = Signal::derive(move || value.with(|days| days.contains(&day)));
                    view! {
                        <button
                            type="button"
                            aria-pressed=move || on.get().to_string()
                            class=move || format!(
                                "bruhs-control min-h-[28px] min-w-[40px] rounded-grape px-2 text-label-sm focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer {}",
                                if on.get() {
                                    "bg-blueberry-500/15 text-blueberry-200 inset-ring inset-ring-blueberry-400/30"
                                } else {
                                    "bg-longan-950 text-rambutan-400 inset-ring inset-ring-longan-700 hover:text-rambutan-200"
                                }
                            )
                            on:click=move |_| value.update(|days| {
                                if let Some(index) = days.iter().position(|d| *d == day) {
                                    days.remove(index);
                                } else {
                                    days.push(day);
                                    days.sort_unstable();
                                }
                            })
                        >{name}</button>
                    }
                }).collect_view()}
            </div>
            <p class="mt-1 text-label-sm text-rambutan-500">"None selected runs every day."</p>
        </fieldset>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_split_on_commas_and_skip_blanks() {
        assert_eq!(parse_times(" 08:00, 17:30 ,,"), vec!["08:00", "17:30"]);
        assert!(parse_times(" , ").is_empty());
    }

    #[test]
    fn calendar_sources_are_validated_and_normalized() {
        assert_eq!(
            calendar_source("08:00", &[5, 1, 1], " Europe/Paris "),
            Ok(SourceKind::Calendar {
                times: vec!["08:00".into()],
                weekdays: vec![1, 5],
                time_zone: "Europe/Paris".into(),
            })
        );
        assert_eq!(
            calendar_source("8:00", &[], "UTC"),
            Err("“8:00” is not a 24-hour HH:MM time.".into())
        );
        assert!(calendar_source("", &[], "UTC").is_err());
        assert!(calendar_source("08:00", &[], " ").is_err());
    }

    #[test]
    fn native_builds_fall_back_to_utc() {
        assert_eq!(local_time_zone(), "UTC");
    }
}
