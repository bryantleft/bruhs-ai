//! The room composer: its draft and bruh chips, the `@` list of the
//! room's members, and posting.
use super::*;
use bruhs_ui::Sizable as _;

/// Web `rounded-[16px]` on the composer surface.
const COMPOSER_RADIUS_PX: f32 = 16.0;
/// Web `inset-ring-white/10`, and `focus-within:inset-ring-persimmon-400/40`.
const COMPOSER_RING_OPACITY: f32 = 0.10;
const COMPOSER_FOCUS_RING_OPACITY: f32 = 0.40;
/// Web `px-3 pt-2.5 pb-1` around the composer's text. The field is an
/// extra-small [`Textarea`], which keeps 4px of its own on each side (and
/// none above or below), so the wrapper adds the rest.
const COMPOSER_X_PADDING_PX: f32 = 12.0 - COMPOSER_FIELD_X_PADDING_PX;
const COMPOSER_FIELD_X_PADDING_PX: f32 = 4.0;
const COMPOSER_TOP_PADDING_PX: f32 = 10.0;
const COMPOSER_BOTTOM_PADDING_PX: f32 = 4.0;
/// Web `EDITOR_MIN_HEIGHT_PX` (52) for the text area, less the 1px border
/// above and below that the web's inset ring does not take.
const COMPOSER_FIELD_MIN_HEIGHT_PX: f32 = 52.0 - 2.0;
/// The composer grows from one line to eight, like the web textarea.
const COMPOSER_MIN_ROWS: usize = 1;
const COMPOSER_MAX_ROWS: usize = 8;
/// Web `max-h-72` on the `@` list and `inset-ring-white/15` around it.
const MENTIONS_MAX_PX: f32 = 288.0;
const MENTIONS_RING_OPACITY: f32 = 0.15;
/// Web `py-1.5` on an `@` row.
const MENTION_ROW_Y_PADDING_PX: f32 = 6.0;
/// Shown in the `@` list when no member matches.
const NO_MATCHES: &str = "No matches";

/// The page's composer: Enter posts, a change reads the `@` being typed,
/// and the `@` list's keys come before the field sees them.
pub(super) fn make_room_composer(
    page: &mut RoomPage,
    window: &mut Window,
    cx: &mut Context<'_, ReactiveAutomations>,
) {
    let composer = cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(COMPOSER_MIN_ROWS, COMPOSER_MAX_ROWS)
            .submit_on_enter(true)
            .placeholder("Message")
    });
    page._subscriptions.push(cx.subscribe_in(
        &composer,
        window,
        |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter {
                secondary: false,
                shift: false,
            } => this.post_room(window, cx),
            InputEvent::Change => this.room_draft_changed(cx),
            _ => {}
        },
    ));
    // The `@` list's keys, and Backspace over a chip, before the
    // field sees them.
    let panel = cx.weak_entity();
    page._subscriptions
        .push(cx.intercept_keystrokes(move |event, window, cx| {
            let keystroke = &event.keystroke;
            let modifiers = keystroke.modifiers;
            if modifiers.platform || modifiers.control || modifiers.alt {
                return;
            }
            let Some(panel) = panel.upgrade() else {
                return;
            };
            let key = keystroke.key.clone();
            let handled = panel.update(cx, |this, cx| {
                this.room_composer_key(&key, modifiers.shift, window, cx)
            });
            if handled {
                window.prevent_default();
                cx.stop_propagation();
            }
        }));
    page.composer = Some(composer);
}

impl ReactiveAutomations {
    /// The draft changed: keep the chips on their text and open the `@`
    /// list while an `@` is being typed.
    fn room_draft_changed(&mut self, cx: &mut Context<'_, Self>) {
        let Some(page) = self.room.as_ref() else {
            return;
        };
        let Some(composer) = page.composer.as_ref() else {
            return;
        };
        let (text, caret) = {
            let input = composer.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let room = self
            .room_state(&page.room_id)
            .map(|state| state.room.clone());
        let query = mention_query(&text, caret);
        let hits: Vec<String> = match (&room, &query) {
            (Some(room), Some((_, query))) => member_hits(room, &self.crew.bruhs, query)
                .into_iter()
                .map(|bruh| bruh.agent_id.clone())
                .collect(),
            _ => Vec::new(),
        };
        let Some(page) = self.room.as_mut() else {
            return;
        };
        if text != page.draft {
            page.chips = retarget(&page.chips, &page.draft, &text);
            page.draft.clone_from(&text);
        }
        if page
            .dismissed
            .as_ref()
            .is_some_and(|closed| *closed != text)
        {
            page.dismissed = None;
        }
        match query {
            Some((at, _)) if page.dismissed.is_none() => {
                if page.mention != Some(at) || page.hits != hits {
                    page.selected = 0;
                }
                page.mention = Some(at);
                page.hits = hits;
            }
            _ => page.close_mentions(),
        }
        cx.notify();
    }

    /// A key in the composer: the `@` list's Up, Down, Enter and Escape,
    /// and Backspace or Delete over a chip. True when it was used. Tab
    /// stays focus navigation, as in every `@` list on both apps.
    fn room_composer_key(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(page) = self.room.as_mut() else {
            return false;
        };
        let Some(composer) = page.composer.clone() else {
            return false;
        };
        if !composer.focus_handle(cx).is_focused(window) {
            return false;
        }
        if matches!(key, "backspace" | "delete") && !shift {
            let (text, range) = {
                let input = composer.read(cx);
                (input.value().to_string(), input.selected_range())
            };
            if range.start != range.end {
                return false;
            }
            let removed = if key == "backspace" {
                backspace_chip(&text, &page.chips, range.start)
            } else {
                delete_forward_chip(&text, &page.chips, range.start)
            };
            let Some((next, caret, chips)) = removed else {
                return false;
            };
            self.set_room_draft(next, chips, caret, window, cx);
            return true;
        }
        if page.mention.is_none() {
            return false;
        }
        let count = page.hits.len();
        match key {
            "escape" => {
                page.dismissed = Some(page.draft.clone());
                page.close_mentions();
            }
            "down" | "up" => {
                if count > 0 {
                    page.selected = if key == "down" {
                        (page.selected + 1) % count
                    } else {
                        page.selected.checked_sub(1).unwrap_or(count - 1)
                    };
                    page.hits_scroll.scroll_to_item(page.selected);
                }
            }
            "enter" if !shift && count > 0 => {
                let index = page.selected.min(count - 1);
                self.pick_room_member(index, window, cx);
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Put member `index` of the `@` list into the draft as a chip, over
    /// the `@` being typed.
    fn pick_room_member(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(page) = self.room.as_ref() else {
            return;
        };
        let (Some(composer), Some(at), Some(agent_id)) =
            (page.composer.as_ref(), page.mention, page.hits.get(index))
        else {
            return;
        };
        let Some(bruh) = self
            .crew
            .bruhs
            .iter()
            .find(|bruh| bruh.agent_id == *agent_id)
        else {
            return;
        };
        let (text, caret) = {
            let input = composer.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let (cleared, kept) = replace_range(&text, &page.chips, at, caret.max(at), "");
        let kind = ChipKind::Bruh {
            agent_id: bruh.agent_id.clone(),
            name: bruh.name.clone(),
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (next, caret, chips) = insert_chip(&cleared, &kept, at, id, kind);
        self.set_room_draft(next, chips, caret, window, cx);
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE=room-draft:<text>`: type `text` into the
    /// open room's composer, as the owner would, so the `@` list can be
    /// captured.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_room_draft(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(composer) = self.room.as_ref().and_then(|page| page.composer.clone()) else {
            return;
        };
        composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(text.len()..text.len(), cx);
            input.focus(window, cx);
        });
        self.room_draft_changed(cx);
    }

    /// Write the draft, its chips and the caret, and close the `@` list.
    pub(super) fn set_room_draft(
        &mut self,
        text: String,
        chips: Vec<ContextChip>,
        caret: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(page) = self.room.as_mut() else {
            return;
        };
        let Some(composer) = page.composer.clone() else {
            return;
        };
        page.chips = chips;
        page.draft.clone_from(&text);
        page.dismissed = None;
        page.close_mentions();
        let caret = caret.min(text.len());
        composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(caret..caret, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Post the draft, with its chips read out, and clear it.
    pub(super) fn post_room(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.room_composer_reason().is_some() {
            return;
        }
        let Some(page) = self.room.as_ref() else {
            return;
        };
        let Some(composer) = page.composer.as_ref() else {
            return;
        };
        let text = value(composer, cx);
        let Some(command) = room_post(&page.room_id, &text, &page.chips) else {
            return;
        };
        let posted = (text, page.chips.clone());
        self.send_all(vec![command], Scope::General, cx);
        if let Some(page) = self.room.as_mut() {
            page.posted = Some(posted);
        }
        self.set_room_draft(String::new(), Vec::new(), 0, window, cx);
    }

    pub(super) fn render_room_composer(
        &self,
        page: &RoomPage,
        room: &Room,
        layout: Layout,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let blocked = self.room_composer_reason();
        let hint = room_composer_hint(room).filter(|_| blocked.is_none());
        let empty = page
            .composer
            .as_ref()
            .is_none_or(|input| value(input, cx).trim().is_empty());
        let focused = page
            .composer
            .as_ref()
            .is_some_and(|input| input.focus_handle(cx).is_focused(window));
        let surface = div()
            .debug_selector(|| "desktop-room-composer".to_string())
            .relative()
            .min_w_0()
            .rounded(px(COMPOSER_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(if focused {
                theme::persimmon_400().opacity(COMPOSER_FOCUS_RING_OPACITY)
            } else {
                gpui::white().opacity(COMPOSER_RING_OPACITY)
            })
            .when_some(page.composer.as_ref(), |el, composer| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-room-input".to_string())
                        .min_h(px(COMPOSER_FIELD_MIN_HEIGHT_PX))
                        .px(px(COMPOSER_X_PADDING_PX))
                        .pt(px(COMPOSER_TOP_PADDING_PX))
                        .pb(px(COMPOSER_BOTTOM_PADDING_PX))
                        .child(
                            // The wrapper holds the web textarea's padding,
                            // less the extra-small field's own 4px a side.
                            Textarea::new(composer)
                                .xsmall()
                                .aria_label("Message")
                                .p_0()
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .disabled(blocked.is_some())
                                .font_family(theme::CODE_FONT_FAMILY)
                                .text_size(px(BODY_TEXT_PX)),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px_2()
                    .pb_2()
                    .child(page_button(
                        "desktop-room-send",
                        "Send",
                        WebButtonVariant::Primary,
                        blocked.is_some() || empty,
                        cx,
                        |this, window, cx| this.post_room(window, cx),
                    )),
            )
            .when_some(self.render_room_mentions(page, cx), |el, list| {
                el.child(list)
            });
        let column = layout
            .column()
            .gap_2()
            .when_some(self.error.clone(), |el, message| {
                el.child(alert("desktop-room-error", message))
            })
            .when_some(blocked.filter(|reason| !reason.is_empty()), |el, reason| {
                el.child(
                    small_text(reason, theme::rambutan_400())
                        .debug_selector(|| "desktop-room-blocked".to_string()),
                )
            })
            .child(surface)
            .when_some(hint, |el, hint| {
                el.child(
                    small_text(hint, theme::rambutan_500())
                        .debug_selector(|| "desktop-room-hint".to_string()),
                )
            });
        div()
            .flex_shrink_0()
            .flex()
            .justify_center()
            .border_t_1()
            .border_color(theme::longan_800().opacity(HAIRLINE_OPACITY))
            .child(column)
    }

    /// The `@` list above the composer: the members the `@` names, each
    /// with its face, the lead marked.
    fn render_room_mentions(&self, page: &RoomPage, cx: &Context<'_, Self>) -> Option<AnyElement> {
        page.mention?;
        let lead = self
            .room_state(&page.room_id)
            .and_then(|state| state.room.lead.clone());
        let list = div()
            .id("desktop-room-mentions")
            .debug_selector(|| "desktop-room-mentions".to_string())
            .role(Role::ListBox)
            .aria_label("Members")
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.0))
            .mb_1()
            .max_h(px(MENTIONS_MAX_PX))
            .overflow_y_scroll()
            .track_scroll(&page.hits_scroll)
            .py_1()
            .rounded(px(RADIUS_GRAPE_PX))
            .bg(theme::longan_700())
            .border_1()
            .border_color(gpui::white().opacity(MENTIONS_RING_OPACITY))
            .occlude();
        if page.hits.is_empty() {
            return Some(
                list.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_size(px(LABEL_TEXT_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(NO_MATCHES),
                )
                .into_any_element(),
            );
        }
        let rows = page
            .hits
            .iter()
            .enumerate()
            .filter_map(|(index, agent_id)| {
                let bruh = self
                    .crew
                    .bruhs
                    .iter()
                    .find(|bruh| bruh.agent_id == *agent_id)?;
                let chosen = index == page.selected;
                let selector = format!("desktop-room-mention-{agent_id}");
                let is_lead = lead.as_deref() == Some(agent_id.as_str());
                let sub = if is_lead { ROOM_LEAD } else { "" };
                let lead_selector = format!("desktop-room-mention-lead-{agent_id}");
                Some(
                    div()
                        .id(SharedString::from(selector.clone()))
                        .debug_selector(move || selector)
                        .role(Role::ListBoxOption)
                        .aria_label(bruh.name.clone())
                        .aria_selected(chosen)
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py(px(MENTION_ROW_Y_PADDING_PX))
                        .cursor_pointer()
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(SMALL_TEXT_PX))
                        .map(|el| {
                            if chosen {
                                el.bg(theme::longan_800()).text_color(theme::rambutan_100())
                            } else {
                                el.text_color(theme::rambutan_300())
                            }
                        })
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if let Some(page) = this.room.as_mut().filter(|_| *hovered) {
                                page.selected = index;
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.pick_room_member(index, window, cx);
                            }),
                        )
                        .child(div().flex_none().child(braille_face(
                            RAIL_FACE_PX,
                            &still_view(&bruh.look, bruh.status),
                        )))
                        .child(
                            div()
                                .flex_shrink_0()
                                .font_weight(FontWeight::MEDIUM)
                                .child(bruh.name.clone()),
                        )
                        .child(
                            small_text(sub, theme::rambutan_500())
                                .flex_1()
                                .truncate()
                                .when(is_lead, |el| el.debug_selector(move || lead_selector)),
                        ),
                )
            });
        Some(list.children(rows).into_any_element())
    }
}
