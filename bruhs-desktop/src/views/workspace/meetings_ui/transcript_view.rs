//! Variable-height transcript rows. The list owns geometry, never transcript text.
use super::*;
use bruhs_models::num::ToIntSaturating;
use bruhs_models::TranscriptSegment;
use gpui_component::scroll::{Scrollbar, ScrollbarShow};

pub(super) const LIVE_ROW_OFFSET: usize = bruhs_models::MAX_TRANSCRIPT_SEGMENTS + 1;

pub(in crate::views::workspace) struct TranscriptViewport {
    meeting_id: String,
    pub(in crate::views::workspace) list: ListState,
    segments: usize,
    detail: bool,
    pinned: Option<usize>,
    pinned_record: Option<String>,
    header_focus: FocusHandle,
    footer_focus: FocusHandle,
}

impl TranscriptViewport {
    pub(super) fn prepare(
        slot: &std::cell::RefCell<Option<Self>>,
        meeting_id: &str,
        segments: usize,
        detail: bool,
        cx: &App,
    ) -> ListState {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|view| view.meeting_id != meeting_id)
        {
            *slot = None;
        }
        let view = slot.get_or_insert_with(|| {
            let list = ListState::new(
                segments + if detail { 2 } else { 0 },
                ListAlignment::Top,
                px(128.0),
            )
            .with_uniform_item_height(px(48.0));
            if !detail {
                list.set_follow_mode(FollowMode::Tail);
            }
            let header_focus = cx.focus_handle();
            let footer_focus = cx.focus_handle();
            if detail {
                list.splice_focusable(0..1, [Some(header_focus.clone())]);
                list.splice_focusable(segments + 1..segments + 2, [Some(footer_focus.clone())]);
            }
            Self {
                meeting_id: meeting_id.into(),
                list,
                segments,
                detail,
                pinned: None,
                pinned_record: None,
                header_focus,
                footer_focus,
            }
        });
        view.sync_len(segments);
        view.list.clone()
    }

    fn sync_len(&mut self, segments: usize) {
        let offset = usize::from(self.detail);
        if segments > self.segments {
            self.list.splice(
                self.segments + offset..self.segments + offset,
                segments - self.segments,
            );
        } else if segments < self.segments {
            self.list
                .splice(segments + offset..self.segments + offset, 0);
        }
        self.segments = segments;
    }

    pub(in crate::views::workspace) fn invalidate(
        &self,
        meeting_id: &str,
        range: Option<&std::ops::Range<usize>>,
    ) {
        if self.meeting_id != meeting_id {
            return;
        }
        let offset = usize::from(self.detail);
        let range = range.map_or(0..self.segments, Clone::clone);
        let start = range.start.min(self.segments) + offset;
        // A changed speaker also changes whether the next row needs a chip.
        let end = range.end.saturating_add(1).min(self.segments) + offset;
        if start < end {
            self.list.remeasure_items(start..end);
        }
    }

    pub(in crate::views::workspace) fn invalidate_footer(&self) {
        if self.detail {
            self.list
                .remeasure_items(self.segments + 1..self.segments + 2);
        }
    }

    pub(in crate::views::workspace) fn rebase_editor(
        &mut self,
        hub: &MeetingsHub,
        focus: &FocusHandle,
    ) {
        if let Some(record_id) = &self.pinned_record {
            let located = hub.segments.get(&self.meeting_id).and_then(|records| {
                records
                    .iter()
                    .position(|record| &record.id == record_id)
                    .map(|row| (row, records.len()))
            });
            if let Some((row, count)) = located {
                self.sync_len(count);
                self.pin(Some(row + usize::from(self.detail)), focus);
            }
        }
    }

    fn pin(&mut self, row: Option<usize>, focus: &FocusHandle) {
        if self.pinned == row {
            return;
        }
        let top = self.list.logical_scroll_top();
        let following = self.list.is_following_tail();
        if let Some(previous) = self
            .pinned
            .take()
            .filter(|index| *index < self.list.item_count())
        {
            self.list.splice_focusable(previous..previous + 1, [None]);
        }
        if let Some(row) = row.filter(|index| *index < self.list.item_count()) {
            self.list
                .splice_focusable(row..row + 1, [Some(focus.clone())]);
            self.pinned = Some(row);
        }
        if following && row.is_none() {
            self.list.set_follow_mode(FollowMode::Tail);
        } else {
            self.list.scroll_to(top);
        }
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn transcript_section_focus(
        &self,
        footer: bool,
    ) -> Option<FocusHandle> {
        self.meeting_transcript_view.borrow().as_ref().map(|view| {
            if footer {
                view.footer_focus.clone()
            } else {
                view.header_focus.clone()
            }
        })
    }
    pub(super) fn transcript_editor_is_at(&self, index: usize) -> bool {
        if index < LIVE_ROW_OFFSET {
            self.meeting_transcript_view
                .borrow()
                .as_ref()
                .is_some_and(|view| view.pinned == Some(index + 1))
        } else {
            self.live_transcript_view
                .borrow()
                .as_ref()
                .is_some_and(|view| view.pinned == Some(index - LIVE_ROW_OFFSET))
        }
    }

    pub(super) fn pin_transcript_editor(&self, group_index: Option<usize>, cx: &App) {
        let focus = self.speaker_rename_input.read(cx).focus_handle(cx);
        let hub = self.meetings_hub.read(cx);
        if let Some(view) = self.meeting_transcript_view.borrow_mut().as_mut() {
            view.pinned_record = group_index
                .filter(|index| *index < LIVE_ROW_OFFSET)
                .and_then(|index| hub.segments.get(&view.meeting_id)?.get(index))
                .map(|record| record.id.clone());
            view.pin(
                group_index
                    .filter(|index| *index < LIVE_ROW_OFFSET)
                    .map(|index| index + 1),
                &focus,
            );
        }
        if let Some(view) = self.live_transcript_view.borrow_mut().as_mut() {
            view.pinned_record = group_index
                .and_then(|index| index.checked_sub(LIVE_ROW_OFFSET))
                .and_then(|index| hub.segments.get(&view.meeting_id)?.get(index))
                .map(|record| record.id.clone());
            view.pin(
                group_index.and_then(|index| index.checked_sub(LIVE_ROW_OFFSET)),
                &focus,
            );
        }
    }

    fn transcript_key(
        &mut self,
        event: &KeyDownEvent,
        list: &ListState,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Never turn cursor movement inside a speaker-name draft into page navigation.
        if self
            .speaker_rename_input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "home" => list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.0),
            }),
            "end" => list.scroll_to_end(),
            "pageup" => list.scroll_by(-window.viewport_size().height * 0.7),
            "pagedown" => list.scroll_by(window.viewport_size().height * 0.7),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn selected_live_segment(&self, cx: &Context<'_, Self>) -> Option<TranscriptSegment> {
        let selected = self.live_chunk.as_deref()?;
        let hub = self.meetings_hub.read(cx);
        let meeting_id = hub.live_meeting_id.as_deref()?;
        hub.segments
            .get(meeting_id)?
            .iter()
            .find(|row| row.id == selected)
            .cloned()
    }

    fn toggle_live_chunk(&mut self, segment_id: String, cx: &mut Context<'_, Self>) {
        if self.live_chunk.as_deref() == Some(segment_id.as_str()) {
            self.live_chunk = None;
        } else {
            self.live_chunk = Some(segment_id);
        }
        cx.notify();
    }

    fn copy_live_chunk(&mut self, cx: &mut Context<'_, Self>) {
        let Some(segment) = self.selected_live_segment(cx) else {
            self.live_chunk = None;
            self.meetings_hub.update(cx, |hub, cx| {
                hub.show_notice("That part of the transcript is already gone.", true, cx);
            });
            cx.notify();
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(segment.text));
    }

    fn remove_live_chunk(&mut self, cx: &mut Context<'_, Self>) {
        let Some(segment) = self.selected_live_segment(cx) else {
            self.live_chunk = None;
            self.meetings_hub.update(cx, |hub, cx| {
                hub.show_notice("That part of the transcript is already gone.", true, cx);
            });
            cx.notify();
            return;
        };
        self.live_chunk = None;
        let meeting_id = segment.meeting_id.clone();
        let segment_id = segment.id;
        self.meetings_hub.update(cx, |hub, cx| {
            hub.delete_transcript_chunk(&meeting_id, &segment_id, cx);
        });
        cx.notify();
    }

    /// Escape clears the live chunk. Backspace or Delete removes it.
    /// The platform copy shortcut copies its words.
    fn handle_live_chunk_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self
            .speaker_rename_input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        if key == "escape" && self.live_chunk.is_some() {
            self.live_chunk = None;
            cx.stop_propagation();
            cx.notify();
            return true;
        }
        if self.live_chunk.is_none() {
            return false;
        }
        let plain = !modifiers.platform
            && !modifiers.shift
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.function;
        if plain && matches!(key, "backspace" | "delete") {
            self.remove_live_chunk(cx);
            cx.stop_propagation();
            return true;
        }
        if modifiers.platform && !modifiers.shift && key.eq_ignore_ascii_case("c") {
            self.copy_live_chunk(cx);
            cx.stop_propagation();
            return true;
        }
        false
    }

    pub(super) fn render_transcript_list(
        &self,
        meeting_id: &str,
        detail: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let count = self
            .meetings_hub
            .read(cx)
            .segments
            .get(meeting_id)
            .map_or(0, Vec::len);
        let slot = if detail {
            &self.meeting_transcript_view
        } else {
            &self.live_transcript_view
        };
        let state = TranscriptViewport::prepare(slot, meeting_id, count, detail, cx);
        let keyboard = state.clone();
        let scrollbar = Scrollbar::vertical(&state).scrollbar_show(ScrollbarShow::Hover);
        let id = meeting_id.to_owned();
        let rows = list(
            state,
            cx.processor(move |this, index, _, cx| {
                if detail && index == 0 {
                    return page_row(this.render_meeting_header(&id, cx));
                }
                let segment_index = index - usize::from(detail);
                if detail && segment_index == count {
                    return page_row(this.render_meeting_footer(&id, cx));
                }
                let row = this.render_transcript_row(&id, segment_index, detail, cx);
                if detail {
                    page_row(row)
                } else {
                    row
                }
            }),
        );
        let rows = if detail {
            rows.h_full()
        } else {
            rows.with_sizing_behavior(ListSizingBehavior::Infer)
                .max_h(px(LIVE_PANEL_MAX_HEIGHT_PX - 18.0))
        };
        div()
            .relative()
            .id(if detail {
                "desktop-transcript-viewport"
            } else {
                "desktop-live-transcript-viewport"
            })
            .debug_selector(move || {
                if detail {
                    "desktop-transcript-viewport"
                } else {
                    "desktop-live-transcript-viewport"
                }
                .into()
            })
            .tab_index(0)
            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
            .role(Role::List)
            .aria_label(if detail {
                "Meeting transcript and summary"
            } else {
                "Live transcript"
            })
            .when(detail, |el| el.h_full().flex_1().min_h_0())
            .on_key_down(cx.listener(move |this, event, window, cx| {
                if !detail && this.handle_live_chunk_key(event, window, cx) {
                    return;
                }
                this.transcript_key(event, &keyboard, window, cx)
            }))
            .child(rows)
            .child(scrollbar)
            .into_any_element()
    }

    fn render_transcript_row(
        &self,
        meeting_id: &str,
        index: usize,
        detail: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        #[cfg(test)]
        self.transcript_rendered_rows
            .set(self.transcript_rendered_rows.get() + 1);
        let hub = self.meetings_hub.read(cx);
        let Some(meeting) = hub.meeting(meeting_id) else {
            return div().into_any_element();
        };
        let Some(segments) = hub.segments.get(meeting_id) else {
            return div().into_any_element();
        };
        let Some(segment) = segments.get(index) else {
            return div().into_any_element();
        };
        let group_start = index == 0 || segments[index - 1].speaker_id != segment.speaker_id;
        let group_index = index + if detail { 0 } else { LIVE_ROW_OFFSET };
        let editing = self.transcript_editor_is_at(group_index)
            && self
                .editing_speaker
                .as_ref()
                .is_some_and(|speaker| speaker.meeting_id.as_str() == meeting_id);
        let mut row = div()
            .id((
                if detail {
                    "desktop-transcript-row"
                } else {
                    "desktop-live-transcript-row"
                },
                index,
            ))
            .debug_selector(move || {
                format!(
                    "{}-{index}",
                    if detail {
                        "desktop-transcript-row"
                    } else {
                        "desktop-live-transcript-row"
                    }
                )
            })
            .role(Role::ListItem)
            .w_full()
            .when(detail, |el| el.max_w(px(PAGE_MAX_WIDTH_PX)).px_6())
            .pt(px(if index == 0 {
                0.0
            } else if group_start {
                12.0
            } else if detail {
                4.0
            } else {
                2.0
            }))
            .flex()
            .flex_col()
            .gap_1();
        if group_start || editing {
            // Snapshot insertion/annotation may move the edited record into a
            // speaker run. Keep its existing draft mounted until commit/cancel.
            let speaker_id = if editing {
                self.editing_speaker
                    .as_ref()
                    .map(|speaker| speaker.speaker_id.as_str().to_string())
            } else {
                segment.speaker_id.clone()
            };
            let (text, ring) = speaker_colors(speaker_id.as_deref());
            row = row.child(div().flex().items_center().child(self.render_speaker_chip(
                meeting_id.into(),
                speaker_id.clone(),
                group_index,
                speaker_label(&meeting.speakers, speaker_id.as_deref()),
                text,
                ring,
                &hub.people,
                cx,
            )));
        }
        let selected = !detail && self.live_chunk.as_deref() == Some(segment.id.as_str());
        let mut words = div()
            .flex()
            .items_baseline()
            .pl_3()
            .font_family(BODY_FONT)
            .text_size(px(BODY_SM_PX))
            .line_height(relative(1.375))
            .text_color(if detail {
                theme::rambutan_200()
            } else {
                theme::rambutan_300()
            })
            .when(segment.partial, |el| {
                el.opacity(if detail { 0.5 } else { 0.6 })
            })
            .child(segment.text.clone());
        if detail {
            if let Some(confidence) = segment
                .speaker_confidence
                .filter(|confidence| *confidence < 0.85)
            {
                words = words.child(
                    label(
                        format!("{}%", (confidence * 100.0).to_u32_saturating()),
                        LABEL_SM_PX,
                        theme::rambutan_500(),
                    )
                    .ml_1(),
                );
            }
        }
        let words = if detail {
            words.into_any_element()
        } else {
            let segment_id = segment.id.clone();
            let toggle_id = segment_id.clone();
            words
                .id(("desktop-live-chunk", index))
                .role(Role::Button)
                .tab_index(0)
                .cursor_pointer()
                .rounded(px(6.0))
                .hover(|el| el.bg(theme::longan_800()))
                .when(selected, |el| el.bg(theme::longan_800()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_live_chunk(segment_id.clone(), cx);
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.toggle_live_chunk(toggle_id.clone(), cx);
                    }
                }))
                .into_any_element()
        };
        row = row.child(words);
        if selected {
            row = row.child(
                div()
                    .flex()
                    .gap_2()
                    .pl_3()
                    .child(
                        web_button(
                            format!("desktop-live-copy-{index}"),
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Copy")
                        .on_click(cx.listener(|this, _, _, cx| this.copy_live_chunk(cx)))
                        .on_key_down(cx.listener(
                            |this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.copy_live_chunk(cx);
                                }
                            },
                        )),
                    )
                    .child(
                        web_button(
                            format!("desktop-live-delete-{index}"),
                            WebButtonVariant::Destructive,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Delete")
                        .on_click(cx.listener(|this, _, _, cx| this.remove_live_chunk(cx)))
                        .on_key_down(cx.listener(
                            |this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.remove_live_chunk(cx);
                                }
                            },
                        )),
                    ),
            );
        }
        row.into_any_element()
    }
}

// GPUI lays a list item out as a root, where auto margins do not center a
// max-width child. The full-width wrapper supplies that containing layout.
fn page_row(content: AnyElement) -> AnyElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .child(content)
        .into_any_element()
}
