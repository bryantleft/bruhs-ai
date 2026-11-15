//! Former preparation paths; both benchmark modes use the current renderer.
use crate::views::workspace::*;

impl Workspace {
    pub(in crate::views::workspace) fn legacy_visible_calendar_events(
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
                ..Default::default()
            });
        }
        events
    }

    pub(in crate::views::workspace) fn legacy_render_calendar_month_cell(
        &self,
        index: usize,
        date: NaiveDate,
        in_month: bool,
        is_today: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let events = self
            .legacy_visible_calendar_events(cx)
            .into_iter()
            .filter(|event| event_overlaps_day(event, date))
            .collect::<Vec<_>>();
        let tasks = self
            .work_items
            .iter()
            .filter(|task| work_item_due_on(task, date))
            .cloned()
            .collect::<Vec<_>>();
        let data = CalendarMonthDay {
            date,
            start: at_hour(date, 0),
            end: at_hour(date + Duration::days(1), 0),
            total_records: events.len() + tasks.len(),
            events: events.iter().take(3).collect(),
            tasks: tasks.iter().take(3).collect(),
        };
        self.render_calendar_month_cell(index, &data, in_month, is_today, cx)
    }

    pub(in crate::views::workspace) fn legacy_render_calendar_timegrid_day_column(
        &self,
        date: NaiveDate,
        cx: &Context<'_, Self>,
    ) -> Div {
        let events = self
            .legacy_visible_calendar_events(cx)
            .into_iter()
            .filter(|event| event_overlaps_day(event, date))
            .collect::<Vec<_>>();
        self.render_calendar_timegrid_day_column(date, &events, cx)
    }
}
