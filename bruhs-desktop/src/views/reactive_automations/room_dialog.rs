//! New room and Room settings (web `crew/room_dialog.rs`): a room's name, its
//! members and its lead. Save sends `room_command`, so the dialog and the
//! service hold one set of rules; with a room, Delete room asks once inline
//! before it sends. The workspace draws the shell and footer; this module
//! owns the fields and the confirm.
use super::create_bruh::{hint, label, labelled, text_field, ERROR_FILL_OPACITY, FIELD_RADIUS_PX};
use super::*;
use crate::views::crew_face::bruh_face;
use crate::views::dropdown::{dropdown, DropdownGroup, DropdownOption, Listbox};
use bruhs_models::crew::room::{
    lead_choices, room_command, room_form_for, Room, RoomForm, RoomState, DELETE_ROOM_CONFIRM,
    DELETE_ROOM_NO, DELETE_ROOM_YES, NEW_ROOM, NO_BRUHS_YET, ROOM_LEAD, ROOM_MEMBERS,
    ROOM_SETTINGS,
};
use bruhs_models::crew::RAIL_FACE_PX;
use bruhs_ui::press::on_press_view;
use bruhs_ui::scroll::ScrollableElement;

/// The dialog is done: the service took what it sent, or nothing needed
/// sending. `open` is a room it made, to open now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactiveRoomDone {
    pub open: Option<String>,
}

/// Web `max-h-[30vh]` on the member list.
const MEMBERS_MAX_SHARE: f32 = 0.3;
/// Web `space-y-0.5` between member rows.
const MEMBER_GAP_PX: f32 = 2.0;
/// A row's transparent focus border, taken out of web `px-2 py-1`.
const MEMBER_BORDER_PX: f32 = 1.0;
const MEMBER_X_PADDING_PX: f32 = 8.0 - MEMBER_BORDER_PX;
const MEMBER_Y_PADDING_PX: f32 = 4.0 - MEMBER_BORDER_PX;
const MEMBER_RADIUS_GRAPE_PX: f32 = 8.0;
const CHECK_RADIUS_SEED_PX: f32 = 4.0;
/// Web `bg-white/5 inset-ring-white/15` on an unchecked box.
const CHECK_FILL_OPACITY: f32 = 0.05;
const CHECK_RING_OPACITY: f32 = 0.15;
const MEMBER_HOVER_OPACITY: f32 = 0.40;
const LEAD_DROPDOWN: &str = "desktop-room-lead";

/// What the dialog sent and is waiting on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    Create(String),
    Save,
    Delete,
}

/// The dialog's state while it is open.
pub(super) struct RoomDialog {
    /// The room being edited; `None` makes a new room.
    room_id: Option<String>,
    name: Entity<InputState>,
    /// Members and lead; the name lives in `name`.
    form: RoomForm,
    lead_open: bool,
    confirming: bool,
    invalid: Option<&'static str>,
    pending: Option<Pending>,
    members_scroll: ScrollHandle,
    delete_focus: FocusHandle,
    keep_focus: FocusHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

/// The dropdown takes `'static` values; each agent id is kept once for the
/// life of the app, so the set grows only with the crew.
fn lead_value(id: &str) -> &'static str {
    static VALUES: std::sync::OnceLock<std::sync::Mutex<HashSet<&'static str>>> =
        std::sync::OnceLock::new();
    if id.is_empty() {
        return "";
    }
    let mut values = VALUES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(value) = values.get(id) {
        return value;
    }
    let value: &'static str = Box::leak(id.to_owned().into_boxed_str());
    values.insert(value);
    value
}

impl ReactiveAutomations {
    /// Every room, as the service last reported them.
    pub(crate) fn rooms(&self) -> &[RoomState] {
        &self.crew.rooms
    }

    fn room(&self, id: &str) -> Option<&Room> {
        self.crew
            .rooms
            .iter()
            .map(|state| &state.room)
            .find(|room| room.id == id)
    }

    /// Open the dialog on a blank room, or on `room_id`'s settings.
    pub(crate) fn open_room_dialog(
        &mut self,
        room_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let existing = room_id.as_deref().and_then(|id| self.room(id)).cloned();
        let form = room_form_for(existing.as_ref(), &self.crew.bruhs);
        let name = cx.new(|cx| InputState::new(window, cx));
        name.update(cx, |input, cx| input.set_value(&form.name, window, cx));
        // Enter in the name saves, like the web form's submit.
        let subscriptions = vec![cx.subscribe_in(
            &name,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter {
                    secondary: false, ..
                } => this.submit_room(cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        )];
        self.room_dialog = Some(RoomDialog {
            room_id,
            name,
            form,
            lead_open: false,
            confirming: false,
            invalid: None,
            pending: None,
            members_scroll: ScrollHandle::new(),
            delete_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            keep_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            _subscriptions: subscriptions,
        });
        self.clear_room_error();
        cx.notify();
    }

    pub(crate) fn room_dialog_open(&self) -> bool {
        self.room_dialog.is_some()
    }

    /// The dialog edits a room, so it has Delete room.
    pub(crate) fn room_dialog_editing(&self) -> bool {
        self.room_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.room_id.is_some())
    }

    pub(crate) fn room_dialog_heading(&self) -> &'static str {
        if self.room_dialog_editing() {
            ROOM_SETTINGS
        } else {
            NEW_ROOM
        }
    }

    /// The inline delete confirm is showing.
    pub(crate) fn room_dialog_confirming(&self) -> bool {
        self.room_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.confirming)
    }

    /// Delete room's focus, so Keep can hand focus back to it.
    pub(crate) fn room_delete_focus(&self) -> Option<FocusHandle> {
        self.room_dialog
            .as_ref()
            .map(|dialog| dialog.delete_focus.clone())
    }

    /// Delete room's and Keep's focus.
    #[cfg(test)]
    pub(crate) fn room_focus_for_test(&self) -> Option<(FocusHandle, FocusHandle)> {
        self.room_dialog
            .as_ref()
            .map(|dialog| (dialog.delete_focus.clone(), dialog.keep_focus.clone()))
    }

    /// The Name field, which takes focus when the dialog opens.
    pub(crate) fn room_name_input(&self) -> Option<Entity<InputState>> {
        self.room_dialog.as_ref().map(|dialog| dialog.name.clone())
    }

    /// The Name field has focus.
    #[cfg(test)]
    pub(crate) fn room_name_focused_for_test(&self, window: &Window, cx: &App) -> bool {
        self.room_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.name.focus_handle(cx).is_focused(window))
    }

    /// Save and Delete room are off while a command is on its way or the
    /// service has no rooms.
    pub(crate) fn room_dialog_disabled(&self) -> bool {
        self.busy
            || !self.rooms_ready()
            || self
                .room_dialog
                .as_ref()
                .is_none_or(|dialog| dialog.pending.is_some())
    }

    /// Close the dialog. What it sent still applies, but nothing opens when
    /// the service answers.
    pub(crate) fn close_room_dialog(&mut self, cx: &mut Context<'_, Self>) {
        if self.room_dialog.take().is_some() {
            self.clear_room_error();
            cx.notify();
        }
    }

    /// Close the dialog and tell the workspace, opening `open` if given.
    fn finish_room_dialog(&mut self, open: Option<String>, cx: &mut Context<'_, Self>) {
        self.room_dialog = None;
        cx.emit(ReactiveRoomDone { open });
        cx.notify();
    }

    pub(crate) fn submit_room(&mut self, cx: &mut Context<'_, Self>) {
        if self.room_dialog_disabled() {
            return;
        }
        let Some(dialog) = self.room_dialog.as_mut() else {
            return;
        };
        let existing = dialog.room_id.as_deref().and_then(|id| {
            self.crew
                .rooms
                .iter()
                .map(|state| &state.room)
                .find(|room| room.id == id)
        });
        let id = match &dialog.room_id {
            // The room went while the dialog was open: nothing left to save.
            Some(_) if existing.is_none() => {
                self.finish_room_dialog(None, cx);
                return;
            }
            Some(id) => id.clone(),
            None => uuid::Uuid::new_v4().to_string(),
        };
        let form = RoomForm {
            name: dialog.name.read(cx).value().to_string(),
            ..dialog.form.clone()
        };
        let eligible: Vec<&str> = self
            .crew
            .bruhs
            .iter()
            .map(|bruh| bruh.agent_id.as_str())
            .collect();
        match room_command(existing, &id, &form, &eligible) {
            Err(reason) => {
                dialog.invalid = Some(reason);
                cx.notify();
            }
            Ok(None) => self.finish_room_dialog(None, cx),
            Ok(Some(command)) => {
                dialog.invalid = None;
                dialog.pending = Some(if existing.is_some() {
                    Pending::Save
                } else {
                    Pending::Create(id)
                });
                // A new try drops the last refusal's reason.
                self.clear_room_error();
                self.send_in(command, Scope::Room, cx);
                cx.notify();
            }
        }
    }

    fn toggle_room_member(&mut self, agent_id: &str, cx: &mut Context<'_, Self>) {
        if let Some(dialog) = self.room_dialog.as_mut() {
            dialog.form.toggle_member(agent_id);
            cx.notify();
        }
    }

    fn toggle_room_lead_list(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(dialog) = self.room_dialog.as_mut() {
            dialog.lead_open = !dialog.lead_open;
            cx.notify();
        }
    }

    fn choose_room_lead(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        if let Some(dialog) = self.room_dialog.as_mut() {
            dialog.form.lead = (!value.is_empty()).then(|| value.to_string());
            dialog.lead_open = false;
            cx.notify();
        }
    }

    /// Delete room: show the confirm, with focus on Keep, the safe choice.
    pub(crate) fn ask_delete_room(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(dialog) = self.room_dialog.as_mut() else {
            return;
        };
        if dialog.room_id.is_none() {
            return;
        }
        dialog.confirming = true;
        dialog.keep_focus.focus(window, cx);
        cx.notify();
    }

    /// Keep: drop the confirm, with focus back on Delete room.
    fn keep_room(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(dialog) = self.room_dialog.as_mut() {
            dialog.confirming = false;
            dialog.delete_focus.focus(window, cx);
            cx.notify();
        }
    }

    /// The confirm's Delete: sends once.
    fn delete_room(&mut self, cx: &mut Context<'_, Self>) {
        if self.room_dialog_disabled() {
            return;
        }
        let Some(dialog) = self.room_dialog.as_mut() else {
            return;
        };
        let Some(id) = dialog.room_id.clone() else {
            return;
        };
        dialog.pending = Some(Pending::Delete);
        self.send_in(ReactiveCommand::DeleteRoom { id }, Scope::Room, cx);
        cx.notify();
    }

    /// The service took a room command: close the dialog that sent it, and
    /// open a room it made.
    pub(super) fn room_done(&mut self, room_id: &str, cx: &mut Context<'_, Self>) {
        let Some(dialog) = self.room_dialog.as_ref() else {
            return;
        };
        let open = match &dialog.pending {
            // A room the service did not keep closes the dialog but opens
            // nothing.
            Some(Pending::Create(id)) if id == room_id => {
                Some(self.room(id).is_some().then(|| id.clone()))
            }
            Some(Pending::Save | Pending::Delete) if dialog.room_id.as_deref() == Some(room_id) => {
                Some(None)
            }
            _ => None,
        };
        if let Some(open) = open {
            self.finish_room_dialog(open, cx);
        }
    }

    /// The service refused: keep the dialog open with its reason.
    pub(super) fn room_failed(&mut self) {
        if let Some(dialog) = self.room_dialog.as_mut() {
            dialog.pending = None;
        }
    }

    fn clear_room_error(&mut self) {
        if self
            .scoped_error
            .as_ref()
            .is_some_and(|(scope, _)| *scope == Scope::Room)
        {
            self.scoped_error = None;
        }
    }

    /// One member's checkbox row: Space checks it, Enter saves.
    fn render_room_member(
        &self,
        bruh: &BruhState,
        checked: bool,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let agent_id = bruh.agent_id.clone();
        let key_id = agent_id.clone();
        let selector = format!("desktop-room-member-{agent_id}");
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .role(Role::CheckBox)
            .aria_label(bruh.name.clone())
            .aria_toggled(checked.into())
            .tab_index(0)
            .w_full()
            .min_w_0()
            .min_h(px(theme::CONTROL_PX))
            .px(px(MEMBER_X_PADDING_PX))
            .py(px(MEMBER_Y_PADDING_PX))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .rounded(px(MEMBER_RADIUS_GRAPE_PX))
            .border(px(MEMBER_BORDER_PX))
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .hover(|el| el.bg(theme::longan_700().opacity(MEMBER_HOVER_OPACITY)))
            .child(
                div()
                    .size(px(theme::CONTROL_XS_PX))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(CHECK_RADIUS_SEED_PX))
                    .map(|el| {
                        if checked {
                            // Blueberry marks a selection; persimmon stays the brand's.
                            el.bg(theme::blueberry_400()).child(
                                svg()
                                    .path(crate::assets::CHECK_ICON_PATH)
                                    .size(px(theme::ICON_INLINE_PX))
                                    .text_color(theme::longan_950()),
                            )
                        } else {
                            el.bg(gpui::white().opacity(CHECK_FILL_OPACITY))
                                .border_1()
                                .border_color(gpui::white().opacity(CHECK_RING_OPACITY))
                        }
                    }),
            )
            .child(bruh_face(
                SharedString::from(format!("room-member-{agent_id}")),
                RAIL_FACE_PX,
                bruh.look.clone(),
                bruh.status,
                cx,
            ))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_100())
                    .child(bruh.name.clone()),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.is_keyboard() {
                    this.toggle_room_member(&agent_id, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.submit_room(cx);
                    }
                    "space" => {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.toggle_room_member(&key_id, cx);
                    }
                    _ => {}
                }
            }))
    }

    /// The dialog's fields, in the web's order.
    pub(crate) fn render_room_dialog(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Option<Div> {
        let dialog = self.room_dialog.as_ref()?;
        let bruhs = &self.crew.bruhs;
        let members = if bruhs.is_empty() {
            hint(NO_BRUHS_YET).into_any_element()
        } else {
            div()
                .id("desktop-room-members")
                .debug_selector(|| "desktop-room-members".to_string())
                .role(Role::List)
                .aria_label(ROOM_MEMBERS)
                .max_h(window.viewport_size().height * MEMBERS_MAX_SHARE)
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&dialog.members_scroll)
                .flex()
                .flex_col()
                .gap(px(MEMBER_GAP_PX))
                .children(bruhs.iter().map(|bruh| {
                    let checked = dialog.form.members.contains(&bruh.agent_id);
                    self.render_room_member(bruh, checked, cx)
                }))
                .vertical_scrollbar(&dialog.members_scroll)
                .into_any_element()
        };
        let panel = cx.entity().downgrade();
        let on_toggle = {
            let panel = panel.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = panel.update(cx, |panel, cx| panel.toggle_room_lead_list(cx));
            })
        };
        let on_select = Rc::new(move |value: &'static str, _: &mut Window, cx: &mut App| {
            let _ = panel.update(cx, |panel, cx| panel.choose_room_lead(value, cx));
        });
        let lead = dropdown(
            LEAD_DROPDOWN,
            ROOM_LEAD,
            dialog.form.lead.as_deref().map_or("", lead_value),
            vec![DropdownGroup {
                label: None,
                mark: None,
                options: lead_choices(&dialog.form, bruhs)
                    .into_iter()
                    .map(|(id, name)| DropdownOption {
                        value: lead_value(&id),
                        label: name,
                        description: None,
                    })
                    .collect(),
            }],
            Listbox::open_if(dialog.lead_open),
            on_toggle,
            on_select,
        );
        let error = dialog.invalid.map(str::to_string).or_else(|| {
            self.scoped_error
                .as_ref()
                .filter(|(scope, _)| *scope == Scope::Room)
                .map(|(_, message)| message.clone())
        });
        let rooms_on = self.rooms_ready();
        Some(
            div()
                .debug_selector(|| "desktop-room-dialog".to_string())
                .w_full()
                .min_w_0()
                .px_4()
                .py_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(labelled(
                    "Name",
                    div()
                        .debug_selector(|| "desktop-room-name".to_string())
                        .child(text_field(&dialog.name, "Name", false)),
                ))
                .child(
                    div()
                        .min_w_0()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(label(ROOM_MEMBERS))
                        .child(members),
                )
                .child(labelled(ROOM_LEAD, lead))
                .when_some(error, |el, message| {
                    el.child(
                        div()
                            .id("desktop-room-error")
                            .debug_selector(|| "desktop-room-error".to_string())
                            .role(Role::Alert)
                            .p_3()
                            .rounded(px(FIELD_RADIUS_PX))
                            .bg(theme::lychee_500().opacity(ERROR_FILL_OPACITY))
                            .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                            .text_color(theme::lychee_200())
                            .child(message),
                    )
                })
                .when_some(self.rooms_reason(), |el, reason| {
                    el.child(hint(reason).text_color(theme::rambutan_400()))
                })
                .when(dialog.confirming, |el| {
                    // Delete stays on while the delete is on its way, so focus
                    // stays on it; `delete_room` sends only once.
                    let yes = web_button(
                        "desktop-room-delete-yes",
                        WebButtonVariant::Destructive,
                        WebButtonSize::Small,
                        ButtonState::disabled_if(!rooms_on),
                    )
                    .debug_selector(|| "desktop-room-delete-yes".to_string())
                    .aria_label(DELETE_ROOM_YES)
                    .child(DELETE_ROOM_YES);
                    let yes = if rooms_on {
                        on_press_view(
                            yes.focus_visible(|el| el.border_color(theme::persimmon_400())),
                            cx,
                            |this, _, cx| this.delete_room(cx),
                        )
                    } else {
                        yes
                    };
                    let keep = on_press_view(
                        web_button(
                            "desktop-room-delete-keep",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-room-delete-keep".to_string())
                        .aria_label(DELETE_ROOM_NO)
                        .track_focus(&dialog.keep_focus)
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .child(DELETE_ROOM_NO),
                        cx,
                        |this, window, cx| this.keep_room(window, cx),
                    );
                    el.child(
                        div()
                            .debug_selector(|| "desktop-room-delete-confirm".to_string())
                            .p_3()
                            .rounded(px(FIELD_RADIUS_PX))
                            .bg(theme::longan_950().opacity(CONFIRM_FILL_OPACITY))
                            .border_1()
                            .border_color(theme::longan_700().opacity(CONFIRM_RING_OPACITY))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_200())
                                    .child(DELETE_ROOM_CONFIRM),
                            )
                            .child(div().flex().items_center().gap_2().child(yes).child(keep)),
                    )
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::lead_value;

    #[test]
    fn a_lead_value_is_kept_once_per_id() {
        let first = lead_value("agent-1");
        let again = lead_value(&String::from("agent-1"));
        assert_eq!(first, "agent-1");
        assert!(std::ptr::eq(first, again), "the same id reuses its value");
        assert_eq!(lead_value(""), "");
    }
}
