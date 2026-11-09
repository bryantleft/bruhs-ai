//! Calendar page: header, month grid, week/day time grid, event and task pills, time-grid drag, and the date math behind them.

use super::*;
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};

impl DesktopTimegridDrag {
    pub(super) fn new(event: &DesktopCalendarEvent, kind: DesktopTimegridDragKind) -> Self {
        Self {
            event_id: event.id.clone(),
            kind,
            title: event.title.clone(),
            color: event.color.clone(),
        }
    }
}

impl Render for DesktopTimegridDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        let color = event_color(self.color.as_deref());
        div()
            .rounded(px(TIMEGRID_DRAG_PREVIEW_RADIUS_PX))
            .border_1()
            .border_color(color.opacity(TIMEGRID_DRAG_PREVIEW_BORDER_OPACITY))
            .bg(color.opacity(TIMEGRID_DRAG_PREVIEW_BG_OPACITY))
            .px_2()
            .py_1()
            .text_xs()
            .text_color(theme::rambutan_100())
            .child(self.title.clone())
    }
}

pub(super) fn shift_month(anchor: NaiveDate, delta_months: i32) -> NaiveDate {
    let month0 = i32::try_from(anchor.month0()).unwrap_or(0);
    let zero_based = anchor.year() * 12 + month0 + delta_months;
    let year = zero_based.div_euclid(12);
    let month = zero_based.rem_euclid(12).unsigned_abs() + 1;
    // `month` is 1–12 by construction; 28 is never used.
    let day = anchor
        .day()
        .min(bruhs_models::time::days_in_month(year, month).unwrap_or(28));
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or(anchor)
}

pub(super) fn week_start(anchor: NaiveDate) -> NaiveDate {
    anchor - Duration::days(i64::from(anchor.weekday().num_days_from_sunday()))
}

pub(super) fn month_grid_start(anchor: NaiveDate) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(anchor.year(), anchor.month(), 1).unwrap_or(anchor);
    first - Duration::days(i64::from(first.weekday().num_days_from_sunday()))
}

pub(super) fn at_hour(date: NaiveDate, hour: u32) -> NaiveDateTime {
    date.and_hms_opt(hour, 0, 0)
        .or_else(|| date.and_hms_opt(0, 0, 0))
        .expect("midnight is valid for a NaiveDate")
}

pub(super) fn default_event_start(now: NaiveDateTime) -> NaiveDateTime {
    let mut minutes = now.hour() * 60 + now.minute();
    if now.second() > 0 || now.nanosecond() > 0 {
        minutes += 1;
    }
    let rounded = minutes.div_ceil(30) * 30;
    let date = if rounded >= 24 * 60 {
        now.date() + Duration::days(1)
    } else {
        now.date()
    };
    let minutes_in_day = rounded % (24 * 60);
    let hour = minutes_in_day / 60;
    let minute = minutes_in_day % 60;
    let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or(NaiveTime::MIN);
    date.and_time(time)
}

pub(super) fn format_event_form_datetime(value: NaiveDateTime) -> String {
    value.format(DESKTOP_DATETIME_INPUT_FORMAT).to_string()
}

pub(super) fn parse_event_form_datetime(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value.trim(), DESKTOP_DATETIME_INPUT_FORMAT).ok()
}

pub(super) fn normalize_event_form_times(
    start_at: NaiveDateTime,
    end_at: NaiveDateTime,
    all_day: bool,
) -> (NaiveDateTime, NaiveDateTime) {
    if all_day {
        let midnight = at_hour(start_at.date(), 0);
        (midnight, midnight + Duration::days(1))
    } else if end_at <= start_at {
        (start_at, start_at + Duration::hours(1))
    } else {
        (start_at, end_at)
    }
}

pub(super) fn event_form_times_from_inputs(
    start_input: &str,
    end_input: &str,
    all_day: bool,
    fallback_now: NaiveDateTime,
) -> (NaiveDateTime, NaiveDateTime) {
    let start_at =
        parse_event_form_datetime(start_input).unwrap_or_else(|| default_event_start(fallback_now));
    let end_at =
        parse_event_form_datetime(end_input).unwrap_or_else(|| start_at + Duration::hours(1));
    normalize_event_form_times(start_at, end_at, all_day)
}

pub(super) fn calendar_anchor_after_event_dialog_open(
    current_anchor: NaiveDate,
    _event_start_at: NaiveDateTime,
) -> NaiveDate {
    current_anchor
}

/// Wall-clock time of a server timestamp (web: `Date` in the browser zone).
pub(super) fn local_datetime_from_ms(ms: f64) -> Option<NaiveDateTime> {
    Local
        .timestamp_millis_opt(EpochMs::saturating_from_f64(ms).as_millis())
        .single()
        .map(|dt| dt.naive_local())
}

pub(super) fn event_overlaps_day(event: &DesktopCalendarEvent, date: NaiveDate) -> bool {
    let start = at_hour(date, 0);
    let end = at_hour(date + Duration::days(1), 0);
    event.start_at < end && event.end_at > start
}

pub(super) fn timegrid_total_height_px() -> f32 {
    DESKTOP_HOUR_ROW_PX * 24.0
}

pub(super) fn timegrid_datetime_offset_px(value: NaiveDateTime, date: NaiveDate) -> f32 {
    let day_start = at_hour(date, 0);
    let seconds = (value - day_start).num_seconds().clamp(0, 24 * 60 * 60);
    (seconds.to_f32_lossy() / 3600.0) * DESKTOP_HOUR_ROW_PX
}

pub(super) fn snap_timegrid_offset_seconds(offset_px: f32) -> i64 {
    let minutes = (offset_px / DESKTOP_HOUR_ROW_PX) * 60.0;
    let snap = DESKTOP_TIMEGRID_SNAP_MINUTES.to_f32_lossy();
    let snapped_minutes = (minutes / snap).round() * snap;
    snapped_minutes.to_i64_saturating() * 60
}

pub(super) fn move_timegrid_event_to(
    event: &mut DesktopCalendarEvent,
    target_date: NaiveDate,
    pointer_y_px: f32,
    grab_offset_y_px: f32,
) -> bool {
    let duration = event.end_at - event.start_at;
    let duration_seconds = duration.num_seconds().max(0);
    let day_start = at_hour(target_date, 0);
    let top_px = (pointer_y_px - grab_offset_y_px).max(0.0);
    let max_start_seconds =
        (DESKTOP_DAY_SECONDS - duration_seconds.min(DESKTOP_DAY_SECONDS)).max(0);
    let offset_seconds = snap_timegrid_offset_seconds(top_px).clamp(0, max_start_seconds);
    let new_start = day_start + Duration::seconds(offset_seconds);
    let new_end = new_start + Duration::seconds(duration_seconds);

    if event.start_at == new_start && event.end_at == new_end {
        false
    } else {
        event.start_at = new_start;
        event.end_at = new_end;
        true
    }
}

pub(super) fn resize_timegrid_event_end_to(
    event: &mut DesktopCalendarEvent,
    date: NaiveDate,
    pointer_y_px: f32,
) -> bool {
    let day_start = at_hour(date, 0);
    let candidate = day_start + Duration::seconds(snap_timegrid_offset_seconds(pointer_y_px));
    let min_end = event.start_at + Duration::seconds(DESKTOP_TIMEGRID_MIN_DURATION_SECONDS);
    let max_end = day_start + Duration::days(1);
    let new_end = candidate.max(min_end).min(max_end);

    if event.end_at == new_end {
        false
    } else {
        event.end_at = new_end;
        true
    }
}

pub(super) fn lay_out_timegrid_events(
    events: &[DesktopCalendarEvent],
    date: NaiveDate,
) -> Vec<DesktopLaidOutEvent> {
    let day_start = at_hour(date, 0);
    let day_end = at_hour(date + Duration::days(1), 0);
    let clamped: Vec<(&DesktopCalendarEvent, NaiveDateTime, NaiveDateTime)> = events
        .iter()
        .filter(|event| !event.all_day && event_overlaps_day(event, date))
        .filter_map(|event| {
            let effective_start = event.start_at.max(day_start);
            let effective_end = event.end_at.min(day_end);
            (effective_end > effective_start).then_some((event, effective_start, effective_end))
        })
        .collect();
    let starts_ends: Vec<(i64, i64)> = clamped
        .iter()
        .map(|(_, start, end)| (start.and_utc().timestamp(), end.and_utc().timestamp()))
        .collect();
    let layout = bruhs_models::calendar::assign_lanes(&starts_ends);

    layout
        .placements
        .into_iter()
        .map(|placement| {
            let (event, effective_start, effective_end) = clamped[placement.index];
            let top_px = timegrid_datetime_offset_px(effective_start, date);
            let duration_seconds = (effective_end - effective_start)
                .num_seconds()
                .max(DESKTOP_TIMEGRID_MIN_DISPLAY_SECONDS);
            let height_px = ((duration_seconds.to_f32_lossy() / 3600.0) * DESKTOP_HOUR_ROW_PX)
                .max(DESKTOP_TIMEGRID_MIN_DISPLAY_PX);
            DesktopLaidOutEvent {
                event: event.clone(),
                top_px,
                height_px,
                lane: placement.lane,
                lanes: layout.lanes,
            }
        })
        .collect()
}

pub(super) fn format_month_year(date: NaiveDate) -> String {
    format!("{} {}", MONTH_NAMES[(date.month0()) as usize], date.year())
}

pub(super) fn format_full_date(date: NaiveDate) -> String {
    format!(
        "{} {}, {}",
        SHORT_MONTH_NAMES[date.month0() as usize],
        date.day(),
        date.year()
    )
}

pub(super) fn format_week_range(anchor: NaiveDate) -> String {
    let start = week_start(anchor);
    let end = start + Duration::days(6);
    if start.month() == end.month() {
        format!(
            "{} {} — {}, {}",
            SHORT_MONTH_NAMES[start.month0() as usize],
            start.day(),
            end.day(),
            end.year()
        )
    } else {
        format!(
            "{} {} — {} {}, {}",
            SHORT_MONTH_NAMES[start.month0() as usize],
            start.day(),
            SHORT_MONTH_NAMES[end.month0() as usize],
            end.day(),
            end.year()
        )
    }
}

pub(super) fn format_day_header(date: NaiveDate) -> String {
    format!(
        "{} {}",
        WEEKDAY_NAMES[date.weekday().num_days_from_sunday() as usize],
        date.day()
    )
}

pub(super) fn event_color(color: Option<&str>) -> Hsla {
    match color.unwrap_or("") {
        "blueberry" => theme::blueberry_400(),
        "pandan" => theme::pandan_400(),
        "lychee" => theme::lychee_400(),
        "durian" => theme::durian_400(),
        "mangosteen" => theme::mangosteen_400(),
        "dragonfruit" => theme::dragonfruit_400(),
        "persimmon" => theme::persimmon_400(),
        _ => theme::rambutan_500(),
    }
}

pub(super) fn event_timegrid_surface_color(color: Option<&str>) -> Hsla {
    match color.unwrap_or("") {
        "persimmon" => theme::persimmon_500(),
        "blueberry" => theme::blueberry_500(),
        "pandan" => theme::pandan_500(),
        "lychee" => theme::lychee_500(),
        "durian" => theme::durian_500(),
        "mangosteen" => theme::mangosteen_500(),
        "dragonfruit" => theme::dragonfruit_500(),
        _ => theme::rambutan_500(),
    }
}

pub(super) fn event_timegrid_text_color(color: Option<&str>) -> Hsla {
    match color.unwrap_or("") {
        "persimmon" => theme::persimmon_100(),
        "pandan" => theme::pandan_100(),
        "lychee" => theme::lychee_100(),
        "durian" => theme::durian_100(),
        _ => theme::rambutan_100(),
    }
}

pub(super) const MONTH_NAMES: &[&str; 12] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub(super) const SHORT_MONTH_NAMES: &[&str; 12] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(super) const WEEKDAY_NAMES: &[&str; 7] = &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub(super) const DESKTOP_DATETIME_INPUT_FORMAT: &str = "%Y-%m-%dT%H:%M";
pub(super) const DESKTOP_HOUR_ROW_PX: f32 = 40.0;
pub(super) const DESKTOP_DAY_SECONDS: i64 = 24 * 60 * 60;
pub(super) const DESKTOP_TIMEGRID_SNAP_MINUTES: i64 = 15;
pub(super) const DESKTOP_TIMEGRID_MIN_DURATION_SECONDS: i64 = 15 * 60;
pub(super) const DESKTOP_TIMEGRID_MIN_DISPLAY_SECONDS: i64 = 30 * 60;
pub(super) const DESKTOP_TIMEGRID_MIN_DISPLAY_PX: f32 = 20.0;
pub(super) const DESKTOP_TIMEGRID_RESIZE_HANDLE_PX: f32 = 6.0;
pub(super) const DESKTOP_TIMEGRID_NOW_DOT_PX: f32 = 8.0;
pub(super) const TIMEGRID_DRAG_PREVIEW_RADIUS_PX: f32 = 6.0;
pub(super) const TIMEGRID_DRAG_PREVIEW_BORDER_OPACITY: f32 = 0.55;
pub(super) const TIMEGRID_DRAG_PREVIEW_BG_OPACITY: f32 = 0.32;
pub(super) const CALENDAR_PREVIOUS_LABEL: &str = "Previous";
pub(super) const CALENDAR_NEXT_LABEL: &str = "Next";
pub(super) const CALENDAR_NEW_EVENT_LABEL: &str = "New event";
pub(super) const CALENDAR_TASK_DUE_LABEL: &str = "Task due";
pub(super) const CALENDAR_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const CALENDAR_HEADER_MIN_HEIGHT_PX: f32 = 40.0;
pub(super) const CALENDAR_HEADER_X_PADDING_PX: f32 = 16.0;
pub(super) const CALENDAR_HEADER_Y_PADDING_PX: f32 = 8.0;
pub(super) const CALENDAR_HEADER_BORDER_OPACITY: f32 = 0.40;
pub(super) const CALENDAR_HEADER_TITLE_MIN_WIDTH_PX: f32 = 180.0;
pub(super) const CALENDAR_HEADER_TITLE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_HEADER_TITLE_GROUP_GAP_PX: f32 = 12.0;
pub(super) const CALENDAR_NAV_GROUP_GAP_PX: f32 = 4.0;
pub(super) const CALENDAR_NAV_BUTTON_SIZE_PX: f32 = 32.0;
pub(super) const CALENDAR_CONTROL_RADIUS_PX: f32 = 8.0;
pub(super) const CALENDAR_NAV_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_NAV_HOVER_BG_OPACITY: f32 = 0.40;
pub(super) const CALENDAR_TODAY_X_PADDING_PX: f32 = 8.0;
pub(super) const CALENDAR_TODAY_Y_PADDING_PX: f32 = 2.0;
pub(super) const CALENDAR_TODAY_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_ACTION_GROUP_GAP_PX: f32 = 8.0;
pub(super) const CALENDAR_MODE_SWITCHER_GAP_PX: f32 = 2.0;
pub(super) const CALENDAR_MODE_SWITCHER_PADDING_PX: f32 = 2.0;
pub(super) const CALENDAR_MODE_SWITCHER_BORDER_OPACITY: f32 = 0.60;
pub(super) const CALENDAR_MODE_SWITCHER_BG_OPACITY: f32 = 0.40;
pub(super) const CALENDAR_MODE_ACTIVE_BORDER_OPACITY: f32 = 0.20;
pub(super) const CALENDAR_MODE_ACTIVE_BG_OPACITY: f32 = 0.15;
pub(super) const CALENDAR_MODE_INACTIVE_OPACITY: f32 = 0.0;
pub(super) const CALENDAR_MODE_HOVER_BG_OPACITY: f32 = 0.40;
pub(super) const CALENDAR_MODE_X_PADDING_PX: f32 = 8.0;
pub(super) const CALENDAR_MODE_Y_PADDING_PX: f32 = 2.0;
pub(super) const CALENDAR_MODE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_NEW_EVENT_X_PADDING_PX: f32 = 10.0;
pub(super) const CALENDAR_NEW_EVENT_Y_PADDING_PX: f32 = 4.0;
pub(super) const CALENDAR_NEW_EVENT_BORDER_OPACITY: f32 = 0.20;
pub(super) const CALENDAR_NEW_EVENT_BG_OPACITY: f32 = 0.15;
pub(super) const CALENDAR_NEW_EVENT_HOVER_BG_OPACITY: f32 = 0.25;
pub(super) const CALENDAR_NEW_EVENT_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_MONTH_ROW_MIN_HEIGHT_PX: f32 = 92.0;
pub(super) const CALENDAR_MONTH_TODAY_SIZE_PX: f32 = 20.0;
pub(super) const CALENDAR_MONTH_TODAY_RADIUS_PX: f32 = 999.0;
pub(super) const CALENDAR_MONTH_CELL_PADDING_PX: f32 = 6.0;
pub(super) const CALENDAR_MONTH_CELL_HOVER_BG_OPACITY: f32 = 0.20;
pub(super) const CALENDAR_MONTH_CELL_BORDER_OPACITY: f32 = 0.30;
pub(super) const CALENDAR_WEEKDAY_X_PADDING_PX: f32 = 8.0;
pub(super) const CALENDAR_WEEKDAY_Y_PADDING_PX: f32 = 6.0;
pub(super) const CALENDAR_WEEKDAY_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_DATE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_PILL_DOT_SIZE_PX: f32 = 6.0;
pub(super) const CALENDAR_PILL_DOT_RADIUS_PX: f32 = 999.0;
pub(super) const CALENDAR_PILL_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_TIMEGRID_AXIS_WIDTH_PX: f32 = 56.0;
pub(super) const CALENDAR_TIMEGRID_HEADER_X_PADDING_PX: f32 = 8.0;
pub(super) const CALENDAR_TIMEGRID_HEADER_Y_PADDING_PX: f32 = 6.0;
pub(super) const CALENDAR_TIMEGRID_HEADER_BORDER_OPACITY: f32 = 0.40;
pub(super) const CALENDAR_TIMEGRID_COLUMN_BORDER_OPACITY: f32 = 0.30;
pub(super) const CALENDAR_TIMEGRID_HEADER_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_TIMEGRID_HOUR_X_PADDING_PX: f32 = 6.0;
pub(super) const CALENDAR_TIMEGRID_HOUR_TOP_PADDING_PX: f32 = 2.0;
pub(super) const CALENDAR_TIMEGRID_HOUR_LABEL_BORDER_OPACITY: f32 = 0.40;
pub(super) const CALENDAR_TIMEGRID_HOUR_SLOT_BORDER_OPACITY: f32 = 0.30;
pub(super) const CALENDAR_TIMEGRID_HOUR_SLOT_HOVER_BG_OPACITY: f32 = 0.15;
pub(super) const CALENDAR_TIMEGRID_HOUR_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CALENDAR_TIMEGRID_NOW_LINE_HEIGHT_PX: f32 = 1.0;
pub(super) const CALENDAR_TIMEGRID_NOW_DOT_OFFSET_PX: f32 = -4.0;
pub(super) const CALENDAR_TIMEGRID_NOW_DOT_RADIUS_PX: f32 = 999.0;
pub(super) const CALENDAR_TIMEGRID_EVENT_INSET_PX: f32 = 2.0;
pub(super) const CALENDAR_TIMEGRID_EVENT_RADIUS_PX: f32 = 8.0;
pub(super) const CALENDAR_TIMEGRID_EVENT_X_PADDING_PX: f32 = 6.0;
pub(super) const CALENDAR_TIMEGRID_EVENT_Y_PADDING_PX: f32 = 4.0;
pub(super) const CALENDAR_TIMEGRID_EVENT_BORDER_OPACITY: f32 = 0.40;
pub(super) const CALENDAR_TIMEGRID_EVENT_BG_OPACITY: f32 = 0.30;
pub(super) const CALENDAR_TIMEGRID_EVENT_HOVER_BG_OPACITY: f32 = 0.34;
pub(super) const CALENDAR_TIMEGRID_EVENT_TEXT_SIZE_PX: f32 = 12.0;

impl Workspace {
    pub(super) fn calendar_title(&self) -> String {
        match self.calendar_mode {
            DesktopCalendarMode::Month => format_month_year(self.calendar_anchor),
            DesktopCalendarMode::Week => format_week_range(self.calendar_anchor),
            DesktopCalendarMode::Day => format_full_date(self.calendar_anchor),
        }
    }

    pub(super) fn set_calendar_mode(
        &mut self,
        mode: DesktopCalendarMode,
        cx: &mut Context<'_, Self>,
    ) {
        self.calendar_mode = mode;
        self.persist_and_notify(cx);
    }

    pub(super) fn calendar_prev(&mut self, cx: &mut Context<'_, Self>) {
        self.calendar_anchor = match self.calendar_mode {
            DesktopCalendarMode::Month => shift_month(self.calendar_anchor, -1),
            DesktopCalendarMode::Week => self.calendar_anchor - Duration::days(7),
            DesktopCalendarMode::Day => self.calendar_anchor - Duration::days(1),
        };
        self.persist_and_notify(cx);
    }

    pub(super) fn calendar_next(&mut self, cx: &mut Context<'_, Self>) {
        self.calendar_anchor = match self.calendar_mode {
            DesktopCalendarMode::Month => shift_month(self.calendar_anchor, 1),
            DesktopCalendarMode::Week => self.calendar_anchor + Duration::days(7),
            DesktopCalendarMode::Day => self.calendar_anchor + Duration::days(1),
        };
        self.persist_and_notify(cx);
    }

    pub(super) fn calendar_today(&mut self, cx: &mut Context<'_, Self>) {
        self.calendar_anchor = today_date();
        self.persist_and_notify(cx);
    }

    pub(super) fn delete_calendar_event_by_id(
        &mut self,
        event_id: &str,
        cx: &mut Context<'_, Self>,
    ) {
        self.calendar_events.retain(|event| event.id != event_id);
        if self.selected_calendar_event_id.as_deref() == Some(event_id) {
            self.selected_calendar_event_id = None;
        }
        self.persist_and_notify(cx);
    }

    pub(super) fn begin_timegrid_drag(
        &mut self,
        drag: &DesktopTimegridDrag,
        grab_offset_y_px: f32,
        cx: &mut Context<'_, Self>,
    ) {
        self.timegrid_drag = Some(DesktopTimegridDragState {
            event_id: drag.event_id.clone(),
            kind: drag.kind,
            grab_offset_y_px,
        });
        cx.notify();
    }

    pub(super) fn clear_timegrid_drag(&mut self, cx: &mut Context<'_, Self>) {
        if self.timegrid_drag.is_some() {
            self.timegrid_drag = None;
            cx.notify();
        }
    }

    pub(super) fn apply_timegrid_drag(
        &mut self,
        drag: &DragMoveEvent<DesktopTimegridDrag>,
        date: NaiveDate,
        cx: &mut Context<'_, Self>,
    ) {
        let payload = drag.drag(cx).clone();
        if !drag.bounds.contains(&drag.event.position) {
            return;
        }

        let grab_offset_y_px = self
            .timegrid_drag
            .as_ref()
            .filter(|state| state.event_id == payload.event_id && state.kind == payload.kind)
            .map_or(0.0, |state| state.grab_offset_y_px);
        let pointer_y_px = f32::from(drag.event.position.y - drag.bounds.origin.y);
        let changed = self
            .calendar_events
            .iter_mut()
            .find(|event| event.id == payload.event_id)
            .is_some_and(|event| match payload.kind {
                DesktopTimegridDragKind::Move => {
                    move_timegrid_event_to(event, date, pointer_y_px, grab_offset_y_px)
                }
                DesktopTimegridDragKind::ResizeEnd => {
                    resize_timegrid_event_end_to(event, date, pointer_y_px)
                }
            });

        cx.stop_propagation();
        if changed {
            self.persist_and_notify(cx);
        }
    }

    /// Local events plus the Google events streamed by the hub (the web
    /// merges both into one `CalendarEventsSignal`).
    pub(super) fn visible_calendar_events(
        &self,
        cx: &Context<'_, Self>,
    ) -> Vec<DesktopCalendarEvent> {
        let mut events = self.calendar_events.clone();
        for event in &self.meetings_hub.read(cx).google_events {
            if events.iter().any(|existing| existing.id == event.id) {
                continue;
            }
            let (Some(start_at), Some(end_at)) = (
                local_datetime_from_ms(event.start_at_ms),
                local_datetime_from_ms(event.end_at_ms),
            ) else {
                continue;
            };
            events.push(DesktopCalendarEvent {
                id: event.id.clone(),
                title: event.title.clone(),
                start_at,
                end_at,
                all_day: event.all_day,
                color: event.color.clone(),
                notes: event.notes.clone(),
            });
        }
        events
    }

    pub(super) fn render_calendar_page(&self, cx: &Context<'_, Self>) -> Div {
        div()
            .relative()
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_hidden()
            .debug_selector(|| "desktop-calendar-page".to_string())
            .bg(cx.theme().background)
            .font_family(CALENDAR_BODY_FONT_FAMILY)
            .flex()
            .flex_row()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(self.render_calendar_header(cx))
                    .when_some(self.render_search_record_detail(cx), |page, detail| {
                        page.child(detail)
                    })
                    .child(self.render_calendar_body(cx)),
            )
            .when(self.scheduling_visible, |page| {
                page.child(self.render_scheduling_panel(cx))
            })
    }

    pub(super) fn render_calendar_header(&self, cx: &Context<'_, Self>) -> Div {
        div()
            .min_h(px(CALENDAR_HEADER_MIN_HEIGHT_PX))
            .px(px(CALENDAR_HEADER_X_PADDING_PX))
            .py(px(CALENDAR_HEADER_Y_PADDING_PX))
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(theme::longan_800().opacity(CALENDAR_HEADER_BORDER_OPACITY))
            .flex_shrink_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(CALENDAR_HEADER_TITLE_GROUP_GAP_PX))
                    .child(
                        div()
                            .min_w(px(CALENDAR_HEADER_TITLE_MIN_WIDTH_PX))
                            .debug_selector(|| "desktop-calendar-title".to_string())
                            .text_size(px(CALENDAR_HEADER_TITLE_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::rambutan_400())
                            .child(self.calendar_title()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(CALENDAR_NAV_GROUP_GAP_PX))
                            .child(
                                div()
                                    .id("calendar-prev")
                                    .debug_selector(|| "desktop-calendar-prev".to_string())
                                    .size(px(CALENDAR_NAV_BUTTON_SIZE_PX))
                                    .rounded(px(CALENDAR_CONTROL_RADIUS_PX))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(CALENDAR_NAV_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::rambutan_500())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label(CALENDAR_PREVIOUS_LABEL)
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .cursor_pointer()
                                    .hover(|el| {
                                        el.bg(theme::longan_800()
                                            .opacity(CALENDAR_NAV_HOVER_BG_OPACITY))
                                            .text_color(theme::rambutan_300())
                                    })
                                    .tooltip(|window, cx| {
                                        Tooltip::new(CALENDAR_PREVIOUS_LABEL).build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.calendar_prev(cx);
                                    }))
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.calendar_prev(cx);
                                            }
                                        },
                                    ))
                                    .child("‹"),
                            )
                            .child(
                                div()
                                    .id("calendar-today")
                                    .debug_selector(|| "desktop-calendar-today".to_string())
                                    .rounded(px(CALENDAR_CONTROL_RADIUS_PX))
                                    .px(px(CALENDAR_TODAY_X_PADDING_PX))
                                    .py(px(CALENDAR_TODAY_Y_PADDING_PX))
                                    .text_size(px(CALENDAR_TODAY_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_400())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label("Today")
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .cursor_pointer()
                                    .hover(|el| {
                                        el.bg(theme::longan_800()
                                            .opacity(CALENDAR_NAV_HOVER_BG_OPACITY))
                                            .text_color(theme::rambutan_200())
                                    })
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.calendar_today(cx);
                                    }))
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.calendar_today(cx);
                                            }
                                        },
                                    ))
                                    .child("Today"),
                            )
                            .child(
                                div()
                                    .id("calendar-next")
                                    .debug_selector(|| "desktop-calendar-next".to_string())
                                    .size(px(CALENDAR_NAV_BUTTON_SIZE_PX))
                                    .rounded(px(CALENDAR_CONTROL_RADIUS_PX))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(CALENDAR_NAV_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::rambutan_500())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label(CALENDAR_NEXT_LABEL)
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .cursor_pointer()
                                    .hover(|el| {
                                        el.bg(theme::longan_800()
                                            .opacity(CALENDAR_NAV_HOVER_BG_OPACITY))
                                            .text_color(theme::rambutan_300())
                                    })
                                    .tooltip(|window, cx| {
                                        Tooltip::new(CALENDAR_NEXT_LABEL).build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.calendar_next(cx);
                                    }))
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.calendar_next(cx);
                                            }
                                        },
                                    ))
                                    .child("›"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(CALENDAR_ACTION_GROUP_GAP_PX))
                    .child(
                        web_button(
                            "calendar-scheduling",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Schedule")
                        .on_click(cx.listener(
                            |this, _event, _window, cx| {
                                this.toggle_scheduling_panel(cx);
                            },
                        )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(CALENDAR_MODE_SWITCHER_GAP_PX))
                            .rounded(px(CALENDAR_CONTROL_RADIUS_PX))
                            .border_1()
                            .border_color(
                                theme::longan_800().opacity(CALENDAR_MODE_SWITCHER_BORDER_OPACITY),
                            )
                            .bg(theme::longan_900().opacity(CALENDAR_MODE_SWITCHER_BG_OPACITY))
                            .p(px(CALENDAR_MODE_SWITCHER_PADDING_PX))
                            .child(self.render_calendar_mode_button(DesktopCalendarMode::Day, cx))
                            .child(self.render_calendar_mode_button(DesktopCalendarMode::Week, cx))
                            .child(
                                self.render_calendar_mode_button(DesktopCalendarMode::Month, cx),
                            ),
                    )
                    .child(
                        div()
                            .id("calendar-new-event")
                            .debug_selector(|| "desktop-calendar-new-event".to_string())
                            .rounded(px(CALENDAR_CONTROL_RADIUS_PX))
                            .border_1()
                            .border_color(
                                theme::persimmon_400().opacity(CALENDAR_NEW_EVENT_BORDER_OPACITY),
                            )
                            .bg(theme::persimmon_500().opacity(CALENDAR_NEW_EVENT_BG_OPACITY))
                            .px(px(CALENDAR_NEW_EVENT_X_PADDING_PX))
                            .py(px(CALENDAR_NEW_EVENT_Y_PADDING_PX))
                            .text_size(px(CALENDAR_NEW_EVENT_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::persimmon_200())
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(CALENDAR_NEW_EVENT_LABEL)
                            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| {
                                el.bg(theme::persimmon_500()
                                    .opacity(CALENDAR_NEW_EVENT_HOVER_BG_OPACITY))
                            })
                            .tooltip(|window, cx| {
                                Tooltip::new(CALENDAR_NEW_EVENT_LABEL).build(window, cx)
                            })
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                this.open_event_create(None, window, cx);
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.open_event_create(None, window, cx);
                                }
                            }))
                            .child("+ New"),
                    ),
            )
    }

    pub(super) fn render_calendar_mode_button(
        &self,
        mode: DesktopCalendarMode,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let is_active = self.calendar_mode == mode;
        div()
            .id(("calendar-mode", mode as usize))
            .debug_selector(move || format!("desktop-calendar-mode-{}", mode.as_str()))
            .rounded(px(CALENDAR_CONTROL_RADIUS_PX))
            .border_1()
            .border_color(if is_active {
                theme::persimmon_400().opacity(CALENDAR_MODE_ACTIVE_BORDER_OPACITY)
            } else {
                theme::longan_900().opacity(CALENDAR_MODE_INACTIVE_OPACITY)
            })
            .bg(if is_active {
                theme::persimmon_500().opacity(CALENDAR_MODE_ACTIVE_BG_OPACITY)
            } else {
                theme::longan_900().opacity(CALENDAR_MODE_INACTIVE_OPACITY)
            })
            .px(px(CALENDAR_MODE_X_PADDING_PX))
            .py(px(CALENDAR_MODE_Y_PADDING_PX))
            .text_size(px(CALENDAR_MODE_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .font_weight(FontWeight::MEDIUM)
            .text_color(if is_active {
                theme::persimmon_200()
            } else {
                theme::rambutan_500()
            })
            .tab_index(0)
            .role(Role::Tab)
            .aria_label(mode.label())
            .aria_selected(is_active)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .when(!is_active, |el| {
                el.hover(|el| {
                    el.bg(theme::longan_800().opacity(CALENDAR_MODE_HOVER_BG_OPACITY))
                        .text_color(theme::rambutan_200())
                })
            })
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.set_calendar_mode(mode, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.set_calendar_mode(mode, cx);
                }
            }))
            .child(mode.label())
            .into_any_element()
    }

    pub(super) fn render_calendar_body(&self, cx: &Context<'_, Self>) -> AnyElement {
        match self.calendar_mode {
            DesktopCalendarMode::Month => self.render_calendar_month(cx).into_any_element(),
            DesktopCalendarMode::Week => self.render_calendar_timegrid(7, cx),
            DesktopCalendarMode::Day => self.render_calendar_timegrid(1, cx),
        }
    }

    pub(super) fn render_calendar_month(&self, cx: &Context<'_, Self>) -> Div {
        let start = month_grid_start(self.calendar_anchor);
        let today = today_date();
        let weekday_headers: Vec<AnyElement> = WEEKDAY_NAMES
            .iter()
            .map(|day| {
                div()
                    .flex_1()
                    .px(px(CALENDAR_WEEKDAY_X_PADDING_PX))
                    .py(px(CALENDAR_WEEKDAY_Y_PADDING_PX))
                    .text_size(px(CALENDAR_WEEKDAY_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::rambutan_500())
                    .flex()
                    .justify_center()
                    .child(day.to_ascii_uppercase())
                    .into_any_element()
            })
            .collect();

        let weeks: Vec<AnyElement> = (0..6)
            .map(|week| {
                let cells: Vec<AnyElement> = (0..7)
                    .map(|day| {
                        let index = week * 7 + day;
                        let date = start + Duration::days(i64::try_from(index).unwrap_or(0));
                        self.render_calendar_month_cell(
                            index,
                            date,
                            date.month() == self.calendar_anchor.month()
                                && date.year() == self.calendar_anchor.year(),
                            date == today,
                            cx,
                        )
                    })
                    .collect();
                div()
                    .flex_1()
                    .min_h(px(CALENDAR_MONTH_ROW_MIN_HEIGHT_PX))
                    .flex()
                    .flex_row()
                    .children(cells)
                    .into_any_element()
            })
            .collect();

        div()
            .flex_1()
            .min_h_0()
            .debug_selector(|| "desktop-calendar-month".to_string())
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(CALENDAR_HEADER_BORDER_OPACITY))
                    .children(weekday_headers),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .children(weeks),
            )
    }

    pub(super) fn render_calendar_month_cell(
        &self,
        index: usize,
        date: NaiveDate,
        in_month: bool,
        is_today: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let events: Vec<DesktopCalendarEvent> = self
            .visible_calendar_events(cx)
            .into_iter()
            .filter(|event| event_overlaps_day(event, date))
            .collect();
        let tasks: Vec<DesktopWorkItem> = self
            .work_items
            .iter()
            .filter(|item| work_item_due_on(item, date))
            .cloned()
            .collect();
        let event_take = events.len().min(3);
        let task_take = (3usize.saturating_sub(event_take)).min(tasks.len());
        let visible_events: Vec<AnyElement> = events
            .iter()
            .take(event_take)
            .map(|event| self.render_calendar_event_pill(event, cx))
            .collect();
        let visible_tasks: Vec<AnyElement> = tasks
            .iter()
            .take(task_take)
            .map(|item| self.render_calendar_task_pill(item, cx))
            .collect();
        let visible_count = event_take + task_take;
        let extra = (events.len() + tasks.len()).saturating_sub(visible_count);
        let date_for_new = date;
        let date_for_new_key = date;
        let cell_is_last_col = index % 7 == 6;
        let cell_is_last_row = index / 7 == 5;
        let date_label = if is_today {
            div()
                .size(px(CALENDAR_MONTH_TODAY_SIZE_PX))
                .rounded(px(CALENDAR_MONTH_TODAY_RADIUS_PX))
                .bg(theme::persimmon_500())
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(CALENDAR_DATE_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::longan_950())
                .child(date.day().to_string())
        } else if in_month {
            div()
                .text_size(px(CALENDAR_DATE_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_300())
                .child(date.day().to_string())
        } else {
            div()
                .text_size(px(CALENDAR_DATE_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_500())
                .child(date.day().to_string())
        };

        div()
            .id((
                "calendar-day",
                u64::from(date.num_days_from_ce().cast_unsigned()),
            ))
            .debug_selector(move || format!("desktop-calendar-day-{date}"))
            .flex_1()
            .min_w_0()
            .p(px(CALENDAR_MONTH_CELL_PADDING_PX))
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Create event on {}", date.format("%A, %B %-d, %Y")))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .flex()
            .flex_col()
            .gap_1()
            .hover(|el| el.bg(theme::longan_800().opacity(CALENDAR_MONTH_CELL_HOVER_BG_OPACITY)))
            .when(!cell_is_last_col, |el| el.border_r_1())
            .when(!cell_is_last_row, |el| el.border_b_1())
            .border_color(theme::longan_800().opacity(CALENDAR_MONTH_CELL_BORDER_OPACITY))
            .on_click(cx.listener(move |this, _ev, window, cx| {
                this.open_event_create(Some(at_hour(date_for_new, 9)), window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.open_event_create(Some(at_hour(date_for_new_key, 9)), window, cx);
                }
            }))
            .child(date_label)
            .children(visible_events)
            .children(visible_tasks)
            .when(extra > 0, |el| {
                el.child(
                    div()
                        .text_size(px(CALENDAR_PILL_TEXT_SIZE_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(format!("+{extra} more")),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_calendar_task_pill(
        &self,
        item: &DesktopWorkItem,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let item_id = item.id.clone();
        let item_id_for_key = item.id.clone();
        let debug_key = element_key(&item.id);
        div()
            .id(("calendar-task-pill", element_key(&item.id)))
            .debug_selector(move || format!("desktop-calendar-task-pill-{debug_key}"))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(CALENDAR_PILL_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_500())
            .italic()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open task {}", item.title))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.text_color(theme::rambutan_400()))
            .tooltip(|window, cx| Tooltip::new(CALENDAR_TASK_DUE_LABEL).build(window, cx))
            .on_click(cx.listener(move |this, _ev, window, cx| {
                cx.stop_propagation();
                this.select_work_item_by_id(&item_id, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.select_work_item_by_id(&item_id_for_key, window, cx);
                }
            }))
            .child(
                div()
                    .size(px(CALENDAR_PILL_DOT_SIZE_PX))
                    .rounded(px(CALENDAR_PILL_DOT_RADIUS_PX))
                    .border_1()
                    .border_color(theme::rambutan_500())
                    .flex_shrink_0(),
            )
            .child(div().min_w_0().truncate().child(item.title.clone()))
            .into_any_element()
    }

    pub(super) fn render_calendar_event_pill(
        &self,
        event: &DesktopCalendarEvent,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let event_id = event.id.clone();
        let event_id_for_key = event.id.clone();
        let debug_key = element_key(&event.id);
        div()
            .id(("calendar-event-pill", element_key(&event.id)))
            .debug_selector(move || format!("desktop-calendar-event-pill-{debug_key}"))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(CALENDAR_PILL_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Edit event {}", event.title))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.text_color(theme::rambutan_200()))
            .on_click(cx.listener(move |this, _ev, window, cx| {
                cx.stop_propagation();
                this.open_event_edit(event_id.clone(), window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.open_event_edit(event_id_for_key.clone(), window, cx);
                }
            }))
            .child(
                div()
                    .size(px(CALENDAR_PILL_DOT_SIZE_PX))
                    .rounded(px(CALENDAR_PILL_DOT_RADIUS_PX))
                    .bg(event_color(event.color.as_deref()))
                    .flex_shrink_0(),
            )
            .child(div().min_w_0().truncate().child(event.title.clone()))
            .into_any_element()
    }

    pub(super) fn render_calendar_timegrid(
        &self,
        columns: usize,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let start = if columns == 7 {
            week_start(self.calendar_anchor)
        } else {
            self.calendar_anchor
        };
        let days: Vec<NaiveDate> = (0..columns)
            .map(|index| start + Duration::days(i64::try_from(index).unwrap_or(i64::MAX)))
            .collect();
        let header_cells: Vec<AnyElement> = days
            .iter()
            .map(|date| {
                let is_today = *date == today_date();
                div()
                    .flex_1()
                    .px(px(CALENDAR_TIMEGRID_HEADER_X_PADDING_PX))
                    .py(px(CALENDAR_TIMEGRID_HEADER_Y_PADDING_PX))
                    .border_l_1()
                    .border_color(
                        theme::longan_800().opacity(CALENDAR_TIMEGRID_COLUMN_BORDER_OPACITY),
                    )
                    .text_size(px(CALENDAR_TIMEGRID_HEADER_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(if is_today {
                        theme::persimmon_300()
                    } else {
                        theme::rambutan_400()
                    })
                    .flex()
                    .justify_center()
                    .child(format_day_header(*date))
                    .into_any_element()
            })
            .collect();
        let day_columns: Vec<AnyElement> = days
            .iter()
            .map(|date| {
                self.render_calendar_timegrid_day_column(*date, cx)
                    .into_any_element()
            })
            .collect();
        let hour_rows: Vec<AnyElement> = (0..24)
            .map(|hour| {
                div()
                    .h(px(DESKTOP_HOUR_ROW_PX))
                    .border_b_1()
                    .border_color(
                        theme::longan_800().opacity(CALENDAR_TIMEGRID_HOUR_LABEL_BORDER_OPACITY),
                    )
                    .px(px(CALENDAR_TIMEGRID_HOUR_X_PADDING_PX))
                    .pt(px(CALENDAR_TIMEGRID_HOUR_TOP_PADDING_PX))
                    .text_size(px(CALENDAR_TIMEGRID_HOUR_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child(format!("{hour:02}:00"))
                    .into_any_element()
            })
            .collect();

        div()
            .flex_1()
            .min_h_0()
            .debug_selector(move || format!("desktop-calendar-timegrid-{columns}"))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .border_b_1()
                    .border_color(
                        theme::longan_800().opacity(CALENDAR_TIMEGRID_HEADER_BORDER_OPACITY),
                    )
                    .child(div().w(px(CALENDAR_TIMEGRID_AXIS_WIDTH_PX)))
                    .children(header_cells),
            )
            .child(
                div().flex_1().min_h_0().overflow_y_scrollbar().child(
                    div()
                        .h(px(timegrid_total_height_px()))
                        .flex()
                        .flex_row()
                        .child(
                            div()
                                .w(px(CALENDAR_TIMEGRID_AXIS_WIDTH_PX))
                                .flex_shrink_0()
                                .border_r_1()
                                .border_color(
                                    theme::longan_800()
                                        .opacity(CALENDAR_TIMEGRID_COLUMN_BORDER_OPACITY),
                                )
                                .children(hour_rows),
                        )
                        .children(day_columns),
                ),
            )
            .into_any_element()
    }

    pub(super) fn render_calendar_timegrid_day_column(
        &self,
        date: NaiveDate,
        cx: &Context<'_, Self>,
    ) -> Div {
        let hour_rows: Vec<AnyElement> = (0..24)
            .map(|hour| {
                let start_for_new = at_hour(date, hour);
                let start_for_new_key = start_for_new;
                div()
                    .id((
                        "calendar-timegrid-hour",
                        element_key(&format!("{date}-{hour}")),
                    ))
                    .debug_selector(move || format!("desktop-calendar-timegrid-hour-{date}-{hour}"))
                    .h(px(DESKTOP_HOUR_ROW_PX))
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!(
                        "Create event on {} at {hour:02}:00",
                        date.format("%A, %B %-d, %Y")
                    ))
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .border_b_1()
                    .border_color(
                        theme::longan_800().opacity(CALENDAR_TIMEGRID_HOUR_SLOT_BORDER_OPACITY),
                    )
                    .hover(|el| {
                        el.bg(theme::longan_800()
                            .opacity(CALENDAR_TIMEGRID_HOUR_SLOT_HOVER_BG_OPACITY))
                    })
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        this.open_event_create(Some(start_for_new), window, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.open_event_create(Some(start_for_new_key), window, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        let day_events: Vec<DesktopCalendarEvent> = self
            .visible_calendar_events(cx)
            .into_iter()
            .filter(|event| event_overlaps_day(event, date))
            .collect();
        let event_blocks: Vec<AnyElement> = lay_out_timegrid_events(&day_events, date)
            .into_iter()
            .map(|event| self.render_calendar_timegrid_event_block(event, cx))
            .collect();
        let now = Local::now().naive_local();
        let now_y = timegrid_datetime_offset_px(now, date);

        div()
            .relative()
            .h(px(timegrid_total_height_px()))
            .flex_1()
            .min_w_0()
            .border_l_1()
            .border_color(theme::longan_800().opacity(CALENDAR_TIMEGRID_COLUMN_BORDER_OPACITY))
            .on_drag_move(cx.listener(
                move |this, drag: &DragMoveEvent<DesktopTimegridDrag>, _window, cx| {
                    this.apply_timegrid_drag(drag, date, cx);
                },
            ))
            .capture_any_mouse_up(cx.listener(|this, _event, _window, cx| {
                this.clear_timegrid_drag(cx);
            }))
            .children(hour_rows)
            .when(date == today_date(), |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(now_y))
                        .left_0()
                        .right_0()
                        .h(px(CALENDAR_TIMEGRID_NOW_LINE_HEIGHT_PX))
                        .bg(theme::persimmon_500())
                        .child(
                            div()
                                .absolute()
                                .left(px(CALENDAR_TIMEGRID_NOW_DOT_OFFSET_PX))
                                .top(px(CALENDAR_TIMEGRID_NOW_DOT_OFFSET_PX))
                                .size(px(DESKTOP_TIMEGRID_NOW_DOT_PX))
                                .rounded(px(CALENDAR_TIMEGRID_NOW_DOT_RADIUS_PX))
                                .bg(theme::persimmon_500()),
                        ),
                )
            })
            .children(event_blocks)
    }

    pub(super) fn render_calendar_timegrid_event_block(
        &self,
        laid_out: DesktopLaidOutEvent,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let event = laid_out.event;
        let event_id = event.id.clone();
        let event_id_for_key = event.id.clone();
        let workspace = cx.weak_entity();
        let lane_width = 1.0 / laid_out.lanes.to_f32_lossy();
        let lane_left = lane_width * laid_out.lane.to_f32_lossy();
        let accent_color = event_color(event.color.as_deref());
        let surface_color = event_timegrid_surface_color(event.color.as_deref());
        let text_color = event_timegrid_text_color(event.color.as_deref());
        let move_drag = DesktopTimegridDrag::new(&event, DesktopTimegridDragKind::Move);
        let resize_drag = DesktopTimegridDrag::new(&event, DesktopTimegridDragKind::ResizeEnd);
        let debug_key = element_key(&event.id);

        div()
            .id(("calendar-time-event", element_key(&event.id)))
            .debug_selector(move || format!("desktop-calendar-time-event-{debug_key}"))
            .absolute()
            .top(px(laid_out.top_px))
            .left(relative(lane_left))
            .w(relative(lane_width))
            .h(px(laid_out.height_px))
            .child(
                div()
                    .id(("calendar-time-event-inner", element_key(&event.id)))
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(CALENDAR_TIMEGRID_EVENT_INSET_PX))
                    .right(px(CALENDAR_TIMEGRID_EVENT_INSET_PX))
                    .overflow_hidden()
                    .rounded(px(CALENDAR_TIMEGRID_EVENT_RADIUS_PX))
                    .border_1()
                    .border_color(accent_color.opacity(CALENDAR_TIMEGRID_EVENT_BORDER_OPACITY))
                    .bg(surface_color.opacity(CALENDAR_TIMEGRID_EVENT_BG_OPACITY))
                    .px(px(CALENDAR_TIMEGRID_EVENT_X_PADDING_PX))
                    .py(px(CALENDAR_TIMEGRID_EVENT_Y_PADDING_PX))
                    .text_size(px(CALENDAR_TIMEGRID_EVENT_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(text_color)
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Edit event {}", event.title))
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_grab()
                    .hover(|el| {
                        el.bg(surface_color.opacity(CALENDAR_TIMEGRID_EVENT_HOVER_BG_OPACITY))
                    })
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(event.title.clone()),
                    )
                    .on_drag(move_drag, {
                        let workspace = workspace.clone();
                        move |drag: &DesktopTimegridDrag, offset, _window, cx| {
                            let grab_offset_y_px = f32::from(offset.y);
                            let _ = workspace.update(cx, |this, cx| {
                                this.begin_timegrid_drag(drag, grab_offset_y_px, cx);
                            });
                            cx.stop_propagation();
                            cx.new(|_| drag.clone())
                        }
                    })
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        this.open_event_edit(event_id.clone(), window, cx);
                    }))
                    .on_key_down(cx.listener(move |this, key: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(key.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.open_event_edit(event_id_for_key.clone(), window, cx);
                        }
                    }))
                    .child(
                        div()
                            .id(("calendar-time-event-resize", element_key(&event.id)))
                            .debug_selector(move || {
                                format!("desktop-calendar-time-event-resize-{debug_key}")
                            })
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(DESKTOP_TIMEGRID_RESIZE_HANDLE_PX))
                            .cursor_ns_resize()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .on_drag(resize_drag, {
                                move |drag: &DesktopTimegridDrag, _offset, _window, cx| {
                                    let _ = workspace.update(cx, |this, cx| {
                                        this.begin_timegrid_drag(drag, 0.0, cx);
                                    });
                                    cx.stop_propagation();
                                    cx.new(|_| drag.clone())
                                }
                            })
                            .on_click(|_, _, cx| {
                                cx.stop_propagation();
                            }),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    }),
            )
            .into_any_element()
    }
}
