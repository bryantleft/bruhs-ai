//! Today page: the day plan (draft, apply, undo) and its task and agenda sections.

use super::*;
use bruhs_ui::zoom::px;

use bruhs_models::today_copy;

/// Web "Calendar →" at the right of Now & next.
const TODAY_CALENDAR_LINK_LABEL: &str = "Calendar →";
/// Web `text-h4`: each Today card's title.
const TODAY_CARD_TITLE_PX: f32 = 16.0;
/// Web `rounded-2xl`: Today's cards.
const TODAY_CARD_RADIUS_PX: f32 = 16.0;
/// Web `rounded-grape`: a row inside a card.
const TODAY_ROW_RADIUS_PX: f32 = 8.0;
/// Web `size-6`: a focus queue row's rank.
const TODAY_RANK_PX: f32 = 24.0;
/// Web `w-9`: a focus queue row's estimate.
const TODAY_ESTIMATE_WIDTH_PX: f32 = 36.0;
/// Web `size-2`: a status card's dot.
const TODAY_DOT_PX: f32 = 8.0;
/// Web's `lg` breakpoint: Now & next beside Focus queue, 0.9 to 1.1.
const TODAY_TWO_COLUMNS_PX: f32 = 1024.0;
/// Web's `md` and `xl` breakpoints: the status cards two, then four, to a row.
const TODAY_STATUS_TWO_COLUMNS_PX: f32 = 768.0;
const TODAY_STATUS_FOUR_COLUMNS_PX: f32 = 1280.0;
/// Today's day-plan wording, as the web's Today page and day-plan store
/// say it.
const DAY_PLAN_APPLIED: &str = "Day plan applied.";
const DAY_PLAN_SAVING: &str = "Saving day plan…";
const DAY_PLAN_SAVING_SHORT: &str = "Saving…";
const DAY_PLAN_NOT_SAVED: &str = "Day plan was not saved.";
const DAY_PLAN_NOT_LOADED: &str =
    "Day-plan recovery has not loaded. Retry loading before planning.";
const DAY_PLAN_REFRESH_LABEL: &str = "Refresh saved tasks and calendar";
const DAY_PLAN_RETRY_LABEL: &str = "Retry day-plan recovery";
const DAY_PLAN_DISMISS_LABEL: &str = "Dismiss";
/// Desktop reads the saved workspace for both recoveries; these say when
/// it could not.
const DAY_PLAN_UNREADABLE: &str =
    "The saved day plan could not be read. Keep this window open and retry.";
const DAY_PLAN_SOURCES_UNREADABLE: &str =
    "Saved tasks and calendar could not be read. Keep this window open and retry.";

/// What the saved workspace held when Today's refresh read it again.
enum SavedSources {
    /// The version this window last read or wrote: nothing newer exists.
    Unchanged,
    /// Another instance saved over it.
    Changed,
    Unreadable,
}

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
                updated_at: item.updated_at.as_millis(),
            })
            .collect()
    }

    pub(super) fn planning_calendar_events(
        &self,
        cx: &Context<'_, Self>,
    ) -> Vec<bruhs_models::CalendarEvent> {
        planning_calendar_events_in(self.visible_calendar_events(cx), local_offset_minutes_east)
    }

    pub(super) fn today_snapshot(&self, cx: &Context<'_, Self>) -> TodaySnapshot {
        let now = self.planning_now();
        let day_start = now.date_naive().and_time(NaiveTime::MIN);
        let day_end = day_start + Duration::days(1);
        let day_start_ms = EpochMs::saturating_from_i64(instant_from_wall_clock(
            day_start,
            local_offset_minutes_east,
        ))
        .as_f64();
        let day_end_ms = EpochMs::saturating_from_i64(instant_from_wall_clock(
            day_end,
            local_offset_minutes_east,
        ))
        .as_f64();
        let hub = self.meetings_hub.read(cx);
        TodaySnapshot::build(
            EpochMs::saturating_from_i64(now.timestamp_millis()).as_f64(),
            day_start_ms,
            day_end_ms,
            &now.date_naive().format(DESKTOP_DATE_FORMAT).to_string(),
            &self.planning_work_items(),
            &self.planning_calendar_events(cx),
            &hub.meetings,
            hub.people.iter().map(Arc::as_ref),
        )
    }

    pub(super) fn planning_now(&self) -> chrono::DateTime<Local> {
        #[cfg(test)]
        if let Some(now) = self.day_plan_test_now {
            return now;
        }
        Local::now()
    }

    /// Web Today's local suggestion, offered once the first unfinished task
    /// is stored and saved with the change that added it.
    pub(super) fn offer_focus_suggestion(&mut self) {
        if self
            .automation_state
            .suggestions
            .iter()
            .any(|suggestion| suggestion.id == bruhs_models::PROTECT_FOCUS_SUGGESTION_ID)
        {
            return;
        }
        self.automation_state.offer_focus_suggestion(
            unfinished_task_count(&self.work_items),
            self.calendar_events.len(),
            EpochMs::now().as_millis(),
        );
    }

    /// A Today task row opens the task on Tasks.
    fn open_today_task(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.selected_work_item_id = Some(id.to_string());
        self.active_view = AppView::Tasks;
        self.workspace_focus_handle.focus(window, cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn draft_desktop_day_plan(&mut self, cx: &mut Context<'_, Self>) {
        let now = self.planning_now();
        let result = native_workday_window(now.date_naive()).and_then(|window| {
            bruhs_models::propose_day_plan_in_window(
                EpochMs::saturating_from_i64(now.timestamp_millis()).as_f64(),
                window,
                &self.planning_work_items(),
                &self.planning_calendar_events(cx),
            )
        });
        match result {
            Ok(proposal) => {
                self.day_plan_proposal = Some(proposal);
                self.day_plan_error = None;
            }
            Err(error) => self.day_plan_error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(super) fn apply_desktop_day_plan(&mut self, cx: &mut Context<'_, Self>) {
        if self.day_plan_busy() {
            return;
        }
        let Some(proposal) = self.day_plan_proposal.as_ref() else {
            return;
        };
        if !self.day_plan_loaded {
            self.day_plan_error = Some(DAY_PLAN_NOT_LOADED.into());
            cx.notify();
            return;
        }
        if !self.save_status.is_saved() {
            self.day_plan_error =
                Some("Finish saving current changes before applying the day plan.".into());
            cx.notify();
            return;
        }
        let now = self.planning_now();
        let result = native_workday_window(now.date_naive()).and_then(|window| {
            bruhs_models::prepare_day_plan(
                proposal,
                &self.planning_work_items(),
                &self.planning_calendar_events(cx),
                window,
                now.timestamp_millis(),
            )
        });
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(error) => {
                self.day_plan_error = Some(error.to_string());
                self.day_plan_proposal = None;
                cx.notify();
                return;
            }
        };
        let events = prepared
            .added_events
            .iter()
            .map(|event| {
                Some(DesktopCalendarEvent {
                    id: event.id.clone(),
                    title: event.title.clone(),
                    start_at: local_datetime_from_ms(event.start_at_ms)?,
                    end_at: local_datetime_from_ms(event.end_at_ms)?,
                    all_day: event.all_day,
                    color: event.color.clone(),
                    notes: event.notes.clone(),
                    ..Default::default()
                })
            })
            .collect::<Option<Vec<_>>>();
        let Some(events) = events else {
            self.day_plan_error =
                Some("A planned time cannot be represented locally. Review a new plan.".into());
            cx.notify();
            return;
        };
        for changed in prepared.changed_tasks {
            if let Some(current) = self
                .work_items
                .iter_mut()
                .find(|task| task.id == changed.id)
            {
                current.planning = changed.planning;
                current.updated_at = current
                    .updated_at
                    .max(EpochMs::saturating_from_i64(now.timestamp_millis()));
            }
        }
        self.calendar_events.extend(events);
        self.day_plan_undo = Some(DesktopAppliedPlan {
            ownership: prepared.ownership,
        });
        self.day_plan_proposal = None;
        self.day_plan_error = None;
        self.save_day_plan(cx);
    }

    pub(super) fn undo_desktop_day_plan(&mut self, cx: &mut Context<'_, Self>) {
        if self.day_plan_busy() {
            return;
        }
        let Some(applied) = self.day_plan_undo.as_ref() else {
            return;
        };
        if !self.day_plan_loaded {
            self.day_plan_error = Some(DAY_PLAN_NOT_LOADED.into());
            cx.notify();
            return;
        }
        if !self.save_status.is_saved() {
            self.day_plan_error =
                Some("Finish saving current changes before undoing the day plan.".into());
            cx.notify();
            return;
        }
        let now = self.planning_now();
        let prepared = match bruhs_models::prepare_day_plan_undo(
            &applied.ownership,
            &self.planning_work_items(),
            &self.planning_calendar_events(cx),
            now.timestamp_millis(),
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.day_plan_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        for changed in prepared.changed_tasks {
            if let Some(current) = self
                .work_items
                .iter_mut()
                .find(|task| task.id == changed.id)
            {
                current.planning = changed.planning;
                current.updated_at = current
                    .updated_at
                    .max(EpochMs::saturating_from_i64(now.timestamp_millis()));
            }
        }
        self.calendar_events
            .retain(|event| !prepared.removed_event_ids.contains(&event.id));
        self.day_plan_undo = None;
        self.day_plan_error = None;
        self.save_day_plan(cx);
    }

    /// Web `DayPlanState::busy`: a day plan's save or a recovery read is
    /// running.
    pub(super) fn day_plan_busy(&self) -> bool {
        self.day_plan_saving || self.day_plan_reading
    }

    /// Saves an applied or undone plan and waits for that save, so Today
    /// can say whether it reached the disk.
    fn save_day_plan(&mut self, cx: &mut Context<'_, Self>) {
        self.persist_and_notify(cx);
        // A workspace without a state file (most tests) saves nothing.
        self.day_plan_saving = self.save_status == SaveStatus::Saving;
    }

    /// The save carrying a day plan ended. Once it is on disk Today stops
    /// saying "Saving…"; when it failed, Today says so and offers the web's
    /// recovery.
    pub(super) fn finish_day_plan_save(&mut self) {
        if self.save_status.is_saved() && self.day_plan_error.as_deref() == Some(DAY_PLAN_NOT_SAVED)
        {
            // A later save, such as the save strip's Retry, carried the plan.
            self.day_plan_error = None;
        }
        if !self.day_plan_saving {
            return;
        }
        match self.save_status {
            // A later batch carries the plan.
            SaveStatus::Saving => {}
            SaveStatus::Saved => self.day_plan_saving = false,
            SaveStatus::Failed(_) | SaveStatus::Conflict | SaveStatus::RecoveryRequired(_) => {
                self.day_plan_saving = false;
                self.day_plan_error = Some(DAY_PLAN_NOT_SAVED.into());
            }
        }
    }

    #[cfg(not(test))]
    fn saved_workspace_path(&self) -> Option<PathBuf> {
        desktop_state_path()
    }

    /// Tests save only when they opt in with a file of their own.
    #[cfg(test)]
    fn saved_workspace_path(&self) -> Option<PathBuf> {
        self.persistence_test_path.clone()
    }

    /// Web "Refresh saved tasks and calendar". The web re-reads its stores
    /// because another tab may have changed them. Desktop has one writer
    /// per workspace, so its saved tasks and calendar are never newer than
    /// this window's unless another instance saved over them. The saved
    /// workspace is read again off the UI thread: when it is still the
    /// version this window last read or wrote, the window's records stay
    /// (they hold every unsaved change), the error clears and a failed
    /// save is tried again; when another instance changed it, Today says so
    /// and saving pauses, as the save strip's conflict does.
    pub(super) fn refresh_day_plan_sources(&mut self, cx: &mut Context<'_, Self>) {
        if self.day_plan_busy() {
            return;
        }
        if !self.day_plan_loaded {
            self.day_plan_error = Some(DAY_PLAN_NOT_LOADED.into());
            cx.notify();
            return;
        }
        if let SaveStatus::RecoveryRequired(_) = self.save_status {
            // Saving is paused until the saved workspace is recovered.
            self.day_plan_error = self.save_status.alert().map(str::to_string);
            cx.notify();
            return;
        }
        let Some(path) = self.saved_workspace_path() else {
            self.finish_day_plan_refresh(SavedSources::Unchanged, cx);
            return;
        };
        self.day_plan_reading = true;
        cx.notify();
        let coordinator = self.persistence.clone();
        cx.spawn(async move |workspace, cx| {
            let sources = cx
                .background_spawn(async move {
                    // Holding the writer's lock keeps a save from landing
                    // between the two reads.
                    let Ok(expected) = coordinator.write_lock.lock() else {
                        return SavedSources::Unreadable;
                    };
                    let sources = match DiskVersion::read(&path) {
                        Ok(actual) if actual == *expected => SavedSources::Unchanged,
                        Ok(_) => SavedSources::Changed,
                        Err(_) => SavedSources::Unreadable,
                    };
                    drop(expected);
                    sources
                })
                .await;
            let _ = workspace.update(cx, |this, cx| this.finish_day_plan_refresh(sources, cx));
        })
        .detach();
    }

    fn finish_day_plan_refresh(&mut self, sources: SavedSources, cx: &mut Context<'_, Self>) {
        self.day_plan_reading = false;
        match sources {
            SavedSources::Unchanged => {
                self.day_plan_error = None;
                if matches!(self.save_status, SaveStatus::Failed(_)) {
                    self.save_day_plan(cx);
                }
            }
            SavedSources::Changed => {
                self.day_plan_error = Some(SaveError::Conflict.to_string());
                self.save_status = SaveStatus::Conflict;
            }
            SavedSources::Unreadable => {
                self.day_plan_error = Some(DAY_PLAN_SOURCES_UNREADABLE.into());
            }
        }
        cx.notify();
    }

    /// Web "Retry day-plan recovery": read the saved day plan again after
    /// the saved workspace could not be read at startup, from the state
    /// file or its backup as startup does. Only the plan's undo receipt is
    /// restored; saving stays paused until the workspace is recovered.
    pub(super) fn retry_day_plan_recovery(&mut self, cx: &mut Context<'_, Self>) {
        if self.day_plan_loaded || self.day_plan_busy() {
            return;
        }
        let Some(path) = self.saved_workspace_path() else {
            self.finish_day_plan_recovery(Ok(None), cx);
            return;
        };
        self.day_plan_reading = true;
        cx.notify();
        cx.spawn(async move |workspace, cx| {
            let receipt = cx
                .background_spawn(async move {
                    match load_desktop_state_with_backup(&path) {
                        (Some(state), _, _) => Ok(state.day_plan_undo),
                        // No saved workspace: no plan to recover.
                        (None, None, _) => Ok(None),
                        (None, Some(_), _) => Err(()),
                    }
                })
                .await;
            let _ = workspace.update(cx, |this, cx| this.finish_day_plan_recovery(receipt, cx));
        })
        .detach();
    }

    fn finish_day_plan_recovery(
        &mut self,
        receipt: Result<Option<DesktopAppliedPlan>, ()>,
        cx: &mut Context<'_, Self>,
    ) {
        self.day_plan_reading = false;
        match receipt {
            Ok(receipt) => {
                if self.day_plan_undo.is_none() {
                    self.day_plan_undo = receipt;
                }
                self.day_plan_loaded = true;
                self.day_plan_error = None;
            }
            Err(()) => self.day_plan_error = Some(DAY_PLAN_UNREADABLE.into()),
        }
        cx.notify();
    }

    fn open_today_calendar_event(
        &mut self,
        event_id: &str,
        event_date: Option<NaiveDate>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(date) = event_date {
            self.calendar_anchor = date;
        }
        self.selected_calendar_event_id = Some(event_id.to_string());
        self.calendar_mode = DesktopCalendarMode::Day;
        self.active_view = AppView::Calendar;
        self.workspace_focus_handle.focus(window, cx);
        self.persist_and_notify(cx);
    }

    /// A recent decision opens the meeting it was made in (web
    /// `MeetingDetail`).
    fn open_today_decision(
        &mut self,
        meeting_id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_meeting_detail(meeting_id, cx);
        self.workspace_focus_handle.focus(window, cx);
    }

    /// A follow-up opens its person's profile on People.
    fn open_today_follow_up(
        &mut self,
        person_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_person_profile(person_id, window, cx);
        self.workspace_focus_handle.focus(window, cx);
    }

    /// A Today row that opens a record: web's `w-full rounded-grape px-2
    /// py-2 text-left hover:bg-longan-800/60` button.
    fn today_record_row(id: ElementId, label: String) -> Stateful<Div> {
        div()
            .id(id)
            .w_full()
            .rounded(px(6.0))
            .px_2()
            .py_2()
            .flex()
            .flex_col()
            .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
    }

    /// Web: the person's name over two lines of the follow-up.
    fn render_today_follow_up_row(
        &self,
        index: usize,
        item: &bruhs_models::models::planning::FollowUp,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let person_id = item.person_id.clone();
        Self::today_record_row(
            ElementId::from(("desktop-today-follow-up", index)),
            format!("Open {}: {}", item.person_name, item.text),
        )
        .debug_selector(move || format!("desktop-today-follow-up-{index}"))
        .on_click(cx.listener(move |this, _event, window, cx| {
            this.open_today_follow_up(&person_id, window, cx);
        }))
        .child(
            div()
                .font_family(theme::UI_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(13.0))
                .text_color(theme::rambutan_200())
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.person_name.clone()),
        )
        .child(
            div()
                .mt(px(2.0))
                .line_clamp(2)
                .font_family(theme::CODE_FONT_FAMILY)
                .text_size(px(13.0))
                .text_color(theme::rambutan_500())
                .child(item.text.clone()),
        )
        .into_any_element()
    }

    /// Web: two lines of the decision over its meeting's title.
    fn render_today_decision_row(
        &self,
        index: usize,
        item: &bruhs_models::RecentDecision,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let meeting_id = item.meeting_id.clone();
        Self::today_record_row(
            ElementId::from(("desktop-today-decision", index)),
            format!("Open meeting {}: {}", item.meeting_title, item.text),
        )
        .debug_selector(move || format!("desktop-today-decision-{index}"))
        .on_click(cx.listener(move |this, _event, window, cx| {
            this.open_today_decision(meeting_id.clone(), window, cx);
        }))
        .child(
            div()
                .line_clamp(2)
                .font_family(theme::CODE_FONT_FAMILY)
                .text_size(px(13.0))
                .text_color(theme::rambutan_300())
                .child(item.text.clone()),
        )
        .child(
            div()
                .mt(px(2.0))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(12.0))
                .text_color(theme::rambutan_400())
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.meeting_title.clone()),
        )
        .into_any_element()
    }

    /// Web "Calendar →" beside Now & next: a quiet text button that opens
    /// the calendar.
    fn render_today_calendar_link(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("desktop-today-calendar-link")
            .debug_selector(|| "desktop-today-calendar-link".to_string())
            .flex_shrink_0()
            .min_h(px(theme::CONTROL_XS_PX))
            .flex()
            .items_center()
            .rounded(px(4.0))
            .font_family(theme::UI_FONT_FAMILY)
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
            .text_color(theme::rambutan_500())
            .hover(|el| el.text_color(theme::rambutan_200()))
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(TODAY_CALENDAR_LINK_LABEL)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .on_click(cx.listener(|this, _event, window, cx| {
                this.activate_sidebar_view(AppView::Calendar, window, cx);
            }))
            .child(TODAY_CALENDAR_LINK_LABEL)
            .into_any_element()
    }

    /// A quiet text action on Today: web's `text-button` buttons.
    fn today_text_action(
        id: &'static str,
        label: &'static str,
        color: Hsla,
        hover: Hsla,
        min_height: f32,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .debug_selector(move || format!("desktop-{id}"))
            .flex_shrink_0()
            .min_h(px(min_height))
            .flex()
            .items_center()
            .rounded(px(4.0))
            .font_family(theme::UI_FONT_FAMILY)
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
            .text_color(color)
            .hover(move |el| el.text_color(hover))
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .child(label)
    }

    /// Web's day-plan recovery above Today's cards: the error with Dismiss
    /// and "Refresh saved tasks and calendar" while no review is open,
    /// "Saving day plan…" while busy, and "Retry day-plan recovery" while
    /// the saved plan is unread.
    fn render_day_plan_recovery(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let busy = self.day_plan_busy();
        let reviewing = self.day_plan_proposal.is_some();
        let mut rows = Vec::new();
        if let Some(error) = self.day_plan_error.clone().filter(|_| !reviewing) {
            rows.push(
                div()
                    .id("desktop-day-plan-error")
                    .debug_selector(|| "desktop-day-plan-error".to_string())
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme::lychee_400().opacity(0.2))
                    .bg(theme::lychee_500().opacity(0.1))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(theme::CODE_FONT_FAMILY)
                            .text_size(px(13.0))
                            .text_color(theme::lychee_200())
                            .child(error),
                    )
                    .child(
                        Self::today_text_action(
                            "day-plan-dismiss",
                            DAY_PLAN_DISMISS_LABEL,
                            theme::lychee_200(),
                            theme::lychee_100(),
                            theme::CONTROL_XS_PX,
                        )
                        .on_click(cx.listener(
                            |this, _event, _window, cx| {
                                this.day_plan_error = None;
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element(),
            );
            if !busy {
                rows.push(
                    div()
                        .flex()
                        .child(
                            Self::today_text_action(
                                "day-plan-refresh",
                                DAY_PLAN_REFRESH_LABEL,
                                theme::persimmon_300(),
                                theme::persimmon_200(),
                                theme::CONTROL_PX,
                            )
                            .px_2()
                            .on_click(cx.listener(
                                |this, _event, _window, cx| {
                                    this.refresh_day_plan_sources(cx);
                                },
                            )),
                        )
                        .into_any_element(),
                );
            }
        }
        if busy {
            rows.push(
                div()
                    .id("desktop-day-plan-saving")
                    .debug_selector(|| "desktop-day-plan-saving".to_string())
                    .role(Role::Status)
                    .aria_label(DAY_PLAN_SAVING)
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(13.0))
                    .text_color(theme::rambutan_400())
                    .child(DAY_PLAN_SAVING)
                    .into_any_element(),
            );
        }
        if !self.day_plan_loaded && !busy {
            rows.push(
                div()
                    .flex()
                    .child(
                        Self::today_text_action(
                            "day-plan-retry",
                            DAY_PLAN_RETRY_LABEL,
                            theme::persimmon_300(),
                            theme::persimmon_200(),
                            theme::CONTROL_XS_PX,
                        )
                        .on_click(cx.listener(
                            |this, _event, _window, cx| {
                                this.retry_day_plan_recovery(cx);
                            },
                        )),
                    )
                    .into_any_element(),
            );
        }
        rows
    }

    /// Web "Day plan applied." with Undo, or "Saving…" while busy.
    fn render_day_plan_applied(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        self.day_plan_undo.as_ref()?;
        let busy = self.day_plan_busy();
        Some(
            div()
                .id("desktop-day-plan-applied")
                .debug_selector(|| "desktop-day-plan-applied".to_string())
                .role(Role::Status)
                .aria_label(DAY_PLAN_APPLIED)
                .rounded(px(8.0))
                .border_1()
                .border_color(theme::pandan_400().opacity(0.2))
                .bg(theme::pandan_500().opacity(0.1))
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .font_family(theme::CODE_FONT_FAMILY)
                .text_size(px(13.0))
                .text_color(theme::pandan_200())
                .child(DAY_PLAN_APPLIED)
                .map(|banner| {
                    if busy {
                        banner.child(
                            div()
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .font_weight(FontWeight::MEDIUM)
                                .text_size(px(12.0))
                                .text_color(theme::pandan_300().opacity(0.7))
                                .child(DAY_PLAN_SAVING_SHORT),
                        )
                    } else {
                        banner.child(
                            web_button(
                                "desktop-undo-day-plan",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-today-plan-undo".to_string())
                            .aria_label("Undo")
                            .text_color(theme::pandan_100())
                            .child("Undo")
                            .on_click(cx.listener(
                                |this, _event, _window, cx| {
                                    this.undo_desktop_day_plan(cx);
                                },
                            )),
                        )
                    }
                })
                .into_any_element(),
        )
    }

    /// Web `TodayTaskSection`'s row: the task's title over its due date or
    /// whom it waits on.
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
            .rounded(px(TODAY_ROW_RADIUS_PX))
            .px_2()
            .py_2()
            .flex()
            .flex_col()
            .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(bruhs_models::today_copy::task_row_label(&item.title))
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.open_today_task(&item_id, window, cx);
            }))
            .child(
                div()
                    .font_family(theme::UI_FONT_FAMILY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(13.0))
                    .text_color(theme::rambutan_200())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(item.title.clone()),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_400())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(detail),
            )
            .into_any_element()
    }

    /// Web's focus queue row: its rank (the first in persimmon), the title,
    /// the priority in its color and the estimate.
    fn render_today_focus_row(
        &self,
        index: usize,
        item: &WorkItem,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let item_id = item.id.clone();
        let estimate = bruhs_models::today_copy::estimate(item.planning.estimate_minutes);
        let first = index == 0;
        let group = SharedString::from(format!("desktop-today-focus-row-{index}"));
        div()
            .id(("desktop-today-focus", element_key(&item.id)))
            .debug_selector(move || format!("desktop-today-focus-{index}"))
            .group(group.clone())
            .w_full()
            .rounded(px(TODAY_ROW_RADIUS_PX))
            .px(px(10.0))
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .hover(|element| element.bg(theme::longan_800().opacity(0.6)))
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(bruhs_models::today_copy::focus_row_label(
                &item.title,
                item.priority.label(),
                &estimate,
            ))
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.open_today_task(&item_id, window, cx);
            }))
            .child(
                div()
                    .debug_selector(move || format!("desktop-today-focus-rank-{index}"))
                    .flex_shrink_0()
                    .size(px(TODAY_RANK_PX))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if first {
                        theme::persimmon_500().opacity(0.15)
                    } else {
                        theme::longan_800()
                    })
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(12.0))
                    .text_color(if first {
                        theme::persimmon_300()
                    } else {
                        theme::rambutan_500()
                    })
                    .child((index + 1).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(14.0))
                    .text_color(theme::rambutan_200())
                    .group_hover(group, |el| el.text_color(theme::rambutan_100()))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(item.title.clone()),
            )
            .child(
                div()
                    .debug_selector(move || format!("desktop-today-focus-priority-{index}"))
                    .flex_shrink_0()
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(12.0))
                    .text_color(priority_color(item.priority))
                    .child(item.priority.label()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(TODAY_ESTIMATE_WIDTH_PX))
                    .text_right()
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_400())
                    .child(estimate),
            )
            .into_any_element()
    }

    /// Web: the focus queue's "3 ready" at the right of its title.
    fn render_today_ready_count(count: usize) -> AnyElement {
        div()
            .debug_selector(|| "desktop-today-focus-ready".to_string())
            .flex_shrink_0()
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(12.0))
            .text_color(theme::rambutan_400())
            .child(today_copy::ready(count))
            .into_any_element()
    }

    /// Web's colored dot at the right of a status card's title.
    fn render_today_dot(key: &'static str, color: Hsla) -> AnyElement {
        div()
            .debug_selector(move || format!("desktop-today-dot-{key}"))
            .flex_shrink_0()
            .size(px(TODAY_DOT_PX))
            .rounded_full()
            .bg(color)
            .into_any_element()
    }

    /// A Today card: its title, with `action` at the right of the title
    /// row (web "Calendar →", "3 ready" or a status dot), over its rows,
    /// `row_gap` apart, or `empty`.
    pub(super) fn render_today_section(
        &self,
        title: &'static str,
        action: Option<AnyElement>,
        rows: Vec<AnyElement>,
        empty: &'static str,
        row_gap: f32,
    ) -> Stateful<Div> {
        let key = title.to_lowercase().replace(' ', "-");
        div()
            .id(("desktop-today-section", element_key(title)))
            .debug_selector(move || format!("desktop-today-section-{key}"))
            .role(Role::Group)
            .aria_label(title)
            .min_w_0()
            .rounded(px(TODAY_CARD_RADIUS_PX))
            .border_1()
            .border_color(gpui::white().opacity(0.05))
            .bg(theme::longan_900())
            .p_4()
            .flex()
            .flex_col()
            .child(
                div()
                    .mb_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        // No `min_w_0`: gpui would lay the title out at its
                        // min-content width, one letter per line.
                        div()
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .text_size(px(TODAY_CARD_TITLE_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::rambutan_100())
                            .child(title),
                    )
                    .children(action),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(row_gap))
                    .when(rows.is_empty(), |element| {
                        // Web `empty_row`: a quiet line, not a box inside the card.
                        element.child(
                            div()
                                .py_1()
                                .font_family(theme::CODE_FONT_FAMILY)
                                .text_size(px(13.0))
                                .text_color(theme::rambutan_500())
                                .child(empty),
                        )
                    })
                    .children(rows),
            )
    }

    pub(super) fn render_today_page(&self, window: &Window, cx: &Context<'_, Self>) -> Div {
        let snapshot = self.today_snapshot(cx);
        let date = self.planning_now().format("%A, %B %-d").to_string();
        let unread_updates = self.reactive_automations.read(cx).unread_count();
        // Web's breakpoints read the window's width.
        let width = bruhs_ui::zoom::css_px(window.viewport_size().width);
        let two_columns = width >= TODAY_TWO_COLUMNS_PX;
        let status_columns = if width >= TODAY_STATUS_FOUR_COLUMNS_PX {
            4
        } else if width >= TODAY_STATUS_TWO_COLUMNS_PX {
            2
        } else {
            1
        };
        let ready = snapshot.focus_queue.len();
        let focus_rows = snapshot
            .focus_queue
            .iter()
            .enumerate()
            .map(|(index, item)| self.render_today_focus_row(index, item, cx))
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
                let event_date = local_datetime_from_ms(event.start_at_ms).map(|date| date.date());
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
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Open event {event_title}"))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|element| element.border_color(theme::persimmon_400()))
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.open_today_calendar_event(&event_id, event_date, window, cx);
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
        let follow_up_rows = snapshot
            .follow_ups
            .iter()
            .take(4)
            .enumerate()
            .map(|(index, item)| self.render_today_follow_up_row(index, item, cx))
            .collect::<Vec<_>>();
        let decision_rows = snapshot
            .recent_decisions
            .iter()
            .take(4)
            .enumerate()
            .map(|(index, item)| self.render_today_decision_row(index, item, cx))
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
                    .flex_shrink_0()
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
                    .min_w_0()
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
                // Web: label, title, explanation and evidence in its type
                // scale, with a quiet Dismiss and a blueberry Review.
                div()
                    .id("desktop-today-suggestion")
                    .debug_selector(|| "desktop-today-suggestion".to_string())
                    .role(Role::Group)
                    .aria_label("Proactive agent suggestion")
                    .rounded(px(TODAY_CARD_RADIUS_PX))
                    .border_1()
                    .border_color(theme::blueberry_400().opacity(0.2))
                    .bg(theme::blueberry_500().opacity(0.1))
                    .px_4()
                    .py_3()
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
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(12.0))
                                    .text_color(theme::blueberry_300())
                                    .child("SUGGESTED LOCALLY"),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .font_family(theme::UI_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(13.0))
                                    .text_color(theme::rambutan_100())
                                    .child(suggestion.title.clone()),
                            )
                            .child(
                                div()
                                    .debug_selector(|| {
                                        "desktop-today-suggestion-explanation".to_string()
                                    })
                                    .mt_1()
                                    .font_family(theme::CODE_FONT_FAMILY)
                                    .text_size(px(13.0))
                                    .text_color(theme::rambutan_400())
                                    .child(suggestion.explanation.clone()),
                            )
                            .child(
                                div()
                                    .debug_selector(|| {
                                        "desktop-today-suggestion-evidence".to_string()
                                    })
                                    .mt_1()
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(12.0))
                                    .text_color(theme::blueberry_300())
                                    .child(format!("Evidence: {}", suggestion.evidence)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Self::today_text_action(
                                    "today-dismiss-agent-suggestion",
                                    "Dismiss",
                                    theme::rambutan_500(),
                                    theme::rambutan_200(),
                                    theme::CONTROL_SM_PX,
                                )
                                .aria_label("Dismiss proactive suggestion")
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
                                // Web: a blueberry-tinted button.
                                div()
                                    .id("today-review-agent-suggestion")
                                    .debug_selector(|| {
                                        "desktop-today-suggestion-review".to_string()
                                    })
                                    .role(Role::Button)
                                    .tab_index(0)
                                    .aria_label("Review")
                                    .min_h(px(theme::CONTROL_PX))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(TODAY_ROW_RADIUS_PX))
                                    .border_1()
                                    .border_color(theme::blueberry_400().opacity(0.2))
                                    .bg(theme::blueberry_500().opacity(0.15))
                                    .hover(|el| el.bg(theme::blueberry_500().opacity(0.2)))
                                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                    .font_family(theme::UI_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                                    .text_color(theme::blueberry_200())
                                    .cursor_pointer()
                                    .child("Review")
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.activate_sidebar_view(AppView::Automations, window, cx)
                                    })),
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
                                .child(format!("{start} to {end}")),
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
                // Web shows the error inside the open review.
                .when_some(self.day_plan_error.clone(), |card, error| {
                    card.child(
                        div()
                            .id("desktop-day-plan-review-error")
                            .debug_selector(|| "desktop-day-plan-review-error".to_string())
                            .role(Role::Alert)
                            .aria_label(error.clone())
                            .font_family(theme::CODE_FONT_FAMILY)
                            .text_size(px(13.0))
                            .text_color(theme::lychee_200())
                            .child(error),
                    )
                })
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
                            .aria_label("Cancel")
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
                                ButtonState::disabled_if(
                                    proposal.blocks.is_empty() || self.day_plan_busy(),
                                ),
                            )
                            .debug_selector(|| "desktop-today-plan-apply".to_string())
                            .aria_label("Apply plan")
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
                crate::views::page_header::page_header("Today", Some(date.into())).child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            web_button(
                                "open-agent-workflows",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-today-agent-workflows".to_string())
                            .aria_label(crate::views::reactive_automations::unread_label(
                                today_copy::AGENT_WORKFLOWS,
                                unread_updates,
                            ))
                            .gap_1()
                            .child(today_copy::AGENT_WORKFLOWS)
                            .when(unread_updates > 0, |button| {
                                button.child(crate::views::reactive_automations::unread_badge(
                                    unread_updates,
                                ))
                            })
                            .on_click(cx.listener(
                                |this, _event, window, cx| {
                                    this.activate_sidebar_view(AppView::Automations, window, cx)
                                },
                            )),
                        )
                        .child(
                            web_button(
                                "open-life-context",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-today-life".to_string())
                            .aria_label(today_copy::LIFE)
                            .child(today_copy::LIFE)
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
                                ButtonState::disabled_if(
                                    self.day_plan_proposal.is_some() || self.day_plan_busy(),
                                ),
                            )
                            .debug_selector(|| "desktop-today-plan".to_string())
                            .aria_label("Plan my day")
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
                    .overflow_y_scrollbar()
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
                            .children(self.render_day_plan_recovery(cx))
                            .children(proposal)
                            .children(self.render_day_plan_applied(cx))
                            .children(suggestion)
                            .child(briefing)
                            // Web: Now & next beside Focus queue, 0.9 to 1.1,
                            // from its `lg` breakpoint; stacked below it.
                            .child(
                                div()
                                    .flex()
                                    .when(two_columns, |row| row.flex_row())
                                    .when(!two_columns, |row| row.flex_col())
                                    .gap_4()
                                    .child(
                                        self.render_today_section(
                                            "Now & next",
                                            Some(self.render_today_calendar_link(cx)),
                                            schedule_rows,
                                            today_copy::SCHEDULE_EMPTY,
                                            8.0,
                                        )
                                        .when(two_columns, |card| {
                                            card.flex_basis(px(0.0)).flex_grow(0.9)
                                        }),
                                    )
                                    .child(
                                        self.render_today_section(
                                            "Focus queue",
                                            Some(Self::render_today_ready_count(ready)),
                                            focus_rows,
                                            today_copy::FOCUS_QUEUE_EMPTY,
                                            4.0,
                                        )
                                        .when(two_columns, |card| {
                                            card.flex_basis(px(0.0)).flex_grow(1.1)
                                        }),
                                    ),
                            )
                            // Web: the four status cards, each with its dot,
                            // four to a row from `xl`, two from `md`.
                            .child(
                                div()
                                    .debug_selector(|| "desktop-today-status-cards".to_string())
                                    .grid()
                                    .grid_cols(status_columns)
                                    .gap_4()
                                    .child(self.render_today_section(
                                        "Overdue",
                                        Some(Self::render_today_dot(
                                            "overdue",
                                            theme::lychee_400(),
                                        )),
                                        overdue_rows,
                                        today_copy::OVERDUE_EMPTY,
                                        6.0,
                                    ))
                                    .child(self.render_today_section(
                                        "Waiting on",
                                        Some(Self::render_today_dot(
                                            "waiting-on",
                                            theme::blueberry_400(),
                                        )),
                                        waiting_rows,
                                        today_copy::WAITING_ON_EMPTY,
                                        6.0,
                                    ))
                                    .child(self.render_today_section(
                                        "Follow ups",
                                        Some(Self::render_today_dot(
                                            "follow-ups",
                                            theme::durian_400(),
                                        )),
                                        follow_up_rows,
                                        today_copy::FOLLOW_UPS_EMPTY,
                                        6.0,
                                    ))
                                    .child(self.render_today_section(
                                        "Decisions",
                                        Some(Self::render_today_dot(
                                            "decisions",
                                            theme::pandan_400(),
                                        )),
                                        decision_rows,
                                        today_copy::DECISIONS_EMPTY,
                                        6.0,
                                    )),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod parity_tests;

#[cfg(test)]
mod audit_tests {
    use super::{theme, AppView, DesktopCalendarMode, DesktopState, NaiveDate, Workspace};
    use gpui::TestAppContext;

    type TaskPaths = std::rc::Rc<std::cell::RefCell<Vec<gpui::GlobalElementId>>>;

    struct RepeatedTaskSections {
        workspace: gpui::Entity<Workspace>,
        task: bruhs_models::WorkItem,
        paths: TaskPaths,
    }

    impl gpui::Render for RepeatedTaskSections {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<'_, Self>,
        ) -> impl gpui::IntoElement {
            use gpui::prelude::*;
            use gpui::{canvas, div, ElementId};

            self.paths.borrow_mut().clear();
            let paths = self.paths.clone();
            let task = self.task.clone();
            self.workspace.update(cx, |workspace, cx| {
                div().size_full().flex().flex_col().children(
                    ["Overdue", "Waiting on"].into_iter().map(|title| {
                        let paths = paths.clone();
                        let row_id =
                            ElementId::from(("desktop-today-task", super::element_key(&task.id)));
                        let probe = canvas(
                            move |_, window, _| {
                                window.with_global_id(row_id, |id, _| {
                                    paths.borrow_mut().push(id.clone());
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .h(gpui::px(0.0));
                        workspace.render_today_section(
                            title,
                            None,
                            vec![
                                workspace.render_today_task_row(&task, String::new(), cx),
                                probe.into_any_element(),
                            ],
                            "",
                            6.0,
                        )
                    }),
                )
            })
        }
    }

    #[gpui::test]
    fn repeated_today_tasks_have_distinct_accessibility_paths(cx: &mut TestAppContext) {
        use gpui::AppContext as _;

        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let paths = TaskPaths::default();
        let captured = paths.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let mut task = bruhs_models::WorkItem::new_at("Blocked overdue task", 0);
            task.due_date = Some("2000-01-01".into());
            task.planning.waiting_on = Some("Avery".into());
            RepeatedTaskSections {
                workspace: cx.new(|cx| Workspace::new(window, cx)),
                task,
                paths,
            }
        });
        cx.run_until_parked();
        let paths = captured.borrow();
        assert_eq!(paths.len(), 2, "both Today task rows must be rendered");
        assert_ne!(
            paths[0], paths[1],
            "AccessKit derives node IDs from these global paths"
        );
    }

    #[gpui::test]
    fn today_event_opens_its_day_and_restores_workspace_focus(cx: &mut TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let (workspace, cx) = cx.add_window_view(Workspace::new);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.calendar_anchor = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
            let event_day = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
            workspace.open_today_calendar_event("today-event", Some(event_day), window, cx);
            assert_eq!(workspace.calendar_anchor, event_day);
            assert_eq!(
                workspace.selected_calendar_event_id.as_deref(),
                Some("today-event")
            );
            assert_eq!(workspace.calendar_mode, DesktopCalendarMode::Day);
            assert_eq!(workspace.active_view, AppView::Calendar);
            assert!(workspace.workspace_focus_handle.is_focused(window));
        });
    }
}

fn native_workday_window(
    date: NaiveDate,
) -> Result<bruhs_models::DayPlanWindow, bruhs_models::DayPlanChangeError> {
    let at = |hour| {
        date.and_hms_opt(hour, 0, 0)
            .and_then(|local| Local.from_local_datetime(&local).single())
            .map(|time| EpochMs::saturating_from_i64(time.timestamp_millis()).as_f64())
            .ok_or(bruhs_models::DayPlanChangeError::InvalidWindow)
    };
    Ok(bruhs_models::DayPlanWindow {
        start_at_ms: at(9)?,
        end_at_ms: at(18)?,
    })
}

#[cfg(test)]
mod clock_tests {
    use super::{native_workday_window, EpochMs, NaiveDate};
    use chrono::TimeZone as _;

    #[test]
    fn native_workday_preserves_local_hours_on_dst_transition_dates() {
        const CHILD: &str = "BRUHS_PLANNING_CLOCK_TEST_CHILD";
        if let Ok(zone) = std::env::var(CHILD) {
            for (month, day, la_start, la_end) in [
                (3, 8, 1_772_985_600_000_i64, 1_773_018_000_000_i64),
                (11, 1, 1_793_552_400_000_i64, 1_793_584_800_000_i64),
            ] {
                let date = NaiveDate::from_ymd_opt(2026, month, day).unwrap();
                let window = native_workday_window(date).unwrap();
                let (start, end) = if zone == "America/Los_Angeles" {
                    (la_start, la_end)
                } else {
                    (
                        chrono::Utc
                            .with_ymd_and_hms(2026, month, day, 9, 0, 0)
                            .single()
                            .unwrap()
                            .timestamp_millis(),
                        chrono::Utc
                            .with_ymd_and_hms(2026, month, day, 18, 0, 0)
                            .single()
                            .unwrap()
                            .timestamp_millis(),
                    )
                };
                assert_eq!(
                    window.start_at_ms,
                    EpochMs::saturating_from_i64(start).as_f64(),
                    "{zone} {date}"
                );
                assert_eq!(
                    window.end_at_ms,
                    EpochMs::saturating_from_i64(end).as_f64(),
                    "{zone} {date}"
                );
            }
            return;
        }
        // TZ is per child process, never mutated in the parallel test runner.
        let full_name = concat!(
            module_path!(),
            "::native_workday_preserves_local_hours_on_dst_transition_dates"
        );
        let (_, name) = full_name.split_once("::").unwrap();
        for zone in ["America/Los_Angeles", "UTC"] {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--nocapture"])
                .env(CHILD, zone)
                .env("TZ", zone)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{zone}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                String::from_utf8_lossy(&result.stdout).contains("1 passed; 0 failed"),
                "The timezone child must execute the selected native test."
            );
        }
    }
}

/// Events as the shared planners read them, at their UTC instants in the
/// zone `offset_at` describes. Every event is kept: a time in the repeated
/// hour of a fall-back day used to have no single local reading and was
/// dropped from Today and from slot search.
pub(super) fn planning_calendar_events_in(
    events: Vec<DesktopCalendarEvent>,
    offset_at: impl Fn(i64) -> i32 + Copy,
) -> Vec<bruhs_models::CalendarEvent> {
    events
        .into_iter()
        .map(|event| {
            let (start_ms, end_ms) = event.instants_in(offset_at);
            let read_only = event.is_read_only();
            bruhs_models::CalendarEvent {
                id: event.id,
                title: event.title,
                start_at_ms: EpochMs::saturating_from_i64(start_ms).as_f64(),
                end_at_ms: EpochMs::saturating_from_i64(end_ms).as_f64(),
                all_day: event.all_day,
                color: event.color,
                notes: event.notes,
                created_at: 0,
                updated_at: 0,
                source: if read_only {
                    bruhs_models::EventSource::Google
                } else {
                    bruhs_models::EventSource::Local
                },
                google_event_id: None,
                calendar_id: None,
                attendees: Vec::new(),
                organizer: None,
                location: None,
                conferencing: None,
                recurring_event_id: None,
                html_link: event.provider.and_then(|provider| provider.html_link),
            }
        })
        .collect()
}
