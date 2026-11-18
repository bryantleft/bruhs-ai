//! The new / edit event dialog: its form state, color select, all-day toggle and save.

use super::*;
use bruhs_ui::press::on_press_view;
use gpui_component::zoom::px;
pub(super) const EVENT_COLOR_OPTIONS: &[(&str, &str)] = &[
    ("persimmon", "Persimmon"),
    ("blueberry", "Blueberry"),
    ("pandan", "Pandan"),
    ("lychee", "Lychee"),
    ("durian", "Durian"),
    ("mangosteen", "Mangosteen"),
    ("dragonfruit", "Dragonfruit"),
];
pub(super) const EVENT_TITLE_PLACEHOLDER: &str = "What's happening?";
pub(super) const EVENT_DATETIME_PLACEHOLDER: &str = "";
pub(super) const EVENT_NOTES_PLACEHOLDER: &str = "Add details (optional)";
pub(super) const EVENT_NOTES_ROWS: usize = 3;
pub(super) const EVENT_EDIT_LABEL: &str = "Edit event";
pub(super) const EVENT_READ_ONLY_LABEL: &str = "Google event (read only)";
pub(super) const EVENT_PROVIDER_LINK_LABEL: &str = "Open in Google Calendar";
pub(super) const EVENT_OPEN_TASK_LABEL: &str = "Open task";
pub(super) const EVENT_RECORD_MEETING_LABEL: &str = "Record meeting";
/// Shown for a provider event without its own color: it uses its
/// calendar's color, not one of the local choices.
pub(super) const EVENT_PROVIDER_DEFAULT_COLOR_LABEL: &str = "Calendar color";
/// Web `disabled:opacity-60` on fields and `disabled:opacity-50` on the
/// all-day checkbox.
pub(super) const EVENT_FIELD_DISABLED_OPACITY: f32 = 0.6;
pub(super) const EVENT_CHECK_DISABLED_OPACITY: f32 = 0.5;
pub(super) const EVENT_CREATE_SUBMIT_LABEL: &str = "Create";
pub(super) const EVENT_SAVE_SUBMIT_LABEL: &str = "Save";
pub(super) const EVENT_DELETE_LABEL: &str = "Delete";
pub(super) const EVENT_CANCEL_LABEL: &str = "Cancel";
pub(super) const EVENT_DIALOG_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const EVENT_DIALOG_LABEL_FONT_FAMILY: &str = "Monaspace Argon";
pub(super) const EVENT_DIALOG_TITLE_TEXT_SIZE_PX: f32 = 13.0;
pub(super) const EVENT_FORM_LABEL_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const EVENT_CREATE_OVERLAY_OPACITY: f32 = 0.70;
pub(super) const EVENT_CREATE_HEADER_Y_PADDING_PX: f32 = 12.0;
pub(super) const EVENT_CREATE_BODY_Y_PADDING_PX: f32 = 12.0;
pub(super) const EVENT_CREATE_BODY_GAP_PX: f32 = 12.0;
pub(super) const EVENT_CREATE_FOOTER_Y_PADDING_PX: f32 = 12.0;
pub(super) const EVENT_CREATE_FOOTER_GAP_PX: f32 = 8.0;
pub(super) const EVENT_CREATE_FOOTER_BG_OPACITY: f32 = 0.40;
pub(super) const EVENT_CREATE_ACTION_GAP_PX: f32 = 8.0;
pub(super) const EVENT_CREATE_DELETE_X_PADDING_PX: f32 = 12.0;
pub(super) const EVENT_CREATE_DELETE_Y_PADDING_PX: f32 = 8.0;
pub(super) const EVENT_CREATE_DELETE_HOVER_BG_OPACITY: f32 = 0.10;
pub(super) const EVENT_COLOR_OPTION_X_PADDING_PX: f32 = 12.0;
pub(super) const EVENT_COLOR_OPTION_Y_PADDING_PX: f32 = 6.0;
pub(super) const EVENT_ALL_DAY_GAP_PX: f32 = 8.0;
pub(super) const EVENT_ALL_DAY_CHECK_SIZE_PX: f32 = 14.0;
pub(super) const EVENT_ALL_DAY_CHECK_RADIUS_PX: f32 = 4.0;
pub(super) const EVENT_ALL_DAY_CHECK_TEXT_SIZE_PX: f32 = 12.0;

impl Workspace {
    pub(super) fn set_event_create_form(
        &mut self,
        values: DesktopEventFormValues<'_>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.event_create_all_day = values.all_day;
        self.event_create_color = values.color.to_string();
        self.set_event_create_color_menu_open(false);
        self.date_picker = None;
        self.event_create_error = None;
        self.event_create_title_input.update(cx, |input, cx| {
            input.set_value(values.title.to_string(), window, cx);
        });
        self.event_create_start_input.update(cx, |input, cx| {
            input.set_value(format_event_form_datetime(values.start_at), window, cx);
        });
        self.event_create_end_input.update(cx, |input, cx| {
            input.set_value(format_event_form_datetime(values.end_at), window, cx);
        });
        self.event_create_notes_input.update(cx, |input, cx| {
            input.set_value(values.notes.unwrap_or_default().to_string(), window, cx);
        });
    }

    pub(super) fn focus_event_title_input(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let input = self.event_create_title_input.clone();
        cx.defer_in(window, move |_this, window, cx| {
            input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        });
    }

    pub(super) fn open_event_create(
        &mut self,
        start_at: Option<NaiveDateTime>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.active_view = AppView::Calendar;
        self.open_overlay(Overlay::EventCreate {
            color_menu_open: false,
        });
        self.selected_calendar_event_id = None;
        self.event_dialog_provider = None;
        let start_at = start_at.unwrap_or_else(|| default_event_start(Local::now().naive_local()));
        self.calendar_anchor =
            calendar_anchor_after_event_dialog_open(self.calendar_anchor, start_at);
        self.set_event_create_form(
            DesktopEventFormValues {
                title: "",
                start_at,
                end_at: start_at + Duration::hours(1),
                all_day: false,
                color: "persimmon",
                notes: None,
            },
            window,
            cx,
        );
        self.focus_event_title_input(window, cx);
        cx.notify();
    }

    pub(super) fn open_event_edit(
        &mut self,
        event_id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Local events first, then the provider events streamed by the hub,
        // which open read only with their provider link (CAL-06).
        let Some(event) = self
            .calendar_events
            .iter()
            .find(|event| event.id == event_id)
            .cloned()
            .or_else(|| {
                self.visible_calendar_events(cx)
                    .into_iter()
                    .find(|event| event.id == event_id)
            })
        else {
            return;
        };
        self.active_view = AppView::Calendar;
        self.selected_calendar_event_id = Some(event.id.clone());
        self.event_dialog_provider = event.provider.clone();
        self.open_overlay(Overlay::EventCreate {
            color_menu_open: false,
        });
        let default_color = if event.is_read_only() {
            ""
        } else {
            "persimmon"
        };
        self.set_event_create_form(
            DesktopEventFormValues {
                title: &event.title,
                start_at: event.start_at,
                end_at: event.end_at,
                all_day: event.all_day,
                color: event.color.as_deref().unwrap_or(default_color),
                notes: event.notes.as_deref(),
            },
            window,
            cx,
        );
        if event.is_read_only() {
            self.modal_start_focus_handle.focus(window, cx);
        } else {
            self.focus_event_title_input(window, cx);
        }
        cx.notify();
    }

    /// Whether the event dialog shows a read-only provider event.
    pub(super) fn event_dialog_read_only(&self) -> bool {
        self.event_dialog_provider.is_some()
    }

    /// The task a focus block was placed from (CAL-14).
    pub(super) fn event_source_task_id(&self, event_id: &str) -> Option<String> {
        self.work_items
            .iter()
            .find(|item| item.planning.has_time_block(event_id))
            .map(|item| item.id.clone())
    }

    /// Web "Record meeting": the same start as auto-record. The meeting
    /// takes the saved event's title and the event's id as its link key; a
    /// capture that does not start stops the meeting and says so.
    pub(super) fn record_meeting_for_event(&mut self, event_id: &str, cx: &mut Context<'_, Self>) {
        let Some(event) = self
            .calendar_events
            .iter()
            .find(|event| event.id == event_id && !event.is_read_only())
            .cloned()
        else {
            return;
        };
        let Some(event) =
            today_ui::planning_calendar_events_in(vec![event], local_offset_minutes_east).pop()
        else {
            return;
        };
        self.meetings_hub
            .update(cx, |hub, cx| hub.start_recording_for_event(event, cx));
    }

    /// Closes the event dialog and opens its focus block's source task.
    pub(super) fn open_event_source_task(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_event_create(window, cx);
        self.open_calendar_task(task_id, window, cx);
    }

    pub(super) fn close_event_create(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        self.date_picker = None;
        self.selected_calendar_event_id = None;
        self.event_dialog_provider = None;
        self.event_create_error = None;
        self.event_create_title_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.event_create_start_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.event_create_end_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.event_create_notes_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.persist_and_notify(cx);
    }

    pub(super) fn toggle_event_color_menu(&mut self, cx: &mut Context<'_, Self>) {
        self.set_event_create_color_menu_open(!self.event_create_color_menu_open());
        cx.notify();
    }

    pub(super) fn set_event_color(&mut self, token: &'static str, cx: &mut Context<'_, Self>) {
        self.event_create_color = token.to_string();
        self.set_event_create_color_menu_open(false);
        cx.notify();
    }

    pub(super) fn save_event_from_form(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // Provider events are read only, even for a forged Enter or click.
        if self.event_dialog_read_only() {
            return;
        }
        let title = self.event_create_title_input.read(cx).value().to_string();
        let title = title.trim();
        if title.is_empty() {
            self.event_create_error = Some(REQUIRED_TITLE_ERROR.to_string());
            self.event_create_title_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
            cx.notify();
            return;
        }
        self.event_create_error = None;
        let (start_at, end_at) = event_form_times_from_inputs(
            &self.event_create_start_input.read(cx).value(),
            &self.event_create_end_input.read(cx).value(),
            self.event_create_all_day,
            Local::now().naive_local(),
        );
        let notes = self.event_create_notes_input.read(cx).value().to_string();
        let notes = if notes.trim().is_empty() {
            None
        } else {
            Some(notes)
        };
        let color = Some(self.event_create_color.clone());

        if let Some(editing_id) = self.selected_calendar_event_id.clone() {
            if let Some(event) = self
                .calendar_events
                .iter_mut()
                .find(|event| event.id == editing_id)
            {
                event.title = title.to_string();
                event.start_at = start_at;
                event.end_at = end_at;
                event.all_day = self.event_create_all_day;
                event.color = color;
                event.notes = notes;
            }
        } else {
            self.calendar_events.push(DesktopCalendarEvent {
                id: uuid::Uuid::new_v4().to_string(),
                title: title.to_string(),
                start_at,
                end_at,
                all_day: self.event_create_all_day,
                color,
                notes,
                ..Default::default()
            });
        }
        self.close_event_create(window, cx);
    }

    pub(super) fn event_create_field_label(&self, label: &'static str) -> Div {
        div()
            .font_family(EVENT_DIALOG_LABEL_FONT_FAMILY)
            .text_size(px(EVENT_FORM_LABEL_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .child(label)
    }

    pub(super) fn event_color_label(&self) -> &'static str {
        EVENT_COLOR_OPTIONS
            .iter()
            .find_map(|(token, label)| {
                if *token == self.event_create_color {
                    Some(*label)
                } else {
                    None
                }
            })
            .unwrap_or(if self.event_create_color.is_empty() {
                EVENT_PROVIDER_DEFAULT_COLOR_LABEL
            } else {
                "Persimmon"
            })
    }

    pub(super) fn render_event_all_day_toggle(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("event-create-all-day")
            .debug_selector(|| "desktop-event-create-all-day".to_string())
            .flex()
            .items_center()
            .gap(px(EVENT_ALL_DAY_GAP_PX))
            .font_family(EVENT_DIALOG_LABEL_FONT_FAMILY)
            .text_size(px(EVENT_FORM_LABEL_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_300())
            .role(Role::CheckBox)
            .aria_label("All day")
            .aria_toggled(if self.event_create_all_day {
                Toggled::True
            } else {
                Toggled::False
            })
            .border_1()
            .border_color(gpui::transparent_black())
            .when(self.event_dialog_read_only(), |el| {
                el.opacity(EVENT_CHECK_DISABLED_OPACITY)
            })
            .when(!self.event_dialog_read_only(), |el| {
                el.tab_index(0)
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    // Space toggles on key down; gpui's keyboard click
                    // would toggle it back.
                    .on_click(cx.listener(|this, ev: &ClickEvent, _window, cx| {
                        if !ev.is_keyboard() {
                            this.event_create_all_day = !this.event_create_all_day;
                            cx.notify();
                        }
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if is_web_checkbox_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.event_create_all_day = !this.event_create_all_day;
                            cx.notify();
                        }
                    }))
            })
            .child(
                div()
                    .size(px(EVENT_ALL_DAY_CHECK_SIZE_PX))
                    .flex_shrink_0()
                    .rounded(px(EVENT_ALL_DAY_CHECK_RADIUS_PX))
                    .border_1()
                    .border_color(if self.event_create_all_day {
                        theme::persimmon_500()
                    } else {
                        theme::rambutan_500()
                    })
                    .bg(if self.event_create_all_day {
                        theme::persimmon_500()
                    } else {
                        theme::longan_900()
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(EVENT_ALL_DAY_CHECK_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::longan_950())
                            .child(if self.event_create_all_day { "✓" } else { "" }),
                    ),
            )
            .child("All day")
            .into_any_element()
    }

    pub(super) fn render_event_color_select(&self, cx: &Context<'_, Self>) -> Div {
        let rows = EVENT_COLOR_OPTIONS
            .iter()
            .enumerate()
            .map(|(index, (token, label))| {
                let token = *token;
                let label = *label;
                let debug_token = token;
                let is_selected = self.event_create_color == token;
                div()
                    .id(("event-color-option", index))
                    .debug_selector(move || format!("desktop-event-color-option-{debug_token}"))
                    .w_full()
                    .flex()
                    .items_center()
                    .px(px(EVENT_COLOR_OPTION_X_PADDING_PX))
                    .py(px(EVENT_COLOR_OPTION_Y_PADDING_PX))
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(if is_selected {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(label)
                    .aria_selected(is_selected)
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .map(|el| {
                        on_press_view(el, cx, move |this, _window, cx| {
                            this.set_event_color(token, cx);
                        })
                    })
                    .child(div().min_w_0().truncate().child(label))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .relative()
            .w_full()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(FORM_FIELD_GAP_PX))
            .child(self.event_create_field_label("Color"))
            .child(
                div()
                    .id("event-color-select")
                    .debug_selector(|| "desktop-event-color-select".to_string())
                    .h(px(FORM_SELECT_HEIGHT_PX))
                    .w_full()
                    .rounded(px(FORM_CONTROL_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_card())
                    .bg(theme::control_surface())
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .font_family(FORM_SELECT_BODY_FONT_FAMILY)
                    .text_size(px(FORM_SELECT_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_200())
                    .role(Role::ComboBox)
                    .aria_label("Event color")
                    .aria_expanded(self.event_create_color_menu_open())
                    .when(self.event_dialog_read_only(), |el| {
                        el.opacity(EVENT_FIELD_DISABLED_OPACITY)
                    })
                    .when(!self.event_dialog_read_only(), |el| {
                        el.tab_index(0)
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| el.border_color(theme::ring_floating()))
                            .map(|el| {
                                on_press_view(el, cx, |this, _window, cx| {
                                    this.toggle_event_color_menu(cx);
                                })
                            })
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .when(self.event_create_color.is_empty(), |el| {
                                el.text_color(theme::rambutan_500())
                            })
                            .child(self.event_color_label()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::rambutan_500())
                            .child("⌄"),
                    ),
            )
            .when(self.event_create_color_menu_open(), |el| {
                el.child(
                    div()
                        .id("event-color-options")
                        .debug_selector(|| "desktop-event-color-options".into())
                        .role(Role::ListBox)
                        .aria_label("Event color")
                        .w_full()
                        .max_h(px(FORM_SELECT_MENU_MAX_HEIGHT_PX))
                        .overflow_y_scrollbar()
                        .rounded(px(FORM_CONTROL_RADIUS_PX))
                        .border_1()
                        .border_color(theme::ring_floating())
                        .bg(theme::longan_700())
                        .py(px(FORM_SELECT_MENU_Y_PADDING_PX))
                        .children(rows),
                )
            })
    }

    pub(super) fn render_event_create_dialog(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let editing = self.selected_calendar_event_id.is_some();
        let read_only = self.event_dialog_read_only();
        let dialog_label = if read_only {
            EVENT_READ_ONLY_LABEL
        } else if editing {
            EVENT_EDIT_LABEL
        } else {
            CALENDAR_NEW_EVENT_LABEL
        };
        let delete_id = self
            .selected_calendar_event_id
            .clone()
            .filter(|_| !read_only);
        let source_task_id = delete_id
            .as_deref()
            .and_then(|id| self.event_source_task_id(id));
        let record_id = delete_id.clone();
        let provider_link = self
            .event_dialog_provider
            .as_ref()
            .and_then(|provider| provider.html_link.clone())
            .filter(|link| is_provider_web_link(link));
        let dialog_height = modal_ui::form_dialog_height(window);
        let fields = div()
            .flex_shrink_0()
            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
            .py(px(EVENT_CREATE_BODY_Y_PADDING_PX))
            .flex()
            .flex_col()
            .gap(px(EVENT_CREATE_BODY_GAP_PX))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(FORM_FIELD_GAP_PX))
                    .child(self.event_create_field_label("Title"))
                    .child(
                        form_text_input(
                            Input::new(&self.event_create_title_input).disabled(read_only),
                        )
                        .aria_label("Event title"),
                    )
                    .when_some(self.event_create_error.clone(), |el, error| {
                        el.child(
                            div()
                                .id("event-create-title-error")
                                .debug_selector(|| "desktop-event-create-title-error".to_string())
                                .role(Role::Alert)
                                .aria_label(error.clone())
                                .font_family(EVENT_DIALOG_LABEL_FONT_FAMILY)
                                .text_size(px(EVENT_FORM_LABEL_TEXT_SIZE_PX))
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_color(theme::lychee_300())
                                .child(error),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(FORM_GRID_GAP_PX))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(FORM_FIELD_GAP_PX))
                            .child(self.event_create_field_label("Start"))
                            .child(self.render_date_field(
                                date_picker_ui::DatePickerTarget::EventStart,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(FORM_FIELD_GAP_PX))
                            .child(self.event_create_field_label("End"))
                            .child(
                                self.render_date_field(
                                    date_picker_ui::DatePickerTarget::EventEnd,
                                    cx,
                                ),
                            ),
                    ),
            )
            .child(self.render_event_all_day_toggle(cx))
            .child(self.render_event_color_select(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(FORM_FIELD_GAP_PX))
                    .child(self.event_create_field_label("Notes"))
                    .child(
                        form_textarea(
                            Textarea::new(&self.event_create_notes_input).disabled(read_only),
                            FORM_THREE_ROW_TEXTAREA_HEIGHT_PX,
                        )
                        .aria_label("Event notes"),
                    ),
            );

        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-event-create-overlay")
            .debug_selector(|| "desktop-event-create-overlay".to_string())
            .bg(theme::longan_950().opacity(EVENT_CREATE_OVERLAY_OPACITY))
            .flex()
            .items_center()
            .justify_center()
            .py(px(DESKTOP_DIALOG_EDGE_GAP_PX))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.close_transient_surfaces();
                cx.notify();
            }))
            .child(
                div()
                    .id("desktop-event-create-dialog")
                    .role(Role::Dialog)
                    .aria_label(dialog_label)
                    .child(self.blur_scrim.backdrop())
                    .debug_selector(|| "desktop-event-create-dialog".to_string())
                    .w(px(DESKTOP_DIALOG_MD_WIDTH_PX))
                    .max_w(relative(DESKTOP_DIALOG_MAX_WIDTH_RATIO))
                    .max_h(px(dialog_height))
                    .overflow_hidden()
                    .rounded(px(DESKTOP_DIALOG_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_modal())
                    .bg(theme::longan_800())
                    .text_color(cx.theme().popover_foreground)
                    .font_family(EVENT_DIALOG_BODY_FONT_FAMILY)
                    .flex()
                    .flex_col()
                    .track_focus(&self.modal_start_focus_handle)
                    .tab_group()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.trap_modal_tab_focus(event, window, cx);
                    }))
                    .on_click(cx.listener(|_this, _ev, _window, cx| {
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                            .py(px(EVENT_CREATE_HEADER_Y_PADDING_PX))
                            .border_b_1()
                            .border_color(theme::longan_800())
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(EVENT_DIALOG_TITLE_TEXT_SIZE_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_100())
                                    .child(dialog_label),
                            )
                            .child(
                                web_button(
                                    "event-create-close",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::IconSmall,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-event-create-close".to_string())
                                .aria_label("Close event dialog")
                                .child(
                                    svg()
                                        .path(assets::CLOSE_ICON_PATH)
                                        .size(px(14.0))
                                        .text_color(theme::rambutan_400()),
                                )
                                .map(|el| {
                                    on_press_view(el, cx, |this, window, cx| {
                                        this.close_event_create(window, cx);
                                    })
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id("event-create-body")
                            .debug_selector(|| "desktop-event-create-body".to_string())
                            .relative()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.modal_scroll)
                            .child(fields)
                            .vertical_scrollbar(&self.modal_scroll),
                    )
                    .child(
                        div()
                            .debug_selector(|| "desktop-event-create-footer".to_string())
                            .flex_shrink_0()
                            .border_t_1()
                            .border_color(theme::longan_800())
                            .bg(theme::longan_900().opacity(EVENT_CREATE_FOOTER_BG_OPACITY))
                            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                            .py(px(EVENT_CREATE_FOOTER_Y_PADDING_PX))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(EVENT_CREATE_FOOTER_GAP_PX))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(EVENT_CREATE_ACTION_GAP_PX))
                                    .when_some(provider_link, |el, link| {
                                        el.child(self.render_event_provider_link(link, cx))
                                    })
                                    .when_some(source_task_id, |el, task_id| {
                                        el.child(self.render_event_open_task(task_id, cx))
                                    })
                                    .when_some(record_id, |el, event_id| {
                                        el.child(self.render_event_record_meeting(event_id, cx))
                                    })
                                    .when_some(delete_id, |el, delete_id| {
                                        el.child(
                                            div()
                                                .id("event-create-delete")
                                                .debug_selector(|| {
                                                    "desktop-event-create-delete".to_string()
                                                })
                                                .rounded(px(FORM_CONTROL_RADIUS_PX))
                                                .px(px(EVENT_CREATE_DELETE_X_PADDING_PX))
                                                .py(px(EVENT_CREATE_DELETE_Y_PADDING_PX))
                                                .font_family(EVENT_DIALOG_LABEL_FONT_FAMILY)
                                                .text_size(px(EVENT_FORM_LABEL_TEXT_SIZE_PX))
                                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(theme::lychee_400())
                                                .tab_index(0)
                                                .role(Role::Button)
                                                .aria_label(EVENT_DELETE_LABEL)
                                                .border_1()
                                                .border_color(gpui::transparent_black())
                                                .focus_visible(|el| {
                                                    el.border_color(theme::persimmon_400())
                                                })
                                                .cursor_pointer()
                                                .hover(|el| {
                                                    el.bg(theme::lychee_500().opacity(
                                                        EVENT_CREATE_DELETE_HOVER_BG_OPACITY,
                                                    ))
                                                })
                                                .map(|el| {
                                                    on_press_view(
                                                        el,
                                                        cx,
                                                        move |this, window, cx| {
                                                            this.delete_calendar_event_by_id(
                                                                &delete_id, cx,
                                                            );
                                                            this.close_event_create(window, cx);
                                                        },
                                                    )
                                                })
                                                .child(EVENT_DELETE_LABEL),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap(px(EVENT_CREATE_ACTION_GAP_PX))
                                    .child(
                                        web_button(
                                            "event-create-cancel",
                                            WebButtonVariant::Ghost,
                                            WebButtonSize::Default,
                                            ButtonState::Enabled,
                                        )
                                        .aria_label(EVENT_CANCEL_LABEL)
                                        .debug_selector(|| {
                                            "desktop-event-create-cancel".to_string()
                                        })
                                        .child(EVENT_CANCEL_LABEL)
                                        .tab_index(0)
                                        .role(Role::Button)
                                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                        .map(|el| {
                                            on_press_view(el, cx, |this, window, cx| {
                                                this.close_event_create(window, cx)
                                            })
                                        }),
                                    )
                                    .child(
                                        // A read-only provider event has
                                        // nothing to save: neutral, disabled.
                                        web_button(
                                            "event-create-submit",
                                            if read_only {
                                                WebButtonVariant::Outline
                                            } else {
                                                WebButtonVariant::Primary
                                            },
                                            WebButtonSize::Default,
                                            ButtonState::disabled_if(read_only),
                                        )
                                        .aria_label(if editing {
                                            EVENT_SAVE_SUBMIT_LABEL
                                        } else {
                                            EVENT_CREATE_SUBMIT_LABEL
                                        })
                                        .debug_selector(|| {
                                            "desktop-event-create-submit".to_string()
                                        })
                                        .child(if editing {
                                            EVENT_SAVE_SUBMIT_LABEL
                                        } else {
                                            EVENT_CREATE_SUBMIT_LABEL
                                        })
                                        .map(|el| {
                                            on_press_view(el, cx, |this, window, cx| {
                                                this.save_event_from_form(window, cx)
                                            })
                                        }),
                                    ),
                            ),
                    )
                    .child(self.render_modal_focus_sentinel()),
            )
            .into_any_element()
    }
}

/// The new / edit event dialog fields.
pub(super) struct EventFormInputs {
    pub(super) event_create_title_input: Entity<InputState>,
    pub(super) event_create_start_input: Entity<InputState>,
    pub(super) event_create_end_input: Entity<InputState>,
    pub(super) event_create_notes_input: Entity<TextareaState>,
}

impl Workspace {
    /// The new / edit event dialog fields; Enter in the title saves.
    pub(super) fn new_event_form_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> EventFormInputs {
        let event_create_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(EVENT_TITLE_PLACEHOLDER));
        let event_create_start_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(EVENT_DATETIME_PLACEHOLDER));
        let event_create_end_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(EVENT_DATETIME_PLACEHOLDER));
        let event_create_notes_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(EVENT_NOTES_PLACEHOLDER)
                .rows(EVENT_NOTES_ROWS)
        });
        subscriptions.push(cx.subscribe_in(
            &event_create_title_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.event_create_error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { secondary, .. } if !*secondary => {
                    this.save_event_from_form(window, cx);
                }
                _ => {}
            },
        ));
        subscriptions.push(redraw_on_edit(&event_create_start_input, window, cx));
        subscriptions.push(redraw_on_edit(&event_create_end_input, window, cx));
        subscriptions.push(redraw_on_edit(&event_create_notes_input, window, cx));
        EventFormInputs {
            event_create_title_input,
            event_create_start_input,
            event_create_end_input,
            event_create_notes_input,
        }
    }
}

/// Only a web link from the provider is opened (the web follows the same
/// rule through `markdown::safe_url`).
pub(super) fn is_provider_web_link(link: &str) -> bool {
    let lower = link.trim_start().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

impl Workspace {
    /// A footer action in the event dialog: blueberry text, as the web's
    /// "Open in Google Calendar" and "Open task" links.
    fn event_dialog_link_action(id: &'static str, label: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .debug_selector(move || format!("desktop-{id}"))
            .min_h(px(theme::APP_CONTROL_HEIGHT_PX))
            .flex()
            .items_center()
            .rounded(px(FORM_CONTROL_RADIUS_PX))
            .px(px(EVENT_CREATE_DELETE_X_PADDING_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_size(px(EVENT_FORM_LABEL_TEXT_SIZE_PX))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::blueberry_400())
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.bg(theme::blueberry_500().opacity(EVENT_CREATE_DELETE_HOVER_BG_OPACITY)))
            .child(label)
    }

    pub(super) fn render_event_provider_link(
        &self,
        link: String,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        Self::event_dialog_link_action("event-provider-link", EVENT_PROVIDER_LINK_LABEL)
            .map(|el| {
                on_press_view(el, cx, move |_this, _window, cx| {
                    cx.open_url(&link);
                })
            })
            .into_any_element()
    }

    /// Web "Record meeting": persimmon text beside Delete, for an event
    /// this device owns.
    pub(super) fn render_event_record_meeting(
        &self,
        event_id: String,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .id("event-record-meeting")
            .debug_selector(|| "desktop-event-record-meeting".to_string())
            .rounded(px(FORM_CONTROL_RADIUS_PX))
            .px(px(EVENT_CREATE_DELETE_X_PADDING_PX))
            .py(px(EVENT_CREATE_DELETE_Y_PADDING_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_size(px(EVENT_FORM_LABEL_TEXT_SIZE_PX))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::persimmon_400())
            .tab_index(0)
            .role(Role::Button)
            .aria_label(EVENT_RECORD_MEETING_LABEL)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.bg(theme::persimmon_500().opacity(EVENT_CREATE_DELETE_HOVER_BG_OPACITY)))
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.record_meeting_for_event(&event_id, cx);
            }))
            .child(EVENT_RECORD_MEETING_LABEL)
            .into_any_element()
    }

    pub(super) fn render_event_open_task(
        &self,
        task_id: String,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        Self::event_dialog_link_action("event-open-task", EVENT_OPEN_TASK_LABEL)
            .map(|el| {
                on_press_view(el, cx, move |this, window, cx| {
                    this.open_event_source_task(&task_id, window, cx);
                })
            })
            .into_any_element()
    }
}
