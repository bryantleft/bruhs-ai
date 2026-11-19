//! A bruh's own page (web `crew/page.rs`, `crew/chat.rs` and
//! `crew/settings.rs`): its face, name, job and status with Stop or Resume,
//! and tabs for its chat, memory, routines and settings. The workspace shows
//! the panel for `AppView::Bruh` and tells it which bruh; the panel owns the
//! page's state. Save sends `settings_commands`, so a face edit never
//! touches the agent.
use super::computer_view::ComputerView;
use super::create_bruh::{
    hint, labelled, text_field, ERROR_FILL_OPACITY, FIELD_RADIUS_PX, FIELD_TEXT_PX,
    FIELD_X_PADDING_PX,
};
use super::face_editor::still_view;
use super::transcript_list::{transcript_view, TranscriptList};
use super::vault_grants::{toggle, vault_grants, VaultGrantsState};
use super::*;
use crate::views::braille::braille_face;
use crate::views::chat::{desktop_markdown_style, untrusted_markdown};
use crate::views::crew_face::bruh_face;
use crate::views::dropdown::{
    dropdown, DropdownGroup, DropdownOption, Listbox, SelectHandler, ToggleHandler,
};
use bruhs_models::agent::{AgentProvider, ModelId};
use bruhs_models::crew::room::{APPROVAL_WANTS, LOAD_EARLIER};
use bruhs_models::crew::{
    access_options, approval_outcome, chat_follow_key, chat_markdown, clamp_level, composer_block,
    computer_switch_follows, computer_switch_off, computer_switch_reason, current_settings,
    job_line, line_key, may_have_more, merge_runs, settings_commands_for, status_line, transcript,
    vault_grants_follow, vault_unavailable, AccessLevel, BruhAccess, BruhLook, BruhSettings,
    BruhStatus, TranscriptItem, COMPUTER_HINT, COMPUTER_SWITCH, COMPUTER_TAB, PAGE_FACE_PX,
    RAIL_FACE_PX, SEND_AS_ME_HINT,
};
use bruhs_models::logic::rich_response::markdown_with_allowed_images;
use bruhs_models::logic::run_place::CLAUDE_EFFORTS;
use bruhs_ui::press::on_press_view;
use bruhs_ui::scroll::ScrollableElement;
use bruhs_ui::{press::Interactivity, switch::switch, text_field::TextInput};
use std::cell::Cell;

/// Web `text-h3`.
pub(super) const TITLE_TEXT_PX: f32 = 18.0;
/// Web `text-body`.
pub(super) const BODY_TEXT_PX: f32 = 14.0;
/// Web `text-body-sm` and `text-label`.
pub(super) const SMALL_TEXT_PX: f32 = 13.0;
/// Web `text-label-sm`.
pub(super) const LABEL_TEXT_PX: f32 = 12.0;
/// Web `max-w-3xl`.
const COLUMN_MAX_PX: f32 = 768.0;
/// Web `p-4`, and `sm:px-6` from the `sm` breakpoint.
pub(super) const PAGE_PADDING_PX: f32 = 16.0;
/// Web `gap-3` between a transcript's lines.
pub(super) const TRANSCRIPT_GAP_PX: f32 = 12.0;
const PAGE_WIDE_PADDING_PX: f32 = 24.0;
const SM_BREAKPOINT_PX: f32 = 640.0;
/// Web `max-w-[80%]` on chat bubbles and cards.
const BUBBLE_SHARE: f32 = 0.8;
pub(super) const RADIUS_GRAPE_PX: f32 = 8.0;
/// Web `max-h-64` on an approval's full request.
const DETAIL_MAX_PX: f32 = 256.0;
const RADIUS_SEED_PX: f32 = 4.0;
/// Web `border-longan-800/50` hairlines.
pub(super) const HAIRLINE_OPACITY: f32 = 0.5;
/// Web `inset-ring-longan-700/50` on bruh bubbles and settings sections.
pub(super) const SOFT_RING_OPACITY: f32 = 0.5;
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
const DISABLED_OPACITY: f32 = 0.5;
const EFFORT_DROPDOWN: &str = "desktop-bruh-effort";
const DELIVER_NEEDS_FULL: &str = "Needs Full access.";

/// The page's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BruhTab {
    Chat,
    Computer,
    Memory,
    Routines,
    Settings,
}

impl BruhTab {
    const ALL: [BruhTab; 5] = [
        BruhTab::Chat,
        BruhTab::Computer,
        BruhTab::Memory,
        BruhTab::Routines,
        BruhTab::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            BruhTab::Chat => "Chat",
            BruhTab::Computer => COMPUTER_TAB,
            BruhTab::Memory => "Memory",
            BruhTab::Routines => "Routines",
            BruhTab::Settings => "Settings",
        }
    }

    fn key(self) -> &'static str {
        match self {
            BruhTab::Chat => "chat",
            BruhTab::Computer => "computer",
            BruhTab::Memory => "memory",
            BruhTab::Routines => "routines",
            BruhTab::Settings => "settings",
        }
    }
}

/// "Open Agent workflows" on the page of a bruh that no longer exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactiveOpenAutomations;

/// An open question card waits for a typed reply.
fn open_question(item: &OutboxItem, agent_id: &str) -> bool {
    item.agent_id == agent_id
        && item.kind == OutboxKind::Ask
        && item.approval.is_none()
        && item.resolved_at.is_none()
}

/// The runs the chat shows, newest page first, then older pages.
#[derive(Default)]
struct RunsPage {
    runs: Vec<ReactiveRun>,
    /// The last run shown; older pages start after it.
    oldest: Option<String>,
    more: bool,
    error: Option<String>,
    /// Bumped whenever `runs` changes, so the transcript is built again.
    version: u64,
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
        self.version = self.version.wrapping_add(1);
    }

    /// An older page, from Load earlier.
    fn take_earlier(&mut self, page: Vec<ReactiveRun>) {
        self.more = may_have_more(page.len());
        if let Some(last) = page.last() {
            self.oldest = Some(last.id.clone());
        }
        merge_runs(&mut self.runs, page);
        self.version = self.version.wrapping_add(1);
    }
}

/// The settings form, filled from the service unless the owner has unsaved
/// edits.
struct SettingsForm {
    /// What the form was last filled from.
    shown: BruhSettings,
    name: Entity<InputState>,
    job: Entity<TextareaState>,
    model: Entity<InputState>,
    effort: Option<String>,
    effort_open: bool,
    level: AccessLevel,
    send_as_me: bool,
    can_deliver: bool,
    computer: bool,
    /// The Vault section's picks. Save sends SetVault only when they
    /// differ from the stored grants ([`Self::sync_vault`]).
    vault: Vec<String>,
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
        let name = edit_input(&settings.name, window, cx);
        let job = edit_area(&settings.instructions, window, cx);
        let model = edit_input(&settings.model, window, cx);
        let redraw = |event: &InputEvent, cx: &mut Context<'_, ReactiveAutomations>| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&name, window, move |_, _, event, _, cx| redraw(event, cx)),
            cx.subscribe_in(&job, window, move |_, _, event, _, cx| redraw(event, cx)),
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
            computer: settings.computer,
            vault: settings.vault.clone(),
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
            computer: self.computer,
            vault: self.vault.clone(),
        }
    }

    fn fill(&mut self, settings: &BruhSettings, window: &mut Window, cx: &mut App) {
        let fields: [(&dyn TextInput, &String); 3] = [
            (&self.name, &settings.name),
            (&self.job, &settings.instructions),
            (&self.model, &settings.model),
        ];
        for (input, text) in fields {
            if *input.text_value(cx) != **text {
                input.set_text_value(text.clone().into(), window, cx);
            }
        }
        self.effort.clone_from(&settings.effort);
        self.level = settings.access.level;
        self.send_as_me = settings.access.send_as_me;
        self.can_deliver = settings.access.can_deliver;
        self.computer = settings.computer;
        self.vault.clone_from(&settings.vault);
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

    /// Why the Computer switch can't work, and whether it is off: the
    /// shared reason and rule (`computer_switch_reason`,
    /// `computer_switch_off`).
    fn computer_block(
        &self,
        protocol: Option<u32>,
        unsupported: bool,
        cx: &App,
    ) -> (Option<&'static str>, bool) {
        let provider = AgentProvider::for_model(&value(&self.model, cx));
        (
            computer_switch_reason(self.level, provider, protocol, unsupported),
            computer_switch_off(self.level, provider, protocol),
        )
    }

    /// The switch follows the stored value where the owner can't or didn't
    /// change it (`computer_switch_follows`).
    fn sync_computer(&mut self, stored: bool, protocol: Option<u32>, cx: &App) {
        let (_, off) = self.computer_block(protocol, false, cx);
        if let Some(stored) =
            computer_switch_follows(self.computer, self.shown.computer, stored, off)
        {
            self.computer = stored;
            self.shown.computer = stored;
        }
    }

    /// Why the Vault section is off: the model and level first, then the
    /// service's `reason` (web `vault_block`).
    fn vault_block(&self, reason: Option<&'static str>, cx: &App) -> Option<&'static str> {
        let provider = AgentProvider::for_model(&value(&self.model, cx));
        vault_unavailable(self.level, provider).or(reason)
    }

    /// The grants follow the stored ones while the section is `off`, while
    /// the owner hasn't changed them since the fill, and once the picks
    /// match them (`vault_grants_follow`), so Save never puts back grants
    /// another app replaced or sends a pick the section no longer offers.
    fn sync_vault(&mut self, stored: &[String], off: bool) {
        if let Some(stored) = vault_grants_follow(&self.vault, &self.shown.vault, stored, off) {
            self.vault.clone_from(&stored);
            self.shown.vault = stored;
        }
    }
}

/// The bruh page while it is on screen.
pub(super) struct BruhPage {
    agent_id: String,
    tab: BruhTab,
    message: Option<Entity<TextareaState>>,
    runs: RunsPage,
    runs_requested: bool,
    runs_revision: u64,
    loading_earlier: bool,
    newest_request: Option<Task<()>>,
    earlier_request: Option<Task<()>>,
    /// Reply fields of open question cards, by outbox id.
    replies: HashMap<String, (Entity<TextareaState>, gpui::Subscription)>,
    settings: Option<SettingsForm>,
    list: TranscriptList,
    /// What the transcript last followed (`chat_follow_key`); a new line
    /// scrolls into view.
    followed: (Option<String>, Option<String>),
    /// The conversation, built when the snapshot or the runs change rather
    /// than on every frame (a sweeping face redraws the page often).
    lines: Rc<Vec<TranscriptItem>>,
    /// Each answer's Markdown by run id, ready to draw, built with `lines`.
    prose: HashMap<String, SharedString>,
    /// The snapshot revision and runs version `lines` was built from.
    built_for: Option<(u64, u64)>,
    /// The page's width, measured as it is drawn.
    width: Rc<Cell<f32>>,
    /// The Computer tab, while it shows: its live view watches the
    /// browser only then.
    computer: Option<Entity<ComputerView>>,
    /// The Computer tab takes control once it can (a smoke scene).
    hold_computer: bool,
    /// The vault listing behind Settings' Vault section, while it shows.
    vault: VaultGrantsState,
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
            list: TranscriptList::new(),
            followed: (None, None),
            lines: Rc::new(Vec::new()),
            prose: HashMap::new(),
            built_for: None,
            width: Rc::new(Cell::new(0.0)),
            computer: None,
            hold_computer: false,
            vault: VaultGrantsState::default(),
            _subscriptions: Vec::new(),
        }
    }
}

/// A web `Button`: handlers only while enabled.
pub(super) fn page_button(
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
        on_press_view(button, cx, action)
    }
}

/// Web `Switch`: persimmon track when on, longan with a white/10 ring off.
pub(super) fn page_switch(
    id: &'static str,
    text: impl Into<SharedString>,
    checked: bool,
    disabled: bool,
    cx: &Context<'_, ReactiveAutomations>,
    action: impl Fn(&mut ReactiveAutomations, &mut Window, &mut Context<'_, ReactiveAutomations>)
        + 'static,
) -> Stateful<Div> {
    let switch = switch(id, text, checked, Interactivity::disabled_if(disabled));
    if disabled {
        switch
    } else {
        on_press_view(switch, cx, action)
    }
}

/// An open question card's reply field, which redraws the page as it
/// changes so Reply turns on and off. A bruh's chat and a room keep one per
/// open card.
pub(super) fn reply_field(
    window: &mut Window,
    cx: &mut Context<'_, ReactiveAutomations>,
) -> (Entity<TextareaState>, gpui::Subscription) {
    let input = cx.new(|cx| TextareaState::new(window, cx));
    let watch = cx.subscribe_in(&input, window, |_, _, event: &InputEvent, _, cx| {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    });
    (input, watch)
}

pub(super) fn small_text(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .min_w_0()
        .text_size(px(LABEL_TEXT_PX))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(color)
        .child(text.into())
}

pub(super) fn alert(id: &'static str, message: impl Into<SharedString>) -> Stateful<Div> {
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

pub(super) fn bubble(max_width: f32) -> Div {
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

/// A bruh's words as the thread chat draws a reply: Markdown without raw
/// HTML, and a remote image as a link to it rather than loaded. Its links
/// follow [`untrusted_markdown`].
pub(super) fn crew_markdown(text: &str) -> SharedString {
    markdown_with_allowed_images(text, &std::collections::BTreeSet::new()).into()
}

/// A bruh's bubble holding [`crew_markdown`] in the thread chat's
/// Markdown style. `id` names it, so its parsed text survives redraws.
pub(super) fn prose_bubble(
    id: String,
    markdown: SharedString,
    layout: Layout,
    cx: &App,
) -> AnyElement {
    let selector = id.clone();
    bubble(layout.bubble())
        .debug_selector(move || selector)
        .bg(theme::longan_900())
        .border_1()
        .border_color(theme::longan_700().opacity(SOFT_RING_OPACITY))
        .text_color(theme::rambutan_200())
        .child(
            untrusted_markdown(SharedString::from(id), markdown)
                .style(desktop_markdown_style(cx))
                .selectable(true),
        )
        .into_any_element()
}

/// The page's measured layout: the centred column and its padding.
#[derive(Clone, Copy)]
pub(super) struct Layout {
    column: f32,
    pub(super) padding: f32,
    /// Web `sm:`, a media query: the window, not the page, is 640px or wider.
    wide: bool,
}

impl Layout {
    pub(super) fn of(width: f32, viewport: f32) -> Self {
        let wide = viewport >= SM_BREAKPOINT_PX;
        Self {
            column: width.min(COLUMN_MAX_PX),
            padding: if wide {
                PAGE_WIDE_PADDING_PX
            } else {
                PAGE_PADDING_PX
            },
            wide,
        }
    }

    /// The layout of a page last measured at `measured` CSS px wide (0
    /// before its first frame, when the window's width stands in).
    pub(super) fn measured(measured: f32, window: &Window) -> Self {
        let viewport = bruhs_ui::zoom::css_px(window.viewport_size().width);
        let width = if measured > 0.0 { measured } else { viewport };
        Self::of(width, viewport)
    }

    /// The content width inside the column's padding.
    fn inner(self) -> f32 {
        (self.column - self.padding * 2.0).max(0.0)
    }

    pub(super) fn bubble(self) -> f32 {
        self.inner() * BUBBLE_SHARE
    }

    /// One row of a transcript: the column's width and side padding.
    pub(super) fn row(self) -> Div {
        div()
            .w(px(self.column))
            .min_w_0()
            .flex_shrink_0()
            .px(px(self.padding))
            .flex()
            .flex_col()
    }

    /// Web `mx-auto w-full max-w-3xl min-w-0 p-4 sm:px-6`. A px width,
    /// because percentages inside a scroll container measure as indefinite.
    pub(super) fn column(self) -> Div {
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

/// Measures the page it fills into `width` as it is drawn, redrawing the
/// panel when the width changes.
pub(super) fn measure_width(
    width: Rc<Cell<f32>>,
    cx: &Context<'_, ReactiveAutomations>,
) -> impl IntoElement {
    let entity = cx.entity_id();
    canvas(
        move |bounds, _, cx| {
            let measured = bruhs_ui::zoom::css_px(bounds.size.width);
            if (width.get() - measured).abs() > 0.5 {
                width.set(measured);
                cx.notify(entity);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
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

    /// `BRUHS_DESKTOP_SMOKE_SCENE=bruh-settings:<id>`: the Settings tab.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_show_bruh_settings(&mut self, agent_id: &str, cx: &mut Context<'_, Self>) {
        self.show_bruh(Some(agent_id.to_string()), cx);
        self.set_bruh_tab(BruhTab::Settings, cx);
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE=bruh-computer:<id>`: the Computer tab;
    /// `bruh-computer-held:<id>` also takes control.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_show_bruh_computer(
        &mut self,
        agent_id: &str,
        take_control: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.show_bruh(Some(agent_id.to_string()), cx);
        self.set_bruh_tab(BruhTab::Computer, cx);
        if let Some(page) = self.bruh.as_mut() {
            page.hold_computer = take_control;
        }
    }

    /// The open page's bruh name, as the snapshot has it.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_bruh_name(&self) -> Option<String> {
        let agent_id = self.bruh_page_agent()?;
        let snapshot = self.snapshot.as_ref()?;
        let state = snapshot
            .bruhs
            .iter()
            .find(|bruh| bruh.agent_id == agent_id)?;
        Some(state.name.clone())
    }

    #[cfg(test)]
    pub(super) fn computer_view_for_test(&self) -> Option<Entity<ComputerView>> {
        self.bruh.as_ref()?.computer.clone()
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

    /// Keep the open settings' Computer switch and vault grants on the
    /// stored values where the owner can't or didn't change them.
    pub(super) fn sync_bruh_stored(&mut self, cx: &App) {
        let protocol = self.crew.protocol;
        let vault_reason = self.vault_reason();
        let Some(agent_id) = self.bruh_page_agent() else {
            return;
        };
        let Some((stored, vault)) = self
            .bruh_state(agent_id)
            .map(|state| (state.computer, state.vault.clone()))
        else {
            return;
        };
        if let Some(form) = self.bruh_settings_mut() {
            form.sync_computer(stored, protocol, cx);
            let off = form.vault_block(vault_reason, cx).is_some();
            form.sync_vault(&vault, off);
        }
    }

    #[cfg(test)]
    pub(super) fn bruh_vault(&self) -> Option<&VaultGrantsState> {
        self.bruh.as_ref().map(|page| &page.vault)
    }

    pub(super) fn bruh_vault_mut(&mut self) -> Option<&mut VaultGrantsState> {
        self.bruh.as_mut().map(|page| &mut page.vault)
    }

    /// Grant entry `id` to the open bruh, or take it back, in the form.
    pub(super) fn toggle_bruh_grant(&mut self, id: &str, on: bool, cx: &mut Context<'_, Self>) {
        self.edit_bruh_settings(|form| toggle(&mut form.vault, id, on), cx);
    }

    fn bruh_state(&self, agent_id: &str) -> Option<&BruhState> {
        self.snapshot
            .as_ref()?
            .bruhs
            .iter()
            .find(|bruh| bruh.agent_id == agent_id)
    }

    /// Why crew commands are off, once the service is known to be older.
    pub(crate) fn crew_reason(&self) -> Option<&'static str> {
        if self.crew_ready() {
            return None;
        }
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

    /// The page's inputs and data, made before it draws. The work runs only
    /// when the snapshot or the runs changed since the last frame.
    pub(super) fn ensure_bruh_page(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // Settings reads the vault listing as it opens and forgets it as
        // it closes, as the web's section does.
        match self.bruh.as_mut() {
            Some(page) if page.tab != BruhTab::Settings => page.vault.reset(),
            Some(page) if page.vault.idle() => self.load_bruh_vault(cx),
            _ => {}
        }
        let client = self.client.clone();
        let panel = cx.weak_entity();
        let Some(page) = self.bruh.as_mut() else {
            return;
        };
        if page.tab != BruhTab::Computer {
            page.computer = None;
        } else if page.computer.is_none() {
            let agent_id = page.agent_id.clone();
            let hold = std::mem::take(&mut page.hold_computer);
            page.computer = Some(cx.new(|cx| {
                let mut view = ComputerView::new(agent_id, client, panel, window, cx);
                if hold {
                    view.take_control_when_ready();
                }
                view
            }));
        }
        page.list.sync_header(format!(
            "{}:{}:{}",
            page.runs.more,
            page.runs.error.is_some(),
            page.lines.is_empty()
        ));
        let stamp = (self.snapshot_rev, page.runs.version);
        let current = page.built_for == Some(stamp) && page.message.is_some();
        let requested = page.runs_requested;
        if current {
            if !requested {
                self.reload_bruh_runs(cx);
            }
            return;
        }
        let saved = self
            .bruh_page_agent()
            .and_then(|agent_id| current_settings(self.snapshot.as_ref()?, agent_id));
        let outbox = self
            .snapshot
            .as_ref()
            .map_or(&[][..], |snapshot| snapshot.outbox.as_slice());
        let Some(page) = self.bruh.as_mut() else {
            return;
        };
        page.built_for = Some(stamp);
        if page.message.is_none() {
            let message = cx.new(|cx| {
                TextareaState::new(window, cx)
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
                page.replies
                    .insert(item.id.clone(), reply_field(window, cx));
            }
        }
        page.lines = Rc::new(transcript(&agent_id, &page.runs.runs, outbox));
        page.prose = page
            .lines
            .iter()
            .filter_map(|line| match line {
                TranscriptItem::Bruh { run_id, .. } => {
                    Some((run_id.clone(), crew_markdown(chat_markdown(line)?)))
                }
                _ => None,
            })
            .collect();
        page.list.sync(page.lines.iter().map(line_key).collect());
        let shown = chat_follow_key(&page.lines);
        if shown != page.followed {
            page.list.follow();
            page.followed = shown;
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
                run_id: None,
            }],
            Scope::General,
            cx,
        );
        message.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// Reply to question `id` with what `input` holds, from a bruh's chat
    /// or a room.
    pub(super) fn answer_question(
        &mut self,
        id: &str,
        input: Entity<TextareaState>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
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
        // A level the clamp lowered can turn the Computer switch and the
        // Vault section off.
        self.sync_bruh_stored(cx);
        let Some(page) = self.bruh.as_mut() else {
            return;
        };
        let Some(form) = page.settings.as_mut() else {
            return;
        };
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
                Some(settings_commands_for(
                    self.crew.protocol,
                    agent,
                    state,
                    &edited,
                ))
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
        let layout = Layout::measured(
            self.bruh.as_ref().map_or(0.0, |page| page.width.get()),
            window,
        );
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
                root.child(measure_width(page.width.clone(), cx))
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
                    .child(on_press_view(
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
            BruhTab::Chat => self.render_bruh_chat(state, layout, cx),
            BruhTab::Computer => div().flex_1().min_h_0().flex().flex_col().when_some(
                self.bruh.as_ref().and_then(|page| page.computer.clone()),
                |body, view| body.child(view),
            ),
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
            .debug_selector(|| "desktop-bruh-header".to_string())
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
                        on_press_view(
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

    fn render_bruh_chat(&self, state: &BruhState, layout: Layout, cx: &Context<'_, Self>) -> Div {
        let Some(page) = self.bruh.as_ref() else {
            return div();
        };
        let blocked = composer_block(Some(state.status));
        let empty = page
            .message
            .as_ref()
            .is_none_or(|message| value(message, cx).trim().is_empty());
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
                                    Textarea::new(message)
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
            .child(transcript_view(
                "desktop-bruh-transcript",
                "Conversation",
                &page.list,
                layout,
                cx,
                move |this, index, cx| {
                    let page = this.bruh.as_ref()?;
                    Some(match index.checked_sub(1) {
                        None => this.render_bruh_head(page, cx),
                        Some(at) => {
                            let look = this
                                .bruh_state(&page.agent_id)
                                .map(|state| state.look.clone())
                                .unwrap_or_default();
                            this.render_bruh_line(page.lines.get(at)?.clone(), &look, layout, cx)
                        }
                    })
                },
            ))
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

    /// What sits above the chat's lines: Load earlier, the error and the
    /// empty line.
    fn render_bruh_head(&self, page: &BruhPage, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when(page.runs.more, |el| {
                el.child(div().flex().justify_center().child(page_button(
                    "desktop-bruh-load-earlier",
                    LOAD_EARLIER,
                    WebButtonVariant::Ghost,
                    page.loading_earlier,
                    cx,
                    |this, _, cx| this.load_earlier_bruh_runs(cx),
                )))
            })
            .when_some(page.runs.error.clone(), |el, message| {
                el.child(alert("desktop-bruh-runs-error", message))
            })
            .when(page.lines.is_empty() && page.runs.error.is_none(), |el| {
                el.child(
                    div()
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(SMALL_TEXT_PX))
                        .text_color(theme::rambutan_500())
                        .child("Nothing here yet. Say hi."),
                )
            })
            .into_any_element()
    }

    fn render_bruh_line(
        &self,
        line: TranscriptItem,
        look: &BruhLook,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        match line {
            // A message from a thread's agent names its sender, as thread
            // chats do.
            TranscriptItem::Owner {
                text,
                pending,
                caption,
                ..
            } => div()
                .min_w_0()
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .when_some(caption, |el, caption| {
                    el.child(small_text(caption, theme::blueberry_400()))
                })
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
                run_id,
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
                            match self
                                .bruh
                                .as_ref()
                                .and_then(|page| page.prose.get(&run_id))
                                .filter(|_| !failed)
                            {
                                Some(markdown) => prose_bubble(
                                    format!("bruh-md-{run_id}"),
                                    markdown.clone(),
                                    layout,
                                    cx,
                                ),
                                None => bubble(layout.bubble())
                                    .bg(theme::lychee_500().opacity(ERROR_FILL_OPACITY))
                                    .text_color(theme::lychee_200())
                                    .child(text)
                                    .into_any_element(),
                            },
                        ),
                )
                .into_any_element(),
            TranscriptItem::Card { item } if item.approval.is_some() => {
                self.render_bruh_approval(&item, layout, cx)
            }
            TranscriptItem::Card { item } => {
                let reply = self
                    .bruh
                    .as_ref()
                    .and_then(|page| page.replies.get(&item.id))
                    .map(|(input, _)| input);
                self.render_bruh_question(&item, reply, layout, cx)
            }
            TranscriptItem::Review {
                run_id,
                trigger,
                prompt,
                ..
            } => self.render_bruh_review(&run_id, trigger, prompt, layout, cx),
        }
    }

    /// A run waiting for review (web `review_card`): Run starts it, Dismiss
    /// cancels it. Every service knows both commands.
    fn render_bruh_review(
        &self,
        run_id: &str,
        trigger: &'static str,
        prompt: String,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let (run, dismiss) = (run_id.to_string(), run_id.to_string());
        let prompt_id = format!("desktop-bruh-review-prompt-{run_id}");
        self.render_bruh_card(run_id, "Run waiting for review", layout)
            .child(small_text(
                format!("{trigger}: waiting for your review"),
                theme::rambutan_400(),
            ))
            .child(
                div()
                    .id(SharedString::from(prompt_id))
                    .max_h(px(DETAIL_MAX_PX))
                    .overflow_y_scroll()
                    .p_2()
                    .rounded(px(RADIUS_GRAPE_PX))
                    .bg(theme::longan_950())
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(LABEL_TEXT_PX))
                    .text_color(theme::rambutan_300())
                    .child(prompt),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(page_button(
                        format!("desktop-bruh-run-{run_id}"),
                        "Run",
                        WebButtonVariant::Primary,
                        self.busy,
                        cx,
                        move |this, _, cx| {
                            this.send_all(
                                vec![ReactiveCommand::ApproveRun { id: run.clone() }],
                                Scope::General,
                                cx,
                            );
                        },
                    ))
                    .child(page_button(
                        format!("desktop-bruh-dismiss-{run_id}"),
                        "Dismiss",
                        WebButtonVariant::Outline,
                        self.busy,
                        cx,
                        move |this, _, cx| {
                            this.send_all(
                                vec![ReactiveCommand::CancelRun {
                                    id: dismiss.clone(),
                                }],
                                Scope::General,
                                cx,
                            );
                        },
                    )),
            )
            .into_any_element()
    }

    /// A question, approval or review card at the chat's card width.
    pub(super) fn render_bruh_card(
        &self,
        id: &str,
        aria: &'static str,
        layout: Layout,
    ) -> Stateful<Div> {
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

    /// An approval card (web `approval_card`), decided inline.
    pub(super) fn render_bruh_approval(
        &self,
        item: &OutboxItem,
        layout: Layout,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let (summary, detail) = item
            .approval
            .as_ref()
            .map(|approval| (approval.summary.clone(), approval.detail.clone()))
            .unwrap_or_default();
        let card = self
            .render_bruh_card(&item.id, "Approval", layout)
            .child(small_text(APPROVAL_WANTS, theme::rambutan_400()))
            .child(
                div()
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(BODY_TEXT_PX))
                    .text_color(theme::rambutan_100())
                    .child(summary),
            )
            .when_some(detail, |card, detail| {
                let id = format!("desktop-bruh-detail-{}", item.id);
                card.child(
                    div()
                        .id(SharedString::from(id.clone()))
                        .debug_selector(move || id)
                        .aria_label("Full request")
                        .max_h(px(DETAIL_MAX_PX))
                        .overflow_y_scroll()
                        .p_2()
                        .rounded(px(RADIUS_GRAPE_PX))
                        .bg(theme::longan_950())
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(LABEL_TEXT_PX))
                        .text_color(theme::rambutan_300())
                        .child(detail),
                )
            });
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

    /// A question card (web `question_card`): the reply field is `reply`,
    /// which the page showing the card keeps.
    pub(super) fn render_bruh_question(
        &self,
        item: &OutboxItem,
        reply: Option<&Entity<TextareaState>>,
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
        let Some(input) = reply else {
            return card.into_any_element();
        };
        let empty = value(input, cx).trim().is_empty();
        let id = item.id.clone();
        let answer = input.clone();
        card.child(
            Textarea::new(input)
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
            move |this, window, cx| this.answer_question(&id, answer.clone(), window, cx),
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
        let Some(page) = self.bruh.as_ref() else {
            return div();
        };
        let Some(form) = page.settings.as_ref() else {
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
                Textarea::new(&form.job)
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
        let run = section("How it runs").child(if layout.wide {
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
        let (computer_reason, computer_off) =
            form.computer_block(self.crew.protocol, self.computer_unsupported(), cx);
        let vault_block = form.vault_block(self.vault_reason(), cx);
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
                            on_press_view(
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
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(small_text("Send as me", theme::rambutan_300()))
                            .child(
                                div()
                                    .debug_selector(|| "desktop-bruh-send-as-me-hint".to_string())
                                    .child(hint(SEND_AS_ME_HINT)),
                            ),
                    )
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
                            .child(small_text(COMPUTER_SWITCH, theme::rambutan_300()))
                            .child(
                                div()
                                    .debug_selector(|| "desktop-bruh-computer-hint".to_string())
                                    .child(hint(COMPUTER_HINT)),
                            )
                            .when_some(computer_reason, |el, reason| {
                                el.child(
                                    div()
                                        .debug_selector(|| {
                                            "desktop-bruh-computer-reason".to_string()
                                        })
                                        .child(hint(reason)),
                                )
                            }),
                    )
                    .child(page_switch(
                        "desktop-bruh-computer",
                        COMPUTER_SWITCH,
                        form.computer,
                        computer_off,
                        cx,
                        |this, _, cx| {
                            this.edit_bruh_settings(|form| form.computer = !form.computer, cx)
                        },
                    )),
            )
            .child(vault_grants(
                &page.vault,
                &page.agent_id,
                &form.vault,
                vault_block,
                cx,
            ));
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
    use super::{BruhTab, Editor, Layout, ReactiveAutomations, RunsPage};
    use crate::theme;
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::crew::{
        computer_unavailable, AccessLevel, BruhAccess, BruhColor, BruhLook, BruhState, BruhStatus,
        ServiceNotice, CREW_PROTOCOL,
    };
    use bruhs_models::reactive::{
        OutboxItem, ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveRun, ReactiveSnapshot,
        RunStatus, HISTORY_LIMIT,
    };
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
            computer: false,
            vault: Vec::new(),
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
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
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

    fn computer_snapshot(level: AccessLevel, protocol: u32, on: bool) -> ReactiveSnapshot {
        let mut snapshot = snapshot(BruhStatus::Asleep, Vec::new());
        snapshot.bruhs[0].access = BruhAccess {
            level,
            ..BruhAccess::default()
        };
        snapshot.bruhs[0].computer = on;
        snapshot.protocol = protocol;
        snapshot
    }

    /// The switch as drawn, and why it is off.
    fn computer_switch(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
    ) -> (bool, Option<&'static str>) {
        panel.update(cx, |panel, cx| {
            let protocol = panel.crew.protocol;
            let unsupported = panel.computer_unsupported();
            let form = panel.bruh_settings_mut().unwrap();
            (
                form.computer,
                form.computer_block(protocol, unsupported, cx).0,
            )
        })
    }

    #[gpui::test]
    fn the_computer_switch_follows_access_model_and_service(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(computer_snapshot(AccessLevel::Full, CREW_PROTOCOL, false)),
            BruhTab::Settings,
        );
        assert!(cx.debug_bounds("desktop-bruh-computer-hint").is_some());
        assert!(cx.debug_bounds("desktop-bruh-computer-reason").is_none());
        assert_eq!(computer_switch(&panel, cx), (false, None));
        click(cx, "desktop-bruh-computer");
        assert_eq!(computer_switch(&panel, cx), (true, None), "it is enabled");
        click(cx, "desktop-bruh-save");
        assert_eq!(
            sent(&panel, cx).last(),
            Some(&ReactiveCommand::SetComputer {
                agent_id: "a".into(),
                on: true
            })
        );
        let saved = computer_snapshot(AccessLevel::Full, CREW_PROTOCOL, true);
        panel.update(cx, |panel, cx| panel.answer_last_for_test(saved, cx));
        cx.run_until_parked();
        let before = sent(&panel, cx).len();

        // Read only can't have a Computer.
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(
                computer_snapshot(AccessLevel::ReadOnly, CREW_PROTOCOL, false),
                cx,
            );
            cx.notify();
        });
        cx.run_until_parked();
        let reason = computer_unavailable(AccessLevel::ReadOnly, AgentProvider::Claude);
        assert_eq!(reason, Some("Computer needs Ask first or Full access."));
        assert_eq!(computer_switch(&panel, cx), (false, reason));
        assert!(cx.debug_bounds("desktop-bruh-computer-reason").is_some());
        click(cx, "desktop-bruh-computer");
        click(cx, "desktop-bruh-save");
        assert_eq!(computer_switch(&panel, cx), (false, reason));
        assert_eq!(sent(&panel, cx).len(), before, "nothing is sent");

        // A protocol 2 service has no Computers: the switch shows the
        // stored value, off.
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(computer_snapshot(AccessLevel::Full, 2, true), cx);
            cx.notify();
        });
        cx.run_until_parked();
        let old = Some(ServiceNotice::ServingOld.message());
        assert_eq!(computer_switch(&panel, cx), (true, old));
        click(cx, "desktop-bruh-computer");
        click(cx, "desktop-bruh-save");
        assert_eq!(computer_switch(&panel, cx), (true, old));
        assert_eq!(sent(&panel, cx).len(), before, "nothing is sent");
    }

    /// A system that runs no bruh browser, as the Computer tab's state read
    /// said: the switch says so, and still turns off.
    #[gpui::test]
    fn an_unsupported_system_says_so_on_the_switch(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(computer_snapshot(AccessLevel::Full, CREW_PROTOCOL, true)),
            BruhTab::Settings,
        );
        assert_eq!(computer_switch(&panel, cx), (true, None));
        panel.update(cx, |panel, cx| panel.set_computer_unsupported(true, cx));
        cx.run_until_parked();
        assert_eq!(
            computer_switch(&panel, cx),
            (true, Some(bruhs_models::crew::COMPUTER_UNSUPPORTED))
        );
        assert!(cx.debug_bounds("desktop-bruh-computer-reason").is_some());
        click(cx, "desktop-bruh-computer");
        assert!(!computer_switch(&panel, cx).0, "it still switches off");
    }

    /// Turning the switch on and then picking Read only puts it back to the
    /// stored value, so Save sends only the level.
    #[gpui::test]
    fn a_blocked_switch_goes_back_to_the_stored_value(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(computer_snapshot(AccessLevel::Full, CREW_PROTOCOL, false)),
            BruhTab::Settings,
        );
        click(cx, "desktop-bruh-computer");
        assert!(computer_switch(&panel, cx).0);
        click(cx, "desktop-bruh-access-read-only");
        let reason = computer_unavailable(AccessLevel::ReadOnly, AgentProvider::Claude);
        assert_eq!(computer_switch(&panel, cx), (false, reason));
        click(cx, "desktop-bruh-save");
        let sent = sent(&panel, cx);
        assert!(
            matches!(&sent[..], [ReactiveCommand::SetBruh { access, .. }] if access.level == AccessLevel::ReadOnly),
            "{sent:?}"
        );
    }

    /// Another app turned the Computer on while a face edit was unsaved:
    /// Save keeps it on.
    #[gpui::test]
    fn a_computer_switch_left_alone_never_puts_back_an_older_value(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(computer_snapshot(AccessLevel::Full, CREW_PROTOCOL, false)),
            BruhTab::Settings,
        );
        click(cx, "desktop-bruh-face-swatch-Pandan");
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(
                computer_snapshot(AccessLevel::Full, CREW_PROTOCOL, true),
                cx,
            );
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(computer_switch(&panel, cx), (true, None), "it shows on");
        click(cx, "desktop-bruh-save");
        let sent = sent(&panel, cx);
        assert!(
            matches!(&sent[..], [ReactiveCommand::SetBruh { look, .. }] if look.face == BruhColor::Pandan),
            "{sent:?}"
        );
    }

    /// The owner turned the switch on, then another app did too: one click
    /// turns it off, and Save sends that.
    #[gpui::test]
    fn one_click_turns_off_a_switch_another_app_also_turned_on(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(computer_snapshot(AccessLevel::Full, CREW_PROTOCOL, false)),
            BruhTab::Settings,
        );
        click(cx, "desktop-bruh-face-swatch-Pandan");
        click(cx, "desktop-bruh-computer");
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(
                computer_snapshot(AccessLevel::Full, CREW_PROTOCOL, true),
                cx,
            );
            cx.notify();
        });
        cx.run_until_parked();
        click(cx, "desktop-bruh-computer");
        assert_eq!(computer_switch(&panel, cx), (false, None), "it stays off");
        click(cx, "desktop-bruh-save");
        assert_eq!(
            sent(&panel, cx).last(),
            Some(&ReactiveCommand::SetComputer {
                agent_id: "a".into(),
                on: false
            })
        );
    }

    fn vault_entry(
        id: &str,
        name: &str,
        scope: bruhs_models::vault::CredentialScope,
        kind: bruhs_models::vault::VaultKind,
    ) -> bruhs_models::vault::VaultEntry {
        bruhs_models::vault::VaultEntry {
            id: id.into(),
            name: name.into(),
            scope,
            kind,
            state: bruhs_models::secrets::SavedCredentialState::Saved,
            attention: None,
        }
    }

    /// Two shared entries, one of Ada's own and one of another bruh's.
    fn vault_listing() -> bruhs_models::vault::VaultListing {
        use bruhs_models::vault::{CredentialScope, LoginFields, LoginMeta, VaultKind};
        let login = VaultKind::Login(LoginMeta {
            url: "https://github.com/login".into(),
            fields: LoginFields::Saved {
                username: "me".into(),
                totp: false,
            },
        });
        let own = |agent_id: &str| CredentialScope::Bruh {
            agent_id: agent_id.into(),
        };
        bruhs_models::vault::VaultListing {
            entries: vec![
                vault_entry("v1-g", "GitHub", CredentialScope::Global, login),
                vault_entry("v1-k", "Deploy", CredentialScope::Global, VaultKind::Value),
                vault_entry("v1-s", "Shop", own("a"), VaultKind::Value),
                vault_entry("v1-o", "Other", own("b"), VaultKind::Value),
            ],
            ..bruhs_models::vault::VaultListing::default()
        }
    }

    fn vault_snapshot(level: AccessLevel, protocol: u32, grants: &[&str]) -> ReactiveSnapshot {
        let mut snapshot = computer_snapshot(level, protocol, false);
        snapshot.bruhs[0].vault = grants.iter().map(|id| (*id).to_string()).collect();
        snapshot
    }

    fn answer_vault(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
        result: Result<bruhs_models::vault::VaultListing, crate::server_client::ServerError>,
    ) {
        panel.update(cx, |panel, cx| panel.answer_bruh_vault_for_test(result, cx));
        cx.run_until_parked();
    }

    fn refused(code: reqwest::StatusCode) -> crate::server_client::ServerError {
        crate::server_client::ServerError::Status {
            request: "GET /api/vault".into(),
            code,
            body: String::new(),
        }
    }

    /// The picks as the form has them, and why the section is off.
    fn vault_picks(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
    ) -> (Vec<String>, Option<&'static str>) {
        panel.update(cx, |panel, cx| {
            let service = panel.vault_reason();
            let form = panel.bruh_settings_mut().unwrap();
            (form.vault.clone(), form.vault_block(service, cx))
        })
    }

    fn set_vault_count(sent: &[ReactiveCommand]) -> usize {
        sent.iter()
            .filter(|command| matches!(command, ReactiveCommand::SetVault { .. }))
            .count()
    }

    #[gpui::test]
    fn vault_grants_follow_the_store_and_save_once(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(vault_snapshot(AccessLevel::Ask, CREW_PROTOCOL, &[])),
            BruhTab::Settings,
        );
        assert_eq!(
            panel.read_with(cx, |panel, _| panel.bruh_vault_requests_for_test()),
            vec!["GET /api/vault".to_string()],
            "Settings asks for the listing once"
        );
        assert!(cx.debug_bounds("desktop-bruh-vault-loading").is_some());
        answer_vault(&panel, cx, Ok(vault_listing()));
        assert!(cx.debug_bounds("desktop-bruh-vault").is_some());
        assert!(cx.debug_bounds("desktop-bruh-vault-reason").is_none());
        // Its own entry is a line; shared ones are switches; another bruh's
        // entry is never offered.
        assert!(cx.debug_bounds("desktop-bruh-vault-own-v1-s").is_some());
        assert!(cx.debug_bounds("desktop-bruh-vault-grant-v1-s").is_none());
        assert!(cx.debug_bounds("desktop-bruh-vault-own-v1-o").is_none());
        assert!(cx.debug_bounds("desktop-bruh-vault-grant-v1-o").is_none());
        assert!(cx.debug_bounds("desktop-bruh-vault-none").is_none());
        // By name, one control row apart plus the 4px gap.
        let deploy = cx.debug_bounds("desktop-bruh-vault-grant-v1-k").unwrap();
        let github = cx.debug_bounds("desktop-bruh-vault-grant-v1-g").unwrap();
        assert_eq!(
            github.center().y - deploy.center().y,
            gpui::px(theme::CONTROL_PX + 4.0)
        );
        assert_eq!(vault_picks(&panel, cx), (Vec::new(), None));

        click(cx, "desktop-bruh-vault-grant-v1-g");
        assert_eq!(vault_picks(&panel, cx).0, vec!["v1-g".to_string()]);
        // A snapshot that changes another field leaves the picks alone.
        let mut other = vault_snapshot(AccessLevel::Ask, CREW_PROTOCOL, &[]);
        other.bruhs[0].access.send_as_me = true;
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(other, cx);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(vault_picks(&panel, cx).0, vec!["v1-g".to_string()]);
        click(cx, "desktop-bruh-save");
        let sent_now = sent(&panel, cx);
        assert_eq!(
            sent_now.last(),
            Some(&ReactiveCommand::SetVault {
                agent_id: "a".into(),
                entries: vec!["v1-g".into()],
            }),
            "{sent_now:?}"
        );
        assert_eq!(set_vault_count(&sent_now), 1);

        // Stored; then another app replaces the grants while the form is
        // untouched: the picks follow.
        for grants in [&["v1-g"], &["v1-k"]] {
            panel.update(cx, |panel, cx| {
                panel.set_snapshot(vault_snapshot(AccessLevel::Ask, CREW_PROTOCOL, grants), cx);
                cx.notify();
            });
            cx.run_until_parked();
        }
        assert_eq!(vault_picks(&panel, cx).0, vec!["v1-k".to_string()]);
        // With only the face edited, replaced grants follow too, and Save
        // doesn't put the older ones back.
        click(cx, "desktop-bruh-face-swatch-Pandan");
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(
                vault_snapshot(AccessLevel::Ask, CREW_PROTOCOL, &["v1-g", "v1-k"]),
                cx,
            );
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(
            vault_picks(&panel, cx).0,
            vec!["v1-g".to_string(), "v1-k".to_string()]
        );
        click(cx, "desktop-bruh-save");
        let sent_now = sent(&panel, cx);
        assert_eq!(set_vault_count(&sent_now), 1, "{sent_now:?}");
        assert!(
            matches!(sent_now.last(), Some(ReactiveCommand::SetBruh { look, .. }) if look.face == BruhColor::Pandan),
            "{sent_now:?}"
        );
    }

    #[gpui::test]
    fn an_older_service_disables_the_vault_section(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(vault_snapshot(AccessLevel::Ask, 3, &[])),
            BruhTab::Settings,
        );
        // A listing from somewhere still draws its switches, all off.
        answer_vault(&panel, cx, Ok(vault_listing()));
        let reason = bruhs_models::crew::vault_reason(Some(3));
        assert!(reason.is_some());
        assert_eq!(vault_picks(&panel, cx), (Vec::new(), reason));
        assert!(cx.debug_bounds("desktop-bruh-vault-reason").is_some());
        click(cx, "desktop-bruh-vault-grant-v1-g");
        assert_eq!(vault_picks(&panel, cx).0, Vec::<String>::new());
        // A pick made before the section went off snaps back.
        panel.update(cx, |panel, cx| {
            panel.bruh_settings_mut().unwrap().vault = vec!["v1-g".into()];
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(vault_picks(&panel, cx).0, Vec::<String>::new());
        click(cx, "desktop-bruh-face-swatch-Pandan");
        panel.update(cx, |panel, _| {
            panel.bruh_settings_mut().unwrap().vault = vec!["v1-g".into()];
        });
        click(cx, "desktop-bruh-save");
        let sent_now = sent(&panel, cx);
        assert_eq!(set_vault_count(&sent_now), 0, "{sent_now:?}");
        assert!(
            matches!(&sent_now[..], [ReactiveCommand::SetBruh { .. }]),
            "{sent_now:?}"
        );
        // The older server has no listing; the reason already says why.
        answer_vault(&panel, cx, Err(refused(reqwest::StatusCode::NOT_FOUND)));
        assert!(cx.debug_bounds("desktop-bruh-vault-notice").is_none());
        assert!(cx.debug_bounds("desktop-bruh-vault-reason").is_some());
        assert!(cx.debug_bounds("desktop-bruh-vault-grant-v1-g").is_none());
    }

    #[gpui::test]
    fn a_blocked_vault_section_snaps_back_to_the_stored_grants(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(vault_snapshot(AccessLevel::Ask, CREW_PROTOCOL, &["v1-k"])),
            BruhTab::Settings,
        );
        answer_vault(&panel, cx, Ok(vault_listing()));
        click(cx, "desktop-bruh-vault-grant-v1-g");
        assert_eq!(
            vault_picks(&panel, cx).0,
            vec!["v1-g".to_string(), "v1-k".to_string()]
        );
        click(cx, "desktop-bruh-access-read-only");
        assert_eq!(
            vault_picks(&panel, cx),
            (
                vec!["v1-k".to_string()],
                Some(bruhs_models::crew::VAULT_NEEDS_ASK_OR_FULL)
            )
        );
        assert!(cx.debug_bounds("desktop-bruh-vault-reason").is_some());
        click(cx, "desktop-bruh-save");
        let sent_now = sent(&panel, cx);
        assert_eq!(set_vault_count(&sent_now), 0, "{sent_now:?}");
    }

    #[gpui::test]
    fn the_vault_section_shows_why_it_has_no_list(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(vault_snapshot(AccessLevel::Ask, CREW_PROTOCOL, &[])),
            BruhTab::Settings,
        );
        // Only the owner's own app may list the vault (ruling F26).
        answer_vault(&panel, cx, Err(refused(reqwest::StatusCode::FORBIDDEN)));
        assert!(cx.debug_bounds("desktop-bruh-vault-notice").is_some());
        assert!(cx.debug_bounds("desktop-bruh-vault-grant-v1-g").is_none());
        assert!(cx.debug_bounds("desktop-bruh-vault-refresh").is_none());
        // Any other failure offers Refresh, which asks again.
        answer_vault(
            &panel,
            cx,
            Err(refused(reqwest::StatusCode::INTERNAL_SERVER_ERROR)),
        );
        assert!(cx.debug_bounds("desktop-bruh-vault-notice").is_none());
        assert!(cx.debug_bounds("desktop-bruh-vault-error").is_some());
        click(cx, "desktop-bruh-vault-refresh");
        assert_eq!(
            panel.read_with(cx, |panel, _| panel.bruh_vault_requests_for_test()),
            vec!["GET /api/vault".to_string(), "GET /api/vault".to_string()]
        );
        assert!(cx.debug_bounds("desktop-bruh-vault-loading").is_some());
        // No shared entries yet.
        answer_vault(&panel, cx, Ok(bruhs_models::vault::VaultListing::default()));
        assert!(cx.debug_bounds("desktop-bruh-vault-none").is_some());
        // Leaving Settings forgets the listing; coming back asks again.
        panel.update(cx, |panel, cx| panel.set_bruh_tab(BruhTab::Chat, cx));
        cx.run_until_parked();
        panel.update(cx, |panel, cx| panel.set_bruh_tab(BruhTab::Settings, cx));
        cx.run_until_parked();
        assert_eq!(
            panel
                .read_with(cx, |panel, _| panel.bruh_vault_requests_for_test())
                .len(),
            3
        );
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
    fn a_run_awaiting_review_can_run_or_be_dismissed(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::NeedsYou, Vec::new())),
            BruhTab::Chat,
        );
        panel.update(cx, |panel, cx| {
            panel.show_bruh_runs_for_test(vec![run("r1", "awaiting_review")]);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-bruh-dismiss-r1").is_some());
        click(cx, "desktop-bruh-run-r1");
        assert_eq!(
            sent(&panel, cx).last(),
            Some(&ReactiveCommand::ApproveRun { id: "r1".into() })
        );
        click(cx, "desktop-bruh-dismiss-r1");
        assert_eq!(
            sent(&panel, cx).last(),
            Some(&ReactiveCommand::CancelRun { id: "r1".into() })
        );
    }

    /// A redraw with nothing new reuses the conversation instead of
    /// building it again on every frame.
    #[gpui::test]
    fn the_transcript_is_built_only_when_runs_or_the_snapshot_change(
        cx: &mut gpui::TestAppContext,
    ) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Working, vec![ask("i1", false)])),
            BruhTab::Chat,
        );
        let lines = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
            panel.read_with(cx, |panel, _| {
                Rc::clone(&panel.bruh.as_ref().unwrap().lines)
            })
        };
        let first = lines(&panel, cx);
        assert_eq!(first.len(), 1);
        panel.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(
            Rc::ptr_eq(&first, &lines(&panel, cx)),
            "rebuilt on a redraw"
        );
        panel.update(cx, |panel, cx| {
            panel.show_bruh_runs_for_test(vec![run("r1", "awaiting_review")]);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(lines(&panel, cx).len(), 2, "new runs rebuild it");
        panel.update(cx, |panel, cx| {
            panel.set_snapshot(snapshot(BruhStatus::Working, Vec::new()), cx);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(lines(&panel, cx).len(), 1, "a new snapshot rebuilds it");
    }

    /// Send as me says what it already lets a Full bruh do.
    #[gpui::test]
    fn send_as_me_explains_what_it_allows(cx: &mut gpui::TestAppContext) {
        let (_, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Asleep, Vec::new())),
            BruhTab::Settings,
        );
        assert!(cx.debug_bounds("desktop-bruh-send-as-me").is_some());
        assert!(cx.debug_bounds("desktop-bruh-send-as-me-hint").is_some());
    }

    #[gpui::test]
    fn an_approval_card_shows_the_full_request(cx: &mut gpui::TestAppContext) {
        let mut long = ask("i2", true);
        if let Some(approval) = long.approval.as_mut() {
            approval.summary = "Run cd x (+1 more line)".into();
            approval.detail = Some("cd x\nrm -rf y".into());
        }
        let (_, cx) = open(
            cx,
            Some(snapshot(BruhStatus::NeedsYou, vec![ask("i1", true), long])),
            BruhTab::Chat,
        );
        assert!(cx.debug_bounds("desktop-bruh-detail-i2").is_some());
        assert!(cx.debug_bounds("desktop-bruh-detail-i1").is_none());
    }

    #[test]
    fn the_sm_breakpoint_reads_the_window_like_the_web_media_query() {
        // A 640px window leaves the page about 392px beside the sidebar; the
        // web's `sm:px-6` and `sm:grid-cols-2` still apply.
        let docked = Layout::of(392.0, 640.0);
        assert_eq!(docked.padding, super::PAGE_WIDE_PADDING_PX);
        assert!(docked.wide);
        let phone = Layout::of(390.0, 390.0);
        assert_eq!(phone.padding, super::PAGE_PADDING_PX);
        assert!(!phone.wide);
        assert_eq!(Layout::of(1032.0, 1280.0).column, super::COLUMN_MAX_PX);
    }

    #[gpui::test]
    fn stop_stays_inside_the_header_on_narrow_pages(cx: &mut gpui::TestAppContext) {
        let mut wide = snapshot(BruhStatus::Working, Vec::new());
        wide.agents[0].instructions =
            "Watch every bill and statement that arrives, and draft the replies.".into();
        let (_panel, cx) = open(cx, Some(wide), BruhTab::Chat);
        for width in [390.0, 640.0] {
            cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(800.0)));
            cx.run_until_parked();
            let header = cx.debug_bounds("desktop-bruh-header").expect("header");
            let stop = cx.debug_bounds("desktop-bruh-stop").expect("stop");
            assert!(
                header.contains(&stop.origin) && header.contains(&stop.bottom_right()),
                "{width}: stop {stop:?} outside header {header:?}"
            );
        }
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
                run_id: None,
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

    #[test]
    fn the_computer_tab_sits_after_chat() {
        assert_eq!(
            BruhTab::ALL.map(BruhTab::label),
            ["Chat", "Computer", "Memory", "Routines", "Settings"]
        );
        assert_eq!(BruhTab::Computer.key(), "computer");
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
                .set_value("/tmp/scans", window, cx);
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
        // A full page may have older runs behind it.
        let full: Vec<ReactiveRun> = (0..HISTORY_LIMIT)
            .map(|n| run(&format!("old{n}"), "completed"))
            .collect();
        page.take_earlier(full);
        assert!(page.more);
        assert_eq!(page.oldest.as_deref(), Some("old49"));
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

    /// What the bruh answered draws as the thread chat's Markdown; a failed
    /// run's error stays as written.
    #[gpui::test]
    fn answers_draw_as_markdown(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(
            cx,
            Some(snapshot(BruhStatus::Asleep, Vec::new())),
            BruhTab::Chat,
        );
        let mut answered = run("r1", "completed");
        answered.result = Some("**Paid** the bill".into());
        let mut failed = run("r2", "failed");
        failed.error = Some("**no** network".into());
        panel.update(cx, |panel, cx| {
            panel.show_bruh_runs_for_test(vec![failed, answered]);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("bruh-md-r1").is_some(), "the answer");
        assert!(cx.debug_bounds("bruh-md-r2").is_none(), "the failed run");
    }

    /// One crew bubble alone in a window.
    struct Prose(gpui::SharedString);

    impl gpui::Render for Prose {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<'_, Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{ParentElement as _, Styled as _};
            gpui::div().size_full().child(super::prose_bubble(
                "crew-link-test".into(),
                self.0.clone(),
                Layout::of(600.0, 900.0),
                cx,
            ))
        }
    }

    /// A bruh can write a link to a launcher (a `.command` file, another
    /// app's scheme). A click opens only a web or mail link; the rest draw
    /// as their text.
    #[gpui::test]
    fn crew_links_open_only_web_and_mail_addresses(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let mut click = |url: &str| {
            let markdown = super::crew_markdown(&format!("[open the full report]({url})"));
            let (_, cx) = cx.add_window_view(|_, _| Prose(markdown));
            cx.run_until_parked();
            let bounds = cx.debug_bounds("crew-link-test").expect("the bubble");
            let on_link = gpui::point(bounds.origin.x + gpui::px(20.0), bounds.center().y);
            cx.simulate_click(on_link, gpui::Modifiers::default());
            cx.run_until_parked();
            cx.opened_url()
        };
        for url in [
            "file:///Users/me/Downloads/run.command",
            "javascript:alert(1)",
            "vscode://file/etc/hosts",
            "x-apple.systempreferences:com.apple.preference.security",
        ] {
            assert_eq!(click(url), None, "{url}");
        }
        assert_eq!(
            click("https://example.com/report").as_deref(),
            Some("https://example.com/report")
        );
    }
}
