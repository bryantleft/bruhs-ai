//! The Move in dialog (D1b), the desktop half of the web's
//! `crew/move_in.rs`: a bruh from an OpenClaw workspace or a Hermes Agent
//! home, or memory another assistant has of the owner, By prompt.
//!
//! A persona never becomes a bruh here: **Make this bruh** opens the New
//! bruh dialog prefilled, and the owner makes it there. The bruh starts
//! Read only, with empty memory and its own folder; the persona's memory
//! waits as a proposal on its Memory tab (D11, D12). The server scans; the
//! desktop also offers its native folder picker (**Choose folder**).
//!
//! The header stays put, the tab's body scrolls, and its action and status
//! lines stay pinned below, so at 640 by 480 they never leave the dialog.

use super::import_ui::live_line::{
    live_line, LiveKind, LiveLine, MOVE_IN_ERROR_LIVE, MOVE_IN_STATUS_LIVE,
};
use super::import_ui::{IMPORT_DIALOG_WIDTH_PX, IMPORT_LABEL_SM_TEXT_SIZE_PX, IMPORT_RADIUS_PX};
use super::*;
use bruhs_models::imports::{
    looked_in_line, persona_lines, prefill_from_persona, AgentScan, PersonaCandidate,
    AGENTS_PRIVACY, BY_PROMPT_TAB, CHOOSE_FOLDER, FOLDER, LOOK_HERE, MAKE_BRUH, MOVE_IN_TITLE,
    NONE_FOUND_HERMES, NONE_FOUND_OPENCLAW,
};
pub(super) use bruhs_models::imports::{MoveInStart, MoveInTab};
use bruhs_models::logic::portable::ImportSource;
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
use gpui::PathPromptOptions;
use std::cell::RefCell;
use std::rc::Rc;

pub(super) mod by_prompt;
#[cfg(test)]
mod tests;

/// The tabs, in order.
pub(super) const TABS: [MoveInTab; 3] =
    [MoveInTab::OpenClaw, MoveInTab::Hermes, MoveInTab::ByPrompt];
/// Web `max-h-48`: the personas scroll in a short list.
const ROWS_MAX_HEIGHT_PX: f32 = 192.0;
/// Web `bg-longan-950/60 inset-ring-longan-700/50` around the preview.
const PANEL_FILL_OPACITY: f32 = 0.6;
const PANEL_RING_OPACITY: f32 = 0.5;
/// Web `bg-blueberry-600/20 inset-ring-blueberry-500/30`: the selection.
const ROW_SELECTED_BG_OPACITY: f32 = 0.20;
const ROW_SELECTED_RING_OPACITY: f32 = 0.30;
const ROW_HOVER_BG_OPACITY: f32 = 0.40;
/// Web `aria-disabled:opacity-50`: a button that waits keeps its focus.
pub(super) const WAITING_OPACITY: f32 = 0.5;

fn tab_label(tab: MoveInTab) -> &'static str {
    match tab {
        MoveInTab::OpenClaw => ImportSource::OpenClaw.label(),
        MoveInTab::Hermes => ImportSource::Hermes.label(),
        MoveInTab::ByPrompt => BY_PROMPT_TAB,
    }
}

/// The agent a tab scans, or `None` for By prompt.
fn tab_source(tab: MoveInTab) -> Option<ImportSource> {
    match tab {
        MoveInTab::OpenClaw => Some(ImportSource::OpenClaw),
        MoveInTab::Hermes => Some(ImportSource::Hermes),
        MoveInTab::ByPrompt => None,
    }
}

/// `GET /api/import/agents`: a source and a folder, or its default places.
pub(super) type ScanCall =
    Rc<dyn Fn((ImportSource, Option<String>), &App) -> Task<Result<AgentScan, String>>>;

/// The scan through the app's server client, on the desktop's tokio
/// runtime. A failure is the line the tab shows (`import_failure_line`).
fn server_scan() -> ScanCall {
    Rc::new(|(source, path): (ImportSource, Option<String>), cx: &App| {
        let client = crate::server_client::ServerClient::workbench_client();
        crate::app_state::spawn_tokio(cx, async move {
            client.scan_agents(source, path.as_deref()).await
        })
    })
}

/// `BRUHS_DESKTOP_SMOKE_SCENE=move-in`: the web screenshot's scan, Pinch
/// in a workspace, answered with no server. With `workspaces`, that many
/// more personas follow Pinch, so the list scrolls (`move-in-many`).
#[cfg(debug_assertions)]
pub(super) fn smoke_scan(workspaces: usize) -> ScanCall {
    use bruhs_models::imports::{FileUse, PersonaFile, SkipReason};
    Rc::new(
        move |(source, _path): (ImportSource, Option<String>), _cx: &App| {
            let file = |name: &str, use_| PersonaFile {
                name: name.into(),
                use_,
            };
            let mut personas = if source == ImportSource::OpenClaw {
                vec![PersonaCandidate {
                    id: "smoke-pinch".into(),
                    source,
                    folder: "~/clawd".into(),
                    name: "Pinch".into(),
                    instructions:
                        "Identity from IDENTITY.md\nName: Pinch\n\n## SOUL.md\n\nBe kind and brief."
                            .into(),
                    memory: "## OpenClaw: USER.md\n\nLikes short answers.".into(),
                    files: vec![
                        file("IDENTITY.md", FileUse::Identity),
                        file("SOUL.md", FileUse::Instructions),
                        file("USER.md", FileUse::Memory),
                        file("MEMORY.md", FileUse::Memory),
                        file(
                            "HEARTBEAT.md",
                            FileUse::Skipped {
                                reason: SkipReason::NotImported,
                            },
                        ),
                    ],
                    clipped: false,
                    dropped_identity: vec!["Emoji".into()],
                    left_out_logs: 0,
                }]
            } else {
                Vec::new()
            };
            if let Some(pinch) = personas.first().cloned() {
                personas.extend((1..=workspaces).map(|n| PersonaCandidate {
                    id: format!("smoke-workspace-{n}"),
                    folder: format!("~/.openclaw/workspace-{n}"),
                    name: format!("Workspace {n}"),
                    ..pinch.clone()
                }));
            }
            let looked_in = if source == ImportSource::OpenClaw {
                vec![
                    "~/.openclaw/workspace".into(),
                    "~/clawd".into(),
                    "~/openclaw".into(),
                ]
            } else {
                vec!["~/.hermes".into()]
            };
            Task::ready(Ok(AgentScan {
                personas,
                looked_in,
            }))
        },
    )
}

/// The dialog's state: its scan seam, and the open dialog, made fresh each
/// time it opens.
#[derive(Default)]
pub(super) struct MoveInPicker {
    /// The server, or a test's stand-in.
    pub(super) scan: Option<ScanCall>,
    pub(super) open: Option<MoveIn>,
}

/// One opening of the dialog.
pub(super) struct MoveIn {
    pub(super) tab: MoveInTab,
    pub(super) tabs: [FocusHandle; 3],
    pub(super) openclaw: AgentPane,
    pub(super) hermes: AgentPane,
    pub(super) look: FocusHandle,
    pub(super) choose: FocusHandle,
    pub(super) make: FocusHandle,
    pub(super) by_prompt: by_prompt::ByPrompt,
    body_scroll: ScrollHandle,
    /// The control focus was on at the last draw, the body block holding
    /// it, where that block sat and how tall the body was: when any of them
    /// changes (Tab, a block appearing above it, a pinned line below), the
    /// body scrolls to it again. A wheel
    /// scroll moves none of them, so it is never undone.
    last_focus: RefCell<FocusSpot>,
    /// Settles a By prompt proposal once the crew answers.
    _watch: Subscription,
}

impl MoveIn {
    pub(super) fn pane(&self, source: ImportSource) -> &AgentPane {
        if source == ImportSource::Hermes {
            &self.hermes
        } else {
            &self.openclaw
        }
    }

    fn pane_mut(&mut self, source: ImportSource) -> &mut AgentPane {
        if source == ImportSource::Hermes {
            &mut self.hermes
        } else {
            &mut self.openclaw
        }
    }
}

/// One agent tab's scan: the folder typed, what it found, the persona
/// chosen, and the call in flight.
pub(super) struct AgentPane {
    pub(super) folder: Entity<InputState>,
    pub(super) scan: Option<AgentScan>,
    /// Each persona's row, a Tab stop, made when its scan lands.
    pub(super) rows: Vec<FocusHandle>,
    pub(super) picked: usize,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    rows_scroll: ScrollHandle,
    /// Replacing it drops (cancels) the scan before.
    task: Option<Task<()>>,
    _enter: Subscription,
}

impl AgentPane {
    /// The chosen persona, once a scan found any.
    pub(super) fn chosen(&self) -> Option<&PersonaCandidate> {
        let personas = &self.scan.as_ref()?.personas;
        personas.get(self.picked.min(personas.len().saturating_sub(1)))
    }
}

/// One line of the dialog's text, findable by its words in tests.
pub(super) fn move_in_text(kind: &'static str, text: String, color: Hsla) -> Div {
    let selector = format!("desktop-move-in-{kind}:{text}");
    div()
        .debug_selector(move || selector)
        .min_w_0()
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
        .line_height(relative(1.5))
        .text_color(color)
        .child(text)
}

/// A dialog button. It keeps its Tab stop while it waits, so focus never
/// drops out of the dialog, and its handler does nothing until then.
pub(super) fn move_in_button(
    label: &'static str,
    focus: &FocusHandle,
    waiting: bool,
    cx: &Context<'_, Workspace>,
    handler: impl Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + 'static,
) -> Stateful<Div> {
    let el = web_button(
        SharedString::from(format!("desktop-move-in-{label}")),
        WebButtonVariant::Outline,
        WebButtonSize::Small,
        ButtonState::Enabled,
    )
    .debug_selector(move || format!("desktop-move-in-button:{label}"))
    .track_focus(focus)
    .flex_shrink_0()
    .min_h(px(theme::CONTROL_PX))
    .px_3()
    .border_color(theme::longan_700())
    .text_color(theme::rambutan_200())
    .focus_visible(|el| el.border_color(theme::persimmon_400()))
    .aria_label(label)
    .when(waiting, |el| {
        el.opacity(WAITING_OPACITY).cursor_not_allowed()
    })
    .child(label);
    on_press_view(el, cx, handler)
}

/// A field's label above its control (web `LABEL`).
pub(super) fn move_in_label(text: &'static str) -> Div {
    div()
        .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
        .line_height(relative(1.5))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(theme::rambutan_300())
        .child(text)
}

/// Where focus was at the last draw (see [`MoveIn::last_focus`]).
#[derive(Default, PartialEq)]
pub(super) struct FocusSpot {
    focus: Option<FocusHandle>,
    block: Option<usize>,
    top: Option<Pixels>,
    /// The body's own height: pinned lines appearing below shrink it.
    height: Pixels,
}

/// What stays pinned under a tab's body: its buttons and its live lines.
/// The live lines are there even while empty, so the room above the
/// pinned part opens only when something in it is drawn.
#[derive(Default)]
pub(super) struct Pinned {
    items: Vec<AnyElement>,
    drawn: bool,
}

impl Pinned {
    pub(super) fn push(&mut self, element: impl IntoElement) {
        self.items.push(element.into_any_element());
        self.drawn = true;
    }

    /// A live status or error line, kept in the tree while empty.
    pub(super) fn push_live(&mut self, line: LiveLine) {
        self.drawn |= line.drawn();
        self.items.push(line.into_any_element());
    }
}

/// One block of a tab's scrolling body, and the controls inside it: when
/// one of them takes focus, the body scrolls the block into view, since
/// GPUI never scrolls to focus by itself (I1).
pub(super) struct BodyPart {
    pub(super) element: AnyElement,
    pub(super) focus: Vec<FocusHandle>,
}

impl BodyPart {
    pub(super) fn new(element: impl IntoElement, focus: Vec<FocusHandle>) -> Self {
        Self {
            element: element.into_any_element(),
            focus,
        }
    }

    /// A block with nothing to focus.
    pub(super) fn text(element: impl IntoElement) -> Self {
        Self::new(element, Vec::new())
    }
}

impl Workspace {
    /// Whether New bruh can open yet, and why not when the service is
    /// known to be older. Make this bruh and Make a bruh wait for it.
    pub(super) fn crew_make_state(&self, cx: &App) -> (bool, Option<&'static str>) {
        let panel = self.reactive_automations.read(cx);
        (panel.crew_ready(), panel.crew_reason())
    }

    /// Opens Move in on `start`'s tab, from the palette (OpenClaw) or the
    /// Export file tab's **Use as a bruh's memory** (By prompt, filled).
    /// Each opening starts fresh, and focus goes to the tab's first field.
    pub(super) fn open_move_in(
        &mut self,
        start: MoveInStart,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Escape returns focus to the control that opened it, unless that
        // was another dialog's.
        if !self.modal_start_focus_handle.contains_focused(window, cx) {
            self.palette_return_focus = window
                .focused(cx)
                .map(|focus| (self.active_view.clone(), focus));
        }
        self.open_top_layer(TopLayer::MoveIn);
        if self.move_in.scan.is_none() {
            self.move_in.scan = Some(server_scan());
        }
        let openclaw = self.new_agent_pane(ImportSource::OpenClaw, window, cx);
        let hermes = self.new_agent_pane(ImportSource::Hermes, window, cx);
        let by_prompt = by_prompt::ByPrompt::new(start.text, start.from, window, cx);
        let watch = cx.observe_in(
            &self.reactive_automations,
            window,
            |this: &mut Self, _panel, window, cx| this.settle_move_in_proposal(window, cx),
        );
        let stop = || cx.focus_handle().tab_stop(true);
        self.move_in.open = Some(MoveIn {
            tab: start.tab,
            tabs: [stop(), stop(), stop()],
            openclaw,
            hermes,
            look: stop(),
            choose: stop(),
            make: stop(),
            by_prompt,
            body_scroll: ScrollHandle::new(),
            last_focus: RefCell::new(FocusSpot::default()),
            _watch: watch,
        });
        self.show_move_in_tab(start.tab, window, cx);
        self.focus_move_in_field(window, cx);
        cx.notify();
    }

    fn new_agent_pane(
        &mut self,
        source: ImportSource,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AgentPane {
        let folder = cx.new(|cx| InputState::new(window, cx));
        // Enter in Folder looks there, as Look here does.
        let enter = cx.subscribe_in(
            &folder,
            window,
            move |this: &mut Self, _input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: false, ..
                } => this.look_here(source, window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        );
        AgentPane {
            folder,
            scan: None,
            rows: Vec::new(),
            picked: 0,
            loading: false,
            error: None,
            rows_scroll: ScrollHandle::new(),
            task: None,
            _enter: enter,
        }
    }

    /// The tab's first field: Folder, or Their answer on By prompt.
    fn focus_move_in_field(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(open) = &self.move_in.open else {
            return;
        };
        let handle = match tab_source(open.tab) {
            Some(source) => open.pane(source).folder.focus_handle(cx),
            None => open.by_prompt.answer.focus_handle(cx),
        };
        cx.defer_in(window, move |_this, window, cx| handle.focus(window, cx));
    }

    /// Shows `tab`. Each agent tab scans its default places the first time
    /// it shows.
    pub(super) fn show_move_in_tab(
        &mut self,
        tab: MoveInTab,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(open) = self.move_in.open.as_mut() else {
            return;
        };
        open.tab = tab;
        open.body_scroll.set_offset(Point::default());
        if let Some(source) = tab_source(tab) {
            let pane = open.pane(source);
            if pane.scan.is_none() && !pane.loading {
                self.scan_move_in(source, None, window, cx);
            }
        }
        cx.notify();
    }

    /// Scans `source`'s default places, or `path`; the newest scan wins.
    pub(super) fn scan_move_in(
        &mut self,
        source: ImportSource,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(scan) = self.move_in.scan.clone() else {
            return;
        };
        let Some(open) = self.move_in.open.as_mut() else {
            return;
        };
        let answer = scan((source, path), cx);
        let pane = open.pane_mut(source);
        pane.loading = true;
        pane.error = None;
        pane.task = Some(cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _closed = this.update(cx, |this, cx| {
                this.move_in_scanned(source, answer, cx);
            });
        }));
        cx.notify();
    }

    fn move_in_scanned(
        &mut self,
        source: ImportSource,
        answer: Result<AgentScan, String>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(open) = self.move_in.open.as_mut() else {
            return;
        };
        let pane = open.pane_mut(source);
        pane.loading = false;
        pane.task = None;
        match answer {
            Ok(scan) => {
                pane.rows = scan
                    .personas
                    .iter()
                    .map(|_| cx.focus_handle().tab_stop(true))
                    .collect();
                pane.picked = 0;
                pane.rows_scroll.set_offset(Point::default());
                pane.scan = Some(scan);
            }
            Err(message) => pane.error = Some(message),
        }
        cx.notify();
    }

    /// **Look here**: scans the folder typed, or the default places when
    /// it is blank. A scan on its way finishes first.
    pub(super) fn look_here(
        &mut self,
        source: ImportSource,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(open) = &self.move_in.open else {
            return;
        };
        let pane = open.pane(source);
        if pane.loading {
            return;
        }
        let typed = pane.folder.read(cx).value().trim().to_string();
        let path = (!typed.is_empty()).then_some(typed);
        self.scan_move_in(source, path, window, cx);
    }

    /// **Choose folder**: the native picker, one folder, then a scan of it.
    pub(super) fn choose_move_in_folder(
        &mut self,
        source: ImportSource,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .move_in
            .open
            .as_ref()
            .is_none_or(|open| open.pane(source).loading)
        {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(CHOOSE_FOLDER.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picker.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path = path.to_string_lossy().into_owned();
            let _closed = this.update_in(cx, |this, window, cx| {
                if this.top_layer != Some(TopLayer::MoveIn) {
                    return;
                }
                let Some(open) = &this.move_in.open else {
                    return;
                };
                let folder = open.pane(source).folder.clone();
                folder.update(cx, |input, cx| input.set_value(&path, window, cx));
                this.scan_move_in(source, Some(path), window, cx);
            });
        })
        .detach();
    }

    fn pick_persona(&mut self, source: ImportSource, index: usize, cx: &mut Context<'_, Self>) {
        if let Some(open) = self.move_in.open.as_mut() {
            open.pane_mut(source).picked = index;
            cx.notify();
        }
    }

    /// **Make this bruh**: closes Move in and opens New bruh on the
    /// persona's name and job, its memory waiting as a proposal.
    pub(super) fn make_this_bruh(
        &mut self,
        source: ImportSource,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(persona) = self
            .move_in
            .open
            .as_ref()
            .and_then(|open| open.pane(source).chosen())
        else {
            return;
        };
        let prefill = prefill_from_persona(persona);
        self.open_create_bruh_from_import_dialog(prefill, window, cx);
    }

    fn close_move_in(&mut self, cx: &mut Context<'_, Self>) {
        self.close_transient_surfaces();
        cx.notify();
    }

    fn render_move_in_tabs(&self, open: &MoveIn, cx: &Context<'_, Self>) -> AnyElement {
        let selected_index = TABS.iter().position(|&tab| tab == open.tab).unwrap_or(0);
        let view = cx.entity().downgrade();
        bruhs_ui::tabs::Tabs::new(
            "desktop-move-in-tabs",
            MOVE_IN_TITLE,
            bruhs_ui::tabs::TabStyle::Pill,
            selected_index,
        )
        .mt_2()
        .items(TABS.into_iter().zip(&open.tabs).map(|(tab, focus)| {
            let label = tab_label(tab);
            bruhs_ui::tabs::TabItem::new(
                SharedString::from(format!("desktop-move-in-tab-{label}")),
                label,
            )
            .debug_selector(format!("desktop-move-in-tab:{label}"))
            .focus_handle(focus.clone())
            .child(label)
        }))
        .on_select(move |index, window, cx| {
            if let Some(&tab) = TABS.get(index) {
                let _ = view.update(cx, |this, cx| {
                    this.show_move_in_tab(tab, window, cx);
                });
            }
        })
        .into_any_element()
    }

    /// An agent tab: where it reads from, the folder, the personas found,
    /// the chosen one's preview; **Make this bruh** and the error line stay
    /// pinned below.
    fn render_agent_pane(
        &self,
        open: &MoveIn,
        source: ImportSource,
        cx: &Context<'_, Self>,
    ) -> (Vec<BodyPart>, Pinned) {
        let pane = open.pane(source);
        let none_found = if source == ImportSource::Hermes {
            NONE_FOUND_HERMES
        } else {
            NONE_FOUND_OPENCLAW
        };
        let mut body = vec![
            BodyPart::text(move_in_text(
                "privacy",
                AGENTS_PRIVACY.to_owned(),
                theme::rambutan_300(),
            )),
            BodyPart::new(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(move_in_label(FOLDER))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .debug_selector(|| "desktop-move-in-folder".to_string())
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        Styled::h(
                                            Input::new(&pane.folder).aria_label(FOLDER),
                                            px(theme::CONTROL_PX),
                                        )
                                        .px(px(10.0))
                                        .font_family(theme::CODE_FONT_FAMILY)
                                        .text_size(px(theme::APP_INPUT_TEXT_SIZE_PX)),
                                    ),
                            )
                            .child(move_in_button(
                                LOOK_HERE,
                                &open.look,
                                pane.loading,
                                cx,
                                move |this, window, cx| this.look_here(source, window, cx),
                            ))
                            .child(move_in_button(
                                CHOOSE_FOLDER,
                                &open.choose,
                                pane.loading,
                                cx,
                                move |this, window, cx| {
                                    this.choose_move_in_folder(source, window, cx)
                                },
                            )),
                    ),
                vec![
                    pane.folder.focus_handle(cx),
                    open.look.clone(),
                    open.choose.clone(),
                ],
            ),
        ];
        if let Some(scan) = pane.scan.as_ref() {
            if !scan.looked_in.is_empty() {
                body.push(BodyPart::text(move_in_text(
                    "places",
                    looked_in_line(&scan.looked_in),
                    theme::rambutan_300(),
                )));
            }
            if scan.personas.is_empty() {
                body.push(BodyPart::text(move_in_text(
                    "none",
                    none_found.to_owned(),
                    theme::rambutan_200(),
                )));
            } else {
                body.push(BodyPart {
                    element: self.render_persona_rows(pane, source, cx),
                    focus: pane.rows.clone(),
                });
            }
        }
        let chosen = pane.chosen();
        if let Some(persona) = chosen {
            body.push(BodyPart::text(
                div()
                    .debug_selector(|| "desktop-move-in-preview".to_string())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .rounded(px(IMPORT_RADIUS_PX))
                    .bg(theme::longan_950().opacity(PANEL_FILL_OPACITY))
                    .border_1()
                    .border_color(theme::longan_700().opacity(PANEL_RING_OPACITY))
                    .children(persona_lines(persona).into_iter().map(|line| {
                        move_in_text("line", line, theme::rambutan_300()).into_any_element()
                    })),
            ));
        }
        let mut pinned = Pinned::default();
        if chosen.is_some() {
            let (ready, reason) = self.crew_make_state(cx);
            pinned.push(
                div()
                    .flex()
                    .child(
                        move_in_button(
                            MAKE_BRUH,
                            &open.make,
                            !ready,
                            cx,
                            move |this, window, cx| this.make_this_bruh(source, window, cx),
                        )
                        .when_some(reason, |el, reason| el.aria_description(reason)),
                    )
                    .into_any_element(),
            );
        }
        pinned.push_live(live_line(
            MOVE_IN_ERROR_LIVE,
            LiveKind::Alert,
            pane.error.clone(),
            |error| move_in_text("error", error, theme::lychee_300()),
        ));
        (body, pinned)
    }

    fn render_persona_rows(
        &self,
        pane: &AgentPane,
        source: ImportSource,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let Some(scan) = pane.scan.as_ref() else {
            return div().into_any_element();
        };
        let picked = pane.picked.min(scan.personas.len().saturating_sub(1));
        div()
            .id("desktop-move-in-rows")
            .debug_selector(|| "desktop-move-in-rows".to_string())
            .role(Role::Group)
            .aria_label(source.label())
            .flex_shrink_0()
            .max_h(px(ROWS_MAX_HEIGHT_PX))
            .overflow_y_scroll()
            .track_scroll(&pane.rows_scroll)
            .flex()
            .flex_col()
            .gap_1()
            .children(scan.personas.iter().zip(&pane.rows).enumerate().map(
                |(index, (persona, focus))| {
                    let chosen = index == picked;
                    let name = persona.name.clone();
                    let row = div()
                        .id(("desktop-move-in-row", index))
                        .debug_selector({
                            let name = name.clone();
                            move || format!("desktop-move-in-row:{name}")
                        })
                        .track_focus(focus)
                        .role(Role::Button)
                        .aria_label(name.clone())
                        .aria_toggled(if chosen {
                            Toggled::True
                        } else {
                            Toggled::False
                        })
                        .flex_shrink_0()
                        .w_full()
                        .min_h(px(theme::CONTROL_PX))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_0p5()
                        .px_3()
                        .py_2()
                        .rounded(px(IMPORT_RADIUS_PX))
                        .border_1()
                        .cursor_pointer()
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .map(|el| {
                            // Blueberry marks the selection (design rule).
                            if chosen {
                                el.bg(theme::blueberry_600().opacity(ROW_SELECTED_BG_OPACITY))
                                    .border_color(
                                        theme::blueberry_500().opacity(ROW_SELECTED_RING_OPACITY),
                                    )
                            } else {
                                el.border_color(gpui::transparent_black()).hover(|el| {
                                    el.bg(theme::longan_700().opacity(ROW_HOVER_BG_OPACITY))
                                })
                            }
                        })
                        .child(
                            move_in_text("name", name, theme::rambutan_100())
                                .w_full()
                                .truncate(),
                        )
                        .child(
                            move_in_text("folder", persona.folder.clone(), theme::rambutan_400())
                                .w_full()
                                .truncate(),
                        );
                    on_press_view(row, cx, move |this, _window, cx| {
                        this.pick_persona(source, index, cx);
                    })
                },
            ))
            .into_any_element()
    }

    /// Once focus moves (the dialog opening, Tab, a click), the body scrolls
    /// the block holding the focused control into view, and the persona
    /// list its row, the least distance needed.
    fn scroll_move_in_to_focus(
        &self,
        open: &MoveIn,
        body: &[BodyPart],
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let focused = window.focused(cx);
        let at = focused
            .as_ref()
            .and_then(|focused| body.iter().position(|part| part.focus.contains(focused)));
        let now = FocusSpot {
            focus: focused.clone(),
            block: at,
            top: at
                .and_then(|at| open.body_scroll.bounds_for_item(at))
                .map(|bounds| bounds.top()),
            height: open.body_scroll.bounds().size.height,
        };
        if *open.last_focus.borrow() == now {
            return;
        }
        open.last_focus.replace(now);
        let (Some(focused), Some(at)) = (focused, at) else {
            return;
        };
        // The scroll reads the body's size from the frame before; one more
        // draw settles it once the layout this one draws is known.
        cx.on_next_frame(window, |_this, _window, cx| cx.notify());
        open.body_scroll.scroll_to_item(at);
        if let Some(source) = tab_source(open.tab) {
            let pane = open.pane(source);
            if let Some(row) = pane.rows.iter().position(|row| *row == focused) {
                pane.rows_scroll.scroll_to_item(row);
            }
        }
    }

    pub(super) fn render_move_in(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let Some(open) = &self.move_in.open else {
            return div().into_any_element();
        };
        let (body, pinned) = match tab_source(open.tab) {
            Some(source) => self.render_agent_pane(open, source, cx),
            None => self.render_by_prompt(open, cx),
        };
        self.scroll_move_in_to_focus(open, &body, window, cx);
        let body = body.into_iter().map(|part| part.element);
        let scroll = div()
            .id("desktop-move-in-body")
            .debug_selector(|| "desktop-move-in-body".to_string())
            .role(Role::TabPanel)
            .aria_label(tab_label(open.tab))
            .min_h_0()
            .flex_shrink_1()
            .overflow_y_scroll()
            .track_scroll(&open.body_scroll)
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pt_3()
            .pb_2()
            .children(body);
        let actions = div()
            .debug_selector(|| "desktop-move-in-actions".to_string())
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pb_3()
            .when(pinned.drawn, |el| el.pt_1())
            .children(pinned.items);
        bruhs_ui::dialog::dialog_backdrop(
            "desktop-move-in-overlay",
            bruhs_ui::dialog::DialogPlacement::Top,
        )
        .debug_selector(|| "desktop-move-in-overlay".to_string())
        .on_click(cx.listener(|this, _event, _window, cx| this.close_move_in(cx)))
        .child(
            bruhs_ui::dialog::dialog_card(
                "desktop-move-in-dialog",
                MOVE_IN_TITLE,
                Role::Dialog,
                &self.blur_scrim,
                cx,
            )
            .debug_selector(|| "desktop-move-in-dialog".to_string())
            .w(px(IMPORT_DIALOG_WIDTH_PX))
            .max_h(modal_ui::picker_max_height(window))
            .bg(theme::longan_800())
            .track_focus(&self.modal_start_focus_handle)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.as_str() != "escape"
                    || event.keystroke.modifiers != Modifiers::default()
                {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                // An open bruh list closes first.
                if !this.close_move_in_bruh_list(cx) {
                    this.close_move_in(cx);
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.trap_modal_tab_focus(event, window, cx);
            }))
            .on_click(cx.listener(|_this, _event, _window, cx| cx.stop_propagation()))
            .child(
                div()
                    .flex_shrink_0()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .child(
                        div()
                            .id("desktop-move-in-title")
                            .role(Role::Heading)
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_200())
                            .child(MOVE_IN_TITLE),
                    )
                    .child(self.render_move_in_tabs(open, cx)),
            )
            .child(scroll.vertical_scrollbar(&open.body_scroll))
            .child(actions)
            .child(self.render_modal_focus_sentinel()),
        )
        .into_any_element()
    }
}
