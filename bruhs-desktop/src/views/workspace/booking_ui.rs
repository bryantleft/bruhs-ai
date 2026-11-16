//! The local booking page for a saved scheduling link (web `BookingPage`):
//! the link's next free times, a name, and a time to book as a local event.
//! Times, labels, the event and the wording come from
//! `bruhs_models::logic::booking`, shared with web.

use gpui_component::input::{Input, InputState};

use super::calendar_ui::local_datetime_from_ms;
use super::*;
use crate::views::meetings_hub::local_offset_minutes_east;
use bruhs_models::logic::booking::{
    available_booking_slots, booking_confirmation, booking_duration_line, booking_event,
    booking_link, booking_slot_label, BOOKING_BACK, BOOKING_EYEBROW, BOOKING_NAME_PLACEHOLDER,
    BOOKING_NOT_SAVED, BOOKING_NO_SLOTS, BOOKING_SAVING, BOOKING_UNAVAILABLE_DETAIL,
    BOOKING_UNAVAILABLE_TITLE,
};

/// Web `px-4 py-10` around a `max-w-xl` card with `p-5`.
const BOOKING_PAGE_X_PADDING_PX: f32 = 16.0;
const BOOKING_PAGE_Y_PADDING_PX: f32 = 40.0;
const BOOKING_CARD_WIDTH_PX: f32 = 576.0;
const BOOKING_CARD_PADDING_PX: f32 = 20.0;
const BOOKING_CARD_BG_OPACITY: f32 = 0.65;
/// Web `mt-5` between the sections and `gap-2` between the times.
const BOOKING_SECTION_GAP_PX: f32 = 20.0;
const BOOKING_SLOT_GAP_PX: f32 = 8.0;
/// Web `text-h2`, `text-h3`, `text-body-sm` and `text-label-sm`.
const BOOKING_TITLE_PX: f32 = 20.0;
const BOOKING_UNAVAILABLE_TITLE_PX: f32 = 18.0;
const BOOKING_BODY_PX: f32 = 13.0;
const BOOKING_LABEL_PX: f32 = 12.0;
const BOOKING_RADIUS_PX: f32 = 4.0;
const BOOKING_SLOT_HOVER_OPACITY: f32 = 0.10;

/// The booking page's name field and the booking made on this visit.
pub(super) struct BookingUi {
    pub(super) name: Entity<InputState>,
    /// The page's slug and the booked time's label.
    pub(super) booked: Option<(String, String)>,
}

impl BookingUi {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, Workspace>) -> Self {
        Self {
            name: cx.new(|cx| InputState::new(window, cx).placeholder(BOOKING_NAME_PLACEHOLDER)),
            booked: None,
        }
    }
}

impl Workspace {
    /// Open the booking page for `slug` as a fresh visit, like web's
    /// `#/book/{slug}`: no name yet and no booking.
    pub(super) fn open_booking_page(
        &mut self,
        slug: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.booking.booked = None;
        let name = self.booking.name.clone();
        name.update(cx, |input, cx| input.set_value("", window, cx));
        self.close_transient_surfaces();
        self.active_view = AppView::Booking { slug };
        self.persist_and_notify(cx);
    }

    /// The times the page for `link` offers now.
    fn booking_slots(&self, link: &SchedulingLink, cx: &Context<'_, Self>) -> Vec<(i64, i64)> {
        available_booking_slots(
            &self.scheduling_preferences,
            link,
            &self.planning_calendar_events(cx),
            self.planning_now().timestamp_millis(),
            local_offset_minutes_east,
        )
    }

    /// Book `slot` on the link at `slug`: a local event widened by the
    /// link's buffers, saved with the workspace.
    pub(super) fn book_slot(&mut self, slug: &str, slot: (i64, i64), cx: &mut Context<'_, Self>) {
        let Some(link) = booking_link(&self.scheduling_preferences, slug).cloned() else {
            return;
        };
        let guest = self.booking.name.read(cx).value().to_string();
        let event = booking_event(
            &link,
            &guest,
            slot,
            uuid::Uuid::new_v4().to_string(),
            EpochMs::now().as_millis(),
        );
        let (Some(start_at), Some(end_at)) = (
            local_datetime_from_ms(event.start_at_ms),
            local_datetime_from_ms(event.end_at_ms),
        ) else {
            return;
        };
        self.calendar_events.push(DesktopCalendarEvent {
            id: event.id,
            title: event.title,
            start_at,
            end_at,
            all_day: false,
            color: event.color,
            notes: event.notes,
            ..Default::default()
        });
        self.booking.booked = Some((
            slug.to_string(),
            booking_slot_label(slot.0, local_offset_minutes_east),
        ));
        self.persist_and_notify(cx);
    }

    pub(super) fn render_booking_page(
        &self,
        slug: &str,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> Div {
        let link = booking_link(&self.scheduling_preferences, slug).cloned();
        let content = match link {
            None => div()
                .id("desktop-booking-unavailable")
                .debug_selector(|| "desktop-booking-unavailable".to_string())
                .role(Role::Status)
                .child(
                    div()
                        .font_family(theme::HEADING_FONT_FAMILY)
                        .text_size(px(BOOKING_UNAVAILABLE_TITLE_PX))
                        .text_color(theme::rambutan_100())
                        .child(BOOKING_UNAVAILABLE_TITLE),
                )
                .child(
                    div()
                        .mt_2()
                        .text_size(px(BOOKING_BODY_PX))
                        .text_color(theme::rambutan_500())
                        .child(BOOKING_UNAVAILABLE_DETAIL),
                )
                .into_any_element(),
            Some(link) => self.render_booking_link(slug, &link, compact, cx),
        };
        let page = div()
            .id("desktop-booking-page")
            .debug_selector(|| "desktop-booking-page".to_string())
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_y_scroll()
            .bg(theme::longan_950())
            .px(px(BOOKING_PAGE_X_PADDING_PX))
            .py(px(BOOKING_PAGE_Y_PADDING_PX))
            .flex()
            .justify_center()
            .items_start()
            .text_color(theme::rambutan_300())
            .child(
                div()
                    .w_full()
                    .max_w(px(BOOKING_CARD_WIDTH_PX))
                    .rounded(px(8.0))
                    .bg(theme::longan_900().opacity(BOOKING_CARD_BG_OPACITY))
                    .border_1()
                    .border_color(theme::longan_800())
                    .p(px(BOOKING_CARD_PADDING_PX))
                    .flex()
                    .flex_col()
                    .child(
                        div().mb(px(BOOKING_SECTION_GAP_PX)).flex().child(
                            div()
                                .id("desktop-booking-back")
                                .debug_selector(|| "desktop-booking-back".to_string())
                                .text_size(px(BOOKING_LABEL_PX))
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_color(theme::rambutan_500())
                                .border_1()
                                .border_color(gpui::transparent_black())
                                .rounded(px(BOOKING_RADIUS_PX))
                                .role(Role::Button)
                                .aria_label(BOOKING_BACK)
                                .tab_index(0)
                                .cursor_pointer()
                                .hover(|el| el.text_color(theme::rambutan_300()))
                                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                .on_click(cx.listener(|this, _ev, window, cx| {
                                    this.activate_primary_view(AppView::Calendar, window, cx);
                                }))
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        if is_web_button_activation_key(
                                            event.keystroke.key.as_str(),
                                        ) {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            this.activate_primary_view(
                                                AppView::Calendar,
                                                window,
                                                cx,
                                            );
                                        }
                                    },
                                ))
                                .child(BOOKING_BACK),
                        ),
                    )
                    .child(content),
            );
        div().flex_1().h_full().min_w_0().flex().child(page)
    }

    fn render_booking_link(
        &self,
        slug: &str,
        link: &SchedulingLink,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let booked = self
            .booking
            .booked
            .as_ref()
            .filter(|(booked_slug, _)| booked_slug == slug)
            .map(|(_, label)| label.clone());
        let after_name = match booked {
            Some(label) => div()
                .id("desktop-booking-status")
                .debug_selector(|| "desktop-booking-status".to_string())
                .role(Role::Status)
                .mt(px(BOOKING_SECTION_GAP_PX))
                .rounded(px(BOOKING_RADIUS_PX))
                .bg(theme::longan_800())
                .p_3()
                .text_size(px(BOOKING_BODY_PX))
                .text_color(theme::rambutan_300())
                .child(match self.save_status {
                    SaveStatus::Saving => BOOKING_SAVING.to_string(),
                    SaveStatus::Saved => booking_confirmation(&label),
                    SaveStatus::Failed(_)
                    | SaveStatus::Conflict
                    | SaveStatus::RecoveryRequired(_) => BOOKING_NOT_SAVED.to_string(),
                })
                .into_any_element(),
            None => {
                let slots = self.booking_slots(link, cx);
                let empty = slots.is_empty();
                div()
                    .child(
                        div()
                            .debug_selector(|| "desktop-booking-slots".to_string())
                            .mt(px(BOOKING_SECTION_GAP_PX))
                            .grid()
                            .grid_cols(if compact { 1 } else { 2 })
                            .gap(px(BOOKING_SLOT_GAP_PX))
                            .children(slots.into_iter().enumerate().map(|(index, slot)| {
                                self.render_booking_slot(slug, index, slot, cx)
                            })),
                    )
                    .when(empty, |el| {
                        el.child(
                            div()
                                .id("desktop-booking-no-slots")
                                .debug_selector(|| "desktop-booking-no-slots".to_string())
                                .role(Role::Status)
                                .mt(px(BOOKING_SECTION_GAP_PX))
                                .text_size(px(BOOKING_BODY_PX))
                                .text_color(theme::rambutan_500())
                                .child(BOOKING_NO_SLOTS),
                        )
                    })
                    .into_any_element()
            }
        };
        div()
            .child(
                div()
                    .text_size(px(BOOKING_LABEL_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::persimmon_400())
                    .child(BOOKING_EYEBROW.to_uppercase()),
            )
            .child(
                div()
                    .debug_selector(|| "desktop-booking-title".to_string())
                    .mt_2()
                    .font_family(theme::HEADING_FONT_FAMILY)
                    .text_size(px(BOOKING_TITLE_PX))
                    .text_color(theme::rambutan_100())
                    .child(link.title.clone()),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(BOOKING_BODY_PX))
                    .text_color(theme::rambutan_500())
                    .child(booking_duration_line(link)),
            )
            .child(
                div().mt(px(BOOKING_SECTION_GAP_PX)).child(
                    form_text_input(Input::new(&self.booking.name))
                        .aria_label(BOOKING_NAME_PLACEHOLDER),
                ),
            )
            .child(after_name)
            .into_any_element()
    }

    fn render_booking_slot(
        &self,
        slug: &str,
        index: usize,
        slot: (i64, i64),
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let label = booking_slot_label(slot.0, local_offset_minutes_east);
        let click_slug = slug.to_string();
        let key_slug = slug.to_string();
        div()
            .id(("desktop-booking-slot", index))
            .debug_selector(move || format!("desktop-booking-slot-{index}"))
            .rounded(px(BOOKING_RADIUS_PX))
            .bg(theme::longan_800())
            .border_1()
            .border_color(theme::longan_700())
            .px_3()
            .py_3()
            .text_size(px(BOOKING_BODY_PX))
            .text_color(theme::blueberry_300())
            .role(Role::Button)
            .aria_label(format!("Book {label}"))
            .tab_index(0)
            .cursor_pointer()
            .hover(|el| el.bg(theme::blueberry_500().opacity(BOOKING_SLOT_HOVER_OPACITY)))
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.book_slot(&click_slug, slot, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.book_slot(&key_slug, slot, cx);
                }
            }))
            .child(label)
            .into_any_element()
    }
}
