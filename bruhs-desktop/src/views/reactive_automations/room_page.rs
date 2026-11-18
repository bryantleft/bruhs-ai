//! A room's page (web `crew/room.rs`): its members, its transcript with
//! each bruh's face and the chat's question and approval cards, older
//! messages a page at a time, and a composer whose `@` lists only the
//! room's members. The workspace shows the panel for `AppView::Room` and
//! tells it which room; the panel owns the page's state.
use super::bruh_page::{
    alert, bubble, measure_width, page_button, small_text, Layout, BODY_TEXT_PX, HAIRLINE_OPACITY,
    LABEL_TEXT_PX, RADIUS_GRAPE_PX, SMALL_TEXT_PX, SOFT_RING_OPACITY, TITLE_TEXT_PX,
};
use super::create_bruh::ERROR_FILL_OPACITY;
use super::face_editor::still_view;
use super::*;
use crate::views::braille::braille_face;
use crate::views::crew_face::bruh_face;
use crate::views::web_button::on_press;
use bruhs_models::crew::room::{
    author_name, member_hits, member_states, members_line, mention_query, merge_messages,
    room_composer_block, room_composer_hint, room_follow_key, room_lines, room_post, working_line,
    Room, RoomLine, RoomMessage, RoomState, APPROVAL_DECIDED, APPROVAL_WANTS, EMPTY_ROOM,
    ROOM_GONE, ROOM_LEAD, ROOM_LOADING, ROOM_SETTINGS,
};
use bruhs_models::crew::{may_have_more, BruhLook, BruhStatus, RAIL_FACE_PX};
use bruhs_models::logic::context_chip::{
    backspace_chip, delete_forward_chip, insert_chip, replace_range, retarget, ChipKind,
    ContextChip,
};
use gpui_component::scroll::Scrollbar;
use std::cell::Cell;
use std::collections::hash_map::Entry;

/// Web `rounded-[16px]` on the composer surface.
const COMPOSER_RADIUS_PX: f32 = 16.0;
/// Web `inset-ring-white/10`, and `focus-within:inset-ring-persimmon-400/40`.
const COMPOSER_RING_OPACITY: f32 = 0.10;
const COMPOSER_FOCUS_RING_OPACITY: f32 = 0.40;
/// Web `px-3 pt-2.5 pb-1` around the composer's text.
const COMPOSER_X_PADDING_PX: f32 = 12.0;
const COMPOSER_TOP_PADDING_PX: f32 = 10.0;
const COMPOSER_BOTTOM_PADDING_PX: f32 = 4.0;
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

/// Room settings on a room's page: the workspace opens the room dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactiveOpenRoomSettings {
    pub room_id: String,
}

/// The room page while it is on screen.
pub(super) struct RoomPage {
    room_id: String,
    /// The messages read so far, in `seq` order.
    messages: Vec<RoomMessage>,
    /// Bumped whenever `messages` changes, so the lines are built again.
    messages_version: u64,
    /// The oldest message shown; Load earlier reads the page before it.
    oldest: Option<String>,
    more: bool,
    /// The newest page has been read once.
    loaded: bool,
    error: Option<String>,
    loading_earlier: bool,
    /// The room's `last_seq` the newest page was last read for.
    read_for: Option<u64>,
    newest_revision: u64,
    newest_request: Option<Task<()>>,
    earlier_request: Option<Task<()>>,
    composer: Option<Entity<InputState>>,
    /// The bruh chips in the draft, placed against `draft`.
    chips: Vec<ContextChip>,
    draft: String,
    /// The byte index of the `@` being typed, while the list is open.
    mention: Option<usize>,
    /// The draft Escape closed the list on; it opens again once the draft
    /// changes.
    dismissed: Option<String>,
    /// The members the `@` names, by agent id, and the one Enter picks.
    hits: Vec<String>,
    selected: usize,
    hits_scroll: ScrollHandle,
    /// Reply fields of open question cards, by outbox id.
    replies: HashMap<String, (Entity<InputState>, gpui::Subscription)>,
    /// The transcript, built when the snapshot or the messages change rather
    /// than on every frame (a sweeping face redraws the page often).
    lines: Rc<Vec<RoomLine>>,
    /// The snapshot revision and messages version `lines` was built from.
    built_for: Option<(u64, u64)>,
    scroll: ScrollHandle,
    /// What the transcript last followed; a new line scrolls into view.
    followed: (usize, Option<String>),
    width: Rc<Cell<f32>>,
    /// Room settings, so focus can leave it when the room is deleted.
    settings_focus: FocusHandle,
    /// "This room no longer exists.", which takes focus when the room goes.
    gone_focus: FocusHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

impl RoomPage {
    fn new(room_id: String, cx: &mut App) -> Self {
        Self {
            room_id,
            messages: Vec::new(),
            messages_version: 0,
            oldest: None,
            more: false,
            loaded: false,
            error: None,
            loading_earlier: false,
            read_for: None,
            newest_revision: 0,
            newest_request: None,
            earlier_request: None,
            composer: None,
            chips: Vec::new(),
            draft: String::new(),
            mention: None,
            dismissed: None,
            hits: Vec::new(),
            selected: 0,
            hits_scroll: ScrollHandle::new(),
            replies: HashMap::new(),
            lines: Rc::new(Vec::new()),
            built_for: None,
            scroll: ScrollHandle::new(),
            followed: (0, None),
            width: Rc::new(Cell::new(0.0)),
            settings_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            gone_focus: cx.focus_handle(),
            _subscriptions: Vec::new(),
        }
    }

    /// The newest page: the first one read sets where Load earlier starts.
    fn take_newest(&mut self, page: Vec<RoomMessage>) {
        if self.oldest.is_none() {
            self.more = may_have_more(page.len());
            self.oldest = page.last().map(|message| message.id.clone());
        }
        merge_messages(&mut self.messages, page);
        self.messages_version = self.messages_version.wrapping_add(1);
        self.error = None;
        self.loaded = true;
    }

    /// A page from before the oldest shown.
    fn take_earlier(&mut self, page: Vec<RoomMessage>) {
        self.more = may_have_more(page.len());
        if let Some(last) = page.last() {
            self.oldest = Some(last.id.clone());
        }
        merge_messages(&mut self.messages, page);
        self.messages_version = self.messages_version.wrapping_add(1);
    }

    fn close_mentions(&mut self) {
        self.mention = None;
        self.hits.clear();
        self.selected = 0;
    }
}

/// An open question card waits for a typed reply.
fn open_question(item: &OutboxItem) -> bool {
    item.kind == OutboxKind::Ask && item.approval.is_none() && item.resolved_at.is_none()
}

impl ReactiveAutomations {
    /// The room whose page is open.
    pub(crate) fn room_page_id(&self) -> Option<&str> {
        self.room.as_ref().map(|page| page.room_id.as_str())
    }

    /// Show `room_id`'s page, or none.
    pub(crate) fn show_room(&mut self, room_id: Option<String>, cx: &mut Context<'_, Self>) {
        if self.room_page_id() == room_id.as_deref() {
            return;
        }
        self.room = room_id.map(|room_id| RoomPage::new(room_id, cx));
        cx.notify();
    }

    fn room_state(&self, room_id: &str) -> Option<&RoomState> {
        self.crew
            .rooms
            .iter()
            .find(|state| state.room.id == room_id)
    }

    /// Why the open room's composer is off, or `None` while it posts.
    pub(crate) fn room_composer_reason(&self) -> Option<&'static str> {
        let page = self.room.as_ref()?;
        let members = self.room_state(&page.room_id).map_or(0, |state| {
            member_states(&state.room, &self.crew.bruhs).len()
        });
        room_composer_block(self.rooms_ready(), self.rooms_reason(), members)
    }

    /// The line beside a bruh working in the room.
    fn room_working_text(&self, agent_id: &str) -> String {
        working_line(&author_name(agent_id, &self.crew.bruhs))
    }

    /// Read the open room's newest messages again.
    fn reload_room_messages(&mut self, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let Some(page) = self.room.as_mut() else {
            return;
        };
        // Tests never reach a server.
        if cfg!(test) {
            return;
        }
        page.newest_revision = page.newest_revision.wrapping_add(1);
        let revision = page.newest_revision;
        let room_id = page.room_id.clone();
        let requested = room_id.clone();
        let operation = spawn_tokio(
            cx,
            async move { client.room_messages(&requested, None).await },
        );
        page.newest_request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                let Some(page) = this
                    .room
                    .as_mut()
                    .filter(|page| page.room_id == room_id && page.newest_revision == revision)
                else {
                    return;
                };
                match result {
                    Ok(messages) => page.take_newest(messages),
                    Err(error) => page.error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
    }

    /// Load earlier: the page of messages before the oldest shown.
    fn load_earlier_room_messages(&mut self, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let Some(page) = self.room.as_mut() else {
            return;
        };
        let Some(before) = page.oldest.clone() else {
            return;
        };
        if page.loading_earlier || cfg!(test) {
            return;
        }
        page.loading_earlier = true;
        let room_id = page.room_id.clone();
        let requested = room_id.clone();
        let operation = spawn_tokio(cx, async move {
            client.room_messages(&requested, Some(&before)).await
        });
        page.earlier_request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                let Some(page) = this.room.as_mut().filter(|page| page.room_id == room_id) else {
                    return;
                };
                page.loading_earlier = false;
                match result {
                    Ok(messages) => page.take_earlier(messages),
                    Err(error) => page.error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Whether "This room no longer exists." has focus.
    #[cfg(test)]
    pub(crate) fn room_gone_focused_for_test(&self, window: &Window) -> bool {
        self.room
            .as_ref()
            .is_some_and(|page| page.gone_focus.is_focused(window))
    }

    #[cfg(test)]
    pub(super) fn show_room_messages_for_test(&mut self, messages: Vec<RoomMessage>) {
        if let Some(page) = self.room.as_mut() {
            page.take_newest(messages);
        }
    }

    /// The page's inputs and data, made before it draws. The lines are
    /// built only when the snapshot or the messages changed since the last
    /// frame; a new message in the room reads the newest page again.
    pub(super) fn ensure_room_page(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(room_id) = self.room_page_id().map(str::to_string) else {
            return;
        };
        let last_seq = self.room_state(&room_id).map(|state| state.last_seq);
        let gone = self.snapshot.is_some() && last_seq.is_none();
        let dialog_open = self.room_dialog_open();
        let Some(page) = self.room.as_mut() else {
            return;
        };
        if page.composer.is_none() {
            let composer = cx.new(|cx| {
                InputState::new(window, cx)
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
        // A room deleted from its own settings takes the button focus went
        // back to; focus lands on the notice instead.
        if gone && !dialog_open {
            let stranded = window.focused(cx).is_none()
                || page.settings_focus.is_focused(window)
                || page
                    .composer
                    .as_ref()
                    .is_some_and(|input| input.focus_handle(cx).is_focused(window));
            if stranded {
                page.gone_focus.focus(window, cx);
            }
        }
        let reload = last_seq.is_some() && page.read_for != last_seq;
        if reload {
            page.read_for = last_seq;
        }
        let stamp = (self.snapshot_rev, page.messages_version);
        if page.built_for != Some(stamp) {
            page.built_for = Some(stamp);
            let (runs, outbox) = self
                .snapshot
                .as_ref()
                .map_or((&[][..], &[][..]), |snapshot| {
                    (snapshot.runs.as_slice(), snapshot.outbox.as_slice())
                });
            page.lines = Rc::new(room_lines(&page.messages, runs, outbox, &room_id));
            let open: Vec<String> = page
                .lines
                .iter()
                .filter_map(|line| match line {
                    RoomLine::Card { item, .. } if open_question(item) => Some(item.id.clone()),
                    _ => None,
                })
                .collect();
            page.replies.retain(|id, _| open.contains(id));
            for id in open {
                if let Entry::Vacant(slot) = page.replies.entry(id) {
                    let input = cx.new(|cx| InputState::new(window, cx).multi_line(true));
                    let watch =
                        cx.subscribe_in(&input, window, |_, _, event: &InputEvent, _, cx| {
                            if matches!(event, InputEvent::Change) {
                                cx.notify();
                            }
                        });
                    slot.insert((input, watch));
                }
            }
            let shown = room_follow_key(&page.lines);
            if shown != page.followed {
                page.scroll.scroll_to_bottom();
                page.followed = shown;
            }
        }
        if reload {
            self.reload_room_messages(cx);
        }
    }

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

    /// Write the draft, its chips and the caret, and close the `@` list.
    fn set_room_draft(
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
    fn post_room(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
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
        self.send_all(vec![command], Scope::General, cx);
        self.set_room_draft(String::new(), Vec::new(), 0, window, cx);
    }

    /// The room page: header, transcript and composer, or why it can't show.
    pub(crate) fn render_room_page(
        &self,
        room_id: &str,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let Some(page) = self.room.as_ref() else {
            return div();
        };
        let layout = Layout::measured(page.width.get(), window);
        let root = div()
            .debug_selector(|| "desktop-room-page".to_string())
            .relative()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme::longan_950())
            .text_color(theme::rambutan_300())
            .line_height(relative(1.5))
            .child(measure_width(page.width.clone(), cx));
        if self.snapshot.is_none() {
            let pending = match &self.load_error {
                Some(message) => div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_3()
                    .child(alert("desktop-room-load-error", message.clone()))
                    .child(page_button(
                        "desktop-room-retry",
                        "Retry",
                        WebButtonVariant::Outline,
                        self.reading,
                        cx,
                        |this, _, cx| this.refresh_snapshot(cx),
                    )),
                None => div().p_6().child(
                    div()
                        .id("desktop-room-loading")
                        .debug_selector(|| "desktop-room-loading".to_string())
                        .role(Role::Status)
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(SMALL_TEXT_PX))
                        .text_color(theme::rambutan_400())
                        .child(ROOM_LOADING),
                ),
            };
            return root.child(pending);
        }
        let Some(state) = self.room_state(room_id) else {
            return root.child(
                div().p_6().child(
                    div()
                        .id("desktop-room-gone")
                        .debug_selector(|| "desktop-room-gone".to_string())
                        .track_focus(&page.gone_focus)
                        .role(Role::Heading)
                        .font_family(theme::HEADING_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(TITLE_TEXT_PX))
                        .text_color(theme::rambutan_100())
                        .child(ROOM_GONE),
                ),
            );
        };
        root.child(self.render_room_header(page, &state.room, layout, cx))
            .child(self.render_room_transcript(page, layout, cx))
            .child(self.render_room_composer(page, &state.room, layout, window, cx))
    }

    fn render_room_header(
        &self,
        page: &RoomPage,
        room: &Room,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> Div {
        let members = member_states(room, &self.crew.bruhs);
        let room_id = room.id.clone();
        let settings = on_press(
            web_button(
                "desktop-room-settings",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector(|| "desktop-room-settings".to_string())
            .aria_label(ROOM_SETTINGS)
            .track_focus(&page.settings_focus)
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .child(ROOM_SETTINGS),
            cx,
            move |_, _, cx| {
                cx.emit(ReactiveOpenRoomSettings {
                    room_id: room_id.clone(),
                });
            },
        );
        div()
            .debug_selector(|| "desktop-room-header".to_string())
            .flex_shrink_0()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .border_b_1()
            .border_color(theme::longan_800().opacity(HAIRLINE_OPACITY))
            .px(px(layout.padding))
            .py_3()
            .child(
                // Web `min-w-0 flex-1`: a long members line truncates
                // instead of pushing Room settings to a row of its own.
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("desktop-room-name")
                            .debug_selector(|| "desktop-room-name".to_string())
                            .role(Role::Heading)
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(TITLE_TEXT_PX))
                            .text_color(theme::rambutan_100())
                            .child(room.name.clone()),
                    )
                    .child(
                        div()
                            .mt_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .children(members.into_iter().map(|bruh| {
                                        let selector =
                                            format!("desktop-room-face-{}", bruh.agent_id);
                                        div().debug_selector(move || selector).child(bruh_face(
                                            SharedString::from(format!(
                                                "room-{}-{}",
                                                room.id, bruh.agent_id
                                            )),
                                            RAIL_FACE_PX,
                                            bruh.look,
                                            bruh.status,
                                            cx,
                                        ))
                                    })),
                            )
                            .child(
                                small_text(
                                    members_line(room, &self.crew.bruhs),
                                    theme::rambutan_500(),
                                )
                                .debug_selector(|| "desktop-room-members".to_string())
                                .flex_1()
                                .truncate(),
                            ),
                    ),
            )
            .child(settings)
    }

    fn render_room_transcript(
        &self,
        page: &RoomPage,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> Div {
        let lines = Rc::clone(&page.lines);
        let log = layout
            .column()
            .gap_3()
            .id("desktop-room-log")
            .role(Role::Log)
            .aria_label("Room messages")
            .when(page.more, |el| {
                el.child(div().flex().justify_center().child(page_button(
                    "desktop-room-load-earlier",
                    "Load earlier",
                    WebButtonVariant::Ghost,
                    page.loading_earlier,
                    cx,
                    |this, _, cx| this.load_earlier_room_messages(cx),
                )))
            })
            .when_some(page.error.clone(), |el, message| {
                el.child(alert("desktop-room-messages-error", message))
            })
            .when(
                page.loaded && lines.is_empty() && page.error.is_none(),
                |el| {
                    el.child(
                        div()
                            .debug_selector(|| "desktop-room-empty".to_string())
                            .font_family(theme::CODE_FONT_FAMILY)
                            .text_size(px(SMALL_TEXT_PX))
                            .text_color(theme::rambutan_500())
                            .child(EMPTY_ROOM),
                    )
                },
            )
            .children(
                lines
                    .iter()
                    .map(|line| self.render_room_line(page, line, layout, cx)),
            );
        div().relative().flex_1().min_h_0().flex().flex_col().child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("desktop-room-transcript")
                        .debug_selector(|| "desktop-room-transcript".to_string())
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .overflow_y_scroll()
                        .track_scroll(&page.scroll)
                        .child(log),
                )
                .child(Scrollbar::vertical(&page.scroll)),
        )
    }

    fn bruh_look(&self, agent_id: &str) -> BruhLook {
        self.crew
            .bruhs
            .iter()
            .find(|bruh| bruh.agent_id == agent_id)
            .map(|bruh| bruh.look.clone())
            .unwrap_or_default()
    }

    /// A line a bruh wrote: its face, its current name and `body`, which
    /// takes the chat's bubble and card width.
    fn render_room_author(
        &self,
        selector: String,
        agent_id: &str,
        body: AnyElement,
        layout: Layout,
    ) -> AnyElement {
        let look = self.bruh_look(agent_id);
        let face = selector.replacen("room-line-", "room-face-", 1);
        div()
            .debug_selector(move || selector)
            .min_w_0()
            .flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .debug_selector(move || face)
                    .child(braille_face(
                        RAIL_FACE_PX,
                        &still_view(&look, BruhStatus::Working),
                    )),
            )
            .child(
                div()
                    .min_w_0()
                    .max_w(px(layout.bubble()))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1()
                    .child(small_text(
                        author_name(agent_id, &self.crew.bruhs),
                        theme::rambutan_400(),
                    ))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_room_line(
        &self,
        page: &RoomPage,
        line: &RoomLine,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        match line {
            RoomLine::Owner {
                message_id, text, ..
            } => {
                let selector = format!("room-line-owner-{message_id}");
                div()
                    .debug_selector(move || selector)
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(
                        bubble(layout.bubble())
                            .bg(theme::longan_800())
                            .text_color(theme::rambutan_100())
                            .child(text.clone()),
                    )
                    .into_any_element()
            }
            RoomLine::Bruh {
                message_id,
                agent_id,
                text,
                failed,
                ..
            } => {
                let failed = *failed;
                let body = bubble(layout.bubble())
                    .map(|el| {
                        if failed {
                            el.bg(theme::lychee_500().opacity(ERROR_FILL_OPACITY))
                                .text_color(theme::lychee_200())
                        } else {
                            el.bg(theme::longan_900())
                                .border_1()
                                .border_color(theme::longan_700().opacity(SOFT_RING_OPACITY))
                                .text_color(theme::rambutan_200())
                        }
                    })
                    .child(text.clone())
                    .into_any_element();
                self.render_room_author(
                    format!("room-line-bruh-{message_id}"),
                    agent_id,
                    body,
                    layout,
                )
            }
            RoomLine::System { message_id, text } => {
                let selector = format!("room-line-system-{message_id}");
                div()
                    .debug_selector(move || selector)
                    .min_w_0()
                    .text_center()
                    .text_size(px(LABEL_TEXT_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child(text.clone())
                    .into_any_element()
            }
            RoomLine::Card {
                message_id,
                agent_id,
                item,
            } => {
                let card = if item.approval.is_some() {
                    self.render_bruh_approval(item, layout, cx)
                } else {
                    let reply = page.replies.get(&item.id).map(|(input, _)| input);
                    self.render_bruh_question(item, reply, layout, cx)
                };
                self.render_room_author(
                    format!("room-line-card-{message_id}"),
                    agent_id,
                    card,
                    layout,
                )
            }
            RoomLine::Asked {
                message_id,
                agent_id,
                summary,
            } => {
                let card = self
                    .render_bruh_card(message_id, "Approval", layout)
                    .child(small_text(APPROVAL_WANTS, theme::rambutan_400()))
                    .child(
                        div()
                            .font_family(theme::CODE_FONT_FAMILY)
                            .text_size(px(BODY_TEXT_PX))
                            .text_color(theme::rambutan_100())
                            .child(summary.clone()),
                    )
                    .child(small_text(APPROVAL_DECIDED, theme::rambutan_400()))
                    .into_any_element();
                self.render_room_author(
                    format!("room-line-asked-{message_id}"),
                    agent_id,
                    card,
                    layout,
                )
            }
            RoomLine::Working { run_id, agent_id } => {
                let selector = format!("room-line-working-{run_id}");
                div()
                    .debug_selector(move || selector)
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(bruh_face(
                        SharedString::from(format!("room-working-{run_id}")),
                        RAIL_FACE_PX,
                        self.bruh_look(agent_id),
                        BruhStatus::Working,
                        cx,
                    ))
                    .child(small_text(
                        self.room_working_text(agent_id),
                        theme::rambutan_400(),
                    ))
                    .into_any_element()
            }
        }
    }

    fn render_room_composer(
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
                        .px(px(COMPOSER_X_PADDING_PX))
                        .pt(px(COMPOSER_TOP_PADDING_PX))
                        .pb(px(COMPOSER_BOTTOM_PADDING_PX))
                        .child(
                            Input::new(composer)
                                .aria_label("Message")
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .cleanable(false)
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

#[cfg(test)]
mod tests {
    use super::{ReactiveAutomations, ReactiveOpenRoomSettings};
    use crate::theme;
    use bruhs_models::crew::room::{
        Room, RoomAuthor, RoomMessage, RoomState, NO_MEMBERS, ROOM_GONE, ROOM_LEAD,
    };
    use bruhs_models::crew::{
        BruhAccess, BruhLook, BruhState, BruhStatus, ServiceNotice, CREW_PROTOCOL,
    };
    use bruhs_models::logic::context_chip::ChipKind;
    use bruhs_models::reactive::{
        OutboxItem, OutboxSignal, ReactiveCommand, ReactiveRun, ReactiveSnapshot, RunTrigger,
    };
    use bruhs_models::EpochMs;
    use gpui::{AppContext as _, Entity, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn named(id: &str, name: &str) -> BruhState {
        BruhState {
            agent_id: id.into(),
            name: name.into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status: BruhStatus::Asleep,
            next_wake_ms: None,
            unread: 0,
        }
    }

    fn room(members: &[&str], lead: Option<&str>) -> RoomState {
        RoomState {
            room: Room {
                id: "r".into(),
                name: "Launch".into(),
                members: members.iter().map(|id| (*id).to_string()).collect(),
                lead: lead.map(str::to_string),
            },
            last_seq: 4,
            working: Vec::new(),
        }
    }

    fn snapshot(protocol: u32, bruhs: Vec<BruhState>, rooms: Vec<RoomState>) -> ReactiveSnapshot {
        ReactiveSnapshot {
            protocol,
            bruhs,
            rooms,
            ..ReactiveSnapshot::default()
        }
    }

    fn message(seq: u64, author: RoomAuthor, text: &str) -> RoomMessage {
        RoomMessage {
            id: seq.to_string(),
            room_id: "r".into(),
            seq,
            author,
            text: text.into(),
            run_id: None,
            outbox_id: None,
            failed: false,
            created_at: EpochMs::saturating_from_i64(1),
        }
    }

    fn by(agent_id: &str) -> RoomAuthor {
        RoomAuthor::Bruh {
            agent_id: agent_id.into(),
        }
    }

    fn approval(id: &str) -> OutboxItem {
        serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": "a", "run_id": "ra", "kind": "ask",
            "title": "Approval", "message": "", "created_at": 1,
            "approval": {
                "tool": "SandboxNetworkAccess", "summary": "Reach example.com",
                "fingerprint": "f", "state": "waiting",
            },
        }))
        .unwrap()
    }

    fn room_run(id: &str, agent_id: &str) -> ReactiveRun {
        let mut run: ReactiveRun = serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": agent_id, "source_id": null, "subscription_id": null,
            "event_id": null, "status": "running", "prompt": id, "thread_id": "t",
            "session_id": null, "created_at": 1, "updated_at": 1,
            "not_before": 1, "result": null, "error": null,
        }))
        .unwrap();
        run.trigger = Some(RunTrigger::Room {
            room_id: "r".into(),
            message_id: "1".into(),
            hop: 0,
            resumed: false,
        });
        run
    }

    /// A standalone panel window on room `r`'s page.
    fn open(
        cx: &mut gpui::TestAppContext,
        snapshot: ReactiveSnapshot,
    ) -> (Entity<ReactiveAutomations>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            gpui_component::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(1200.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update(cx, |panel, cx| {
            panel.loaded = true;
            panel.set_snapshot(snapshot, cx);
            panel.show_room(Some("r".into()), cx);
        });
        cx.run_until_parked();
        (panel, cx)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).expect(selector);
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    /// A key press as the app gets it, without the newline the test
    /// platform would also type.
    fn press(cx: &mut VisualTestContext, key: &str) {
        let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
    }

    fn sent(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
    ) -> Vec<ReactiveCommand> {
        panel.update(cx, |panel, _| panel.sent_for_test().to_vec())
    }

    /// Focus the composer and type `text` into it.
    fn type_in(panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext, text: &str) {
        panel.update_in(cx, |panel, window, cx| {
            let composer = panel.room.as_ref().unwrap().composer.clone().unwrap();
            composer.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.run_until_parked();
        cx.simulate_input(text);
        cx.run_until_parked();
    }

    fn draft(panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext) -> String {
        panel.update(cx, |panel, cx| {
            let composer = panel.room.as_ref().unwrap().composer.clone().unwrap();
            composer.read(cx).value().to_string()
        })
    }

    #[gpui::test]
    fn a_room_shows_messages_faces_system_lines_and_cards(cx: &mut gpui::TestAppContext) {
        let mut shown = snapshot(
            CREW_PROTOCOL,
            vec![named("a", "Ada"), named("b", "Agent b")],
            vec![room(&["a", "b"], Some("a"))],
        );
        shown.outbox = vec![approval("i1")];
        shown.runs = vec![room_run("rb", "b")];
        let (panel, cx) = open(cx, shown);
        let mut asked = message(4, by("a"), "Reach example.com");
        asked.outbox_id = Some("i1".into());
        panel.update(cx, |panel, cx| {
            panel.show_room_messages_for_test(vec![
                asked,
                message(3, RoomAuthor::System, "Bo joined."),
                message(2, by("a"), "On it."),
                message(1, RoomAuthor::Owner, "@Ada ship it"),
            ]);
            cx.notify();
        });
        cx.run_until_parked();
        let owner = cx
            .debug_bounds("room-line-owner-1")
            .expect("the owner's line");
        let bruh = cx.debug_bounds("room-line-bruh-2").expect("Ada's line");
        let face = cx.debug_bounds("room-face-bruh-2").expect("Ada's face");
        assert!(bruh.contains(&face.center()), "the face sits in its line");
        assert_eq!(face.left(), bruh.left(), "the face leads the line");
        let system = cx
            .debug_bounds("room-line-system-3")
            .expect("the system line");
        assert!(owner.bottom() <= bruh.top() && bruh.bottom() <= system.top());
        let card = cx
            .debug_bounds("room-line-card-4")
            .expect("the approval card");
        let approve = cx.debug_bounds("desktop-bruh-approve-i1").expect("Approve");
        assert!(card.contains(&approve.center()), "Approve sits in the card");
        let working = cx
            .debug_bounds("room-line-working-rb")
            .expect("b's working line");
        assert!(card.bottom() <= working.top(), "working comes last");
        panel.update(cx, |panel, _| {
            assert_eq!(panel.room_working_text("b"), "Agent b is working");
        });
        // The card keeps the chat's card width beside the face.
        let header = cx.debug_bounds("desktop-room-header").expect("header");
        assert!(cx.debug_bounds("desktop-room-face-a").is_some());
        assert!(header.contains(
            &cx.debug_bounds("desktop-room-settings")
                .expect("Room settings")
                .center()
        ));
        click(cx, "desktop-bruh-approve-i1");
        assert_eq!(
            sent(&panel, cx).last(),
            Some(&ReactiveCommand::DecideApproval {
                id: "i1".into(),
                approve: true,
            })
        );
    }

    #[gpui::test]
    fn at_lists_members_and_enter_inserts_a_chip(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            snapshot(
                CREW_PROTOCOL,
                vec![named("a", "Ada"), named("b", "Bo")],
                vec![room(&["a", "b"], Some("b"))],
            ),
        );
        type_in(&panel, cx, "@b");
        panel.update(cx, |panel, _| {
            assert_eq!(panel.room.as_ref().unwrap().hits, ["b"]);
        });
        let list = cx
            .debug_bounds("desktop-room-mentions")
            .expect("the @ list");
        let row = cx.debug_bounds("desktop-room-mention-b").expect("Bo's row");
        assert!(list.contains(&row.center()));
        let lead = cx
            .debug_bounds("desktop-room-mention-lead-b")
            .expect("Bo's row says Lead");
        assert!(row.contains(&lead.center()));
        // Escape closes the list until the draft changes.
        press(cx, "escape");
        assert!(cx.debug_bounds("desktop-room-mentions").is_none());
        cx.simulate_input("o");
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-room-mention-b").is_some());
        press(cx, "enter");
        assert_eq!(draft(&panel, cx), "@Bo ");
        panel.update(cx, |panel, _| {
            let page = panel.room.as_ref().unwrap();
            assert!(page.mention.is_none(), "picking closes the list");
            let [chip] = page.chips.as_slice() else {
                panic!("one chip, got {:?}", page.chips);
            };
            assert_eq!((chip.start, chip.end), (0, 3));
            assert_eq!(
                chip.kind,
                ChipKind::Bruh {
                    agent_id: "b".into(),
                    name: "Bo".into(),
                }
            );
        });
        assert!(sent(&panel, cx).is_empty(), "picking posts nothing");
        // Backspace at the chip's end takes the whole name out.
        press(cx, "backspace");
        press(cx, "backspace");
        assert_eq!(draft(&panel, cx), "");
        panel.update(cx, |panel, _| {
            assert!(panel.room.as_ref().unwrap().chips.is_empty());
        });
    }

    #[gpui::test]
    fn posting_sends_post_room_and_clears(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            snapshot(
                CREW_PROTOCOL,
                vec![named("a", "Ada"), named("b", "Bo")],
                vec![room(&["a", "b"], None)],
            ),
        );
        assert!(cx.debug_bounds("desktop-room-hint").is_some(), "no lead");
        type_in(&panel, cx, "@Bo check");
        panel.update(cx, |panel, _| {
            assert!(panel.room.as_ref().unwrap().mention.is_none());
        });
        press(cx, "enter");
        assert_eq!(
            sent(&panel, cx).last(),
            Some(&ReactiveCommand::PostRoom {
                room_id: "r".into(),
                text: "@Bo check".into(),
            })
        );
        assert_eq!(draft(&panel, cx), "");
    }

    #[gpui::test]
    fn the_composer_is_off_for_an_empty_room_or_an_older_service(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            snapshot(
                CREW_PROTOCOL,
                vec![named("a", "Ada")],
                vec![room(&[], None)],
            ),
        );
        panel.update(cx, |panel, _| {
            assert_eq!(panel.room_composer_reason(), Some(NO_MEMBERS));
        });
        assert!(cx.debug_bounds("desktop-room-blocked").is_some());
        assert!(cx.debug_bounds("desktop-room-hint").is_none());
        panel.update_in(cx, |panel, window, cx| {
            let composer = panel.room.as_ref().unwrap().composer.clone().unwrap();
            composer.update(cx, |input, cx| input.set_value("hello", window, cx));
            panel.post_room(window, cx);
        });
        click(cx, "desktop-room-send");
        // A room on a service from before rooms.
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(
                snapshot(1, vec![named("a", "Ada")], vec![room(&["a"], None)]),
                cx,
            );
            cx.notify();
        });
        cx.run_until_parked();
        panel.update_in(cx, |panel, window, cx| {
            assert_eq!(
                panel.room_composer_reason(),
                Some(ServiceNotice::ServingOld.message())
            );
            panel.post_room(window, cx);
        });
        click(cx, "desktop-room-send");
        assert!(sent(&panel, cx).is_empty());
    }

    /// A redraw with nothing new reuses the lines instead of building them
    /// again on every frame.
    #[gpui::test]
    fn the_lines_are_built_only_when_messages_or_the_snapshot_change(
        cx: &mut gpui::TestAppContext,
    ) {
        let crew = vec![named("a", "Ada")];
        let mut shown = snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]);
        shown.runs = vec![room_run("ra", "a")];
        let (panel, cx) = open(cx, shown);
        let lines = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
            panel.read_with(cx, |panel, _| {
                Rc::clone(&panel.room.as_ref().unwrap().lines)
            })
        };
        let first = lines(&panel, cx);
        assert_eq!(first.len(), 1, "the working line");
        panel.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(
            Rc::ptr_eq(&first, &lines(&panel, cx)),
            "rebuilt on a redraw"
        );
        panel.update(cx, |panel, cx| {
            panel.show_room_messages_for_test(vec![message(1, RoomAuthor::Owner, "hi")]);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(lines(&panel, cx).len(), 2, "new messages rebuild them");
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(
                snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]),
                cx,
            );
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(lines(&panel, cx).len(), 1, "a new snapshot rebuilds them");
    }

    #[gpui::test]
    fn room_settings_asks_the_workspace_and_a_deleted_room_hides_the_page(
        cx: &mut gpui::TestAppContext,
    ) {
        let crew = vec![named("a", "Ada")];
        let (panel, cx) = open(
            cx,
            snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]),
        );
        let asked = Rc::new(RefCell::new(Vec::new()));
        let seen = asked.clone();
        let _watch = cx.update(|_, cx| {
            cx.subscribe(&panel, move |_, event: &ReactiveOpenRoomSettings, _| {
                seen.borrow_mut().push(event.clone());
            })
        });
        click(cx, "desktop-room-settings");
        assert_eq!(
            *asked.borrow(),
            [ReactiveOpenRoomSettings {
                room_id: "r".into()
            }]
        );
        // The room goes while Room settings has focus.
        panel.update_in(cx, |panel, window, cx| {
            panel
                .room
                .as_ref()
                .unwrap()
                .settings_focus
                .focus(window, cx);
            panel.set_snapshot(snapshot(CREW_PROTOCOL, crew, Vec::new()), cx);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("desktop-room-gone").is_some(),
            "{ROOM_GONE}"
        );
        assert!(cx.debug_bounds("desktop-room-settings").is_none());
        assert!(cx.debug_bounds("desktop-room-composer").is_none());
        panel.update_in(cx, |panel, window, _| {
            assert!(
                panel.room.as_ref().unwrap().gone_focus.is_focused(window),
                "focus lands on the notice"
            );
        });
    }

    /// A new message moves the room's `last_seq`, from a snapshot or a live
    /// crew frame: the page reads its newest messages again.
    #[gpui::test]
    fn a_new_last_seq_reads_the_newest_page_again(cx: &mut gpui::TestAppContext) {
        let crew = vec![named("a", "Ada")];
        let (panel, cx) = open(
            cx,
            snapshot(CREW_PROTOCOL, crew.clone(), vec![room(&["a"], None)]),
        );
        let read_for = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
            panel.read_with(cx, |panel, _| panel.room.as_ref().unwrap().read_for)
        };
        assert_eq!(read_for(&panel, cx), Some(4));
        let mut moved = room(&["a"], None);
        moved.last_seq = 6;
        panel.update(cx, |panel, cx| {
            // A snapshot read is already on its way.
            panel.reading = true;
            panel.apply_crew_signal(
                OutboxSignal {
                    bruhs: crew,
                    rooms: vec![moved],
                    protocol: CREW_PROTOCOL,
                    ..OutboxSignal::default()
                },
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(read_for(&panel, cx), Some(6));
    }

    /// Up and Down move through the `@` list and wrap; Enter picks the
    /// selected member; Tab leaves the list alone and inserts nothing.
    #[gpui::test]
    fn up_and_down_wrap_and_tab_inserts_nothing(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            snapshot(
                CREW_PROTOCOL,
                vec![named("a", "Ada"), named("b", "Bo")],
                vec![room(&["a", "b"], Some("b"))],
            ),
        );
        let selected = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
            panel.read_with(cx, |panel, _| panel.room.as_ref().unwrap().selected)
        };
        type_in(&panel, cx, "@");
        panel.update(cx, |panel, _| {
            assert_eq!(panel.room.as_ref().unwrap().hits, ["a", "b"]);
        });
        // Only the lead's row says Lead.
        assert!(cx.debug_bounds("desktop-room-mention-lead-b").is_some());
        assert!(cx.debug_bounds("desktop-room-mention-lead-a").is_none());
        assert_eq!(ROOM_LEAD, "Lead");
        assert_eq!(selected(&panel, cx), 0);
        press(cx, "down");
        assert_eq!(selected(&panel, cx), 1);
        press(cx, "down");
        assert_eq!(selected(&panel, cx), 0, "Down from the last wraps");
        press(cx, "up");
        assert_eq!(selected(&panel, cx), 1, "Up from the first wraps");
        press(cx, "enter");
        assert_eq!(draft(&panel, cx), "@Bo ");
        cx.simulate_input("@");
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-room-mentions").is_some());
        press(cx, "tab");
        panel.update(cx, |panel, _| {
            let page = panel.room.as_ref().unwrap();
            assert_eq!(page.chips.len(), 1, "Tab inserts no chip");
        });
        assert!(!draft(&panel, cx).contains("@Ada"));
        assert!(sent(&panel, cx).is_empty());
    }

    /// Web `min-w-0 flex-1` on the title: a long members line truncates
    /// and Room settings stays beside the name.
    #[gpui::test]
    fn a_long_members_line_keeps_room_settings_beside_the_name(cx: &mut gpui::TestAppContext) {
        let ids = ["a", "b", "c", "d", "e", "f"];
        let crew: Vec<BruhState> = ids
            .iter()
            .map(|id| named(id, &format!("Bruh with a long name {id}")))
            .collect();
        let (_, cx) = open(cx, snapshot(CREW_PROTOCOL, crew, vec![room(&ids, None)]));
        cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
        cx.run_until_parked();
        let name = cx.debug_bounds("desktop-room-name").expect("the name");
        let settings = cx
            .debug_bounds("desktop-room-settings")
            .expect("Room settings");
        let line = cx.debug_bounds("desktop-room-members").expect("members");
        assert!(
            settings.top() < name.bottom(),
            "Room settings sits on the name's row: {settings:?} {name:?}"
        );
        assert!(line.right() <= settings.left(), "the line stops before it");
        assert!(
            line.size.height < gpui::px(20.0),
            "one line: {:?}",
            line.size.height
        );
    }
}
