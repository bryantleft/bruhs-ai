//! Smart scheduling side panel on the calendar: availability, scheduling links, conflicts and placing a task.

use super::*;

impl Workspace {
    pub(super) fn toggle_scheduling_panel(&mut self, cx: &mut Context<'_, Self>) {
        self.scheduling_visible = !self.scheduling_visible;
        self.close_transient_surfaces();
        cx.notify();
    }

    pub(super) fn cycle_scheduling_task(&mut self, cx: &mut Context<'_, Self>) {
        let ids = self
            .work_items
            .iter()
            .filter(|item| {
                !item.status.is_completed() && item.planning.time_block_event_ids.is_empty()
            })
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        if ids.is_empty() {
            self.scheduling_selected_task_id = None;
        } else {
            let next = self
                .scheduling_selected_task_id
                .as_ref()
                .and_then(|current| ids.iter().position(|id| id == current))
                .map_or(0, |index| (index + 1) % ids.len());
            self.scheduling_selected_task_id = Some(ids[next].clone());
        }
        cx.notify();
    }

    pub(super) fn create_scheduling_link(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let title = self
            .scheduling_link_title_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if title.is_empty() {
            self.persistence_error = Some("Give the scheduling link a title.".to_string());
            cx.notify();
            return;
        }
        let duration = self
            .scheduling_link_duration_input
            .read(cx)
            .value()
            .parse()
            .unwrap_or(30);
        self.scheduling_preferences.add_link(
            SchedulingLink::new(title, duration),
            EpochMs::now().as_millis(),
        );
        self.scheduling_link_title_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.persist_and_notify(cx);
    }

    pub(super) fn adjust_availability(
        &mut self,
        adjust_start: bool,
        delta_minutes: i16,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(rule) = self.scheduling_preferences.availability.first_mut() {
            if adjust_start {
                let start = (i32::from(rule.start_minute) + i32::from(delta_minutes))
                    .clamp(0, i32::from(rule.end_minute.saturating_sub(15)));
                rule.start_minute = u16::try_from(start).unwrap_or(rule.start_minute);
            } else {
                let end = (i32::from(rule.end_minute) + i32::from(delta_minutes))
                    .clamp(i32::from(rule.start_minute + 15), 1_440);
                rule.end_minute = u16::try_from(end).unwrap_or(rule.end_minute);
            }
            self.scheduling_preferences.updated_at_ms = EpochMs::now().as_millis();
            self.persist_and_notify(cx);
        }
    }

    pub(super) fn resolve_scheduling_conflict(
        &mut self,
        conflict_id: &str,
        resolution: ConflictResolution,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(conflict) = self
            .scheduling_preferences
            .conflicts
            .iter()
            .find(|conflict| conflict.id == conflict_id)
            .cloned()
        else {
            return;
        };
        if resolution == ConflictResolution::DuplicateLocal {
            if let (Some(start_at), Some(end_at)) = (
                local_datetime_from_ms(conflict.local_start_at_ms),
                local_datetime_from_ms(conflict.local_end_at_ms),
            ) {
                self.calendar_events.push(DesktopCalendarEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    title: format!("{} (local copy)", conflict.event_title),
                    start_at,
                    end_at,
                    all_day: false,
                    color: Some("persimmon".to_string()),
                    notes: Some("Preserved during remote calendar conflict".to_string()),
                });
            }
        }
        if matches!(
            resolution,
            ConflictResolution::AcceptRemote | ConflictResolution::DuplicateLocal
        ) {
            self.meetings_hub.update(cx, |hub, cx| {
                if let Some(event) = hub
                    .google_events
                    .iter_mut()
                    .find(|event| event.id == conflict.event_id)
                {
                    event.start_at_ms = conflict.remote_start_at_ms;
                    event.end_at_ms = conflict.remote_end_at_ms;
                    event.updated_at = EpochMs::now().as_millis();
                }
                cx.notify();
            });
        }
        if let Some(item) = self
            .scheduling_preferences
            .conflicts
            .iter_mut()
            .find(|item| item.id == conflict_id)
        {
            item.resolution = Some(resolution);
        }
        self.scheduling_preferences.updated_at_ms = EpochMs::now().as_millis();
        self.persist_and_notify(cx);
    }

    pub(super) fn schedule_selected_task(&mut self, cx: &mut Context<'_, Self>) {
        let Some(task_id) = self.scheduling_selected_task_id.clone() else {
            self.persistence_error = Some("Choose an unscheduled task first.".to_string());
            cx.notify();
            return;
        };
        let Some(task_index) = self.work_items.iter().position(|item| item.id == task_id) else {
            return;
        };
        let duration = self.work_items[task_index]
            .planning
            .estimate_minutes
            .unwrap_or(60)
            .max(15);
        let now = EpochMs::now().as_millis();
        let offset = i16::try_from(Local::now().offset().local_minus_utc() / 60).unwrap_or(0);
        let Some((start_ms, end_ms)) = self.scheduling_preferences.first_available_slot(
            &self.planning_calendar_events(cx),
            now,
            now + 30 * 86_400_000,
            duration,
            offset,
        ) else {
            self.persistence_error =
                Some("No open slot matches your availability in the next 30 days.".to_string());
            cx.notify();
            return;
        };
        let (Some(start_at), Some(end_at)) = (
            local_datetime_from_ms(EpochMs::saturating_from_i64(start_ms).as_f64()),
            local_datetime_from_ms(EpochMs::saturating_from_i64(end_ms).as_f64()),
        ) else {
            return;
        };
        let event_id = uuid::Uuid::new_v4().to_string();
        let title = self.work_items[task_index].title.clone();
        self.work_items[task_index]
            .planning
            .time_block_event_ids
            .push(event_id.clone());
        self.work_items[task_index].updated_at = EpochMs::now();
        self.calendar_events.push(DesktopCalendarEvent {
            id: event_id,
            title: format!("Focus: {title}"),
            start_at,
            end_at,
            all_day: false,
            color: Some("blueberry".to_string()),
            notes: Some(format!("Time block for task {task_id}")),
        });
        self.scheduling_selected_task_id = None;
        self.persistence_error = None;
        self.persist_and_notify(cx);
    }

    pub(super) fn ask_agent_about_schedule(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let prompt = bruhs_models::scheduling_agent_prompt(
            &self.scheduling_preferences,
            &self.planning_work_items(),
            &self.planning_calendar_events(cx),
        );
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            self.persistence_error =
                Some("Start a chat before asking the agent to plan.".to_string());
            cx.notify();
            return;
        };
        self.scheduling_visible = false;
        self.active_view = AppView::Chat;
        chat.update(cx, |chat, cx| chat.set_composer_draft(prompt, window, cx));
        self.persist_and_notify(cx);
    }

    pub(super) fn render_scheduling_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let task_label = self
            .scheduling_selected_task_id
            .as_ref()
            .and_then(|id| self.work_items.iter().find(|item| &item.id == id))
            .map_or("Choose an unscheduled task".to_string(), |item| {
                format!(
                    "{} · {} min",
                    item.title,
                    item.planning.estimate_minutes.unwrap_or(60)
                )
            });
        let links = self
            .scheduling_preferences
            .links
            .iter()
            .enumerate()
            .map(|(index, link)| {
                div()
                    .id(("desktop-scheduling-link", index))
                    .rounded(px(4.0))
                    .bg(theme::longan_900().opacity(0.55))
                    .px_2()
                    .py_1()
                    .flex()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_300())
                            .child(format!("{} · {} min", link.title, link.duration_minutes)),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme::blueberry_400())
                            .child(format!("#/book/{}", link.slug)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let availability = self.scheduling_preferences.availability.first().map_or(
            "No availability rule".to_string(),
            |rule| {
                format!(
                    "{} · weekdays · {:02}:{:02}–{:02}:{:02}",
                    rule.label,
                    rule.start_minute / 60,
                    rule.start_minute % 60,
                    rule.end_minute / 60,
                    rule.end_minute % 60
                )
            },
        );
        let conflicts = self
            .scheduling_preferences
            .conflicts
            .iter()
            .filter(|conflict| conflict.resolution.is_none())
            .enumerate()
            .map(|(index, conflict)| {
                let keep_id = conflict.id.clone();
                let remote_id = conflict.id.clone();
                let both_id = conflict.id.clone();
                div()
                    .id(("desktop-scheduling-conflict", index))
                    .rounded(px(4.0))
                    .bg(theme::lychee_500().opacity(0.08))
                    .p_2()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_300())
                            .child(format!(
                                "{} changed remotely. Your local time is preserved.",
                                conflict.event_title
                            )),
                    )
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .gap_1()
                            .child(
                                web_button(
                                    ("conflict-keep", index),
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Keep local")
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        this.resolve_scheduling_conflict(
                                            &keep_id,
                                            ConflictResolution::KeepLocal,
                                            cx,
                                        )
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    ("conflict-remote", index),
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Accept remote")
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        this.resolve_scheduling_conflict(
                                            &remote_id,
                                            ConflictResolution::AcceptRemote,
                                            cx,
                                        )
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    ("conflict-both", index),
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Keep both")
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        this.resolve_scheduling_conflict(
                                            &both_id,
                                            ConflictResolution::DuplicateLocal,
                                            cx,
                                        )
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(px(460.0))
            .max_w_full()
            .occlude()
            .bg(theme::longan_800())
            .border_l_1()
            .border_color(theme::longan_700())
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(
                        div()
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .text_size(px(16.0))
                            .text_color(theme::rambutan_100())
                            .child("Smart scheduling"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                web_button(
                                    "ask-agent-about-schedule",
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Ask agent to plan")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.ask_agent_about_schedule(window, cx)
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "close-scheduling",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Close")
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| this.toggle_scheduling_panel(cx),
                                )),
                            ),
                    ),
            )
            .child(
                div()
                    .rounded(px(6.0))
                    .bg(theme::longan_900().opacity(0.55))
                    .p_3()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_200())
                            .child("Focus protection"),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_500())
                            .child(format!(
                                "{} min/day · minimum {} min",
                                self.scheduling_preferences.focus.daily_target_minutes,
                                self.scheduling_preferences.focus.minimum_block_minutes
                            )),
                    )
                    .child(
                        web_button(
                            "toggle-focus-protection",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(if self.scheduling_preferences.focus.enabled {
                            "Protected"
                        } else {
                            "Off"
                        })
                        .on_click(cx.listener(
                            |this, _event, _window, cx| {
                                this.scheduling_preferences.focus.enabled =
                                    !this.scheduling_preferences.focus.enabled;
                                this.scheduling_preferences.updated_at_ms =
                                    EpochMs::now().as_millis();
                                this.persist_and_notify(cx);
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme::rambutan_500())
                    .child(availability),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        web_button(
                            "availability-start-minus",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Start −30m")
                        .on_click(cx.listener(
                            |this, _event, _window, cx| this.adjust_availability(true, -30, cx),
                        )),
                    )
                    .child(
                        web_button(
                            "availability-start-plus",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Start +30m")
                        .on_click(cx.listener(
                            |this, _event, _window, cx| this.adjust_availability(true, 30, cx),
                        )),
                    )
                    .child(
                        web_button(
                            "availability-end-minus",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("End −30m")
                        .on_click(cx.listener(
                            |this, _event, _window, cx| this.adjust_availability(false, -30, cx),
                        )),
                    )
                    .child(
                        web_button(
                            "availability-end-plus",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("End +30m")
                        .on_click(cx.listener(
                            |this, _event, _window, cx| this.adjust_availability(false, 30, cx),
                        )),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_200())
                    .child("Schedule a task"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        web_button(
                            "cycle-scheduling-task",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(task_label)
                        .on_click(
                            cx.listener(|this, _event, _window, cx| this.cycle_scheduling_task(cx)),
                        ),
                    )
                    .child(
                        web_button(
                            "schedule-task-now",
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(self.scheduling_selected_task_id.is_none()),
                        )
                        .child("Find time")
                        .on_click(
                            cx.listener(|this, _event, _window, cx| {
                                this.schedule_selected_task(cx)
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_200())
                    .child("Scheduling links"),
            )
            .child(Input::new(&self.scheduling_link_title_input))
            .child(Input::new(&self.scheduling_link_duration_input))
            .child(
                web_button(
                    "create-scheduling-link",
                    WebButtonVariant::Primary,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(
                        self.scheduling_link_title_input
                            .read(cx)
                            .value()
                            .trim()
                            .is_empty(),
                    ),
                )
                .child("Create link")
                .on_click(
                    cx.listener(|this, _event, window, cx| this.create_scheduling_link(window, cx)),
                ),
            )
            .children(links)
            .children(conflicts)
            .when_some(self.persistence_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme::lychee_300())
                        .child(error),
                )
            })
            .into_any_element()
    }
}

/// The scheduling-link form fields.
pub(super) struct SchedulingInputs {
    pub(super) scheduling_link_title_input: Entity<InputState>,
    pub(super) scheduling_link_duration_input: Entity<InputState>,
}

impl Workspace {
    /// The scheduling-link form fields (duration starts at 30); edits redraw.
    pub(super) fn new_scheduling_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> SchedulingInputs {
        let scheduling_link_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Product review"));
        subscriptions.push(redraw_on_edit(&scheduling_link_title_input, window, cx));
        let scheduling_link_duration_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("30"));
        scheduling_link_duration_input.update(cx, |input, cx| input.set_value("30", window, cx));
        subscriptions.push(redraw_on_edit(&scheduling_link_duration_input, window, cx));
        SchedulingInputs {
            scheduling_link_title_input,
            scheduling_link_duration_input,
        }
    }
}
