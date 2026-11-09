//! Today page: the day plan (draft, apply, undo) and its task and agenda sections.

use super::*;

impl Workspace {
    pub(super) fn planning_work_items(&self) -> Vec<WorkItem> {
        self.work_items
            .iter()
            .map(|item| WorkItem {
                id: item.id.clone(),
                project_id: item.project_id.clone(),
                title: item.title.clone(),
                description: item.description.clone(),
                status: item.status,
                priority: item.priority,
                due_date: item.due_date.clone(),
                planning: item.planning.clone(),
                created_at: 0,
                updated_at: 0,
            })
            .collect()
    }

    pub(super) fn planning_calendar_events(
        &self,
        cx: &Context<'_, Self>,
    ) -> Vec<bruhs_models::CalendarEvent> {
        self.visible_calendar_events(cx)
            .into_iter()
            .filter_map(|event| {
                let start_at_ms = Local
                    .from_local_datetime(&event.start_at)
                    .single()
                    .map(|date| EpochMs::saturating_from_i64(date.timestamp_millis()))?
                    .as_f64();
                let end_at_ms = Local
                    .from_local_datetime(&event.end_at)
                    .single()
                    .map(|date| EpochMs::saturating_from_i64(date.timestamp_millis()))?
                    .as_f64();
                Some(bruhs_models::CalendarEvent {
                    id: event.id,
                    title: event.title,
                    start_at_ms,
                    end_at_ms,
                    all_day: event.all_day,
                    color: event.color,
                    notes: event.notes,
                    created_at: 0,
                    updated_at: 0,
                    source: Default::default(),
                    google_event_id: None,
                    calendar_id: None,
                    attendees: Vec::new(),
                    organizer: None,
                    location: None,
                    conferencing: None,
                    recurring_event_id: None,
                    html_link: None,
                })
            })
            .collect()
    }

    pub(super) fn today_snapshot(&self, cx: &Context<'_, Self>) -> TodaySnapshot {
        let now = Local::now();
        let day_start = now.date_naive().and_time(NaiveTime::MIN);
        let day_end = day_start + Duration::days(1);
        let day_start_ms = Local
            .from_local_datetime(&day_start)
            .single()
            .map_or(0.0, |date| {
                EpochMs::saturating_from_i64(date.timestamp_millis()).as_f64()
            });
        let day_end_ms = Local
            .from_local_datetime(&day_end)
            .single()
            .map_or(day_start_ms + 86_400_000.0, |date| {
                EpochMs::saturating_from_i64(date.timestamp_millis()).as_f64()
            });
        let hub = self.meetings_hub.read(cx);
        TodaySnapshot::build(
            EpochMs::saturating_from_i64(now.timestamp_millis()).as_f64(),
            day_start_ms,
            day_end_ms,
            &now.date_naive().format(DESKTOP_DATE_FORMAT).to_string(),
            &self.planning_work_items(),
            &self.planning_calendar_events(cx),
            &hub.meetings,
            &hub.people,
        )
    }

    pub(super) fn draft_desktop_day_plan(&mut self, cx: &mut Context<'_, Self>) {
        let now = Local::now();
        let day_start = now.date_naive().and_time(NaiveTime::MIN);
        let day_start_ms = Local
            .from_local_datetime(&day_start)
            .single()
            .map_or(0.0, |date| {
                EpochMs::saturating_from_i64(date.timestamp_millis()).as_f64()
            });
        self.day_plan_proposal = Some(bruhs_models::propose_day_plan(
            EpochMs::saturating_from_i64(now.timestamp_millis()).as_f64(),
            day_start_ms,
            &self.planning_work_items(),
            &self.planning_calendar_events(cx),
        ));
        self.day_plan_undo = None;
        cx.notify();
    }

    pub(super) fn apply_desktop_day_plan(&mut self, cx: &mut Context<'_, Self>) {
        let Some(proposal) = self.day_plan_proposal.take() else {
            return;
        };
        let mut previous_planning = Vec::new();
        let mut event_ids = Vec::new();
        for block in proposal.blocks {
            let Some(start_at) = local_datetime_from_ms(block.start_at_ms) else {
                continue;
            };
            let Some(end_at) = local_datetime_from_ms(block.end_at_ms) else {
                continue;
            };
            if let Some(item) = self
                .work_items
                .iter_mut()
                .find(|item| item.id == block.task_id)
            {
                previous_planning.push((item.id.clone(), item.planning.clone()));
                item.planning.time_block_event_ids.push(block.id.clone());
                item.updated_at = EpochMs::now();
            }
            event_ids.push(block.id.clone());
            self.calendar_events.push(DesktopCalendarEvent {
                id: block.id,
                title: format!("Focus · {}", block.task_title),
                start_at,
                end_at,
                all_day: false,
                color: Some("persimmon".to_string()),
                notes: Some(format!("Planned from task {}", block.task_id)),
            });
        }
        self.day_plan_undo = Some(DesktopAppliedPlan {
            previous_planning,
            event_ids,
        });
        self.persist_and_notify(cx);
    }

    pub(super) fn undo_desktop_day_plan(&mut self, cx: &mut Context<'_, Self>) {
        let Some(applied) = self.day_plan_undo.take() else {
            return;
        };
        for (id, planning) in applied.previous_planning {
            if let Some(item) = self.work_items.iter_mut().find(|item| item.id == id) {
                item.planning = planning;
                item.updated_at = EpochMs::now();
            }
        }
        self.calendar_events
            .retain(|event| !applied.event_ids.contains(&event.id));
        self.persist_and_notify(cx);
    }

    pub(super) fn render_today_task_row(
        &self,
        item: &WorkItem,
        detail: String,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let item_id = item.id.clone();
        div()
            .id(("desktop-today-task", element_key(&item.id)))
            .w_full()
            .rounded(px(6.0))
            .px_2()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open task {}", item.title))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.selected_work_item_id = Some(item_id.clone());
                this.active_view = AppView::Tasks;
                this.persist_and_notify(cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.0))
                    .text_color(theme::rambutan_200())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(item.title.clone()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme::rambutan_500())
                    .child(detail),
            )
            .into_any_element()
    }

    pub(super) fn render_today_section(
        &self,
        title: &'static str,
        rows: Vec<AnyElement>,
        empty: &'static str,
    ) -> AnyElement {
        div()
            .flex_1()
            .min_w(px(250.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(gpui::white().opacity(0.05))
            .bg(theme::longan_900())
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .font_family(theme::HEADING_FONT_FAMILY)
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::rambutan_100())
                    .child(title),
            )
            .when(rows.is_empty(), |element| {
                element.child(
                    div()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme::longan_700().opacity(0.6))
                        .px_3()
                        .py_3()
                        .text_size(px(12.0))
                        .text_color(theme::rambutan_500())
                        .child(empty),
                )
            })
            .children(rows)
            .into_any_element()
    }

    pub(super) fn render_today_page(&self, compact: bool, cx: &Context<'_, Self>) -> Div {
        let snapshot = self.today_snapshot(cx);
        let date = Local::now().format("%A, %B %-d").to_string();
        let focus_rows = snapshot
            .focus_queue
            .iter()
            .map(|item| {
                let estimate = item
                    .planning
                    .estimate_minutes
                    .map_or_else(|| "45m".to_string(), |minutes| format!("{minutes}m"));
                self.render_today_task_row(item, estimate, cx)
            })
            .collect::<Vec<_>>();
        let overdue_rows = snapshot
            .overdue
            .iter()
            .map(|item| {
                self.render_today_task_row(item, item.due_date.clone().unwrap_or_default(), cx)
            })
            .collect::<Vec<_>>();
        let waiting_rows = snapshot
            .waiting_on
            .iter()
            .map(|item| {
                self.render_today_task_row(
                    item,
                    item.planning.waiting_on.clone().unwrap_or_default(),
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let schedule_rows = snapshot
            .now
            .iter()
            .chain(snapshot.next.iter())
            .map(|event| {
                let now_ms = EpochMs::now().as_f64();
                let label = if event.start_at_ms <= now_ms && event.end_at_ms > now_ms {
                    "Now".to_string()
                } else {
                    local_datetime_from_ms(event.start_at_ms)
                        .map_or_else(String::new, |time| time.format("%-I:%M%P").to_string())
                };
                let event_id = event.event_id.clone();
                let event_title = event.title.clone();
                div()
                    .id(("desktop-today-event", element_key(&event.event_id)))
                    .w_full()
                    .rounded(px(6.0))
                    .px_2()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.selected_calendar_event_id = Some(event_id.clone());
                        this.calendar_mode = DesktopCalendarMode::Day;
                        this.active_view = AppView::Calendar;
                        this.persist_and_notify(cx);
                    }))
                    .child(
                        div()
                            .w(px(50.0))
                            .text_size(px(11.0))
                            .text_color(theme::persimmon_300())
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.0))
                            .text_color(theme::rambutan_200())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(event_title),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        let briefing = div()
            .rounded(px(14.0))
            .border_1()
            .border_color(gpui::white().opacity(0.05))
            .bg(theme::longan_900())
            .p_4()
            .flex()
            .gap_3()
            .child(
                div()
                    .size(px(32.0))
                    .rounded(px(6.0))
                    .bg(theme::persimmon_500().opacity(0.15))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::persimmon_300())
                    .child("✦"),
            )
            .child(
                div()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::persimmon_300())
                            .child("BRIEFING"),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(13.0))
                            .text_color(theme::rambutan_200())
                            .child(snapshot.briefing),
                    ),
            );

        let suggestion = self
            .automation_state
            .surfaceable_suggestions(EpochMs::now().as_millis())
            .into_iter()
            .next()
            .map(|suggestion| {
                let id = suggestion.id.clone();
                div()
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(theme::blueberry_400().opacity(0.2))
                    .bg(theme::blueberry_500().opacity(0.1))
                    .p_4()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(theme::blueberry_400())
                                    .child("SUGGESTED LOCALLY"),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(px(12.0))
                                    .text_color(theme::rambutan_100())
                                    .child(suggestion.title.clone()),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_400())
                                    .child(suggestion.explanation.clone()),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(px(10.0))
                                    .text_color(theme::blueberry_400())
                                    .child(format!("Evidence: {}", suggestion.evidence)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                web_button(
                                    "today-dismiss-agent-suggestion",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Dismiss")
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        let _ = this
                                            .automation_state
                                            .dismiss_suggestion(&id, EpochMs::now().as_millis());
                                        this.persist_and_notify(cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "today-review-agent-suggestion",
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Review")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.activate_sidebar_view(AppView::Automations, window, cx)
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            });

        let proposal = self.day_plan_proposal.as_ref().map(|proposal| {
            let rows = proposal
                .blocks
                .iter()
                .map(|block| {
                    let start = local_datetime_from_ms(block.start_at_ms)
                        .map_or_else(String::new, |time| time.format("%-I:%M%P").to_string());
                    let end = local_datetime_from_ms(block.end_at_ms)
                        .map_or_else(String::new, |time| time.format("%-I:%M%P").to_string());
                    div()
                        .flex()
                        .gap_3()
                        .px_2()
                        .py_1()
                        .text_size(px(12.0))
                        .child(
                            div()
                                .w(px(105.0))
                                .text_color(theme::persimmon_300())
                                .child(format!("{start}–{end}")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_color(theme::rambutan_200())
                                .child(block.task_title.clone()),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            div()
                .rounded(px(12.0))
                .border_1()
                .border_color(theme::persimmon_400().opacity(0.25))
                .bg(theme::persimmon_500().opacity(0.08))
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::rambutan_100())
                        .child(format!("Review {} focus block(s)", proposal.blocks.len())),
                )
                .children(rows)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            web_button(
                                "cancel-day-plan",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-today-plan-cancel".to_string())
                            .child("Cancel")
                            .on_click(cx.listener(
                                |this, _event, _window, cx| {
                                    this.day_plan_proposal = None;
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            web_button(
                                "apply-day-plan",
                                WebButtonVariant::Primary,
                                WebButtonSize::Small,
                                ButtonState::disabled_if(proposal.blocks.is_empty()),
                            )
                            .debug_selector(|| "desktop-today-plan-apply".to_string())
                            .child("Apply plan")
                            .on_click(cx.listener(
                                |this, _event, _window, cx| {
                                    this.apply_desktop_day_plan(cx);
                                },
                            )),
                        ),
                )
                .into_any_element()
        });

        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_hidden()
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(0.5))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .child(
                                div()
                                    .font_family(theme::HEADING_FONT_FAMILY)
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_100())
                                    .child("Today"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child(date),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                web_button(
                                    "open-agent-workflows",
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Agent workflows")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.activate_sidebar_view(AppView::Automations, window, cx)
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "open-life-context",
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .child("Life context")
                                .on_click(cx.listener(
                                    |this, _event, window, cx| {
                                        this.activate_sidebar_view(AppView::Life, window, cx)
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    "plan-my-day",
                                    WebButtonVariant::Primary,
                                    WebButtonSize::Small,
                                    ButtonState::disabled_if(self.day_plan_proposal.is_some()),
                                )
                                .debug_selector(|| "desktop-today-plan".to_string())
                                .child("Plan my day")
                                .on_click(cx.listener(
                                    |this, _event, _window, cx| {
                                        this.draft_desktop_day_plan(cx);
                                    },
                                )),
                            ),
                    ),
            )
            .child(
                div()
                    .id("desktop-today-scroll")
                    .debug_selector(|| "desktop-today-scroll".to_string())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_5()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(1120.0))
                            .flex()
                            .flex_col()
                            .gap_4()
                            .children(proposal)
                            .children(suggestion)
                            .when_some(self.day_plan_undo.as_ref(), |element, _| {
                                element.child(
                                    div()
                                        .rounded(px(6.0))
                                        .bg(theme::pandan_500().opacity(0.1))
                                        .px_3()
                                        .py_2()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .text_size(px(12.0))
                                        .text_color(theme::pandan_300())
                                        .child("Day plan applied as one local transaction.")
                                        .child(
                                            div()
                                                .id("desktop-undo-day-plan")
                                                .debug_selector(|| {
                                                    "desktop-today-plan-undo".to_string()
                                                })
                                                .cursor_pointer()
                                                .child("Undo")
                                                .on_click(cx.listener(
                                                    |this, _event, _window, cx| {
                                                        this.undo_desktop_day_plan(cx);
                                                    },
                                                )),
                                        ),
                                )
                            })
                            .child(briefing)
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_4()
                                    .child(self.render_today_section(
                                        "Now & next",
                                        schedule_rows,
                                        "Nothing else on the calendar today.",
                                    ))
                                    .child(self.render_today_section(
                                        "Focus queue",
                                        focus_rows,
                                        "No ready tasks.",
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .when(compact, |element| element.flex_col())
                                    .when(!compact, |element| element.flex_row())
                                    .gap_4()
                                    .child(self.render_today_section(
                                        "Overdue",
                                        overdue_rows,
                                        "Nothing overdue.",
                                    ))
                                    .child(self.render_today_section(
                                        "Waiting on",
                                        waiting_rows,
                                        "Nothing is blocked on someone else.",
                                    ))
                                    .child(
                                        self.render_today_section(
                                            "Follow-ups",
                                            snapshot
                                                .follow_ups
                                                .into_iter()
                                                .take(4)
                                                .map(|item| {
                                                    div()
                                                        .px_2()
                                                        .py_2()
                                                        .text_size(px(12.0))
                                                        .text_color(theme::rambutan_300())
                                                        .child(format!(
                                                            "{} · {}",
                                                            item.person_name, item.text
                                                        ))
                                                        .into_any_element()
                                                })
                                                .collect(),
                                            "No recent follow-ups.",
                                        ),
                                    )
                                    .child(
                                        self.render_today_section(
                                            "Decisions",
                                            snapshot
                                                .recent_decisions
                                                .into_iter()
                                                .take(4)
                                                .map(|item| {
                                                    div()
                                                        .px_2()
                                                        .py_2()
                                                        .text_size(px(12.0))
                                                        .text_color(theme::rambutan_300())
                                                        .child(item.text)
                                                        .into_any_element()
                                                })
                                                .collect(),
                                            "No recent meeting decisions.",
                                        ),
                                    ),
                            ),
                    ),
            )
    }
}
