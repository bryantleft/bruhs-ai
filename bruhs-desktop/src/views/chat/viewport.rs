//! Variable-height messages keep geometry in the list and text in the thread.
use super::*;

impl ChatView {
    pub(super) fn sync_message_viewport(&self) {
        let count = self.messages_viewport.item_count();
        let len = self.messages.len();
        if count == 0 && len > 0 {
            self.messages_viewport
                .reset_with_uniform_height(len, px(CHAT_MESSAGE_ROW_HINT_PX));
        } else if len > count {
            self.messages_viewport.splice(count..count, len - count);
        } else if len < count {
            self.messages_viewport.splice(len..count, 0);
        }
        // A collapse or edit can shrink the row containing the scroll anchor.
        // Invalidate only the previously rendered range so its offset is clamped
        // to the new height without touching the rest of a long history.
        if let Some((start, end)) = self.rendered_messages.replace(None) {
            let end = end.min(len);
            if start < end {
                self.messages_viewport.remeasure_items(start..end);
            }
        }
    }

    pub(super) fn render_message_viewport(
        &self,
        current_message: Option<usize>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let state = self.messages_viewport.clone();
        let keyboard = state.clone();
        let begin_embeds = std::rc::Rc::clone(&self.rich_embeds);
        let finish_embeds = std::rc::Rc::clone(&self.rich_embeds);
        div()
            .id("message-list")
            .debug_selector(|| "desktop-chat-transcript".into())
            .role(Role::Group)
            .aria_label("Conversation transcript")
            .track_focus(&self.messages_focus)
            .tab_index(0)
            .border_1()
            .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
            .focus_visible(|element| element.border_color(theme::persimmon_400()))
            .relative()
            .size_full()
            .overflow_hidden()
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if !this.messages_focus.is_focused(window) {
                    return;
                }
                let modifiers = event.keystroke.modifiers;
                if modifiers.platform || modifiers.control || modifiers.alt || modifiers.shift {
                    return;
                }
                let height = keyboard.viewport_bounds().size.height;
                match event.keystroke.key.as_str() {
                    "home" => keyboard.scroll_to(ListOffset::default()),
                    "end" => keyboard.set_follow_mode(FollowMode::Tail),
                    "pageup" => keyboard.scroll_by(-height),
                    "pagedown" => keyboard.scroll_by(height),
                    "up" => keyboard.scroll_by(-px(timeline_ui::TRANSCRIPT_ROW_MIN_HEIGHT_PX)),
                    "down" => keyboard.scroll_by(px(timeline_ui::TRANSCRIPT_ROW_MIN_HEIGHT_PX)),
                    _ => return,
                }
                window.prevent_default();
                cx.stop_propagation();
                cx.notify();
            }))
            .vertical_scrollbar(&state)
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, _, _| begin_embeds.borrow_mut().begin_frame(),
                )
                .absolute()
                .size_full(),
            )
            .child(
                list(
                    state,
                    cx.processor(move |this, index, window, cx| {
                        let Some(message) = this.messages.get(index) else {
                            return div().into_any_element();
                        };
                        this.rendered_messages
                            .set(Some(match this.rendered_messages.get() {
                                Some((start, end)) => (start.min(index), end.max(index + 1)),
                                None => (index, index + 1),
                            }));
                        div()
                            .id(("message-list-row", index))
                            .w_full()
                            .pb(px(CHAT_MESSAGE_LIST_GAP_PX))
                            .child(
                                div()
                                    .w(px(this.transcript_column_width()))
                                    .mx_auto()
                                    .px(px(CHAT_MESSAGE_LIST_X_PADDING_PX))
                                    .when(current_message == Some(index), |element| {
                                        element
                                            .border_1()
                                            .border_color(theme::persimmon_400())
                                            .rounded(px(4.0))
                                    })
                                    .child(this.render_message(index, message, window, cx)),
                            )
                            .into_any_element()
                    }),
                )
                .size_full()
                .pt(px(CHAT_MESSAGE_LIST_Y_PADDING_PX))
                .pb(px(timeline_ui::TRANSCRIPT_BOTTOM_PADDING_PX)),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, _, _| finish_embeds.borrow_mut().finish_frame(),
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }
}
