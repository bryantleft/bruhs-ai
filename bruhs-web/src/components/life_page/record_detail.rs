use bruhs_models::life_record_detail;
use leptos::prelude::*;

use crate::state::{LifeContextSignal, LifeSectionSignal, SelectedLifeRecordSignal};

fn date(timestamp: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
        bruhs_models::EpochMs::saturating_from_i64(timestamp).as_f64(),
    ));
    if date.get_time().is_finite() {
        date.to_string().into()
    } else {
        "Unknown date".into()
    }
}

#[component]
pub(super) fn LifeRecordDetail() -> impl IntoView {
    let LifeContextSignal(life) = expect_context::<LifeContextSignal>();
    let LifeSectionSignal(tab) = expect_context::<LifeSectionSignal>();
    let SelectedLifeRecordSignal(selected) = expect_context::<SelectedLifeRecordSignal>();
    let current = Memo::new(move |_| selected.get().filter(|record| record.section == tab.get()));
    let record = Memo::new(move |_| {
        current.with(|selected| {
            selected.as_ref().and_then(|selected| {
                life.with(|context| life_record_detail(context, selected, date))
            })
        })
    });
    Effect::new(move |_| {
        if current.get().is_some() && life.ready() {
            super::super::record_focus::focus_record("life-record-title".into());
        }
    });
    view! {
        <Show when=move || current.get().is_some()>
            <section class="mx-auto mb-4 max-w-6xl min-w-0 rounded-grape bg-longan-900/65 p-3 inset-ring inset-ring-longan-800" aria-labelledby="life-record-title" data-record-id=move || current.get().map(|r| r.id)>
                <div class="flex flex-col items-start justify-between gap-3 sm:flex-row">
                    <div class="min-w-0 flex-1">
                        <p class="text-label-sm text-blueberry-300">{move || record.with(|r| r.as_ref().map_or("Selected record", |r| r.kind))}</p>
                        <h2 id="life-record-title" tabindex="-1" class="mt-1 text-h4 text-rambutan-100 [overflow-wrap:anywhere]">{move || record.with(|r| r.as_ref().map_or_else(|| if life.ready() { "Record unavailable".into() } else { "Loading record…".into() }, |r| r.title.clone()))}</h2>
                    </div>
                    <button type="button" class="min-h-[44px] rounded-seed px-3 py-2 text-label-sm text-rambutan-300 hover:bg-longan-800 cursor-pointer" on:click=move |_| {
                        selected.set(None);
                        super::super::record_focus::focus_record("life-title".into());
                    }>"Back to overview"</button>
                </div>
                {move || record.get().map(|r| view! {
                    <dl class="mt-3 grid gap-3 sm:grid-cols-2">{r.fields.into_iter().map(|(label, value)| view! {
                        <div class="min-w-0"><dt class="text-label-sm text-rambutan-500">{label}</dt><dd class="mt-0.5 whitespace-pre-wrap text-body-sm text-rambutan-200 [overflow-wrap:anywhere]">{value}</dd></div>
                    }).collect_view()}</dl>
                })}
                <Show when=move || life.ready() && record.get().is_none()><p class="mt-2 text-body-sm text-rambutan-400">"This record is no longer available in the local workspace. Search again or return to the overview."</p></Show>
                <p class="mt-3 text-label-sm text-rambutan-500 [overflow-wrap:anywhere]">{move || current.get().map(|r| format!("Record ID · {}", r.id))}</p>
            </section>
        </Show>
    }
}
