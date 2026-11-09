use super::*;
use bruhs_models::{life_record_detail, RecordDetail, SelectedLifeRecord};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SelectedSearchRecord {
    Life(SelectedLifeRecord),
    SchedulingLink(String),
    ExternalEvent(String),
}

impl SelectedSearchRecord {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Life(record) => &record.id,
            Self::SchedulingLink(id) | Self::ExternalEvent(id) => id,
        }
    }

    fn view(&self) -> AppView {
        match self {
            Self::Life(_) => AppView::Life,
            Self::SchedulingLink(_) | Self::ExternalEvent(_) => AppView::Calendar,
        }
    }
}

fn date(timestamp_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(timestamp_ms).map_or_else(
        || "Unknown date".into(),
        |date| {
            date.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M %:z")
                .to_string()
        },
    )
}

fn calendar_date(timestamp_ms: f64) -> String {
    if timestamp_ms.is_finite() {
        date(EpochMs::saturating_from_f64(timestamp_ms).as_millis())
    } else {
        "Unknown date".into()
    }
}

impl Workspace {
    pub(super) fn open_search_record(
        &mut self,
        record: SelectedSearchRecord,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_transient_surfaces();
        self.active_view = record.view();
        if let SelectedSearchRecord::Life(record) = &record {
            self.life_tab = record.section;
        }
        self.scheduling_visible = false;
        self.compact_sidebar_open = false;
        self.selected_search_record = Some(record);
        window.focus(&self.search_record_focus, cx);
        // Inspecting a record neither writes the workspace nor changes grants.
        cx.notify();
    }

    fn dismiss_search_record(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.selected_search_record = None;
        window.focus(&self.workspace_focus_handle, cx);
        cx.notify();
    }

    pub(super) fn selected_search_record_detail(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<RecordDetail> {
        match self.selected_search_record.as_ref()? {
            SelectedSearchRecord::Life(record) => {
                life_record_detail(&self.life_context, record, date)
            }
            SelectedSearchRecord::SchedulingLink(id) => {
                let record = self
                    .scheduling_preferences
                    .links
                    .iter()
                    .find(|record| &record.id == id)?;
                Some(RecordDetail {
                    title: record.title.clone(),
                    kind: "Scheduling link",
                    fields: vec![
                        ("Booking route", format!("#/book/{}", record.slug)),
                        ("Duration", format!("{} minutes", record.duration_minutes)),
                        (
                            "Buffer before",
                            format!("{} minutes", record.buffer_before_minutes),
                        ),
                        (
                            "Buffer after",
                            format!("{} minutes", record.buffer_after_minutes),
                        ),
                        (
                            "Minimum notice",
                            format!("{} hours", record.minimum_notice_hours),
                        ),
                        (
                            "Booking window",
                            format!("{} days", record.booking_window_days),
                        ),
                        (
                            "Status",
                            if record.enabled {
                                "Enabled"
                            } else {
                                "Disabled"
                            }
                            .into(),
                        ),
                    ],
                })
            }
            SelectedSearchRecord::ExternalEvent(id) => {
                let record = self
                    .meetings_hub
                    .read(cx)
                    .google_events
                    .iter()
                    .find(|record| &record.id == id)?;
                Some(RecordDetail {
                    title: record.title.clone(),
                    kind: "External calendar event",
                    fields: vec![
                        ("Start", calendar_date(record.start_at_ms)),
                        ("End", calendar_date(record.end_at_ms)),
                        (
                            "All day",
                            if record.all_day {
                                "Yes · end is exclusive"
                            } else {
                                "No"
                            }
                            .into(),
                        ),
                        (
                            "Location",
                            record.location.clone().unwrap_or_else(|| "Not set".into()),
                        ),
                        (
                            "Notes",
                            record.notes.clone().unwrap_or_else(|| "Not set".into()),
                        ),
                        (
                            "Attendees",
                            record
                                .attendees
                                .iter()
                                .map(|attendee| {
                                    attendee.display_name.as_ref().map_or_else(
                                        || attendee.email.clone(),
                                        |name| format!("{name} <{}>", attendee.email),
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        (
                            "Calendar",
                            record
                                .calendar_id
                                .clone()
                                .unwrap_or_else(|| "Not set".into()),
                        ),
                    ],
                })
            }
        }
    }

    pub(super) fn render_search_record_detail(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let selected = self.selected_search_record.as_ref()?;
        if selected.view() != self.active_view
            || matches!(selected, SelectedSearchRecord::Life(record) if record.section != self.life_tab)
        {
            return None;
        }
        let detail = self
            .selected_search_record_detail(cx)
            .unwrap_or_else(|| RecordDetail {
                title: "Record unavailable".into(),
                kind: "Selected record",
                fields: vec![(
                    "Status",
                    "This record is no longer available. Search again or return to the overview."
                        .into(),
                )],
            });
        Some(
            div()
                .id("search-record-detail")
                .debug_selector(|| "desktop-search-record-detail".into())
                .track_focus(&self.search_record_focus)
                .role(Role::Group)
                .aria_label(format!("{}: {}", detail.kind, detail.title))
                .on_action(cx.listener(|this, _: &CloseOverlays, window, cx| {
                    this.dismiss_search_record(window, cx)
                }))
                .min_w_0()
                .flex_shrink_0()
                .max_h(px(420.0))
                .overflow_y_scrollbar()
                .rounded(px(8.0))
                .border_1()
                .border_color(theme::longan_800())
                .bg(theme::longan_900().opacity(0.65))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(theme::blueberry_400())
                        .child(detail.kind),
                )
                .child(
                    div()
                        .id("search-record-title")
                        .role(Role::Heading)
                        .aria_label(detail.title.clone())
                        .font_family(theme::HEADING_FONT_FAMILY)
                        .text_size(px(16.0))
                        .text_color(theme::rambutan_100())
                        .child(detail.title),
                )
                .child(
                    web_button(
                        "close-search-record",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .debug_selector(|| "desktop-close-search-record".into())
                    .aria_label("Back to overview")
                    .child("Back to overview")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.dismiss_search_record(window, cx)),
                    ),
                )
                .children(
                    detail
                        .fields
                        .into_iter()
                        .enumerate()
                        .map(|(index, (label, value))| {
                            div()
                                .id(("search-record-field", index))
                                .role(Role::Group)
                                .aria_label(format!("{label}: {value}"))
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(theme::rambutan_500())
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(theme::rambutan_200())
                                        .child(value),
                                )
                        }),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(theme::rambutan_500())
                        .child(format!("Record ID · {}", selected.id())),
                )
                .into_any_element(),
        )
    }
}
