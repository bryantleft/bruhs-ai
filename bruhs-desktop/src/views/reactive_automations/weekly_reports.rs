//! Native controls for the same host-owned reports used by the web client.
use super::*;
use bruhs_models::{weekly_reports::*, EpochMs};
use gpui_component::zoom::px;

#[derive(Default)]
pub(super) struct WeeklyReportsState {
    snapshot: WeeklyReportsSnapshot,
    editor: Option<Box<WeeklyEditor>>,
    error: Option<String>,
    load_error: Option<String>,
    notice: Option<String>,
    busy: bool,
    reading: bool,
    generation: u64,
    before: Option<String>,
    request: Option<Task<()>>,
    window_end: Option<Entity<InputState>>,
}

enum WeeklyEditor {
    Preset {
        preset: WeeklyReportPreset,
        path: Entity<InputState>,
        model: Entity<InputState>,
        schedule: Box<ScheduleEditor>,
    },
    Report {
        report: Box<EngineeringReport>,
        title: Entity<InputState>,
        body: Entity<InputState>,
    },
}

/// "Seven days after enabling" or a fixed weekly slot.
struct ScheduleEditor {
    fixed: bool,
    weekday: u8,
    time: Entity<InputState>,
    time_zone: Entity<InputState>,
}

/// The preset's schedule, or the message to show instead of saving.
fn weekly_schedule(
    fixed: bool,
    weekday: u8,
    time: &str,
    time_zone: &str,
) -> Result<Option<WeeklySchedule>, String> {
    if !fixed {
        return Ok(None);
    }
    if !(1..=7).contains(&weekday) {
        return Err("Choose a weekday.".into());
    }
    let time = time.trim();
    let time_zone = time_zone.trim();
    if let Some(error) = calendar_shape_error(&[time.to_string()], &[weekday], time_zone) {
        return Err(error);
    }
    Ok(Some(WeeklySchedule {
        weekday,
        time: time.to_string(),
        time_zone: time_zone.to_string(),
    }))
}

fn preset_cadence(preset: &WeeklyReportPreset) -> String {
    match (&preset.schedule, preset.scheduled) {
        (_, false) => "On demand".into(),
        (Some(schedule), true) => schedule.describe(),
        (None, true) => "Repeats every seven days".into(),
    }
}

fn time(time: EpochMs) -> String {
    chrono::DateTime::from_timestamp_millis(time.as_millis())
        .map_or_else(|| time.to_string(), |time| time.to_rfc3339())
}

impl ReactiveAutomations {
    pub(super) fn ensure_weekly_inputs(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.weekly.window_end.is_none() {
            self.weekly.window_end = Some(edit_input("", false, window, cx));
        }
    }

    fn generate_weekly_report(&mut self, preset_id: String, cx: &mut Context<'_, Self>) {
        let window_end_utc = self
            .weekly
            .window_end
            .as_ref()
            .map(|input| value(input, cx))
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string());
        self.weekly_command(
            WeeklyReportCommand::RunNow {
                preset_id,
                request_id: uuid::Uuid::new_v4().to_string(),
                window_end_utc,
            },
            cx,
        );
    }
    pub(super) fn refresh_weekly(&mut self, cx: &mut Context<'_, Self>) {
        if self.weekly.busy || self.weekly.reading {
            return;
        }
        self.weekly.reading = true;
        self.weekly.generation = self.weekly.generation.wrapping_add(1);
        let generation = self.weekly.generation;
        let client = self.client.clone();
        let before = self.weekly.before.clone();
        let operation = spawn_tokio(
            cx,
            async move { client.weekly_reports(before.as_deref()).await },
        );
        self.weekly.request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.weekly.generation != generation {
                    return;
                }
                this.weekly.reading = false;
                match result {
                    Ok(snapshot) => {
                        this.weekly.snapshot = snapshot;
                        this.weekly.load_error = None;
                    }
                    Err(error) => this.weekly.load_error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
    }

    fn weekly_command(&mut self, command: WeeklyReportCommand, cx: &mut Context<'_, Self>) {
        if self.weekly.busy {
            return;
        }
        self.weekly.busy = true;
        self.weekly.reading = false;
        self.weekly.error = None;
        self.weekly.notice = None;
        self.weekly.generation = self.weekly.generation.wrapping_add(1);
        let generation = self.weekly.generation;
        let saved_preset = matches!(&command, WeeklyReportCommand::SavePreset { .. });
        let edited = match &command {
            WeeklyReportCommand::EditReport { id, .. } => Some(id.clone()),
            _ => None,
        };
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move {
            let snapshot = client.weekly_report_command(&command).await?;
            let report = if let Some(id) = edited {
                Some(client.weekly_report(&id).await)
            } else {
                None
            };
            Ok::<_, ServerError>((snapshot, report))
        });
        self.weekly.request=Some(cx.spawn(async move |this,cx|{
            let result=operation.await;
            let _=this.update(cx,|this,cx|{
                if this.weekly.generation!=generation {return;}
                this.weekly.busy=false;
                match result {
                    Ok((snapshot,report))=>{
                        this.weekly.snapshot=snapshot;this.weekly.before=None;
                        this.weekly.notice=Some(if saved_preset {"Report preset saved."}else if report.is_some(){"Report saved."}else{"Report queued. Follow its run in activity below."}.into());
                        if saved_preset {this.weekly.editor=None;}
                        if let Some(report)=report {match report {
                            Ok(saved)=>{if let Some(WeeklyEditor::Report {report,..})=this.weekly.editor.as_deref_mut() {**report=saved;}},
                            Err(error)=>this.weekly.error=Some(format!("Report saved, but reloading failed: {}. Reopen it before editing again.",reactive_error(error))),
                        }}
                    },
                    Err(error)=>this.weekly.error=Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_weekly_preset(
        &mut self,
        preset: Option<WeeklyReportPreset>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.weekly.busy {
            return;
        }
        let preset = preset.unwrap_or_else(|| WeeklyReportPreset {
            id: uuid::Uuid::new_v4().to_string(),
            project_path: std::env::current_dir()
                .ok()
                .and_then(|path| path.to_str().map(str::to_string))
                .unwrap_or_default(),
            model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
            local_git: true,
            github_pull_requests: false,
            github_deployments: false,
            scheduled: false,
            schedule: None,
        });
        let path = edit_input(&preset.project_path, false, window, cx);
        let model = edit_input(&preset.model, false, window, cx);
        let schedule = Box::new(ScheduleEditor {
            fixed: preset.schedule.is_some(),
            weekday: preset.schedule.as_ref().map_or(1, |s| s.weekday),
            time: edit_input(
                preset
                    .schedule
                    .as_ref()
                    .map_or("09:00", |s| s.time.as_str()),
                false,
                window,
                cx,
            ),
            time_zone: edit_input(
                &preset
                    .schedule
                    .as_ref()
                    .map_or_else(super::editor_fields::local_zone, |s| s.time_zone.clone()),
                false,
                window,
                cx,
            ),
        });
        self.weekly.editor = Some(Box::new(WeeklyEditor::Preset {
            preset,
            path,
            model,
            schedule,
        }));
        self.weekly.error = None;
        cx.notify();
    }

    fn open_weekly_report(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.weekly.busy {
            return;
        }
        self.weekly.busy = true;
        self.weekly.reading = false;
        self.weekly.generation = self.weekly.generation.wrapping_add(1);
        let generation = self.weekly.generation;
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.weekly_report(&id).await });
        self.weekly.request = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.weekly.generation != generation {
                    return;
                }
                this.weekly.busy = false;
                match result {
                    Ok(report) => {
                        let title = edit_input(&report.title, false, window, cx);
                        let body = edit_input(&report.body, true, window, cx);
                        set_report_editor(&mut this.weekly, report, title, body);
                    }
                    Err(error) => this.weekly.error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn render_weekly_reports(&self, cx: &Context<'_, Self>) -> Div {
        // Web: the panel sits in the fieldset that waits for the first snapshot.
        let disabled = self.weekly.busy || self.busy || self.snapshot.is_none();
        let mut panel=div().min_w_0().w_full().flex().flex_col().gap_3().pb_4().border_b_1().border_color(theme::longan_700().opacity(0.6))
            .child(div().flex().flex_wrap().justify_between().gap_2().child(heading("Weekly engineering reports")).child(control("weekly-new","New report preset",disabled).on_activate(cx,|this,window,cx|this.open_weekly_preset(None,window,cx))))
            .child(muted("Review seven days of repository activity. Completed reports are saved locally for editing. Choose GitHub feeds when available. Incidents and reliability stay marked unavailable until a feed is connected."));
        if let Some(error) = &self.weekly.load_error {
            panel = panel.child(
                div()
                    .id("weekly-load-error")
                    .role(Role::Alert)
                    .text_size(px(13.0))
                    .text_color(theme::lychee_300())
                    .child(error.clone()),
            );
        }
        if let Some(error) = &self.weekly.error {
            panel = panel.child(
                div()
                    .id("weekly-report-error")
                    .role(Role::Alert)
                    .text_size(px(13.0))
                    .text_color(theme::lychee_300())
                    .child(error.clone()),
            );
        }
        if let Some(notice) = &self.weekly.notice {
            panel = panel.child(muted(notice.clone()));
        }
        if !self.weekly.snapshot.presets.is_empty() {
            if let Some(input) = &self.weekly.window_end {
                panel=panel.child(field("Report window end in UTC (optional)",input,false,disabled)).child(muted("For on-demand reports, leave blank for the latest seven days or enter a date such as 2026-11-13T15:00:00Z."));
            }
        }
        for preset in &self.weekly.snapshot.presets {
            let id = preset.id.clone();
            let edit = preset.clone();
            panel = panel.child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(muted(preset.project_path.clone()))
                    .child(muted(format!(
                        "{}  {}",
                        preset.model,
                        preset_cadence(preset)
                    )))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                control(
                                    SharedString::from(format!("weekly-run-{id}")),
                                    "Generate report now",
                                    disabled,
                                )
                                .on_activate(cx, move |this, _, cx| {
                                    this.generate_weekly_report(id.clone(), cx)
                                }),
                            )
                            .child(
                                control(
                                    SharedString::from(format!("weekly-edit-{}", preset.id)),
                                    "Edit preset",
                                    disabled,
                                )
                                .on_activate(
                                    cx,
                                    move |this, window, cx| {
                                        this.open_weekly_preset(Some(edit.clone()), window, cx)
                                    },
                                ),
                            ),
                    ),
            );
        }
        if self.weekly.snapshot.reports.is_empty() {
            panel=panel.child(muted("Saved reports appear here after a successful run. Set up a preset, then generate a report now or enable repeating reports."));
        }
        for report in &self.weekly.snapshot.reports {
            let id = report.id.clone();
            panel = panel.child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_2()
                    .child(heading(report.title.clone()))
                    .child(
                        control(
                            SharedString::from(format!("weekly-open-{id}")),
                            "Open report",
                            disabled,
                        )
                        .on_activate(cx, move |this, window, cx| {
                            this.open_weekly_report(id.clone(), window, cx)
                        }),
                    ),
            );
        }
        if let Some(before) = &self.weekly.snapshot.next_before {
            let before = before.clone();
            panel = panel.child(
                control("weekly-older", "Older reports", disabled).on_activate(
                    cx,
                    move |this, _, cx| {
                        this.weekly.before = Some(before.clone());
                        this.refresh_weekly(cx);
                    },
                ),
            );
        }
        if self.weekly.before.is_some() {
            panel = panel.child(
                control("weekly-newest", "Newest reports", disabled).on_activate(
                    cx,
                    |this, _, cx| {
                        this.weekly.before = None;
                        this.refresh_weekly(cx);
                    },
                ),
            );
        }
        if let Some(editor) = self.weekly.editor.as_deref() {
            panel = panel.child(self.render_weekly_editor(editor, cx));
        }
        panel
    }

    fn render_weekly_editor(&self, editor: &WeeklyEditor, cx: &Context<'_, Self>) -> Div {
        // Web: the panel sits in the fieldset that waits for the first snapshot.
        let disabled = self.weekly.busy || self.busy || self.snapshot.is_none();
        let mut panel = div().min_w_0().flex().flex_col().gap_3().pt_3();
        match editor {
            WeeklyEditor::Preset {
                preset,
                path,
                model,
                schedule,
            } => {
                panel = panel
                    .child(heading("Report preset"))
                    .child(field("Report project folder", path, false, disabled))
                    .child(field("Report model", model, false, disabled));
                for (key, label, checked) in [
                    ("git", "Local Git commits", preset.local_git),
                    (
                        "github",
                        "GitHub pull requests",
                        preset.github_pull_requests,
                    ),
                    (
                        "deployments",
                        "GitHub deployments",
                        preset.github_deployments,
                    ),
                    ("scheduled", "Repeat every seven days", preset.scheduled),
                ] {
                    panel = panel.child(
                        control(
                            SharedString::from(format!("weekly-check-{key}")),
                            label,
                            disabled,
                        )
                        .role(Role::CheckBox)
                        .when(checked, |button| {
                            button.bg(theme::blueberry_500().opacity(0.15))
                        })
                        .aria_toggled(if checked {
                            Toggled::True
                        } else {
                            Toggled::False
                        })
                        .on_activate(cx, move |this, _, cx| {
                            if let Some(WeeklyEditor::Preset { preset, .. }) =
                                this.weekly.editor.as_deref_mut()
                            {
                                match key {
                                    "git" => preset.local_git = !preset.local_git,
                                    "github" => {
                                        preset.github_pull_requests = !preset.github_pull_requests
                                    }
                                    "deployments" => {
                                        preset.github_deployments = !preset.github_deployments
                                    }
                                    _ => preset.scheduled = !preset.scheduled,
                                }
                            }
                            cx.notify();
                        }),
                    );
                }
                panel = panel.child(self.render_weekly_schedule(schedule, cx));
                panel=panel.child(muted("GitHub uses the host's gh login. Repeating reports run while the local server is available. New schedules first run after seven days. Paused schedules may catch up on resume.")).child(control("weekly-save-preset","Save report preset",disabled).on_activate(cx,|this,_,cx|{if let Some(WeeklyEditor::Preset {preset,path,model,schedule})=this.weekly.editor.as_deref(){let mut preset=preset.clone();preset.project_path=value(path,cx);preset.model=value(model,cx);match weekly_schedule(schedule.fixed,schedule.weekday,&value(&schedule.time,cx),&value(&schedule.time_zone,cx)){Ok(slot)=>{preset.schedule=slot;this.weekly_command(WeeklyReportCommand::SavePreset {preset},cx);}Err(error)=>{this.weekly.error=Some(error);cx.notify();}}}}));
            }
            WeeklyEditor::Report {
                report,
                title,
                body,
            } => {
                panel = panel
                    .child(heading("Edit saved report"))
                    .child(muted(format!("Project: {}", report.project_path)))
                    .child(muted(format!(
                        "{}  {} to {}",
                        report.model,
                        time(report.window_start),
                        time(report.window_end)
                    )))
                    .child(field("Report title", title, false, disabled))
                    .child(field("Report Markdown", body, true, disabled))
                    .child(heading("Sources and missing feeds"));
                for coverage in &report.coverage {
                    match coverage {
                        ReportCoverage::Unavailable { source, reason } => {
                            panel = panel.child(muted(format!("{source}: {reason}")))
                        }
                        ReportCoverage::Available {
                            source,
                            facts,
                            truncated,
                        } => {
                            panel = panel.child(muted(format!(
                                "{source}: {} records{}",
                                facts.len(),
                                if *truncated { " (partial feed)" } else { "" }
                            )));
                            for fact in facts {
                                panel = panel
                                    .child(muted(format!("{} ({})", fact.title, fact.reference)));
                                if let Some(url) = &fact.url {
                                    let url = url.clone();
                                    panel = panel.child(
                                        control(
                                            SharedString::from(format!(
                                                "weekly-source-{}-{}",
                                                report.id, fact.reference
                                            )),
                                            "Open source",
                                            disabled,
                                        )
                                        .on_activate(cx, move |_, _, cx| cx.open_url(&url)),
                                    );
                                }
                            }
                        }
                    }
                }
                let open = report.clone();
                panel = panel.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            control("weekly-save-report", "Save report edits", disabled)
                                .on_activate(cx, |this, _, cx| {
                                    if let Some(WeeklyEditor::Report {
                                        report,
                                        title,
                                        body,
                                    }) = this.weekly.editor.as_deref()
                                    {
                                        this.weekly_command(
                                            WeeklyReportCommand::EditReport {
                                                id: report.id.clone(),
                                                expected_revision: report.revision,
                                                title: value(title, cx),
                                                body: value(body, cx),
                                            },
                                            cx,
                                        );
                                    }
                                }),
                        )
                        .child(
                            control(
                                "weekly-open-thread",
                                "Open project and agent thread",
                                disabled,
                            )
                            .on_activate(
                                cx,
                                move |this, window, cx| {
                                    this.editor = Some(Editor::Chat {
                                        agent_id: format!("weekly-report-{}", open.preset_id),
                                        message: edit_input(&open.body, true, window, cx),
                                    });
                                    cx.emit(ReactiveProjectOpened {
                                        path: open.project_path.clone(),
                                    });
                                    cx.notify();
                                },
                            ),
                        ),
                );
            }
        }
        panel.child(
            control(
                "weekly-close-editor",
                if matches!(editor, WeeklyEditor::Preset { .. }) {
                    "Cancel setup"
                } else {
                    "Close report"
                },
                disabled,
            )
            .on_activate(cx, |this, _, cx| {
                this.weekly.editor = None;
                cx.notify();
            }),
        )
    }
}

impl ReactiveAutomations {
    fn render_weekly_schedule(&self, schedule: &ScheduleEditor, cx: &Context<'_, Self>) -> Div {
        // Web: the panel sits in the fieldset that waits for the first snapshot.
        let disabled = self.weekly.busy || self.busy || self.snapshot.is_none();
        let mut panel = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(muted("Schedule"))
            .child(
                div().flex().flex_wrap().gap_2().children(
                    [
                        (false, "Seven days after enabling"),
                        (true, "Fixed weekday and time"),
                    ]
                    .map(|(fixed, label)| {
                        let selected = schedule.fixed == fixed;
                        control(
                            SharedString::from(format!("weekly-schedule-{fixed}")),
                            label,
                            disabled,
                        )
                        .aria_selected(selected)
                        .when(selected, |button| {
                            button.bg(theme::blueberry_500().opacity(0.15))
                        })
                        .on_activate(cx, move |this, _, cx| {
                            if let Some(WeeklyEditor::Preset { schedule, .. }) =
                                this.weekly.editor.as_deref_mut()
                            {
                                schedule.fixed = fixed;
                            }
                            cx.notify();
                        })
                    }),
                ),
            );
        if schedule.fixed {
            panel =
                panel
                    .child(muted("Weekday"))
                    .child(div().flex().flex_wrap().gap_2().children(
                        bruhs_models::reactive::WEEKDAY_NAMES.iter().zip(1u8..).map(
                            |(name, day)| {
                                let selected = schedule.weekday == day;
                                control(
                                    SharedString::from(format!("weekly-weekday-{day}")),
                                    name,
                                    disabled,
                                )
                                .aria_selected(selected)
                                .when(selected, |button| {
                                    button.bg(theme::blueberry_500().opacity(0.15))
                                })
                                .on_activate(
                                    cx,
                                    move |this, _, cx| {
                                        if let Some(WeeklyEditor::Preset { schedule, .. }) =
                                            this.weekly.editor.as_deref_mut()
                                        {
                                            schedule.weekday = day;
                                        }
                                        cx.notify();
                                    },
                                )
                            },
                        ),
                    ))
                    .child(field("Time", &schedule.time, false, disabled))
                    .child(field("Time zone", &schedule.time_zone, false, disabled));
        }
        panel
    }
}

fn set_report_editor(
    state: &mut WeeklyReportsState,
    report: EngineeringReport,
    title: Entity<InputState>,
    body: Entity<InputState>,
) {
    state.editor = Some(Box::new(WeeklyEditor::Report {
        report: Box::new(report),
        title,
        body,
    }));
    state.error = None;
    state.notice = None;
}

#[cfg(test)]
mod schedule_tests {
    use super::{preset_cadence, weekly_schedule, WeeklyReportPreset, WeeklySchedule};

    #[test]
    fn fixed_weekly_schedule_validates_and_describes_the_slot() {
        assert_eq!(weekly_schedule(false, 1, "nope", ""), Ok(None));
        let slot = WeeklySchedule {
            weekday: 5,
            time: "16:30".into(),
            time_zone: "Europe/Paris".into(),
        };
        assert_eq!(
            weekly_schedule(true, 5, " 16:30 ", "Europe/Paris"),
            Ok(Some(slot.clone()))
        );
        assert!(weekly_schedule(true, 5, "4:30pm", "UTC").is_err());
        assert!(weekly_schedule(true, 5, "16:30", " ").is_err());
        assert_eq!(
            weekly_schedule(true, 0, "16:30", "UTC"),
            Err("Choose a weekday.".into())
        );
        let mut preset = WeeklyReportPreset {
            id: "p".into(),
            project_path: "/repo".into(),
            model: "m".into(),
            local_git: true,
            github_pull_requests: false,
            github_deployments: false,
            scheduled: true,
            schedule: None,
        };
        assert_eq!(preset_cadence(&preset), "Repeats every seven days");
        preset.schedule = Some(slot);
        assert_eq!(preset_cadence(&preset), "Fri at 16:30 (Europe/Paris)");
        preset.scheduled = false;
        assert_eq!(preset_cadence(&preset), "On demand");
    }
}

#[cfg(test)]
mod tests {
    use super::{value, ReactiveAutomations, WeeklyEditor};
    use crate::theme;
    use bruhs_models::{reactive::ReactiveSnapshot, weekly_reports::WeeklyReportsSnapshot};
    use gpui::AppContext as _;
    use std::rc::Rc;
    #[gpui::test]
    fn weekly_preset_controls_retain_inputs_and_toggle_only_on_owner_action(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            gpui_component::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(1400.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, window, cx| {
            panel.loaded = true;
            panel.snapshot = Some(ReactiveSnapshot::default());
            panel.open_weekly_preset(None, window, cx);
            let Some(WeeklyEditor::Preset {
                preset,
                path,
                model,
                ..
            }) = panel.weekly.editor.as_deref_mut()
            else {
                panic!("report preset editor");
            };
            assert!(!preset.scheduled);
            assert!(preset.local_git);
            assert!(!preset.github_pull_requests);
            path.update(cx, |input, cx| {
                input.set_value("/fixture/report project", window, cx)
            });
            model.update(cx, |input, cx| {
                input.set_value("claude-haiku-4-5", window, cx)
            });
            panel.weekly.snapshot = WeeklyReportsSnapshot::default();
            cx.notify();
        });
        cx.run_until_parked();
        let checkbox = cx
            .debug_bounds("reactive-Repeat every seven days")
            .expect("schedule checkbox");
        cx.simulate_click(checkbox.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        panel.update_in(cx, |panel, _, cx| {
            let Some(WeeklyEditor::Preset {
                preset,
                path,
                model,
                ..
            }) = panel.weekly.editor.as_deref()
            else {
                panic!("report preset editor");
            };
            assert!(preset.scheduled);
            assert_eq!(value(path, cx), "/fixture/report project");
            assert_eq!(value(model, cx), "claude-haiku-4-5");
            assert!(
                panel.weekly.snapshot.presets.is_empty(),
                "toggling a local form must not save or enable a preset"
            );
        });
    }
}
