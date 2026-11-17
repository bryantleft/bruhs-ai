//! A bruh's own page (web `crew/page.rs`, `crew/chat.rs` and
//! `crew/settings.rs`): its face, name, job and status with Stop or Resume,
//! and tabs for its chat, memory, routines and settings. The workspace shows
//! the panel for `AppView::Bruh` and tells it which bruh; the panel owns the
//! page's state. Save sends `settings_commands`, so a face edit never
//! touches the agent.
use super::create_bruh::{
    hint, labelled, text_field, ERROR_FILL_OPACITY, FIELD_RADIUS_PX, FIELD_TEXT_PX,
    FIELD_X_PADDING_PX,
};
use super::face_editor::still_view;
use super::*;
use crate::views::braille::braille_face;
use crate::views::crew_face::bruh_face;
use crate::views::dropdown::{
    dropdown, DropdownGroup, DropdownOption, Listbox, SelectHandler, ToggleHandler,
};
use crate::views::web_button::is_web_button_activation_key;
use bruhs_models::agent::{AgentProvider, ModelId};
use bruhs_models::crew::{
    level_unavailable, settings_commands, status_line, transcript, AccessLevel, ApprovalState,
    BruhAccess, BruhLook, BruhSettings, BruhStatus, TranscriptItem, PAGE_FACE_PX, RAIL_FACE_PX,
};
use bruhs_models::logic::run_place::CLAUDE_EFFORTS;
use gpui_component::scroll::{ScrollableElement, Scrollbar};
use std::cell::Cell;

/// Web `text-h3`.
const TITLE_TEXT_PX: f32 = 18.0;
/// Web `text-body`.
const BODY_TEXT_PX: f32 = 14.0;
/// Web `text-body-sm` and `text-label`.
const SMALL_TEXT_PX: f32 = 13.0;
/// Web `text-label-sm`.
const LABEL_TEXT_PX: f32 = 12.0;
/// Web `max-w-3xl`.
const COLUMN_MAX_PX: f32 = 768.0;
/// Web `p-4`, and `sm:px-6` from the `sm` breakpoint.
const PAGE_PADDING_PX: f32 = 16.0;
const PAGE_WIDE_PADDING_PX: f32 = 24.0;
const SM_BREAKPOINT_PX: f32 = 640.0;
/// Web `max-w-[80%]` on chat bubbles and cards.
const BUBBLE_SHARE: f32 = 0.8;
const RADIUS_GRAPE_PX: f32 = 8.0;
const RADIUS_SEED_PX: f32 = 4.0;
/// Web `border-longan-800/50` hairlines.
const HAIRLINE_OPACITY: f32 = 0.5;
/// Web `inset-ring-longan-700/50` on bruh bubbles and settings sections.
const SOFT_RING_OPACITY: f32 = 0.5;
/// Web `bg-longan-950/60` on settings sections.
const SECTION_FILL_OPACITY: f32 = 0.6;
/// Web `py-1.5` around the tabs.
const TABS_Y_PADDING_PX: f32 = 6.0;
/// Web `Textarea rows=2` and `rows=5`: 21px lines, `py-1.5` and the ring.
const TWO_ROWS_PX: f32 = 55.0;
const FIVE_ROWS_PX: f32 = 118.0;
/// Web `p-0.5` around the access segments.
const SEGMENT_GROUP_PADDING_PX: f32 = 2.0;
/// Web `Switch`.
const SWITCH_WIDTH_PX: f32 = 44.0;
const SWITCH_HEIGHT_PX: f32 = 24.0;
const SWITCH_THUMB_PX: f32 = 20.0;
const SWITCH_THUMB_OFF_X_PX: f32 = 2.0;
const SWITCH_THUMB_ON_X_PX: f32 = 22.0;
const SWITCH_RING_OPACITY: f32 = 0.10;
const DISABLED_OPACITY: f32 = 0.5;
const EFFORT_DROPDOWN: &str = "desktop-bruh-effort";
const DELIVER_NEEDS_FULL: &str = "Needs Full access.";

/// The page's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BruhTab {
    Chat,
    Memory,
    Routines,
    Settings,
}

impl BruhTab {
    const ALL: [BruhTab; 4] = [
        BruhTab::Chat,
        BruhTab::Memory,
        BruhTab::Routines,
        BruhTab::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            BruhTab::Chat => "Chat",
            BruhTab::Memory => "Memory",
            BruhTab::Routines => "Routines",
            BruhTab::Settings => "Settings",
        }
    }

    fn key(self) -> &'static str {
        match self {
            BruhTab::Chat => "chat",
            BruhTab::Memory => "memory",
            BruhTab::Routines => "routines",
            BruhTab::Settings => "settings",
        }
    }
}

/// "Open Agent workflows" on the page of a bruh that no longer exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactiveOpenAutomations;

/// The job line under a bruh's name: the first line of its instructions.
fn job_line(instructions: &str) -> &str {
    instructions
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

/// Puts a page of runs into those already shown, replacing runs it updates.
fn merge_runs(shown: &mut Vec<ReactiveRun>, page: Vec<ReactiveRun>) {
    for run in page {
        match shown.iter_mut().find(|known| known.id == run.id) {
            Some(known) => *known = run,
            None => shown.push(run),
        }
    }
}

/// Whether a page this long may have older runs behind it.
fn may_have_more(page_len: usize) -> bool {
    page_len >= HISTORY_LIMIT
}

/// Why the composer is off for a bruh in this state.
fn composer_block(status: Option<BruhStatus>) -> Option<&'static str> {
    match status? {
        BruhStatus::Paused => Some("This bruh is paused. Resume it to chat."),
        BruhStatus::Off => Some("This bruh is off. Turn it on in Agent workflows to chat."),
        _ => None,
    }
}

/// How a decided approval ended; `None` while it still waits.
fn approval_outcome(item: &OutboxItem) -> Option<&'static str> {
    item.resolved_at?;
    Some(
        match item.approval.as_ref().map(|approval| approval.state) {
            Some(ApprovalState::Approved) => "Approved",
            Some(ApprovalState::Denied) => "Denied",
            Some(ApprovalState::Expired) => "Expired",
            Some(ApprovalState::Waiting) | None => "Withdrawn",
        },
    )
}

/// What identifies a line as drawn; the transcript follows a new last line.
fn line_key(item: &TranscriptItem) -> String {
    match item {
        TranscriptItem::Owner {
            run_id, pending, ..
        } => format!("owner:{run_id}:{pending}"),
        TranscriptItem::Bruh {
            run_id,
            text,
            failed,
            ..
        } => format!("bruh:{run_id}:{failed}:{text}"),
        TranscriptItem::Card { item } => format!(
            "card:{}:{}:{:?}",
            item.id,
            item.resolved_at.is_some(),
            item.approval.as_ref().map(|approval| approval.state)
        ),
    }
}

/// An open question card waits for a typed reply.
fn open_question(item: &OutboxItem, agent_id: &str) -> bool {
    item.agent_id == agent_id
        && item.kind == OutboxKind::Ask
        && item.approval.is_none()
        && item.resolved_at.is_none()
}

/// Each access level in display order, with why `model` can't run it.
fn access_options(model: &str) -> Vec<(AccessLevel, Option<&'static str>)> {
    let provider = AgentProvider::for_model(model);
    [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full]
        .into_iter()
        .map(|level| (level, level_unavailable(level, provider)))
        .collect()
}

/// `level`, or Read only when `model` can't run it.
fn clamp_level(level: AccessLevel, model: &str) -> AccessLevel {
    if level_unavailable(level, AgentProvider::for_model(model)).is_some() {
        AccessLevel::ReadOnly
    } else {
        level
    }
}

/// The settings a bruh has now, from the snapshot.
fn current_settings(snapshot: &ReactiveSnapshot, agent_id: &str) -> Option<BruhSettings> {
    let agent = snapshot.agents.iter().find(|agent| agent.id == agent_id)?;
    let state = snapshot
        .bruhs
        .iter()
        .find(|bruh| bruh.agent_id == agent_id)?;
    Some(BruhSettings {
        name: agent.name.clone(),
        instructions: agent.instructions.clone(),
        model: agent.model.clone(),
        effort: state.effort.clone(),
        look: state.look.clone(),
        access: state.access.clone(),
    })
}

/// The runs the chat shows, newest page first, then older pages.
#[derive(Default)]
struct RunsPage {
    runs: Vec<ReactiveRun>,
    /// The last run shown; older pages start after it.
    oldest: Option<String>,
    more: bool,
    error: Option<String>,
}

impl RunsPage {
    /// The newest page, read again after a change.
    fn take_newest(&mut self, page: Vec<ReactiveRun>) {
        if self.oldest.is_none() {
            self.more = may_have_more(page.len());
            self.oldest = page.last().map(|run| run.id.clone());
        }
        merge_runs(&mut self.runs, page);
        self.error = None;
    }

    /// An older page, from Load earlier.
    fn take_earlier(&mut self, page: Vec<ReactiveRun>) {
        self.more = may_have_more(page.len());
        if let Some(last) = page.last() {
            self.oldest = Some(last.id.clone());
        }
        merge_runs(&mut self.runs, page);
    }
}

/// The settings form, filled from the service unless the owner has unsaved
/// edits.
struct SettingsForm {
    /// What the form was last filled from.
    shown: BruhSettings,
    name: Entity<InputState>,
    job: Entity<InputState>,
    model: Entity<InputState>,
    effort: Option<String>,
    effort_open: bool,
    level: AccessLevel,
    send_as_me: bool,
    can_deliver: bool,
    look: BruhLook,
    invalid: Option<String>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl SettingsForm {
    fn new(
        settings: &BruhSettings,
        window: &mut Window,
        cx: &mut Context<'_, ReactiveAutomations>,
    ) -> Self {
        let name = edit_input(&settings.name, false, window, cx);
        let job = edit_input(&settings.instructions, true, window, cx);
        let model = edit_input(&settings.model, false, window, cx);
        let redraw = |_: &mut ReactiveAutomations,
                      _: &Entity<InputState>,
                      event: &InputEvent,
                      _: &mut Window,
                      cx: &mut Context<'_, ReactiveAutomations>| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&name, window, redraw),
            cx.subscribe_in(&job, window, redraw),
            // The model is committed when the field loses focus: a model
            // that can't run the chosen level drops it to Read only, which
            // the service would otherwise refuse.
            cx.subscribe_in(
                &model,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::Blur => {
                        if let Some(form) = this.bruh_settings_mut() {
                            form.clamp(cx);
                        }
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        Self {
            shown: settings.clone(),
            name,
            job,
            model,
            effort: settings.effort.clone(),
            effort_open: false,
            level: settings.access.level,
            send_as_me: settings.access.send_as_me,
            can_deliver: settings.access.can_deliver,
            look: settings.look.clone(),
            invalid: None,
            _subscriptions: subscriptions,
        }
    }

    fn edited(&self, cx: &App) -> BruhSettings {
        BruhSettings {
            name: value(&self.name, cx),
            instructions: value(&self.job, cx),
            model: value(&self.model, cx),
            effort: self.effort.clone(),
            look: self.look.clone(),
            access: BruhAccess {
                level: self.level,
                send_as_me: self.send_as_me,
                can_deliver: self.can_deliver,
            },
        }
    }

    fn fill(&mut self, settings: &BruhSettings, window: &mut Window, cx: &mut App) {
        for (input, text) in [
            (&self.name, &settings.name),
            (&self.job, &settings.instructions),
            (&self.model, &settings.model),
        ] {
            if value(input, cx) != *text {
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
        self.effort.clone_from(&settings.effort);
        self.level = settings.access.level;
        self.send_as_me = settings.access.send_as_me;
        self.can_deliver = settings.access.can_deliver;
        self.look = settings.look.clone();
        self.shown = settings.clone();
    }

    /// Access the model can run; delivery needs Full.
    fn clamp(&mut self, cx: &App) {
        self.level = clamp_level(self.level, &value(&self.model, cx));
        if self.level != AccessLevel::Full {
            self.can_deliver = false;
        }
    }
}

/// The bruh page while it is on screen.
pub(super) struct BruhPage {
    agent_id: String,
    tab: BruhTab,
    message: Option<Entity<InputState>>,
    runs: RunsPage,
    runs_requested: bool,
    runs_revision: u64,
    loading_earlier: bool,
    newest_request: Option<Task<()>>,
    earlier_request: Option<Task<()>>,
    /// Reply fields of open question cards, by outbox id.
    replies: HashMap<String, (Entity<InputState>, gpui::Subscription)>,
    settings: Option<SettingsForm>,
    scroll: ScrollHandle,
    /// The last transcript line drawn; a new one scrolls into view.
    last_line: Option<String>,
    /// The page's width, measured as it is drawn.
    width: Rc<Cell<f32>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl BruhPage {
    fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            tab: BruhTab::Chat,
            message: None,
            runs: RunsPage::default(),
            runs_requested: false,
            runs_revision: 0,
            loading_earlier: false,
            newest_request: None,
            earlier_request: None,
            replies: HashMap::new(),
            settings: None,
            scroll: ScrollHandle::new(),
            last_line: None,
            width: Rc::new(Cell::new(0.0)),
            _subscriptions: Vec::new(),
        }
    }
}

/// A page button: a click or Enter or Space runs `action` once.
fn press(
    el: Stateful<Div>,
    cx: &Context<'_, ReactiveAutomations>,
    action: impl Fn(&mut ReactiveAutomations, &mut Window, &mut Context<'_, ReactiveAutomations>)
        + 'static,
) -> Stateful<Div> {
    let action = Rc::new(action);
    let key = Rc::clone(&action);
    el.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
        if !event.is_keyboard() {
            cx.stop_propagation();
            action(this, window, cx);
        }
    }))
    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
        if is_web_button_activation_key(event.keystroke.key.as_str()) {
            window.prevent_default();
            cx.stop_propagation();
            key(this, window, cx);
        }
    }))
}

/// A web `Button`: handlers only while enabled.
fn page_button(
    id: impl Into<SharedString>,
    text: &'static str,
    variant: WebButtonVariant,
    disabled: bool,
    cx: &Context<'_, ReactiveAutomations>,
    action: impl Fn(&mut ReactiveAutomations, &mut Window, &mut Context<'_, ReactiveAutomations>)
        + 'static,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let selector = id.clone();
    let button = web_button(
        id,
        variant,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .debug_selector(move || selector.to_string())
    .aria_label(text)
    .child(text);
    if disabled {
        button.cursor_not_allowed()
    } else {
        press(button, cx, action)
    }
}

/// Web `Switch`: persimmon track when on, longan with a white/10 ring off.
fn page_switch(
    id: &'static str,
    text: &'static str,
    checked: bool,
    disabled: bool,
    cx: &Context<'_, ReactiveAutomations>,
    action: impl Fn(&mut ReactiveAutomations, &mut Window, &mut Context<'_, ReactiveAutomations>)
        + 'static,
) -> Stateful<Div> {
    let border = 1.0;
    let thumb_x = if checked {
        SWITCH_THUMB_ON_X_PX
    } else {
        SWITCH_THUMB_OFF_X_PX
    } - border;
    let thumb_y = (SWITCH_HEIGHT_PX - SWITCH_THUMB_PX) / 2.0 - border;
    let switch = div()
        .id(id)
        .debug_selector(move || id.to_string())
        .role(Role::Switch)
        .aria_label(text)
        .aria_toggled(checked.into())
        .relative()
        .flex_shrink_0()
        .w(px(SWITCH_WIDTH_PX))
        .h(px(SWITCH_HEIGHT_PX))
        .rounded_full()
        .border_1()
        .map(|el| {
            if checked {
                el.bg(theme::persimmon_500())
                    .border_color(gpui::transparent_black())
            } else {
                el.bg(theme::longan_700())
                    .border_color(gpui::white().opacity(SWITCH_RING_OPACITY))
            }
        })
        .child(
            div()
                .absolute()
                .left(px(thumb_x))
                .top(px(thumb_y))
                .size(px(SWITCH_THUMB_PX))
                .rounded_full()
                .bg(theme::rambutan_100()),
        );
    if disabled {
        switch.opacity(DISABLED_OPACITY).cursor_not_allowed()
    } else {
        press(
            switch
                .tab_index(0)
                .cursor_pointer()
                .focus_visible(|el| el.border_color(theme::persimmon_400())),
            cx,
            action,
        )
    }
}

fn small_text(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .min_w_0()
        .text_size(px(LABEL_TEXT_PX))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(color)
        .child(text.into())
}

fn alert(id: &'static str, message: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .role(Role::Alert)
        .min_w_0()
        .p_3()
        .rounded(px(FIELD_RADIUS_PX))
        .bg(theme::lychee_500().opacity(ERROR_FILL_OPACITY))
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(SMALL_TEXT_PX))
        .text_color(theme::lychee_200())
        .child(message.into())
}

fn bubble(max_width: f32) -> Div {
    div()
        .min_w_0()
        .max_w(px(max_width))
        .rounded(px(RADIUS_GRAPE_PX))
        .px_3()
        .py_2()
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(BODY_TEXT_PX))
        .line_height(relative(1.5))
}

/// The page's measured layout: the centred column and its padding.
#[derive(Clone, Copy)]
struct Layout {
    column: f32,
    padding: f32,
}

impl Layout {
    fn of(width: f32) -> Self {
        let padding = if width >= SM_BREAKPOINT_PX {
            PAGE_WIDE_PADDING_PX
        } else {
            PAGE_PADDING_PX
        };
        Self {
            column: width.min(COLUMN_MAX_PX),
            padding,
        }
    }

    /// The content width inside the column's padding.
    fn inner(self) -> f32 {
        (self.column - self.padding * 2.0).max(0.0)
    }

    fn bubble(self) -> f32 {
        self.inner() * BUBBLE_SHARE
    }

    /// Web `mx-auto w-full max-w-3xl min-w-0 p-4 sm:px-6`. A px width,
    /// because percentages inside a scroll container measure as indefinite.
    fn column(self) -> Div {
        div()
            .w(px(self.column))
            .min_w_0()
            .flex_shrink_0()
            .px(px(self.padding))
            .py(px(PAGE_PADDING_PX))
            .flex()
            .flex_col()
    }
}

impl ReactiveAutomations {
    /// The bruh whose page is open.
    pub(crate) fn bruh_page_agent(&self) -> Option<&str> {
        self.bruh.as_ref().map(|page| page.agent_id.as_str())
    }

    /// Show `agent_id`'s page, or none. Another bruh starts on its Chat tab.
    pub(crate) fn show_bruh(&mut self, agent_id: Option<String>, cx: &mut Context<'_, Self>) {
        if self.bruh_page_agent() == agent_id.as_deref() {
            return;
        }
        self.bruh = agent_id.map(BruhPage::new);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn open_bruh_for_test(
        &mut self,
        agent_id: &str,
        tab: BruhTab,
        cx: &mut Context<'_, Self>,
    ) {
        self.show_bruh(Some(agent_id.to_string()), cx);
        self.set_bruh_tab(tab, cx);
    }

    pub(super) fn set_bruh_tab(&mut self, tab: BruhTab, cx: &mut Context<'_, Self>) {
        if let Some(page) = self.bruh.as_mut() {
            page.tab = tab;
            cx.notify();
        }
    }

    pub(super) fn bruh_settings_look_mut(&mut self) -> Option<&mut BruhLook> {
        self.bruh_settings_mut().map(|form| &mut form.look)
    }

    fn bruh_settings_mut(&mut self) -> Option<&mut SettingsForm> {
        self.bruh.as_mut()?.settings.as_mut()
    }

    fn bruh_state(&self, agent_id: &str) -> Option<&BruhState> {
        self.snapshot
            .as_ref()?
            .bruhs
            .iter()
            .find(|bruh| bruh.agent_id == agent_id)
    }

    /// Why crew commands are off, once the service is known to be older.
    fn crew_reason(&self) -> Option<&'static str> {
        self.crew
            .protocol
            .and_then(|protocol| service_notice(protocol, None))
            .map(ServiceNotice::message)
    }

    /// Stop, Save and the approval buttons wait for a current service.
    fn crew_disabled(&self) -> bool {
        self.busy || !self.crew_ready()
    }

    /// Read the open page's newest runs again.
    pub(super) fn reload_bruh_runs(&mut self, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let Some(page) = self.bruh.as_mut() else {
            return;
        };
        page.runs_requested = true;
        // Tests never reach a server.
        if cfg!(test) {
            return;
        }
        page.runs_revision = page.runs_revision.wrapping_add(1);
        let revision = page.runs_revision;
        let agent_id = page.agent_id.clone();
        let requested = agent_id.clone();
        let operation = spawn_tokio(cx, async move { client.agent_runs(&requested, None).await });
        page.newest_request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                let Some(page) = this
                    .bruh
                    .as_mut()
                    .filter(|page| page.agent_id == agent_id && page.runs_revision == revision)
                else {
                    return;
                };
                match result {
                    Ok(runs) => page.runs.take_newest(runs),
                    Err(error) => page.runs.error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
    }

    /// Load earlier: the page of runs before the oldest shown.
    fn load_earlier_bruh_runs(&mut self, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let Some(page) = self.bruh.as_mut() else {
            return;
        };
        let Some(before) = page.runs.oldest.clone() else {
            return;
        };
        if page.loading_earlier || cfg!(test) {
            return;
        }
        page.loading_earlier = true;
        let agent_id = page.agent_id.clone();
        let requested = agent_id.clone();
        let operation = spawn_tokio(cx, async move {
            client.agent_runs(&requested, Some(&before)).await
        });
        page.earlier_request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                let Some(page) = this.bruh.as_mut().filter(|page| page.agent_id == agent_id) else {
                    return;
                };
                page.loading_earlier = false;
                match result {
                    Ok(runs) => page.runs.take_earlier(runs),
                    Err(error) => page.runs.error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn show_bruh_runs_for_test(&mut self, runs: Vec<ReactiveRun>) {
        if let Some(page) = self.bruh.as_mut() {
            page.runs.take_newest(runs);
        }
    }

    /// The page's inputs and data, made before it draws.
    pub(super) fn ensure_bruh_page(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let saved = self
            .bruh_page_agent()
            .and_then(|agent_id| current_settings(self.snapshot.as_ref()?, agent_id));
        let outbox = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.outbox.clone())
            .unwrap_or_default();
        let Some(page) = self.bruh.as_mut() else {
            return;
        };
        if page.message.is_none() {
            let message = cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .submit_on_enter(true)
                    .placeholder("Message")
            });
            page._subscriptions.push(cx.subscribe_in(
                &message,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    } => this.send_bruh_chat(window, cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            ));
            page.message = Some(message);
        }
        if let Some(saved) = saved {
            match page.settings.as_mut() {
                None => page.settings = Some(SettingsForm::new(&saved, window, cx)),
                Some(form) if form.shown != saved => {
                    let current = form.edited(cx);
                    if current == form.shown || current == saved {
                        form.fill(&saved, window, cx);
                    }
                }
                Some(_) => {}
            }
        }
        let agent_id = page.agent_id.clone();
        page.replies.retain(|id, _| {
            outbox
                .iter()
                .any(|item| item.id == *id && open_question(item, &agent_id))
        });
        for item in outbox.iter().filter(|item| open_question(item, &agent_id)) {
            if !page.replies.contains_key(&item.id) {
                let input = cx.new(|cx| InputState::new(window, cx).multi_line(true));
                let watch = cx.subscribe_in(&input, window, |_, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                });
                page.replies.insert(item.id.clone(), (input, watch));
            }
        }
        let last = transcript(&agent_id, &page.runs.runs, &outbox)
            .last()
            .map(line_key);
        if last != page.last_line {
            page.scroll.scroll_to_bottom();
            page.last_line = last;
        }
        if !page.runs_requested {
            self.reload_bruh_runs(cx);
        }
    }

    fn send_bruh_chat(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(page) = self.bruh.as_ref() else {
            return;
        };
        let Some(message) = page.message.clone() else {
            return;
        };
        let agent_id = page.agent_id.clone();
        let text = value(&message, cx);
        let blocked = composer_block(self.bruh_state(&agent_id).map(|state| state.status));
        if text.trim().is_empty() || blocked.is_some() {
            return;
        }
        self.send_all(
            vec![ReactiveCommand::Chat {
                agent_id,
                message: text,
            }],
            Scope::General,
            cx,
        );
        message.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn answer_bruh_question(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((input, _)) = self.bruh.as_ref().and_then(|page| page.replies.get(id)) else {
            return;
        };
        let input = input.clone();
        let text = value(&input, cx);
        if text.trim().is_empty() || self.busy {
            return;
        }
        self.send_all(
            vec![ReactiveCommand::AnswerOutbox {
                id: id.to_string(),
                message: text,
            }],
            Scope::General,
            cx,
        );
        input.update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Stop this bruh, or resume it when it is paused.
    fn stop_or_resume_bruh(&mut self, cx: &mut Context<'_, Self>) {
        if self.crew_disabled() {
            return;
        }
        let Some(agent_id) = self.bruh_page_agent().map(str::to_string) else {
            return;
        };
        let paused = self
            .bruh_state(&agent_id)
            .is_some_and(|state| state.status == BruhStatus::Paused);
        let command = if paused {
            ReactiveCommand::ResumeBruh { agent_id }
        } else {
            ReactiveCommand::StopBruh { agent_id }
        };
        self.send_all(vec![command], Scope::General, cx);
    }

    fn decide_bruh_approval(&mut self, id: &str, approve: bool, cx: &mut Context<'_, Self>) {
        if self.crew_disabled() {
            return;
        }
        self.send_all(
            vec![ReactiveCommand::DecideApproval {
                id: id.to_string(),
                approve,
            }],
            Scope::General,
            cx,
        );
    }

    fn edit_bruh_settings(
        &mut self,
        change: impl FnOnce(&mut SettingsForm),
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(form) = self.bruh_settings_mut() {
            change(form);
            cx.notify();
        }
    }

    /// Save: the agent when its fields changed, then the bruh.
    fn save_bruh_settings(&mut self, cx: &mut Context<'_, Self>) {
        if self.crew_disabled() {
            return;
        }
        let Some(page) = self.bruh.as_mut() else {
            return;
        };
        let Some(form) = page.settings.as_mut() else {
            return;
        };
        form.clamp(cx);
        let edited = form.edited(cx);
        if edited.name.trim().is_empty() {
            form.invalid = Some("Give the bruh a name.".into());
            cx.notify();
            return;
        }
        if let Err(error) = ModelId::try_from(edited.model.clone()) {
            form.invalid = Some(error.to_string());
            cx.notify();
            return;
        }
        form.invalid = None;
        let agent_id = page.agent_id.clone();
        let commands = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                let agent = snapshot.agents.iter().find(|agent| agent.id == agent_id)?;
                let state = snapshot
                    .bruhs
                    .iter()
                    .find(|bruh| bruh.agent_id == agent_id)?;
                Some(settings_commands(agent, state, &edited))
            })
            .unwrap_or_default();
        self.send_all(commands, Scope::General, cx);
        cx.notify();
    }

    /// The bruh page: header, tabs and the open tab, or why it can't show.
    pub(crate) fn render_bruh_page(
        &self,
        agent_id: &str,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let measured = self.bruh.as_ref().map_or(0.0, |page| page.width.get());
        let width = if measured > 0.0 {
            measured
        } else {
            f32::from(window.viewport_size().width)
        };
        let layout = Layout::of(width);
        let root = div()
            .debug_selector(|| "desktop-bruh-page".to_string())
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
            .when_some(self.bruh.as_ref(), |root, page| {
                let measured = page.width.clone();
                let entity = cx.entity_id();
                root.child(
                    canvas(
                        move |bounds, _, cx| {
                            let width = f32::from(bounds.size.width);
                            if (measured.get() - width).abs() > 0.5 {
                                measured.set(width);
                                cx.notify(entity);
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            });
        let Some(snapshot) = &self.snapshot else {
            let pending = match &self.load_error {
                Some(message) => div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_3()
                    .child(alert("desktop-bruh-load-error", message.clone()))
                    .child(page_button(
                        "desktop-bruh-retry",
                        "Retry",
                        WebButtonVariant::Outline,
                        self.reading,
                        cx,
                        |this, _, cx| this.refresh_snapshot(cx),
                    )),
                None => div().p_6().child(
                    div()
                        .id("desktop-bruh-loading")
                        .debug_selector(|| "desktop-bruh-loading".to_string())
                        .role(Role::Status)
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(SMALL_TEXT_PX))
                        .text_color(theme::rambutan_400())
                        .child("Loading this bruh…"),
                ),
            };
            return root.child(pending);
        };
        let Some(state) = snapshot.bruhs.iter().find(|bruh| bruh.agent_id == agent_id) else {
            return root.child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .id("desktop-bruh-gone")
                            .role(Role::Heading)
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(TITLE_TEXT_PX))
                            .text_color(theme::rambutan_100())
                            .child("This bruh no longer exists."),
                    )
                    .child(press(
                        div()
                            .id("desktop-bruh-open-workflows")
                            .debug_selector(|| "desktop-bruh-open-workflows".to_string())
                            .role(Role::Link)
                            .aria_label("Open Agent workflows")
                            .tab_index(0)
                            .cursor_pointer()
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .font_family(theme::UI_FONT_FAMILY)
                            .text_size(px(SMALL_TEXT_PX))
                            .text_color(theme::blueberry_300())
                            .underline()
                            .hover(|el| el.text_color(theme::blueberry_200()))
                            .child("Open Agent workflows"),
                        cx,
                        |_, _, cx| cx.emit(ReactiveOpenAutomations),
                    )),
            );
        };
        let tab = self.bruh.as_ref().map_or(BruhTab::Chat, |page| page.tab);
        let job = snapshot
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
            .map(|agent| job_line(&agent.instructions).to_string())
            .unwrap_or_default();
        let body = match tab {
            BruhTab::Chat => self.render_bruh_chat(agent_id, state, layout, cx),
            BruhTab::Memory => self.render_bruh_scroll(
                "desktop-bruh-memory",
                layout
                    .column()
                    .child(self.render_agent_records(agent_id, cx)),
            ),
            BruhTab::Routines => self.render_bruh_scroll(
                "desktop-bruh-routines",
                layout
                    .column()
                    .gap_3()
                    .child(self.render_bruh_routines(agent_id, cx)),
            ),
            BruhTab::Settings => self.render_bruh_scroll(
                "desktop-bruh-settings",
                layout.column().child(self.render_bruh_settings(layout, cx)),
            ),
        };
        let error = self.error.clone().or_else(|| self.load_error.clone());
        root.child(self.render_bruh_header(state, &job, layout, cx))
            .child(self.render_bruh_tabs(tab, layout, cx))
            .child(
                div()
                    .id("desktop-bruh-tabpanel")
                    .role(Role::TabPanel)
                    .aria_label(tab.label())
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(body),
            )
            .when_some(
                error.filter(|_| tab != BruhTab::Routines),
                |root, message| {
                    root.child(
                        div()
                            .flex_shrink_0()
                            .mx(px(layout.padding))
                            .mb_3()
                            .child(alert("desktop-bruh-error", message)),
                    )
                },
            )
    }

    fn render_bruh_scroll(&self, id: &'static str, column: Div) -> Div {
        div().flex_1().min_h_0().flex().flex_col().child(
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .overflow_y_scrollbar()
                .child(column),
        )
    }

    fn render_bruh_header(
        &self,
        state: &BruhState,
        job: &str,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> Div {
        let paused = state.status == BruhStatus::Paused;
        let wake = state.next_wake_ms.map(local_time);
        div()
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
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(bruh_face(
                        SharedString::from(format!("page-{}", state.agent_id)),
                        PAGE_FACE_PX,
                        state.look.clone(),
                        state.status,
                        cx,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .id("desktop-bruh-name")
                                    .debug_selector(|| "desktop-bruh-name".to_string())
                                    .role(Role::Heading)
                                    .font_family(theme::HEADING_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(TITLE_TEXT_PX))
                                    .text_color(theme::rambutan_100())
                                    .child(state.name.clone()),
                            )
                            .when(!job.is_empty(), |el| {
                                el.child(
                                    div()
                                        .debug_selector(|| "desktop-bruh-job".to_string())
                                        .min_w_0()
                                        .truncate()
                                        .font_family(theme::CODE_FONT_FAMILY)
                                        .text_size(px(SMALL_TEXT_PX))
                                        .text_color(theme::rambutan_400())
                                        .child(job.to_string()),
                                )
                            })
                            .child(small_text(
                                status_line(state.status, wake.as_deref()),
                                theme::rambutan_500(),
                            )),
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .when_some(self.crew_reason(), |el, reason| {
                        el.child(small_text(reason, theme::rambutan_400()))
                    })
                    .child(page_button(
                        "desktop-bruh-stop",
                        if paused { "Resume" } else { "Stop" },
                        WebButtonVariant::Outline,
                        self.crew_disabled(),
                        cx,
                        |this, _, cx| this.stop_or_resume_bruh(cx),
                    )),
            )
    }

    fn render_bruh_tabs(&self, tab: BruhTab, layout: Layout, cx: &Context<'_, Self>) -> Div {
        div()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::longan_800().opacity(HAIRLINE_OPACITY))
            .px(px(layout.padding))
            .py(px(TABS_Y_PADDING_PX))
            .child(
                div()
                    .id("desktop-bruh-tabs")
                    .role(Role::TabList)
                    .aria_label("Bruh sections")
                    .flex()
                    .gap_1()
                    .children(BruhTab::ALL.into_iter().map(|option| {
                        let chosen = option == tab;
                        let selector = format!("desktop-bruh-tab-{}", option.key());
                        press(
                            div()
                                .id(SharedString::from(selector.clone()))
                                .debug_selector(move || selector)
                                .role(Role::Tab)
                                .aria_label(option.label())
                                .aria_selected(chosen)
                                .tab_index(0)
                                .min_h(px(theme::CONTROL_PX))
                                .px_3()
                                .flex()
                                .items_center()
                                .rounded(px(RADIUS_SEED_PX))
                                .border_1()
                                .border_color(gpui::transparent_black())
                                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                .cursor_pointer()
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_size(px(LABEL_TEXT_PX))
                                .map(|el| {
                                    if chosen {
                                        el.bg(theme::longan_700()).text_color(theme::rambutan_100())
                                    } else {
                                        el.text_color(theme::rambutan_500())
                                            .hover(|el| el.text_color(theme::rambutan_300()))
                                    }
                                })
                                .child(option.label()),
                            cx,
                            move |this, _, cx| this.set_bruh_tab(option, cx),
                        )
                    })),
            )
    }

    fn render_bruh_chat(
        &self,
        agent_id: &str,
        state: &BruhState,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> Div {
        let Some(page) = self.bruh.as_ref() else {
            return div();
        };
        let outbox = self
            .snapshot
            .as_ref()
            .map_or(&[][..], |snapshot| snapshot.outbox.as_slice());
        let lines = transcript(agent_id, &page.runs.runs, outbox);
        let blocked = composer_block(Some(state.status));
        let empty = page
            .message
            .as_ref()
            .is_none_or(|message| value(message, cx).trim().is_empty());
        let log = layout
            .column()
            .gap_3()
            .id("desktop-bruh-log")
            .role(Role::Log)
            .aria_label("Conversation")
            .when(page.runs.more, |el| {
                el.child(div().flex().justify_center().child(page_button(
                    "desktop-bruh-load-earlier",
                    "Load earlier",
                    WebButtonVariant::Ghost,
                    page.loading_earlier,
                    cx,
                    |this, _, cx| this.load_earlier_bruh_runs(cx),
                )))
            })
            .when_some(page.runs.error.clone(), |el, message| {
                el.child(alert("desktop-bruh-runs-error", message))
            })
            .when(lines.is_empty() && page.runs.error.is_none(), |el| {
                el.child(
                    div()
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(SMALL_TEXT_PX))
                        .text_color(theme::rambutan_500())
                        .child("Nothing here yet. Say hi."),
                )
            })
            .children(
                lines
                    .into_iter()
                    .map(|line| self.render_bruh_line(line, &state.look, layout, cx)),
            );
        let composer = layout
            .column()
            .gap_2()
            .when_some(blocked, |el, reason| {
                el.child(small_text(reason, theme::rambutan_400()))
            })
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_end()
                    .gap_2()
                    .when_some(page.message.as_ref(), |el, message| {
                        el.child(
                            div()
                                .debug_selector(|| "desktop-bruh-message".to_string())
                                .min_w_0()
                                .flex_1()
                                .child(
                                    Input::new(message)
                                        .aria_label("Message")
                                        .disabled(blocked.is_some())
                                        .h(px(TWO_ROWS_PX))
                                        .px(px(FIELD_X_PADDING_PX))
                                        .font_family(theme::CODE_FONT_FAMILY)
                                        .text_size(px(FIELD_TEXT_PX)),
                                ),
                        )
                    })
                    .child(page_button(
                        "desktop-bruh-send",
                        "Send",
                        WebButtonVariant::Primary,
                        blocked.is_some() || empty,
                        cx,
                        |this, window, cx| this.send_bruh_chat(window, cx),
                    )),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("desktop-bruh-transcript")
                            .debug_selector(|| "desktop-bruh-transcript".to_string())
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
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .border_t_1()
                    .border_color(theme::longan_800().opacity(HAIRLINE_OPACITY))
                    .child(composer),
            )
    }

    fn render_bruh_line(
        &self,
        line: TranscriptItem,
        look: &BruhLook,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        match line {
            TranscriptItem::Owner { text, pending, .. } => div()
                .min_w_0()
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .child(
                    bubble(layout.bubble())
                        .bg(theme::longan_800())
                        .text_color(theme::rambutan_100())
                        .child(text),
                )
                .when(pending, |el| {
                    el.child(small_text("Waiting for a reply", theme::rambutan_500()))
                })
                .into_any_element(),
            TranscriptItem::Bruh {
                text,
                trigger,
                failed,
                ..
            } => div()
                .min_w_0()
                .flex()
                .items_start()
                .gap_2()
                .child(div().flex_none().child(braille_face(
                    RAIL_FACE_PX,
                    &still_view(look, BruhStatus::Working),
                )))
                .child(
                    div()
                        .min_w_0()
                        .max_w(px(layout.bubble()))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when_some(trigger, |el, trigger| {
                            el.child(small_text(trigger, theme::rambutan_500()))
                        })
                        .child(
                            bubble(layout.bubble())
                                .map(|el| {
                                    if failed {
                                        el.bg(theme::lychee_500().opacity(ERROR_FILL_OPACITY))
                                            .text_color(theme::lychee_200())
                                    } else {
                                        el.bg(theme::longan_900())
                                            .border_1()
                                            .border_color(
                                                theme::longan_700().opacity(SOFT_RING_OPACITY),
                                            )
                                            .text_color(theme::rambutan_200())
                                    }
                                })
                                .child(text),
                        ),
                )
                .into_any_element(),
            TranscriptItem::Card { item } if item.approval.is_some() => {
                self.render_bruh_approval(&item, layout, cx)
            }
            TranscriptItem::Card { item } => self.render_bruh_question(&item, layout, cx),
        }
    }

    fn render_bruh_card(&self, id: &str, aria: &'static str, layout: Layout) -> Stateful<Div> {
        div()
            .id(SharedString::from(format!("desktop-bruh-card-{id}")))
            .role(Role::Article)
            .aria_label(aria)
            .min_w_0()
            .max_w(px(layout.bubble()))
            .p_3()
            .rounded(px(RADIUS_GRAPE_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_700())
            .flex()
            .flex_col()
            .gap_2()
    }

    fn render_bruh_approval(
        &self,
        item: &OutboxItem,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let summary = item
            .approval
            .as_ref()
            .map(|approval| approval.summary.clone())
            .unwrap_or_default();
        let card = self
            .render_bruh_card(&item.id, "Approval", layout)
            .child(small_text("Wants to", theme::rambutan_400()))
            .child(
                div()
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(BODY_TEXT_PX))
                    .text_color(theme::rambutan_100())
                    .child(summary),
            );
        let card = match approval_outcome(item) {
            Some(outcome) => card.child(small_text(outcome, theme::rambutan_400())),
            None => {
                let disabled = self.crew_disabled();
                let (approve, deny) = (item.id.clone(), item.id.clone());
                card.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(page_button(
                            format!("desktop-bruh-approve-{}", item.id),
                            "Approve",
                            WebButtonVariant::Primary,
                            disabled,
                            cx,
                            move |this, _, cx| this.decide_bruh_approval(&approve, true, cx),
                        ))
                        .child(page_button(
                            format!("desktop-bruh-deny-{}", item.id),
                            "Deny",
                            WebButtonVariant::Outline,
                            disabled,
                            cx,
                            move |this, _, cx| this.decide_bruh_approval(&deny, false, cx),
                        ))
                        .when_some(self.crew_reason(), |el, reason| {
                            el.child(small_text(reason, theme::rambutan_400()))
                        }),
                )
            }
        };
        card.into_any_element()
    }

    fn render_bruh_question(
        &self,
        item: &OutboxItem,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let card = self
            .render_bruh_card(&item.id, "Question", layout)
            .child(
                div()
                    .font_family(theme::UI_FONT_FAMILY)
                    .text_size(px(SMALL_TEXT_PX))
                    .text_color(theme::rambutan_100())
                    .child(item.title.clone()),
            )
            .when(!item.message.trim().is_empty(), |el| {
                el.child(
                    div()
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(SMALL_TEXT_PX))
                        .text_color(theme::rambutan_300())
                        .child(item.message.clone()),
                )
            });
        if item.resolved_at.is_some() {
            return match &item.answer {
                Some(answer) => card.child(
                    div()
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(SMALL_TEXT_PX))
                        .text_color(theme::rambutan_400())
                        .child(format!("You answered: {answer}")),
                ),
                None => card.child(small_text("Dismissed", theme::rambutan_400())),
            }
            .into_any_element();
        }
        let Some((input, _)) = self
            .bruh
            .as_ref()
            .and_then(|page| page.replies.get(&item.id))
        else {
            return card.into_any_element();
        };
        let empty = value(input, cx).trim().is_empty();
        let id = item.id.clone();
        card.child(
            Input::new(input)
                .aria_label(format!("Reply to {}", item.title))
                .h(px(TWO_ROWS_PX))
                .px(px(FIELD_X_PADDING_PX))
                .font_family(theme::CODE_FONT_FAMILY)
                .text_size(px(FIELD_TEXT_PX)),
        )
        .child(div().flex().child(page_button(
            format!("desktop-bruh-reply-{}", item.id),
            "Reply",
            WebButtonVariant::Primary,
            self.busy || empty,
            cx,
            move |this, window, cx| this.answer_bruh_question(&id, window, cx),
        )))
        .into_any_element()
    }

    /// Routines: this bruh's automations, and new ones that run as it.
    fn render_bruh_routines(&self, agent_id: &str, cx: &Context<'_, Self>) -> Div {
        let disabled = self.busy || self.snapshot.is_none();
        let editor = match &self.editor {
            Some(Editor::Automation(editor)) if editor.locked_agent() == Some(agent_id) => {
                self.render_editor(cx)
            }
            _ => None,
        };
        let new_for = agent_id.to_string();
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .when_some(self.error.clone(), |el, message| {
                el.child(alert("desktop-bruh-error", message))
            })
            .child(
                div().flex().child(
                    control("desktop-bruh-new-routine", "New automation", disabled)
                        .on_activate(cx, move |this, window, cx| {
                            this.open_routine(None, &new_for, window, cx)
                        }),
                ),
            )
            .children(editor)
            .child(self.render_automation_cards_for(Some(agent_id), cx))
    }

    fn render_bruh_settings(&self, layout: Layout, cx: &Context<'_, Self>) -> Div {
        let Some(form) = self.bruh.as_ref().and_then(|page| page.settings.as_ref()) else {
            return div();
        };
        let saved = self
            .bruh_page_agent()
            .and_then(|agent_id| current_settings(self.snapshot.as_ref()?, agent_id));
        let edited = form.edited(cx);
        let dirty = saved.is_some_and(|saved| saved != edited);
        let section = |heading_text: &'static str| {
            let heading_id = SharedString::from(format!("desktop-bruh-section-{heading_text}"));
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_3()
                .p_3()
                .rounded(px(RADIUS_GRAPE_PX))
                .bg(theme::longan_950().opacity(SECTION_FILL_OPACITY))
                .border_1()
                .border_color(theme::longan_700().opacity(SOFT_RING_OPACITY))
                .child(
                    div()
                        .id(heading_id)
                        .role(Role::Heading)
                        .font_family(theme::UI_FONT_FAMILY)
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(SMALL_TEXT_PX))
                        .text_color(theme::rambutan_100())
                        .child(heading_text),
                )
        };
        let who = section("Who it is")
            .child(labelled(
                "Name",
                div()
                    .debug_selector(|| "desktop-bruh-settings-name".to_string())
                    .child(text_field(&form.name, "Name", false)),
            ))
            .child(labelled(
                "Job",
                Input::new(&form.job)
                    .aria_label("Job")
                    .h(px(FIVE_ROWS_PX))
                    .px(px(FIELD_X_PADDING_PX))
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(FIELD_TEXT_PX)),
            ));
        let model = labelled(
            "Model",
            div()
                .debug_selector(|| "desktop-bruh-settings-model".to_string())
                .child(text_field(&form.model, "Model", false)),
        );
        let effort = labelled("Effort", self.render_effort_dropdown(form, cx));
        // Web `sm:grid-cols-2`.
        let run = section("How it runs").child(if layout.column >= SM_BREAKPOINT_PX {
            div()
                .min_w_0()
                .flex()
                .gap_3()
                .child(div().min_w_0().flex_1().child(model))
                .child(div().min_w_0().flex_1().child(effort))
        } else {
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_3()
                .child(model)
                .child(effort)
        });
        let options = access_options(&edited.model);
        let mut reasons: Vec<&'static str> =
            options.iter().filter_map(|(_, reason)| *reason).collect();
        reasons.dedup();
        let full = form.level == AccessLevel::Full;
        let access = section("What it may do")
            .child(
                div()
                    .id("desktop-bruh-access")
                    .role(Role::RadioGroup)
                    .aria_label("Access")
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .p(px(SEGMENT_GROUP_PADDING_PX))
                    .rounded(px(RADIUS_GRAPE_PX))
                    .bg(theme::longan_900())
                    .border_1()
                    .border_color(theme::longan_800())
                    .children(options.into_iter().map(|(option, reason)| {
                        let chosen = form.level == option;
                        let selector = format!(
                            "desktop-bruh-access-{}",
                            option.label().to_lowercase().replace(' ', "-")
                        );
                        let segment = div()
                            .id(SharedString::from(selector.clone()))
                            .debug_selector(move || selector)
                            .role(Role::RadioButton)
                            .aria_label(option.label())
                            .aria_toggled(chosen.into())
                            .flex_1()
                            .min_h(px(theme::CONTROL_PX))
                            .px_3()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(RADIUS_SEED_PX))
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_size(px(LABEL_TEXT_PX))
                            .map(|el| {
                                if chosen {
                                    el.bg(theme::longan_700()).text_color(theme::rambutan_100())
                                } else {
                                    el.text_color(theme::rambutan_400())
                                }
                            })
                            .child(option.label());
                        if reason.is_some() {
                            segment.opacity(DISABLED_OPACITY).cursor_not_allowed()
                        } else {
                            press(
                                segment
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                    .when(!chosen, |el| {
                                        el.hover(|el| el.text_color(theme::rambutan_200()))
                                    }),
                                cx,
                                move |this, _, cx| {
                                    this.edit_bruh_settings(
                                        |form| {
                                            form.level = option;
                                            if option != AccessLevel::Full {
                                                form.can_deliver = false;
                                            }
                                        },
                                        cx,
                                    )
                                },
                            )
                        }
                    })),
            )
            .children(reasons.into_iter().map(hint))
            .child(
                div()
                    .min_h(px(theme::CONTROL_PX))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(small_text("Send as me", theme::rambutan_300()))
                    .child(page_switch(
                        "desktop-bruh-send-as-me",
                        "Send as me",
                        form.send_as_me,
                        false,
                        cx,
                        |this, _, cx| {
                            this.edit_bruh_settings(|form| form.send_as_me = !form.send_as_me, cx)
                        },
                    )),
            )
            .child(
                div()
                    .min_h(px(theme::CONTROL_PX))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(small_text("Can deliver", theme::rambutan_300()))
                            .when(!full, |el| el.child(hint(DELIVER_NEEDS_FULL))),
                    )
                    .child(page_switch(
                        "desktop-bruh-can-deliver",
                        "Can deliver",
                        form.can_deliver,
                        !full,
                        cx,
                        |this, _, cx| {
                            this.edit_bruh_settings(|form| form.can_deliver = !form.can_deliver, cx)
                        },
                    )),
            );
        let face = section("Face").child(self.render_face_editor(&form.look, cx));
        div()
            .debug_selector(|| "desktop-bruh-settings-form".to_string())
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(who)
            .child(run)
            .child(access)
            .child(face)
            .when_some(form.invalid.clone(), |el, message| {
                el.child(alert("desktop-bruh-settings-error", message))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(page_button(
                        "desktop-bruh-save",
                        "Save",
                        WebButtonVariant::Primary,
                        !dirty || self.crew_disabled(),
                        cx,
                        |this, _, cx| this.save_bruh_settings(cx),
                    ))
                    .when_some(self.crew_reason(), |el, reason| {
                        el.child(small_text(reason, theme::rambutan_400()))
                    }),
            )
    }

    fn render_effort_dropdown(&self, form: &SettingsForm, cx: &Context<'_, Self>) -> AnyElement {
        let entity = cx.entity().downgrade();
        let toggle = entity.clone();
        let on_toggle: ToggleHandler = Rc::new(move |_, cx| {
            let _ = toggle.update(cx, |this, cx| {
                this.edit_bruh_settings(|form| form.effort_open = !form.effort_open, cx);
            });
        });
        let on_select: SelectHandler = Rc::new(move |choice, _, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.edit_bruh_settings(
                    |form| {
                        form.effort = Some(choice.to_string()).filter(|choice| !choice.is_empty());
                        form.effort_open = false;
                    },
                    cx,
                );
            });
        });
        let options = std::iter::once(DropdownOption {
            value: "",
            label: "Default".into(),
            description: None,
        })
        .chain(CLAUDE_EFFORTS.iter().map(|level| DropdownOption {
            value: level,
            label: (*level).to_string(),
            description: None,
        }))
        .collect();
        dropdown(
            EFFORT_DROPDOWN,
            "Effort",
            form.effort.as_deref().unwrap_or_default(),
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            Listbox::open_if(form.effort_open),
            on_toggle,
            on_select,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::setup::{with_agent_fields, AgentFields};
    use super::{
        approval_outcome, clamp_level, composer_block, job_line, line_key, may_have_more, BruhTab,
        Editor, ReactiveAutomations, RunsPage,
    };
    use crate::theme;
    use bruhs_models::crew::{
        AccessLevel, BruhAccess, BruhColor, BruhLook, BruhState, BruhStatus, TranscriptItem,
        CREW_PROTOCOL,
    };
    use bruhs_models::reactive::{
        OutboxItem, ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveRun, ReactiveSnapshot,
        RunStatus, HISTORY_LIMIT,
    };
    use bruhs_models::EpochMs;
    use gpui::{AppContext as _, Entity, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn agent() -> ReactiveAgent {
        ReactiveAgent {
            id: "a".into(),
            name: "Ada".into(),
            instructions: "\n  Watch bills.\nPay nothing.".into(),
            memory: String::new(),
            model: "claude-sonnet-4-6".into(),
            project_path: "/tmp/bruhs/a/work".into(),
            enabled: false,
        }
    }

    fn state(status: BruhStatus) -> BruhState {
        BruhState {
            agent_id: "a".into(),
            name: "Ada".into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status,
            next_wake_ms: None,
            unread: 0,
        }
    }

    fn ask(id: &str, approval: bool) -> OutboxItem {
        let mut item = serde_json::json!({
            "id": id, "agent_id": "a", "run_id": "r", "kind": "ask",
            "title": "Which account?", "message": "Pick one.", "created_at": 1,
        });
        if approval {
            item["approval"] = serde_json::json!({
                "tool": "SandboxNetworkAccess", "summary": "Reach example.com",
                "fingerprint": "f", "state": "waiting",
            });
        }
        serde_json::from_value(item).unwrap()
    }

    fn run(id: &str, status: &str) -> ReactiveRun {
        serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": "a", "source_id": null, "subscription_id": null,
            "event_id": null, "status": status, "prompt": id, "thread_id": "t",
            "session_id": null, "created_at": 1, "updated_at": 1,
            "not_before": 1, "result": null, "error": null,
        }))
        .unwrap()
    }

    fn snapshot(status: BruhStatus, outbox: Vec<OutboxItem>) -> ReactiveSnapshot {
        ReactiveSnapshot {
            agents: vec![agent()],
            bruhs: vec![state(status)],
            outbox,
            protocol: CREW_PROTOCOL,
            ..ReactiveSnapshot::default()
        }
    }

    /// A standalone panel window on `agent`'s page (F13).
    fn open(
        cx: &mut gpui::TestAppContext,
        snapshot: Option<ReactiveSnapshot>,
        tab: BruhTab,
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
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update(cx, |panel, cx| {
            panel.loaded = true;
            if let Some(snapshot) = snapshot {
                panel.set_snapshot(snapshot, cx);
            }
            panel.open_bruh_for_test("a", tab, cx);
        });
        cx.run_until_parked();
        (panel, cx)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).expect(selector);
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    fn sent(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
    ) -> Vec<ReactiveCommand> {
        panel.update(cx, |panel, _| panel.sent_for_test().to_vec())
    }

    #[gpui::test]
    fn a_look_only_edit_sends_only_set_bruh(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Asleep, Vec::new())),
            BruhTab::Settings,
        );
        click(cx, "desktop-bruh-face-swatch-Pandan");
        click(cx, "desktop-bruh-save");
        assert_eq!(
            sent(&panel, cx),
            vec![ReactiveCommand::SetBruh {
                agent_id: "a".into(),
                look: BruhLook {
                    face: BruhColor::Pandan,
                    ..BruhLook::default()
                },
                access: BruhAccess::default(),
                effort: None,
            }]
        );
    }

    #[gpui::test]
    fn approval_cards_decide_inline(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::NeedsYou, vec![ask("i1", true)])),
            BruhTab::Chat,
        );
        assert!(cx.debug_bounds("desktop-bruh-deny-i1").is_some());
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
    fn stop_on_the_page_stops_only_this_bruh(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Working, Vec::new())),
            BruhTab::Chat,
        );
        click(cx, "desktop-bruh-stop");
        assert_eq!(
            sent(&panel, cx),
            vec![ReactiveCommand::StopBruh {
                agent_id: "a".into()
            }]
        );
    }

    #[gpui::test]
    fn a_paused_bruh_resumes_and_cannot_chat(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Paused, Vec::new())),
            BruhTab::Chat,
        );
        panel.update_in(cx, |panel, window, cx| {
            let message = panel.bruh.as_ref().unwrap().message.clone().unwrap();
            message.update(cx, |input, cx| input.set_value("hi", window, cx));
            panel.send_bruh_chat(window, cx);
        });
        assert!(sent(&panel, cx).is_empty(), "a paused bruh can't chat");
        click(cx, "desktop-bruh-stop");
        assert_eq!(
            sent(&panel, cx),
            vec![ReactiveCommand::ResumeBruh {
                agent_id: "a".into()
            }]
        );
    }

    #[gpui::test]
    fn the_composer_sends_a_chat_and_clears(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Asleep, Vec::new())),
            BruhTab::Chat,
        );
        let message = panel.update(cx, |panel, _| {
            panel.bruh.as_ref().unwrap().message.clone().unwrap()
        });
        panel.update_in(cx, |_, window, cx| {
            message.update(cx, |input, cx| {
                input.set_value("Pay the water bill", window, cx)
            });
            cx.notify();
        });
        cx.run_until_parked();
        click(cx, "desktop-bruh-send");
        assert_eq!(
            sent(&panel, cx),
            vec![ReactiveCommand::Chat {
                agent_id: "a".into(),
                message: "Pay the water bill".into(),
            }]
        );
        panel.update(cx, |_, cx| {
            assert_eq!(message.read(cx).value().as_ref(), "")
        });
    }

    #[gpui::test]
    fn an_older_service_keeps_stop_save_and_approvals_off(cx: &mut gpui::TestAppContext) {
        let mut old = snapshot(BruhStatus::NeedsYou, vec![ask("i1", true)]);
        old.protocol = 0;
        let (panel, cx) = open(cx, Some(old), BruhTab::Chat);
        click(cx, "desktop-bruh-stop");
        click(cx, "desktop-bruh-approve-i1");
        panel.update(cx, |panel, cx| panel.set_bruh_tab(BruhTab::Settings, cx));
        cx.run_until_parked();
        click(cx, "desktop-bruh-face-swatch-Pandan");
        click(cx, "desktop-bruh-save");
        assert!(sent(&panel, cx).is_empty());
    }

    #[gpui::test]
    fn a_failed_load_shows_the_error_with_retry(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, None, BruhTab::Chat);
        assert!(cx.debug_bounds("desktop-bruh-loading").is_some());
        panel.update(cx, |panel, cx| {
            panel.load_error = Some("The local server isn't answering.".into());
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-bruh-loading").is_none());
        assert!(cx.debug_bounds("desktop-bruh-load-error").is_some());
        assert!(cx.debug_bounds("desktop-bruh-retry").is_some());
    }

    #[gpui::test]
    fn a_missing_bruh_offers_agent_workflows(cx: &mut gpui::TestAppContext) {
        let mut gone = snapshot(BruhStatus::Asleep, Vec::new());
        gone.bruhs.clear();
        let (_panel, cx) = open(cx, Some(gone), BruhTab::Chat);
        assert!(cx.debug_bounds("desktop-bruh-open-workflows").is_some());
        assert!(cx.debug_bounds("desktop-bruh-stop").is_none());
    }

    #[gpui::test]
    fn settings_drop_ask_to_read_only_for_a_codex_model_at_save(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Asleep, Vec::new())),
            BruhTab::Settings,
        );
        click(cx, "desktop-bruh-access-ask-first");
        panel.update_in(cx, |panel, window, cx| {
            let model = panel.bruh_settings_mut().unwrap().model.clone();
            model.update(cx, |input, cx| input.set_value("gpt-5.1-codex", window, cx));
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("desktop-bruh-access-ask-first").is_some(),
            "Ask stays drawn, off, until the model is committed"
        );
        click(cx, "desktop-bruh-save");
        let sent = sent(&panel, cx);
        assert!(
            matches!(
                &sent[..],
                [
                    ReactiveCommand::Apply {
                        change: ReactiveChange::SaveAgent { agent }
                    }
                ] if agent.model == "gpt-5.1-codex" && agent.name == "Ada"
            ),
            "{sent:?}"
        );
        panel.update(cx, |panel, _| {
            assert_eq!(
                panel.bruh_settings_mut().unwrap().level,
                AccessLevel::ReadOnly
            );
        });
    }

    #[gpui::test]
    fn routines_list_only_this_bruh_and_never_rewrite_it(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Asleep, Vec::new())),
            BruhTab::Routines,
        );
        click(cx, "reactive-New automation");
        // The agent fields give way to a note: the routine runs as this bruh.
        assert!(cx.debug_bounds("automation-runs-as-bruh").is_some());
        panel.update_in(cx, |panel, window, cx| {
            let Some(Editor::Automation(editor)) = &mut panel.editor else {
                panic!("automation editor");
            };
            assert_eq!(editor.locked_agent(), Some("a"));
            editor
                .source_mut()
                .value
                .update(cx, |input, cx| input.set_value("/tmp/scans", window, cx));
            panel.review_automation(cx);
            let Some(Editor::Automation(editor)) = &panel.editor else {
                panic!("automation editor");
            };
            let review = editor.review().expect("a reviewable routine");
            assert_eq!(review.agent, agent(), "the bruh is sent exactly as saved");
            assert_eq!(review.subscription.agent_id, "a");
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("reactive-Enable automation").is_some());
    }

    #[test]
    fn a_locked_routine_keeps_the_agent_and_a_free_one_edits_it() {
        let fields = || AgentFields {
            name: "Renamed".into(),
            purpose: "A new job".into(),
            folder: "/elsewhere".into(),
            model: "claude-sonnet-4-6".into(),
            memory: "new memory".into(),
        };
        assert_eq!(with_agent_fields(agent(), fields(), true), Ok(agent()));
        let free = with_agent_fields(agent(), fields(), false).unwrap();
        assert_eq!(
            (free.name.as_str(), free.project_path.as_str(), free.enabled),
            ("Renamed", "/elsewhere", true)
        );
        let blank = AgentFields {
            name: String::new(),
            ..fields()
        };
        assert!(with_agent_fields(agent(), blank, false).is_err());
    }

    #[test]
    fn the_job_is_the_first_line_with_words() {
        assert_eq!(job_line("\n  Watch bills.\nPay nothing."), "Watch bills.");
        assert_eq!(job_line(""), "");
    }

    #[test]
    fn a_newer_page_updates_runs_in_place_and_older_pages_extend_them() {
        let mut page = RunsPage::default();
        page.take_newest(vec![run("r2", "running"), run("r1", "completed")]);
        assert_eq!(page.oldest.as_deref(), Some("r1"));
        assert!(!page.more);
        page.take_newest(vec![run("r3", "queued"), run("r2", "completed")]);
        let got: Vec<(&str, RunStatus)> = page
            .runs
            .iter()
            .map(|run| (run.id.as_str(), run.status))
            .collect();
        assert_eq!(
            got,
            vec![
                ("r2", RunStatus::Completed),
                ("r1", RunStatus::Completed),
                ("r3", RunStatus::Queued),
            ]
        );
        assert_eq!(
            page.oldest.as_deref(),
            Some("r1"),
            "the newest page keeps the cursor"
        );
        page.take_earlier(vec![run("r0", "completed")]);
        assert_eq!(page.oldest.as_deref(), Some("r0"));
        assert!(may_have_more(HISTORY_LIMIT) && !may_have_more(HISTORY_LIMIT - 1));
    }

    #[test]
    fn the_composer_says_why_a_paused_or_off_bruh_cannot_chat() {
        assert!(composer_block(Some(BruhStatus::Paused))
            .is_some_and(|reason| reason.contains("paused")));
        assert!(composer_block(Some(BruhStatus::Off)).is_some_and(|reason| reason.contains("off")));
        for status in [
            BruhStatus::Asleep,
            BruhStatus::Working,
            BruhStatus::NeedsYou,
        ] {
            assert_eq!(composer_block(Some(status)), None);
        }
        assert_eq!(composer_block(None), None);
    }

    #[test]
    fn a_decided_approval_says_how_and_an_open_one_waits() {
        let mut item = ask("i", true);
        assert_eq!(approval_outcome(&item), None);
        item.resolved_at = Some(EpochMs::saturating_from_i64(2));
        assert_eq!(approval_outcome(&item), Some("Withdrawn"));
        if let Some(approval) = item.approval.as_mut() {
            approval.state = bruhs_models::crew::ApprovalState::Approved;
        }
        assert_eq!(approval_outcome(&item), Some("Approved"));
        let open = line_key(&TranscriptItem::Card { item: item.clone() });
        item.resolved_at = None;
        assert_ne!(open, line_key(&TranscriptItem::Card { item }));
    }

    #[test]
    fn a_model_that_cannot_ask_drops_ask_to_read_only() {
        assert_eq!(
            clamp_level(AccessLevel::Ask, "gpt-5.1-codex"),
            AccessLevel::ReadOnly
        );
        assert_eq!(
            clamp_level(AccessLevel::Full, "gpt-5.1-codex"),
            AccessLevel::Full
        );
        assert_eq!(
            clamp_level(AccessLevel::Ask, "claude-sonnet-4-6"),
            AccessLevel::Ask
        );
    }

    #[gpui::test]
    fn the_transcript_shows_runs_and_question_cards(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::NeedsYou, vec![ask("q1", false)])),
            BruhTab::Chat,
        );
        let mut answered = run("r1", "completed");
        answered.result = Some("Paid.".into());
        panel.update(cx, |panel, cx| {
            panel.show_bruh_runs_for_test(vec![answered]);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-bruh-reply-q1").is_some());
        let card = cx.debug_bounds("desktop-bruh-reply-q1").unwrap();
        let log = cx.debug_bounds("desktop-bruh-transcript").unwrap();
        assert!(
            log.contains(&card.center()),
            "the card sits in the transcript"
        );
        panel.update_in(cx, |panel, window, cx| {
            let input = panel.bruh.as_ref().unwrap().replies["q1"].0.clone();
            input.update(cx, |input, cx| input.set_value("Checking", window, cx));
            cx.notify();
        });
        cx.run_until_parked();
        click(cx, "desktop-bruh-reply-q1");
        assert_eq!(
            sent(&panel, cx),
            vec![ReactiveCommand::AnswerOutbox {
                id: "q1".into(),
                message: "Checking".into(),
            }]
        );
    }
}
