//! Move in's By prompt tab (D13), the desktop half of the web's
//! `crew/move_in/by_prompt.rs`: a prompt to paste into another assistant,
//! and its answer proposed as a chosen bruh's memory. The answer joins the
//! bruh's memory only as a proposal the owner approves on its Memory tab.

use super::*;
use bruhs_models::imports::{
    memory_proposed_line, moved_in_from, moved_in_memory, prefill_for_answer, COPIED, COPY_PROMPT,
    FOR_BRUH, MAKE_A_BRUH, MOVE_IN_HINT, MOVE_IN_PROMPT, NO_BRUHS, PROPOSE_MEMORY, THEIR_ANSWER,
};
use bruhs_ui::text::TextView;
use std::time::Duration;

/// How long **Copy prompt** says it copied.
const COPIED_FOR: Duration = Duration::from_millis(2_000);
/// Web `Textarea rows=6`: six 21px lines, `py-2` and the 1px ring.
const ANSWER_FIELD_PX: f32 = 139.0;
/// The bruh list's option radius and hover, as the New bruh role list.
const OPTION_RADIUS_PX: f32 = 6.0;
const OPTION_HOVER_OPACITY: f32 = 0.60;
/// Web `max-h-40`: the prompt scrolls in its own box, so the fields below
/// it fit at 640 by 480.
const PROMPT_MAX_HEIGHT_PX: f32 = 160.0;

/// The heading a pasted answer goes under: today's date here.
fn pasted_from_today() -> String {
    moved_in_from(&chrono::Local::now().format("%Y-%m-%d").to_string())
}

/// The By prompt tab's text, choice and outcome. Move in owns it, so
/// switching tabs mid-proposal keeps the answer and the confirmation.
pub(in crate::views::workspace) struct ByPrompt {
    pub(in crate::views::workspace) answer: Entity<TextareaState>,
    /// The heading the answer goes under, when it came from an export.
    pub(in crate::views::workspace) from: Option<String>,
    /// The bruh chosen, by id; the first while none is.
    pub(in crate::views::workspace) chosen: Option<String>,
    pub(in crate::views::workspace) list_open: bool,
    /// `memory_proposed_line` once a proposal is accepted.
    pub(in crate::views::workspace) status: Option<String>,
    /// The answer's own refusal copy, or the service's reason.
    pub(in crate::views::workspace) error: Option<String>,
    /// The bruh (id, name) whose proposal is on its way.
    pub(in crate::views::workspace) proposing: Option<(String, String)>,
    pub(in crate::views::workspace) copied: bool,
    pub(in crate::views::workspace) copy: FocusHandle,
    pub(in crate::views::workspace) bruh: FocusHandle,
    pub(in crate::views::workspace) propose: FocusHandle,
    pub(in crate::views::workspace) make_bruh: FocusHandle,
    copied_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ByPrompt {
    /// The tab, its area filled with `text` when Move in opened with some.
    pub(in crate::views::workspace) fn new(
        text: Option<String>,
        from: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
    ) -> Self {
        let answer = cx.new(|cx| TextareaState::new(window, cx).rows(6));
        if let Some(text) = text {
            answer.update(cx, |input, cx| input.set_value(&text, window, cx));
        }
        // Cmd+Enter proposes. The field never inserts a newline for it,
        // so nothing has to be taken back out of the answer.
        let subscriptions = vec![cx.subscribe_in(
            &answer,
            window,
            |this: &mut Workspace, _input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: true, ..
                } => this.propose_move_in(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        )];
        let stop = || cx.focus_handle().tab_stop(true);
        Self {
            answer,
            from,
            chosen: None,
            list_open: false,
            status: None,
            error: None,
            proposing: None,
            copied: false,
            copy: stop(),
            bruh: stop(),
            propose: stop(),
            make_bruh: stop(),
            copied_task: None,
            _subscriptions: subscriptions,
        }
    }

    /// The bruh a proposal goes to: the chosen one while it is still in the
    /// crew, else the first.
    pub(in crate::views::workspace) fn target(
        &self,
        bruhs: &[(String, String)],
    ) -> Option<(String, String)> {
        self.chosen
            .as_ref()
            .and_then(|id| bruhs.iter().find(|(each, _)| each == id))
            .or_else(|| bruhs.first())
            .cloned()
    }
}

impl Workspace {
    fn by_prompt(&self) -> Option<&ByPrompt> {
        self.move_in.open.as_ref().map(|open| &open.by_prompt)
    }

    fn by_prompt_mut(&mut self) -> Option<&mut ByPrompt> {
        self.move_in.open.as_mut().map(|open| &mut open.by_prompt)
    }

    /// A proposal can't go yet: one is on its way, or the crew is busy.
    fn by_prompt_waiting(&self, cx: &App) -> bool {
        self.reactive_automations.read(cx).crew_waiting()
            || self.by_prompt().is_some_and(|tab| tab.proposing.is_some())
    }

    /// **Copy prompt**: the prompt on the clipboard, and `COPIED` for 2 s.
    pub(in crate::views::workspace) fn copy_move_in_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(MOVE_IN_PROMPT.to_string()));
        let reset = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _closed = this.update(cx, |this, cx| {
                if let Some(tab) = this.by_prompt_mut() {
                    tab.copied = false;
                    cx.notify();
                }
            });
        });
        if let Some(tab) = self.by_prompt_mut() {
            tab.copied = true;
            tab.copied_task = Some(reset);
        }
        cx.notify();
    }

    /// **Propose memory** (or Cmd+Enter in Their answer): the answer,
    /// added to the bruh's memory under its heading, as a proposal.
    pub(in crate::views::workspace) fn propose_move_in(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.by_prompt_waiting(cx) {
            return;
        }
        let bruhs = self
            .reactive_automations
            .read(cx)
            .move_in_bruhs()
            .unwrap_or_default();
        let Some(tab) = self.by_prompt() else {
            return;
        };
        let answer = tab.answer.read(cx).value().to_string();
        if answer.trim().is_empty() {
            return;
        }
        let Some((agent_id, name)) = tab.target(&bruhs) else {
            return;
        };
        let heading = tab.from.clone().unwrap_or_else(pasted_from_today);
        let current = self.reactive_automations.read(cx).bruh_memory(&agent_id);
        let proposal = moved_in_memory(&current, &answer, &heading);
        let Some(tab) = self.by_prompt_mut() else {
            return;
        };
        tab.status = None;
        match proposal {
            Err(copy) => tab.error = Some(copy.to_owned()),
            Ok(memory) => {
                tab.error = None;
                tab.proposing = Some((agent_id.clone(), name));
                self.reactive_automations.update(cx, |panel, cx| {
                    panel.propose_bruh_memory(&agent_id, memory, cx);
                });
            }
        }
        cx.notify();
    }

    /// The crew answered: an accepted proposal says where to approve it
    /// and clears the area; a refused one keeps the text, with the
    /// service's reason.
    pub(in crate::views::workspace) fn settle_move_in_proposal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((agent_id, name)) = self.by_prompt().and_then(|tab| tab.proposing.clone()) else {
            return;
        };
        let panel = self.reactive_automations.read(cx);
        if panel.crew_waiting() {
            return;
        }
        let refusal = panel.bruh_refusal(&agent_id);
        let Some(tab) = self.by_prompt_mut() else {
            return;
        };
        tab.proposing = None;
        match refusal {
            Some(reason) => tab.error = Some(reason),
            None => {
                tab.status = Some(memory_proposed_line(&name));
                tab.from = None;
                let answer = tab.answer.clone();
                answer.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
        cx.notify();
    }

    /// **Make a bruh**, with no bruh yet: New bruh, with the answer in the
    /// area waiting as its memory when there is one. Text over the cap is
    /// refused with its copy and stays.
    pub(in crate::views::workspace) fn make_bruh_from_answer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.crew_make_state(cx).0 {
            return;
        }
        let Some(tab) = self.by_prompt() else {
            return;
        };
        let heading = tab.from.clone().unwrap_or_else(pasted_from_today);
        let answer = tab.answer.read(cx).value().to_string();
        match prefill_for_answer(&answer, &heading) {
            Err(copy) => {
                if let Some(tab) = self.by_prompt_mut() {
                    tab.error = Some(copy.to_owned());
                }
                cx.notify();
            }
            Ok(Some(prefill)) => self.open_create_bruh_from_import_dialog(prefill, window, cx),
            Ok(None) => self.open_create_bruh_dialog(window, cx),
        }
    }

    fn toggle_move_in_bruh_list(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(tab) = self.by_prompt_mut() {
            tab.list_open = !tab.list_open;
            cx.notify();
        }
    }

    /// Closes the bruh list, if it is open; whether it was.
    pub(in crate::views::workspace) fn close_move_in_bruh_list(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(tab) = self.by_prompt_mut() else {
            return false;
        };
        let was = std::mem::take(&mut tab.list_open);
        if was {
            cx.notify();
        }
        was
    }

    fn choose_move_in_bruh(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(tab) = self.by_prompt_mut() else {
            return;
        };
        tab.chosen = Some(id);
        tab.list_open = false;
        let trigger = tab.bruh.clone();
        trigger.focus(window, cx);
        cx.notify();
    }

    /// **For bruh**: a trigger naming the target and, while open, every
    /// bruh below it.
    fn render_bruh_picker(
        &self,
        tab: &ByPrompt,
        bruhs: &[(String, String)],
        target: Option<&(String, String)>,
        cx: &Context<'_, Self>,
    ) -> Div {
        let current = target.map(|(_, name)| name.clone()).unwrap_or_default();
        let trigger = div()
            .id("desktop-move-in-bruh")
            .debug_selector(|| "desktop-move-in-bruh".to_string())
            .track_focus(&tab.bruh)
            .role(Role::ComboBox)
            .aria_label(FOR_BRUH)
            .aria_expanded(tab.list_open)
            .h(px(theme::CONTROL_PX))
            .w_full()
            .px(px(10.0))
            .rounded(px(IMPORT_RADIUS_PX))
            .bg(theme::longan_950())
            .border_1()
            .border_color(theme::longan_700())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
            .text_color(theme::rambutan_100())
            .child(div().flex_1().min_w_0().truncate().child(current))
            .child(
                svg()
                    .path(crate::assets::CHEVRON_DOWN_ICON_PATH)
                    .size(px(theme::ICON_PX))
                    .text_color(theme::rambutan_400()),
            );
        let trigger = on_press_view(trigger, cx, |this, _window, cx| {
            this.toggle_move_in_bruh_list(cx);
        });
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(move_in_label(FOR_BRUH))
            .child(trigger)
            .when(tab.list_open, |el| {
                el.child(
                    div()
                        .id("desktop-move-in-bruh-list")
                        .role(Role::ListBox)
                        .aria_label(FOR_BRUH)
                        .p(px(4.0))
                        .rounded(px(IMPORT_RADIUS_PX))
                        .bg(theme::longan_800())
                        .border_1()
                        .border_color(theme::ring_modal())
                        .flex()
                        .flex_col()
                        .children(bruhs.iter().enumerate().map(|(index, (id, name))| {
                            let chosen = target.is_some_and(|(each, _)| each == id);
                            let option = div()
                                .id(("desktop-move-in-bruh-option", index))
                                .debug_selector({
                                    let name = name.clone();
                                    move || format!("desktop-move-in-option:{name}")
                                })
                                .role(Role::ListBoxOption)
                                .aria_label(name.clone())
                                .aria_selected(chosen)
                                .tab_index(0)
                                .min_h(px(theme::CONTROL_PX))
                                .px_2()
                                .rounded(px(OPTION_RADIUS_PX))
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                                .text_color(if chosen {
                                    theme::rambutan_100()
                                } else {
                                    theme::rambutan_300()
                                })
                                .when(chosen, |el| el.bg(theme::longan_700()))
                                .hover(|el| {
                                    el.bg(theme::longan_700().opacity(OPTION_HOVER_OPACITY))
                                })
                                .border_1()
                                .border_color(gpui::transparent_black())
                                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                .child(name.clone());
                            let id = id.clone();
                            on_press_view(option, cx, move |this, window, cx| {
                                this.choose_move_in_bruh(id.clone(), window, cx);
                            })
                        })),
                )
            })
    }

    /// The tab's body, and the action and status lines pinned below it.
    pub(in crate::views::workspace) fn render_by_prompt(
        &self,
        open: &MoveIn,
        cx: &Context<'_, Self>,
    ) -> (Vec<BodyPart>, Vec<AnyElement>) {
        let tab = &open.by_prompt;
        let bruhs = self.reactive_automations.read(cx).move_in_bruhs();
        let target = bruhs.as_deref().and_then(|bruhs| tab.target(bruhs));
        let waiting = self.by_prompt_waiting(cx);
        let empty = tab.answer.read(cx).value().trim().is_empty();
        let mut body = vec![
            BodyPart::text(
                div()
                    .id("desktop-move-in-prompt")
                    .debug_selector(|| format!("desktop-move-in-prompt:{MOVE_IN_PROMPT}"))
                    .w_full()
                    .min_w_0()
                    .p_3()
                    .rounded(px(IMPORT_RADIUS_PX))
                    .bg(theme::longan_950().opacity(PANEL_FILL_OPACITY))
                    .border_1()
                    .border_color(theme::longan_700().opacity(PANEL_RING_OPACITY))
                    .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
                    .line_height(relative(1.5))
                    .text_color(theme::rambutan_200())
                    .max_h(px(PROMPT_MAX_HEIGHT_PX))
                    // A scroll box would otherwise shrink to nothing in the body.
                    .flex_shrink_0()
                    .child(
                        TextView::markdown("desktop-move-in-prompt-text", MOVE_IN_PROMPT)
                            .selectable(true),
                    )
                    .overflow_y_scrollbar(),
            ),
            BodyPart::new(
                div().flex().child(move_in_button(
                    if tab.copied { COPIED } else { COPY_PROMPT },
                    &tab.copy,
                    false,
                    cx,
                    |this, window, cx| this.copy_move_in_prompt(window, cx),
                )),
                vec![tab.copy.clone()],
            ),
            BodyPart::text(move_in_text(
                "hint",
                MOVE_IN_HINT.to_owned(),
                theme::rambutan_300(),
            )),
        ];
        let (ready, reason) = self.crew_make_state(cx);
        match bruhs.as_deref() {
            // Before the crew loads, neither the picker nor "Make a bruh
            // first." shows.
            None => {}
            Some([]) => body.push(BodyPart::new(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(move_in_text(
                        "no-bruhs",
                        NO_BRUHS.to_owned(),
                        theme::rambutan_200(),
                    ))
                    .child(
                        move_in_button(
                            MAKE_A_BRUH,
                            &tab.make_bruh,
                            !ready,
                            cx,
                            |this, window, cx| this.make_bruh_from_answer(window, cx),
                        )
                        .when_some(reason, |el, reason| el.aria_description(reason)),
                    ),
                vec![tab.make_bruh.clone()],
            )),
            Some(bruhs) => body.push(BodyPart::new(
                self.render_bruh_picker(tab, bruhs, target.as_ref(), cx),
                vec![tab.bruh.clone()],
            )),
        }
        body.push(BodyPart::new(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(move_in_label(THEIR_ANSWER))
                .child(
                    div()
                        .debug_selector(|| "desktop-move-in-answer".to_string())
                        .child(
                            Textarea::new(&tab.answer)
                                .aria_label(THEIR_ANSWER)
                                .h(px(ANSWER_FIELD_PX))
                                .px(px(10.0))
                                .font_family(theme::CODE_FONT_FAMILY)
                                .text_size(px(theme::APP_INPUT_TEXT_SIZE_PX)),
                        ),
                ),
            vec![tab.answer.focus_handle(cx)],
        ));
        let mut pinned = Vec::new();
        if bruhs.as_ref().is_some_and(|bruhs| !bruhs.is_empty()) {
            pinned.push(
                div()
                    .flex()
                    .child(move_in_button(
                        PROPOSE_MEMORY,
                        &tab.propose,
                        empty || waiting,
                        cx,
                        |this, window, cx| this.propose_move_in(window, cx),
                    ))
                    .into_any_element(),
            );
        }
        if let Some(status) = tab.status.clone() {
            pinned.push(move_in_text("status", status, theme::pandan_300()).into_any_element());
        }
        if let Some(error) = tab.error.clone() {
            pinned.push(move_in_text("error", error, theme::lychee_300()).into_any_element());
        }
        (body, pinned)
    }
}
