use super::calendar::local_time_zone;
#[cfg(target_arch = "wasm32")]
use super::REFRESH_MS;
use super::{Check, Field, SELECT};
use crate::ui::{Button, ButtonSize, ButtonVariant};
use bruhs_models::weekly_reports::*;
use leptos::prelude::*;

fn window(report: &EngineeringReport) -> String {
    fn time(time: bruhs_models::EpochMs) -> String {
        #[cfg(target_arch = "wasm32")]
        {
            js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(time.as_f64()))
                .to_iso_string()
                .into()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            format!("{} ms since Unix epoch", time.as_millis())
        }
    }
    format!(
        "{} to {}",
        time(report.window_start),
        time(report.window_end)
    )
}

/// The preset's schedule from the editor; `None` runs seven days after enabling.
fn weekly_schedule(
    fixed: bool,
    weekday: &str,
    time: &str,
    time_zone: &str,
) -> Result<Option<WeeklySchedule>, String> {
    if !fixed {
        return Ok(None);
    }
    let weekday = weekday
        .parse::<u8>()
        .ok()
        .filter(|day| (1..=7).contains(day))
        .ok_or_else(|| "Choose a weekday.".to_string())?;
    let time = time.trim().to_owned();
    if let Some(error) = bruhs_models::reactive::calendar_shape_error(
        std::slice::from_ref(&time),
        &[weekday],
        time_zone,
    ) {
        return Err(error);
    }
    Ok(Some(WeeklySchedule {
        weekday,
        time,
        time_zone: time_zone.trim().to_owned(),
    }))
}

fn preset_summary(preset: &WeeklyReportPreset) -> String {
    let cadence = match (preset.scheduled, &preset.schedule) {
        (true, Some(schedule)) => schedule.describe(),
        (true, None) => "Repeats every seven days".into(),
        (false, _) => "On demand".into(),
    };
    format!("{}  {cadence}", preset.model)
}

#[component]
pub(super) fn WeeklyReportsPanel(on_chat: Callback<(String, String)>) -> impl IntoView {
    let set_project = use_context::<crate::state::SetActiveProjectSignal>();
    let snapshot = RwSignal::new(WeeklyReportsSnapshot::default());
    let error = RwSignal::new(None::<String>);
    let load_error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let reading = RwSignal::new(false);
    let alive = RwSignal::new(true);
    let generation = RwSignal::new(0_u64);
    let before = RwSignal::new(None::<String>);
    let editor = RwSignal::new(None::<String>);
    let project = RwSignal::new(
        use_context::<crate::state::ActiveProjectSignal>()
            .and_then(|value| value.0.get_untracked())
            .map(|project| project.path)
            .unwrap_or_default(),
    );
    let model = RwSignal::new(bruhs_models::agent::DEFAULT_AGENT_MODEL.to_string());
    let local_git = RwSignal::new(true);
    let github = RwSignal::new(false);
    let deployments = RwSignal::new(false);
    let window_end = RwSignal::new(String::new());
    let scheduled = RwSignal::new(false);
    let fixed = RwSignal::new(false);
    let weekday = RwSignal::new("1".to_string());
    let slot = RwSignal::new("09:00".to_string());
    let slot_zone = RwSignal::new(local_time_zone());
    let report = RwSignal::new(None::<EngineeringReport>);
    let title = RwSignal::new(String::new());
    let body = RwSignal::new(String::new());
    let notice = RwSignal::new(None::<String>);
    let refresh = Callback::new(move |_: ()| {
        if busy.get_untracked() || reading.get_untracked() {
            return;
        }
        reading.set(true);
        generation.update(|value| *value = value.wrapping_add(1));
        let requested = generation.get_untracked();
        let cursor = before.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::weekly_reports(cursor.as_deref()).await;
            if alive.try_get_untracked() != Some(true)
                || generation.try_get_untracked() != Some(requested)
            {
                return;
            }
            reading.set(false);
            match result {
                Ok(next) => {
                    snapshot.set(next);
                    load_error.set(None);
                }
                Err(err) => load_error.set(Some(err.to_string())),
            }
        });
    });
    let send = Callback::new(move |command: WeeklyReportCommand| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        reading.set(false);
        error.set(None);
        notice.set(None);
        generation.update(|value| *value = value.wrapping_add(1));
        let requested = generation.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let edited_id = match &command {
                WeeklyReportCommand::EditReport { id, .. } => Some(id.clone()),
                _ => None,
            };
            let saved_preset = matches!(&command, WeeklyReportCommand::SavePreset { .. });
            let queued = matches!(&command, WeeklyReportCommand::RunNow { .. });
            let result = crate::api::weekly_report_command(&command).await;
            if alive.try_get_untracked() != Some(true)
                || generation.try_get_untracked() != Some(requested)
            {
                return;
            }
            match result {
                Ok(next) => {
                    before.set(None);
                    snapshot.set(next);
                    if saved_preset {
                        editor.set(None);
                        notice.set(Some("Report preset saved.".into()));
                    }
                    if queued {
                        notice.set(Some(
                            "Report queued. Follow its run in activity below.".into(),
                        ));
                    }
                    if let Some(id) = edited_id {
                        let result = crate::api::weekly_report(&id).await;
                        if alive.try_get_untracked() != Some(true)
                            || generation.try_get_untracked() != Some(requested)
                        {
                            return;
                        }
                        match result {Ok(next)=>{title.set(next.title.clone());body.set(next.body.clone());report.set(Some(next));notice.set(Some("Report saved.".into()));},Err(err)=>error.set(Some(format!("Report saved, but reloading failed: {err}. Reopen it before editing again.")))}
                    }
                }
                Err(err) => error.set(Some(err.to_string())),
            }
            busy.set(false);
        });
    });
    let open_report = Callback::new(move |id: String| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        reading.set(false);
        generation.update(|value| *value = value.wrapping_add(1));
        let requested = generation.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::weekly_report(&id).await;
            if alive.try_get_untracked() != Some(true)
                || generation.try_get_untracked() != Some(requested)
            {
                return;
            }
            busy.set(false);
            match result {
                Ok(next) => {
                    title.set(next.title.clone());
                    body.set(next.body.clone());
                    report.set(Some(next));
                    error.set(None);
                    notice.set(None);
                }
                Err(err) => error.set(Some(err.to_string())),
            }
        });
    });
    let edit = Callback::new(move |preset: WeeklyReportPreset| {
        editor.set(Some(preset.id));
        project.set(preset.project_path);
        model.set(preset.model);
        local_git.set(preset.local_git);
        github.set(preset.github_pull_requests);
        deployments.set(preset.github_deployments);
        scheduled.set(preset.scheduled);
        fixed.set(preset.schedule.is_some());
        match preset.schedule {
            Some(schedule) => {
                weekday.set(schedule.weekday.to_string());
                slot.set(schedule.time);
                slot_zone.set(schedule.time_zone);
            }
            None => {
                weekday.set("1".into());
                slot.set("09:00".into());
                slot_zone.set(local_time_zone());
            }
        }
    });
    Effect::new(move |_| refresh.run(()));
    #[cfg(target_arch = "wasm32")]
    let timer = StoredValue::new_local(Some(gloo_timers::callback::Interval::new(
        REFRESH_MS,
        move || refresh.run(()),
    )));
    on_cleanup(move || {
        alive.set(false);
        #[cfg(target_arch = "wasm32")]
        timer.update_value(|timer| {
            timer.take();
        });
    });
    view! {
        <section class="min-w-0 space-y-3 border-b border-longan-700/60 pb-5" aria-labelledby="weekly-reports-title">
            <div class="flex flex-wrap items-center justify-between gap-2"><h3 id="weekly-reports-title" class="text-label text-rambutan-100">"Weekly engineering reports"</h3><Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=Signal::derive(move ||busy.get()) on_click=Callback::new(move |_|edit.run(WeeklyReportPreset {id:uuid::Uuid::new_v4().to_string(),project_path:project.get_untracked(),model:bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),local_git:true,github_pull_requests:false, github_deployments:false,scheduled:false,schedule:None}))>"New report preset"</Button></div>
            <p class="text-body-sm text-rambutan-400">"Review seven days of repository activity. Completed reports are saved locally for editing. Choose GitHub feeds when available. Incidents and reliability stay marked unavailable until a feed is connected."</p>
            {move ||load_error.get().map(|error|view!{<p role="alert" class="break-words text-body-sm text-lychee-200">{error}</p>})}
            {move ||error.get().map(|error|view!{<p role="alert" class="break-words text-body-sm text-lychee-200">{error}</p>})}
            {move ||notice.get().map(|notice|view!{<p role="status" class="text-body-sm text-blueberry-200">{notice}</p>})}
            <fieldset class="m-0 min-w-0 space-y-3 border-0 p-0" disabled=move ||busy.get()>
                <Show when=move ||editor.get().is_some()><div class="space-y-3">
                    <div class="grid gap-3 sm:grid-cols-2"><Field label="Report project folder" value=project/><Field label="Report model" value=model/></div>
                    <div class="flex flex-wrap gap-x-5"><Check label="Local Git commits" value=local_git/><Check label="GitHub pull requests" value=github/><Check label="GitHub deployments" value=deployments/><Check label="Repeat every seven days" value=scheduled/></div>
                    <Show when=move ||scheduled.get()>
                        <fieldset class="m-0 min-w-0 space-y-2 border-0 p-0">
                            <legend class="mb-1 text-label-sm text-rambutan-300">"Schedule"</legend>
                            <label class="flex min-h-11 cursor-pointer items-center gap-2 text-label-sm text-rambutan-300 sm:min-h-[32px]"><input type="radio" name="weekly-report-schedule" class="size-4 accent-persimmon-500" prop:checked=move ||!fixed.get() on:change=move |_|fixed.set(false) /><span>"Seven days after enabling"</span></label>
                            <label class="flex min-h-11 cursor-pointer items-center gap-2 text-label-sm text-rambutan-300 sm:min-h-[32px]"><input type="radio" name="weekly-report-schedule" class="size-4 accent-persimmon-500" prop:checked=move ||fixed.get() on:change=move |_|fixed.set(true) /><span>"Fixed weekday and time"</span></label>
                            <Show when=move ||fixed.get()>
                                <div class="grid gap-3 sm:grid-cols-3">
                                    <label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Weekday"</span><select aria-label="Weekday" class=SELECT prop:value=move ||weekday.get() on:change=move |event|weekday.set(event_target_value(&event))>{(1_u8..=7).zip(bruhs_models::reactive::WEEKDAY_NAMES).map(|(day,name)|view!{<option value=day.to_string()>{name}</option>}).collect_view()}</select></label>
                                    <Field label="Time" value=slot/>
                                    <Field label="Time zone" value=slot_zone/>
                                </div>
                            </Show>
                        </fieldset>
                    </Show>
                    <p class="text-label-sm text-rambutan-400">"GitHub uses the host's gh login. Repeating reports run while the local server is available. New schedules first run after seven days. Paused schedules may catch up on resume."</p>
                    <div class="flex gap-2"><Button size=ButtonSize::Small on_click=Callback::new(move |_|{if let Some(id)=editor.get_untracked(){
                        let schedule=match weekly_schedule(scheduled.get_untracked()&&fixed.get_untracked(),&weekday.get_untracked(),&slot.get_untracked(),&slot_zone.get_untracked()){Ok(schedule)=>schedule,Err(message)=>{error.set(Some(message));return;}};
                        send.run(WeeklyReportCommand::SavePreset {preset:WeeklyReportPreset {id,project_path:project.get_untracked(),model:model.get_untracked(),local_git:local_git.get_untracked(),github_pull_requests:github.get_untracked(),github_deployments:deployments.get_untracked(),scheduled:scheduled.get_untracked(),schedule}});}})>"Save report preset"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|editor.set(None))>"Cancel setup"</Button></div>
                </div></Show>
                <Show when=move ||!snapshot.get().presets.is_empty()><Field label="Report window end in UTC (optional)" value=window_end/><p class="text-label-sm text-rambutan-400">"For on-demand reports, leave blank for the latest seven days or enter a date such as 2026-11-13T15:00:00Z."</p></Show>
                <For each=move ||snapshot.get().presets key=|preset|preset.id.clone() children=move |preset|{
                    let run_id=preset.id.clone();let edit_preset=preset.clone();let summary=preset_summary(&preset);
                    view!{<div class="flex min-w-0 flex-wrap items-center justify-between gap-2 py-2"><div class="min-w-0"><p class="break-all text-body-sm text-rambutan-200">{preset.project_path}</p><p class="text-label-sm text-rambutan-400">{summary}</p></div><div class="flex flex-wrap gap-2"><Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_|send.run(WeeklyReportCommand::RunNow {preset_id:run_id.clone(),request_id:uuid::Uuid::new_v4().to_string(),window_end_utc:(!window_end.get_untracked().trim().is_empty()).then(||window_end.get_untracked().trim().to_string()),}))>"Generate report now"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|edit.run(edit_preset.clone()))>"Edit preset"</Button></div></div>}
                }/>
                <Show when=move ||snapshot.get().reports.is_empty()><p class="text-body-sm text-rambutan-500">"Saved reports appear here after a successful run. Set up a preset, then generate a report now or enable repeating reports."</p></Show>
                <For each=move ||snapshot.get().reports key=|report|report.id.clone() children=move |item|{
                    let id=item.id.clone();view!{<div class="flex flex-wrap items-center justify-between gap-2 py-1"><span class="min-w-0 break-words text-body-sm text-rambutan-200">{item.title}</span><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|open_report.run(id.clone()))>"Open report"</Button></div>}
                }/>
                <div class="flex flex-wrap gap-2"><Show when=move ||snapshot.get().next_before.is_some()><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|{before.set(snapshot.get_untracked().next_before);refresh.run(());})>"Older reports"</Button></Show><Show when=move ||before.get().is_some()><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|{before.set(None);refresh.run(());})>"Newest reports"</Button></Show></div>
                {move ||report.get().map(|saved|{
                    let selected=saved.clone();let open=saved.clone();
                    view!{<article class="min-w-0 space-y-3 border-t border-longan-700/60 pt-4"><h4 class="text-label text-rambutan-100">"Edit saved report"</h4><dl class="space-y-2 text-label-sm text-rambutan-400"><div><dt>"Project"</dt><dd class="break-all text-rambutan-200">{saved.project_path.clone()}</dd></div><div><dt>"Model"</dt><dd class="break-all text-rambutan-200">{saved.model.clone()}</dd></div><div><dt>"Reporting window"</dt><dd class="break-all text-rambutan-200">{window(&saved)}</dd></div></dl><Field label="Report title" value=title/><Field label="Report Markdown" value=body multiline=true/>
                        <details><summary class="cursor-pointer text-label-sm text-rambutan-300">"Sources and missing feeds"</summary><div class="mt-2 space-y-2">{saved.coverage.into_iter().map(|coverage|match coverage {
                            ReportCoverage::Unavailable {source,reason}=>view!{<p class="text-body-sm text-rambutan-400">{format!("{source}: {reason}")}</p>}.into_any(),
                            ReportCoverage::Available {source,facts,truncated}=>view!{<div class="space-y-1"><p class="text-body-sm text-rambutan-300">{format!("{source}: {} records{}",facts.len(),if truncated {" (partial feed)"}else{""})}</p>{facts.into_iter().map(|fact|{let label=format!("{} ({})",fact.title,fact.reference);fact.url.map_or_else(||view!{<p class="break-words text-body-sm text-rambutan-400">{label.clone()}</p>}.into_any(),|url|view!{<a class="block break-words text-body-sm text-persimmon-300 underline" href=url target="_blank" rel="noopener noreferrer">{label.clone()}</a>}.into_any())}).collect_view()}</div>}.into_any(),
                        }).collect_view()}</div></details>
                        <div class="flex flex-wrap gap-2"><Button size=ButtonSize::Small on_click=Callback::new(move |_|send.run(WeeklyReportCommand::EditReport {id:selected.id.clone(),expected_revision:selected.revision,title:title.get_untracked(),body:body.get_untracked()}))>"Save report edits"</Button><Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_|{
                            if let Some(set)=set_project {set.0.set(Some(crate::state::ActiveProject {path:open.project_path.clone(),name:open.project_path.rsplit('/').next().unwrap_or("Project").to_string()}));}
                            on_chat.run((format!("weekly-report-{}",open.preset_id),open.body.clone()));
                        })>"Open project and agent thread"</Button><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_|report.set(None))>"Close report"</Button></div>
                    </article>}
                })}
            </fieldset>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_are_optional_and_validated() {
        assert_eq!(weekly_schedule(false, "x", "bad", ""), Ok(None));
        assert_eq!(
            weekly_schedule(true, "5", " 17:30 ", " Europe/Paris "),
            Ok(Some(WeeklySchedule {
                weekday: 5,
                time: "17:30".into(),
                time_zone: "Europe/Paris".into(),
            }))
        );
        assert!(weekly_schedule(true, "8", "17:30", "UTC").is_err());
        assert!(weekly_schedule(true, "1", "7:30", "UTC").is_err());
        assert!(weekly_schedule(true, "1", "07:30", " ").is_err());
    }

    #[test]
    fn preset_summary_names_a_fixed_slot() {
        let mut preset = WeeklyReportPreset {
            id: "p".into(),
            project_path: "/tmp/p".into(),
            model: "m".into(),
            local_git: true,
            github_pull_requests: false,
            github_deployments: false,
            scheduled: true,
            schedule: None,
        };
        assert_eq!(preset_summary(&preset), "m  Repeats every seven days");
        preset.schedule = Some(WeeklySchedule {
            weekday: 1,
            time: "09:00".into(),
            time_zone: "UTC".into(),
        });
        assert_eq!(preset_summary(&preset), "m  Mon at 09:00 (UTC)");
        preset.scheduled = false;
        assert_eq!(preset_summary(&preset), "m  On demand");
    }
}
