//! The new / edit event dialog: its form state, color select, all-day toggle and save.

use super::*;
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
        let Some(event) = self
            .calendar_events
            .iter()
            .find(|event| event.id == event_id)
            .cloned()
        else {
            return;
        };
        self.active_view = AppView::Calendar;
        self.selected_calendar_event_id = Some(event.id.clone());
        self.open_overlay(Overlay::EventCreate {
            color_menu_open: false,
        });
        self.set_event_create_form(
            DesktopEventFormValues {
                title: &event.title,
                start_at: event.start_at,
                end_at: event.end_at,
                all_day: event.all_day,
                color: event.color.as_deref().unwrap_or("persimmon"),
                notes: event.notes.as_deref(),
            },
            window,
            cx,
        );
        self.focus_event_title_input(window, cx);
        cx.notify();
    }

    pub(super) fn close_event_create(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        self.selected_calendar_event_id = None;
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
            .unwrap_or("Persimmon")
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
            .tab_index(0)
            .role(Role::CheckBox)
            .aria_label("All-day")
            .aria_toggled(if self.event_create_all_day {
                Toggled::True
            } else {
                Toggled::False
            })
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.event_create_all_day = !this.event_create_all_day;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if is_web_checkbox_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.event_create_all_day = !this.event_create_all_day;
                    cx.notify();
                }
            }))
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
            .child("All-day")
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
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.set_event_color(token, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.set_event_color(token, cx);
                        }
                    }))
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
                    .tab_index(0)
                    .role(Role::ComboBox)
                    .aria_label("Event color")
                    .aria_expanded(self.event_create_color_menu_open())
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.border_color(theme::ring_floating()))
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.toggle_event_color_menu(cx);
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle_event_color_menu(cx);
                        }
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
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
        let delete_id = self.selected_calendar_event_id.clone();
        let dialog_height = (f32::from(window.viewport_size().height)
            - DESKTOP_DIALOG_TOP_PADDING_PX
            - DESKTOP_DIALOG_BOTTOM_GAP_PX)
            .clamp(
                DESKTOP_DIALOG_MIN_AVAILABLE_HEIGHT_PX,
                FORM_DIALOG_PREFERRED_HEIGHT_PX,
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
            .items_start()
            .justify_center()
            .pt(px(DESKTOP_DIALOG_TOP_PADDING_PX))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.close_transient_surfaces();
                cx.notify();
            }))
            .child(
                div()
                    .id("desktop-event-create-dialog")
                    .role(Role::Dialog)
                    .aria_label(if editing {
                        EVENT_EDIT_LABEL
                    } else {
                        CALENDAR_NEW_EVENT_LABEL
                    })
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
                            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                            .py(px(EVENT_CREATE_HEADER_Y_PADDING_PX))
                            .border_b_1()
                            .border_color(theme::longan_800())
                            .child(
                                div()
                                    .text_size(px(EVENT_DIALOG_TITLE_TEXT_SIZE_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_100())
                                    .child(if editing {
                                        EVENT_EDIT_LABEL
                                    } else {
                                        CALENDAR_NEW_EVENT_LABEL
                                    }),
                            ),
                    )
                    .child(
                        div()
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
                                        form_text_input(Input::new(&self.event_create_title_input))
                                            .aria_label("Event title"),
                                    )
                                    .when_some(self.event_create_error.clone(), |el, error| {
                                        el.child(
                                            div()
                                                .id("event-create-title-error")
                                                .debug_selector(|| {
                                                    "desktop-event-create-title-error".to_string()
                                                })
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
                                            .child(self.render_date_field(
                                                date_picker_ui::DatePickerTarget::EventEnd,
                                                cx,
                                            )),
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
                                            Input::new(&self.event_create_notes_input),
                                            FORM_THREE_ROW_TEXTAREA_HEIGHT_PX,
                                        )
                                        .aria_label("Event notes"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(theme::longan_800())
                            .bg(theme::longan_900().opacity(EVENT_CREATE_FOOTER_BG_OPACITY))
                            .px(px(FORM_DIALOG_SECTION_X_PADDING_PX))
                            .py(px(EVENT_CREATE_FOOTER_Y_PADDING_PX))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(EVENT_CREATE_FOOTER_GAP_PX))
                            .child(div().when_some(delete_id, |el, delete_id| {
                                let delete_id_for_key = delete_id.clone();
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
                                        .focus_visible(|el| {
                                            el.border_2().border_color(theme::persimmon_400())
                                        })
                                        .cursor_pointer()
                                        .hover(|el| {
                                            el.bg(theme::lychee_500()
                                                .opacity(EVENT_CREATE_DELETE_HOVER_BG_OPACITY))
                                        })
                                        .on_click(cx.listener(move |this, _ev, window, cx| {
                                            this.delete_calendar_event_by_id(&delete_id, cx);
                                            this.close_event_create(window, cx);
                                        }))
                                        .on_key_down(cx.listener(
                                            move |this, event: &KeyDownEvent, window, cx| {
                                                if is_web_button_activation_key(
                                                    event.keystroke.key.as_str(),
                                                ) {
                                                    window.prevent_default();
                                                    cx.stop_propagation();
                                                    this.delete_calendar_event_by_id(
                                                        &delete_id_for_key,
                                                        cx,
                                                    );
                                                    this.close_event_create(window, cx);
                                                }
                                            },
                                        ))
                                        .child(EVENT_DELETE_LABEL),
                                )
                            }))
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
                                        .focus_visible(|el| {
                                            el.border_2().border_color(theme::persimmon_400())
                                        })
                                        .on_click(cx.listener(|this, _ev, window, cx| {
                                            this.close_event_create(window, cx);
                                        }))
                                        .on_key_down(
                                            cx.listener(
                                                |this, event: &KeyDownEvent, window, cx| {
                                                    if is_web_button_activation_key(
                                                        event.keystroke.key.as_str(),
                                                    ) {
                                                        window.prevent_default();
                                                        cx.stop_propagation();
                                                        this.close_event_create(window, cx);
                                                    }
                                                },
                                            ),
                                        ),
                                    )
                                    .child(
                                        web_button(
                                            "event-create-submit",
                                            WebButtonVariant::Primary,
                                            WebButtonSize::Default,
                                            ButtonState::Enabled,
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
                                        .on_click(cx.listener(|this, _ev, window, cx| {
                                            this.save_event_from_form(window, cx);
                                        }))
                                        .on_key_down(
                                            cx.listener(
                                                |this, event: &KeyDownEvent, window, cx| {
                                                    if is_web_button_activation_key(
                                                        event.keystroke.key.as_str(),
                                                    ) {
                                                        window.prevent_default();
                                                        cx.stop_propagation();
                                                        this.save_event_from_form(window, cx);
                                                    }
                                                },
                                            ),
                                        ),
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
    pub(super) event_create_notes_input: Entity<InputState>,
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
            InputState::new(window, cx)
                .placeholder(EVENT_NOTES_PLACEHOLDER)
                .multi_line(true)
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
