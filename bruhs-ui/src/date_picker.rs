//! Shared month calendar grid and ISO date/time parsing primitives.
//!
//! Keeps scheduling, timezone, and persistence rules at the feature call sites
//! while sharing calendar math, month navigation, and keyboard focus geometry.

use crate::icons;
use crate::press::on_press;
use crate::theme;
use crate::zoom::px;
use bruhs_models::time::days_in_month;
use gpui::prelude::*;
use gpui::{
    div, relative, svg, AnyElement, App, Div, FocusHandle, FontWeight, Hsla, Role, SharedString,
    StyleRefinement, Window,
};
use std::rc::Rc;

/// Full English month names indexed `0..12`.
pub const MONTHS: [&str; 12] = [
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

/// Two-letter Sunday-first weekday headings.
pub const WEEKDAYS: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

const CELL_PX: f32 = theme::APP_COMPACT_CONTROL_HEIGHT_PX;
const GRID_GAP_PX: f32 = 2.0;
const MONTH_BUTTON_PX: f32 = theme::CONTROL_SM_PX;
const LABEL_PX: f32 = 14.0;
const LABEL_SM_PX: f32 = 12.0;
const SEED_RADIUS_PX: f32 = 4.0;

/// Callback invoked when the user clicks previous (`-1`) or next (`+1`) month.
pub type StepMonthHandler = Rc<dyn Fn(i32, &mut Window, &mut App)>;
/// Callback invoked when the user picks a calendar date `(year, month, day)`.
pub type PickDateHandler = Rc<dyn Fn((i32, u32, u32), &mut Window, &mut App)>;

/// Days since `1970-01-01` in the proleptic Gregorian calendar (Howard Hinnant).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let d = i64::from(day);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`]: civil `(year, month, day)` from days since `1970-01-01`.
fn civil_from_days(days: i64) -> Option<(i32, u32, u32)> {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = i32::try_from(y + i64::from(m <= 2)).ok()?;
    let month = u32::try_from(m).ok()?;
    let day = u32::try_from(d).ok()?;
    Some((year, month, day))
}

/// Parse `YYYY-MM-DD` into `(year, month, day)`.
pub fn parse_iso_date(s: &str) -> Option<(i32, u32, u32)> {
    let mut parts = s.trim().split('-');
    let y = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// Parse `YYYY-MM-DDTHH:MM[:SS]` into `(year, month, day, hour, minute)`.
pub fn parse_iso_datetime(s: &str) -> Option<(i32, u32, u32, u32, u32)> {
    let (date, time) = s.trim().split_once('T')?;
    let (y, m, d) = parse_iso_date(date)?;
    let (h, mi) = time.split_once(':')?;
    let mi = mi.split(':').next()?;
    let h = h.parse::<u32>().ok().filter(|h| *h < 24)?;
    let mi = mi.parse::<u32>().ok().filter(|x| *x < 60)?;
    Some((y, m, d, h, mi))
}

/// Weekday (`0 = Sunday ..= 6 = Saturday`) of the 1st of `(year, month)`.
pub fn first_weekday(year: i32, month: u32) -> u32 {
    if !(1..=12).contains(&month) {
        return 0;
    }
    // 1970-01-01 was a Thursday (index 4).
    u32::try_from((days_from_civil(year, month, 1) + 4).rem_euclid(7)).unwrap_or(0)
}

/// Trigger button label for an ISO date or date-time field, or its placeholder.
pub fn trigger_label(with_time: bool, value: &str) -> String {
    if with_time {
        match parse_iso_datetime(value) {
            Some((y, m, d, h, mi)) => format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}"),
            None => "Select date & time".to_string(),
        }
    } else if value.trim().is_empty() {
        "Select date".to_string()
    } else {
        value.trim().to_string()
    }
}

/// Month after stepping `delta` (`±1`) from `(year, month)`.
pub fn step_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let index = year * 12 + (i32::try_from(month).unwrap_or(1) - 1) + delta;
    (index.div_euclid(12), (index.rem_euclid(12) + 1) as u32)
}

fn focus_ring(style: StyleRefinement) -> StyleRefinement {
    style.border_color(theme::persimmon_400())
}

fn label(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        .when(size <= 12.0, |el| {
            el.font_family(theme::UI_SMALL_FONT_FAMILY)
        })
        .text_size(px(size))
        .font_weight(FontWeight::MEDIUM)
        .line_height(relative(1.5))
        .text_color(color)
        .child(text.into())
}

/// Shared month calendar grid with previous/next navigation and selectable days.
///
/// Supplying `navigation` (`day_focus` + custom header title) switches to a
/// fixed 6-week (42-slot) grid with a single roving tab stop on the selected
/// day, matching the scheduled-task date-time picker.
#[derive(IntoElement)]
pub struct MonthGrid {
    year: i32,
    month: u32,
    selected: Option<(i32, u32, u32)>,
    selector_prefix: SharedString,
    navigation: Option<(FocusHandle, AnyElement)>,
    on_step: StepMonthHandler,
    on_pick: PickDateHandler,
}

impl MonthGrid {
    pub fn new(
        year: i32,
        month: u32,
        selected: Option<(i32, u32, u32)>,
        on_step: StepMonthHandler,
        on_pick: PickDateHandler,
    ) -> Self {
        Self {
            year,
            month,
            selected,
            selector_prefix: "date-picker".into(),
            navigation: None,
            on_step,
            on_pick,
        }
    }

    /// Customize the ID and debug-selector prefix (defaults to `"date-picker"`).
    pub fn selector_prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.selector_prefix = prefix.into();
        self
    }

    /// Attach a custom month header element and roving focus handle for the active day cell.
    pub fn navigation(mut self, day_focus: FocusHandle, title: AnyElement) -> Self {
        self.navigation = Some((day_focus, title));
        self
    }
}

impl RenderOnce for MonthGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self {
            year,
            month,
            selected,
            selector_prefix,
            navigation,
            on_step,
            on_pick,
        } = self;

        let lead = first_weekday(year, month) as usize;
        let total = days_in_month(year, month).unwrap_or(30);
        let slots = if navigation.is_some() {
            42
        } else {
            (lead + total as usize).div_ceil(7) * 7
        };
        let first_days = (1..=12)
            .contains(&month)
            .then(|| days_from_civil(year, month, 1));
        let cells: Vec<Option<(i32, u32, u32)>> = (0..slots)
            .map(|i| {
                let offset = i64::try_from(i).ok()? - i64::try_from(lead).ok()?;
                let date = civil_from_days(first_days? + offset)?;
                if navigation.is_some() {
                    (1..=9999).contains(&date.0).then_some(date)
                } else {
                    (date.1 == month).then_some(date)
                }
            })
            .collect();

        let (day_focus, title) = match navigation {
            Some((focus, title)) => (Some(focus), title),
            None => {
                let month_name = MONTHS
                    .get(month.saturating_sub(1) as usize)
                    .copied()
                    .unwrap_or(MONTHS[0]);
                (
                    None,
                    label(
                        format!("{month_name} {year}"),
                        LABEL_PX,
                        theme::rambutan_100(),
                    )
                    .into_any_element(),
                )
            }
        };

        let month_button = |suffix: &'static str, path: &'static str, delta: i32| {
            let id = SharedString::from(format!("{selector_prefix}-{suffix}"));
            let debug_id = id.clone();
            let step = on_step.clone();
            on_press(
                div()
                    .id(id)
                    .debug_selector(move || debug_id.to_string())
                    .size(px(MONTH_BUTTON_PX))
                    .rounded(px(SEED_RADIUS_PX))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::rambutan_300())
                    .role(Role::Button)
                    .aria_label(if delta < 0 {
                        "Previous month"
                    } else {
                        "Next month"
                    })
                    .tab_index(0)
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(focus_ring)
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .child(
                        svg()
                            .path(path)
                            .size(px(14.0))
                            .text_color(theme::rambutan_300()),
                    ),
                move |window, cx| step(delta, window, cx),
            )
        };

        let mut weekday_row = div().flex().gap(px(GRID_GAP_PX)).mb_1();
        for day in WEEKDAYS {
            weekday_row = weekday_row.child(
                div()
                    .flex_1()
                    .h(px(LABEL_SM_PX * 1.5))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(label(day, LABEL_SM_PX, theme::rambutan_500())),
            );
        }

        let mut grid = div().flex().flex_col().gap(px(GRID_GAP_PX));
        for (row_ix, week) in cells.chunks(7).enumerate() {
            let mut row = div().flex().gap(px(GRID_GAP_PX));
            for (col_ix, cell) in week.iter().enumerate() {
                let slot = div()
                    .flex_1()
                    .h(px(CELL_PX))
                    .flex()
                    .items_center()
                    .justify_center();
                let Some(date @ (cell_year, cell_month, day)) = *cell else {
                    row = row.child(slot);
                    continue;
                };
                let adjacent = cell_month != month;
                let is_selected = selected == Some(date);
                let cell_ix = row_ix * 7 + col_ix;
                let prefix_for_day = selector_prefix.clone();
                let pick = on_pick.clone();
                let month_name = MONTHS
                    .get(cell_month.saturating_sub(1) as usize)
                    .copied()
                    .unwrap_or(MONTHS[0]);
                row = row.child(
                    slot.child(on_press(
                        div()
                            .id((
                                SharedString::from(format!("{selector_prefix}-day")),
                                cell_ix,
                            ))
                            .debug_selector(move || {
                                format!(
                                    "{prefix_for_day}-day-{cell_year:04}-{cell_month:02}-{day:02}"
                                )
                            })
                            .size(px(CELL_PX))
                            .rounded(px(SEED_RADIUS_PX))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(LABEL_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .map(|el| {
                                if is_selected {
                                    el.bg(theme::persimmon_500())
                                        .text_color(theme::longan_950())
                                } else {
                                    el.text_color(if adjacent {
                                        theme::rambutan_500()
                                    } else {
                                        theme::rambutan_200()
                                    })
                                    .hover(|el| el.bg(theme::longan_800()))
                                }
                            })
                            .role(Role::Button)
                            .aria_label(format!("{month_name} {day}, {cell_year}"))
                            .aria_selected(is_selected)
                            .tab_index(0)
                            .when_some(day_focus.as_ref(), |el, handle| {
                                let cursor = !adjacent
                                    && selected.map_or(1, |(_, _, d)| d.min(total)) == day;
                                el.tab_stop(cursor).when(cursor, |el| {
                                    el.track_focus(&handle.clone().tab_stop(true))
                                })
                            })
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .focus_visible(focus_ring)
                            .cursor_pointer()
                            .child(day.to_string()),
                        move |window, cx| pick(date, window, cx),
                    )),
                );
            }
            grid = grid.child(row);
        }

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mb_2()
                    .child(month_button("prev", icons::CHEVRON_LEFT, -1))
                    .child(title)
                    .child(month_button("next", icons::CHEVRON_RIGHT, 1)),
            )
            .child(weekday_row)
            .child(grid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, TestAppContext};

    #[test]
    fn calendar_math_matches_gregorian_rules() {
        assert_eq!(first_weekday(1970, 1), 4); // Thursday
        assert_eq!(first_weekday(2024, 2), 4); // Thursday (leap year)
        assert_eq!(first_weekday(2026, 11), 0); // Sunday
        assert_eq!(step_month(2026, 1, -1), (2025, 12));
        assert_eq!(step_month(2026, 12, 1), (2027, 1));
        assert_eq!(parse_iso_date("2026-11-27"), Some((2026, 11, 27)));
        assert_eq!(
            parse_iso_datetime("2026-11-27T09:30:00"),
            Some((2026, 11, 27, 9, 30))
        );
        assert_eq!(trigger_label(false, ""), "Select date");
        assert_eq!(trigger_label(true, "2026-11-27T09:30"), "2026-11-27 09:30");
    }

    struct GridHarness {
        year: i32,
        month: u32,
        selected: Option<(i32, u32, u32)>,
    }

    impl gpui::Render for GridHarness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let step_entity = cx.entity().downgrade();
            let pick_entity = cx.entity().downgrade();
            div().w(px(256.0)).child(MonthGrid::new(
                self.year,
                self.month,
                self.selected,
                Rc::new(move |delta, _, cx| {
                    let _ = step_entity.update(cx, |this, cx| {
                        let (y, m) = step_month(this.year, this.month, delta);
                        this.year = y;
                        this.month = m;
                        cx.notify();
                    });
                }),
                Rc::new(move |date, _, cx| {
                    let _ = pick_entity.update(cx, |this, cx| {
                        this.selected = Some(date);
                        cx.notify();
                    });
                }),
            ))
        }
    }

    #[gpui::test]
    fn month_grid_navigates_months_and_picks_days(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| GridHarness {
            year: 2026,
            month: 11,
            selected: None,
        });
        cx.run_until_parked();

        let next = cx.debug_bounds("date-picker-next").expect("next button");
        cx.simulate_click(next.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| (h.year, h.month)), (2026, 12));

        let day = cx
            .debug_bounds("date-picker-day-2026-12-15")
            .expect("dec 15 cell");
        cx.simulate_click(day.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.selected), Some((2026, 12, 15)));
    }
}
