//! A room's page (web `crew/room.rs`): its members, its transcript with
//! each bruh's face and the chat's question and approval cards, older
//! messages a page at a time, and a composer whose `@` lists only the
//! room's members. The workspace shows the panel for `AppView::Room` and
//! tells it which room; the panel owns the page's state.
use super::bruh_page::{
    alert, bubble, crew_markdown, measure_width, page_button, prose_bubble, reply_field,
    small_text, Layout, BODY_TEXT_PX, HAIRLINE_OPACITY, LABEL_TEXT_PX, RADIUS_GRAPE_PX,
    SMALL_TEXT_PX, TITLE_TEXT_PX,
};
use super::create_bruh::ERROR_FILL_OPACITY;
use super::face_editor::still_view;
use super::transcript_list::{transcript_view, TranscriptList};
use super::*;
use crate::views::braille::braille_face;
use crate::views::crew_face::bruh_face;
use crate::views::web_button::on_press;
use bruhs_models::crew::room::{
    author_name, member_hits, member_states, members_line, mention_query, merge_messages,
    room_composer_block, room_composer_hint, room_follow_key, room_line_key, room_lines,
    room_markdown, room_post, working_line, Room, RoomLine, RoomMessage, RoomState,
    APPROVAL_DECIDED, APPROVAL_WANTS, EMPTY_ROOM, LOAD_EARLIER, ROOM_GONE, ROOM_LEAD, ROOM_LOADING,
    ROOM_SETTINGS,
};
use bruhs_models::crew::{may_have_more, BruhLook, BruhStatus, RAIL_FACE_PX};
use bruhs_models::logic::context_chip::{
    backspace_chip, delete_forward_chip, insert_chip, replace_range, retarget, ChipKind,
    ContextChip,
};
use std::cell::Cell;
use std::collections::hash_map::Entry;

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
    /// Each bruh message's Markdown by message id, ready to draw, built
    /// with `lines`.
    prose: HashMap<String, SharedString>,
    /// The snapshot revision and messages version `lines` was built from.
    built_for: Option<(u64, u64)>,
    list: TranscriptList,
    /// What the transcript last followed; a new line scrolls into view.
    followed: (Option<String>, Option<String>),
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
            prose: HashMap::new(),
            built_for: None,
            list: TranscriptList::new(),
            followed: (None, None),
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

    #[cfg(test)]
    pub(super) fn show_earlier_room_messages_for_test(&mut self, messages: Vec<RoomMessage>) {
        if let Some(page) = self.room.as_mut() {
            page.take_earlier(messages);
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
            composer::make_room_composer(page, window, cx);
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
            page.prose = page
                .lines
                .iter()
                .filter_map(|line| match line {
                    RoomLine::Bruh { message_id, .. } => {
                        Some((message_id.clone(), crew_markdown(room_markdown(line)?)))
                    }
                    _ => None,
                })
                .collect();
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
                    slot.insert(reply_field(window, cx));
                }
            }
            page.list
                .sync(page.lines.iter().map(room_line_key).collect());
            let shown = room_follow_key(&page.lines);
            if shown != page.followed {
                page.list.follow();
                page.followed = shown;
            }
        }
        page.list.sync_header(format!(
            "{}:{}:{}",
            page.more,
            page.error.is_some(),
            page.loaded && page.lines.is_empty()
        ));
        if reload {
            self.reload_room_messages(cx);
        }
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
        transcript_view(
            "desktop-room-transcript",
            "Room messages",
            &page.list,
            layout,
            cx,
            move |this, index, cx| {
                let page = this.room.as_ref()?;
                Some(match index.checked_sub(1) {
                    None => this.render_room_head(page, cx),
                    Some(at) => this.render_room_line(page, page.lines.get(at)?, layout, cx),
                })
            },
        )
    }

    /// What sits above the lines: Load earlier, the error and the empty
    /// line.
    fn render_room_head(&self, page: &RoomPage, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when(page.more, |el| {
                el.child(div().flex().justify_center().child(page_button(
                    "desktop-room-load-earlier",
                    LOAD_EARLIER,
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
                page.loaded && page.lines.is_empty() && page.error.is_none(),
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
            .into_any_element()
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
                let body = match page.prose.get(message_id).filter(|_| !*failed) {
                    Some(markdown) => prose_bubble(
                        format!("room-md-{message_id}"),
                        markdown.clone(),
                        layout,
                        cx,
                    ),
                    None => bubble(layout.bubble())
                        .bg(theme::lychee_500().opacity(ERROR_FILL_OPACITY))
                        .text_color(theme::lychee_200())
                        .child(text.clone())
                        .into_any_element(),
                };
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
}

mod composer;
#[cfg(test)]
mod tests;
