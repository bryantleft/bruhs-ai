//! The room dialog, 1:1 with web `crew/room_dialog.rs`: New room and a
//! room's settings in the shared modal shell, opened from the crew rail or a
//! room's page.

use super::*;
use crate::views::web_button::on_press;
use bruhs_models::crew::room::{CREATE_ROOM, DELETE_ROOM, SAVE_ROOM};

impl Workspace {
    /// New room (`room_id` `None`, once the service takes rooms and the crew
    /// has a bruh) or a room's settings.
    pub(super) fn open_room_dialog(
        &mut self,
        room_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if room_id.is_none() && self.new_room_block(cx).is_some() {
            return;
        }
        // Focus goes back to the opener when the dialog closes in place.
        if !self.modal_start_focus_handle.contains_focused(window, cx) {
            self.palette_return_focus = window
                .focused(cx)
                .map(|focus| (self.active_view.clone(), focus));
        }
        self.reactive_automations
            .update(cx, |panel, cx| panel.open_room_dialog(room_id, window, cx));
        self.open_overlay(Overlay::RoomDialog);
        // Like the web Dialog, focus starts on Name, not the close button.
        match self.reactive_automations.read(cx).room_name_input() {
            Some(name) => cx.defer_in(window, move |_, window, cx| {
                name.update(cx, |input, cx| input.focus(window, cx));
            }),
            None => self.focus_first_modal_tab_stop(window, cx),
        }
        cx.notify();
    }

    /// The room dialog: the panel's fields in the shared modal shell, with
    /// Delete room on the left of the footer when editing.
    pub(super) fn render_room_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let (fields, heading, editing, confirming, disabled, delete_focus) =
            self.reactive_automations.update(cx, |panel, cx| {
                (
                    panel.render_room_dialog(window, cx),
                    panel.room_dialog_heading(),
                    panel.room_dialog_editing(),
                    panel.room_dialog_confirming(),
                    panel.room_dialog_disabled(),
                    panel.room_delete_focus(),
                )
            });
        let delete = (editing && !confirming).then(|| {
            let button = web_button(
                "desktop-room-delete",
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::disabled_if(disabled),
            )
            .debug_selector(|| "desktop-room-delete".to_string())
            .aria_label(DELETE_ROOM)
            .when_some(delete_focus, |el, focus| el.track_focus(&focus))
            .child(DELETE_ROOM);
            if disabled {
                button
            } else {
                on_press(
                    button
                        .tab_index(0)
                        .focus_visible(|el| el.border_color(theme::persimmon_400())),
                    cx,
                    |this, window, cx| {
                        this.reactive_automations
                            .update(cx, |panel, cx| panel.ask_delete_room(window, cx));
                    },
                )
            }
        });
        let cancel = on_press(
            web_button(
                "desktop-room-cancel",
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector(|| "desktop-room-cancel".to_string())
            .aria_label("Cancel")
            .tab_index(0)
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .child("Cancel"),
            cx,
            |this, _window, cx| {
                this.close_transient_surfaces();
                cx.notify();
            },
        );
        let label = if editing { SAVE_ROOM } else { CREATE_ROOM };
        let submit = web_button(
            "desktop-room-submit",
            WebButtonVariant::Primary,
            WebButtonSize::Small,
            ButtonState::disabled_if(disabled),
        )
        .debug_selector(|| "desktop-room-submit".to_string())
        .aria_label(label)
        .child(label);
        let submit = if disabled {
            submit
        } else {
            on_press(
                submit
                    .tab_index(0)
                    .focus_visible(|el| el.border_color(theme::persimmon_400())),
                cx,
                |this, _window, cx| {
                    this.reactive_automations
                        .update(cx, |panel, cx| panel.submit_room(cx));
                },
            )
        };
        let footer = div()
            .pt_3()
            .flex()
            .items_center()
            .gap_2()
            .children(delete)
            .child(div().flex_1())
            .child(cancel)
            .child(submit);
        self.render_modal_shell_sized(
            heading,
            fields.unwrap_or_else(div),
            Some(footer),
            DESKTOP_DIALOG_LG_WIDTH_PX,
            window,
            cx,
        )
    }

    /// A room dialog closed by Escape, the backdrop or another surface
    /// forgets what it sent, so a late answer never opens a room.
    pub(super) fn sync_room_dialog(&mut self, cx: &mut Context<'_, Self>) {
        if !matches!(self.overlay, Some(Overlay::RoomDialog))
            && self.reactive_automations.read(cx).room_dialog_open()
        {
            self.reactive_automations
                .update(cx, |panel, cx| panel.close_room_dialog(cx));
        }
    }

    /// The service took the room dialog's command: close it, and open the
    /// room it made.
    pub(super) fn room_dialog_done(
        &mut self,
        open: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match open {
            Some(room_id) => self.activate_sidebar_view(AppView::Room { room_id }, window, cx),
            None => {
                if matches!(self.overlay, Some(Overlay::RoomDialog)) {
                    self.close_transient_surfaces();
                }
                cx.notify();
            }
        }
    }
}
