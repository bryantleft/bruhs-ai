//! The **Proposed memory** card on a bruh's Memory tab (D12), the desktop
//! half of the web's `crew/proposed_memory.rs`: every pending `SetMemory`
//! for the bruh, newest first, with its full text, a warning when
//! approving would undo a later change, and Approve or Decline through the
//! owner-approval path, never around it.
//!
//! It also holds what imported memory needs from the crew while it waits:
//! the proposal a bruh made from a persona sends once it exists (D11), and
//! Move in's By prompt proposals (D13).

use super::*;
use bruhs_models::imports::{
    memory_proposal_drops, pending_memory_proposals, propose_memory, ImportPrefill, APPROVE_MEMORY,
    DECLINE, MEMORY_CHANGED, PROPOSED_HINT, PROPOSED_MEMORY,
};
use bruhs_ui::scroll::ScrollableElement as _;
use std::cell::{OnceCell, RefCell};

/// Web `max-h-64`: a long proposal scrolls inside its card.
const TEXT_MAX_HEIGHT_PX: f32 = 256.0;
/// Web `bg-longan-950/60` behind the text and `inset-ring-longan-700/50`
/// around the card.
const TEXT_FILL_OPACITY: f32 = 0.6;
const CARD_RING_OPACITY: f32 = 0.5;
/// Web `aria-disabled:opacity-50`: a decision waits, keeping its focus.
const WAITING_OPACITY: f32 = 0.5;

/// Imported memory's state on the crew.
#[derive(Default)]
pub(super) struct ImportCrew {
    /// The bruh a persona's Create asked for, and its prefill: its memory
    /// is proposed once the bruh exists, even after the dialog closed.
    pub(super) follow_up: Option<(String, ImportPrefill)>,
    /// The card's region: focus lands here when no card is left.
    root: OnceCell<FocusHandle>,
    /// Each card's Approve, by proposal id: focus moves to the next one
    /// after a decision.
    approve: RefCell<HashMap<String, FocusHandle>>,
    /// The proposal just decided, with its bruh, until the service answers.
    deciding: Option<(String, String)>,
}

/// A test selector for one line of the card's shared copy.
fn copy(text: &str) -> String {
    format!("desktop-proposed-copy:{text}")
}

/// One pending proposal as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Card {
    pub(super) id: String,
    pub(super) memory: String,
    /// Approving would replace a change made after it was proposed.
    pub(super) stale: bool,
}

/// The bruh's pending memory proposals, newest first. Staleness is read
/// from the memory as it is now, on every draw, so a card already on
/// screen warns as soon as the memory changes under it.
pub(super) fn cards(snapshot: &ReactiveSnapshot, agent_id: &str) -> Vec<Card> {
    let current = snapshot
        .agents
        .iter()
        .find(|agent| agent.id == agent_id)
        .map_or("", |agent| agent.memory.as_str());
    pending_memory_proposals(snapshot, agent_id)
        .into_iter()
        .map(|pending| Card {
            id: pending.proposal_id.to_string(),
            memory: pending.memory.to_string(),
            stale: memory_proposal_drops(current, pending.memory),
        })
        .collect()
}

impl ReactiveAutomations {
    /// The card, above the bruh's agent memory. `None` until the crew has
    /// loaded; an empty region while nothing waits, so focus has somewhere
    /// to go after the last decision.
    pub(crate) fn render_proposed_memory(
        &self,
        agent_id: &str,
        cx: &Context<'_, Self>,
    ) -> Option<Div> {
        let snapshot = self.snapshot.as_ref()?;
        let list = cards(snapshot, agent_id);
        let imports = &self.crew.imports;
        let root = imports.root.get_or_init(|| cx.focus_handle()).clone();
        let approve: Vec<FocusHandle> = {
            let mut handles = imports.approve.borrow_mut();
            handles.retain(|id, _| list.iter().any(|card| card.id == *id));
            list.iter()
                .map(|card| {
                    handles
                        .entry(card.id.clone())
                        .or_insert_with(|| cx.focus_handle().tab_stop(true))
                        .clone()
                })
                .collect()
        };
        let locked = self.busy || imports.deciding.is_some();
        let section = (!list.is_empty()).then(|| {
            div()
                .debug_selector(|| "desktop-proposed-memory".to_string())
                .mb_4()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(heading(PROPOSED_MEMORY).debug_selector(|| copy(PROPOSED_MEMORY)))
                .child(muted(PROPOSED_HINT).debug_selector(|| copy(PROPOSED_HINT)))
                .children(list.into_iter().zip(approve).enumerate().map(
                    |(index, (card, approve))| {
                        self.render_proposal_card(agent_id, index, card, &approve, locked, cx)
                    },
                ))
        });
        Some(
            div()
                .id("desktop-proposed-root")
                .debug_selector(|| "desktop-proposed-root".to_string())
                .track_focus(&root)
                .role(Role::Region)
                .aria_label(PROPOSED_MEMORY)
                .min_w_0()
                .w_full()
                .rounded(px(8.0))
                .border_1()
                .border_color(gpui::transparent_black())
                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                .children(section),
        )
        .map(|root| div().w_full().min_w_0().child(root))
    }

    fn render_proposal_card(
        &self,
        agent_id: &str,
        index: usize,
        card: Card,
        approve: &FocusHandle,
        locked: bool,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let key = card.id.clone();
        let button = |name: &'static str, label: &'static str, yes: bool| {
            let (agent, id) = (agent_id.to_string(), card.id.clone());
            let el = web_button(
                SharedString::from(format!("desktop-proposed-{name}-{}", card.id)),
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector({
                let key = key.clone();
                move || format!("desktop-proposed-{name}-{key}")
            })
            .aria_label(label)
            .min_h(px(theme::CONTROL_PX))
            .px_3()
            .border_color(theme::longan_700())
            .text_color(theme::rambutan_200())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .when(locked, |el| {
                el.opacity(WAITING_OPACITY).cursor_not_allowed()
            })
            .child(div().debug_selector(move || copy(label)).child(label));
            bruhs_ui::press::on_press_view(el, cx, move |this, _window, cx| {
                this.decide_memory(&agent, &id, yes, cx);
            })
        };
        let text_id = SharedString::from(format!("desktop-proposed-text-{}", card.id));
        div()
            .id(("desktop-proposed-card", index))
            .debug_selector({
                let key = key.clone();
                move || format!("desktop-proposed-card-{key}")
            })
            .role(Role::Article)
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.0))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_700().opacity(CARD_RING_OPACITY))
            .when(card.stale, |el| {
                el.child(
                    div()
                        .debug_selector({
                            let key = key.clone();
                            move || format!("desktop-proposed-stale-{key}")
                        })
                        .text_size(px(12.0))
                        .line_height(relative(1.5))
                        .text_color(theme::durian_300())
                        .child(
                            div()
                                .debug_selector(|| copy(MEMORY_CHANGED))
                                .child(MEMORY_CHANGED),
                        ),
                )
            })
            .child(
                div()
                    .id(text_id.clone())
                    .debug_selector({
                        let memory = card.memory.clone();
                        move || format!("desktop-proposed-text:{memory}")
                    })
                    .role(Role::Region)
                    .aria_label(PROPOSED_MEMORY)
                    .tab_index(0)
                    .w_full()
                    .min_w_0()
                    .max_h(px(TEXT_MAX_HEIGHT_PX))
                    .p_2()
                    .rounded(px(6.0))
                    .bg(theme::longan_950().opacity(TEXT_FILL_OPACITY))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .child(updates::selectable_text(
                        SharedString::from(format!("{text_id}-body")),
                        &card.memory,
                    ))
                    .overflow_y_scrollbar(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(button("approve", APPROVE_MEMORY, true).track_focus(approve))
                    .child(button("decline", DECLINE, false)),
            )
    }

    /// Approve or Decline proposal `id`, through the owner-approval path.
    /// Focus moves to the next card's Approve, or to the region when none
    /// is left, so it never stays on a card that is going.
    fn decide_memory(
        &mut self,
        agent_id: &str,
        id: &str,
        approve: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.crew.imports.deciding.is_some() {
            return;
        }
        self.crew.imports.deciding = Some((agent_id.to_string(), id.to_string()));
        self.send_in(
            ReactiveCommand::DecideProposal {
                id: id.to_string(),
                approve,
            },
            Scope::Agent(agent_id.to_string()),
            cx,
        );
        cx.notify();
    }

    /// Once the service has answered a decision: the card it decided has
    /// gone, so focus moves to the next card's Approve, or to the region
    /// when none is left; a refusal leaves the card, and focus, where they
    /// are, ready to be pressed again (M3). Runs before each draw.
    pub(super) fn settle_memory_decision(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((agent_id, id)) = self.crew.imports.deciding.clone() else {
            return;
        };
        if self.crew_waiting() {
            return;
        }
        let left = self
            .snapshot
            .as_ref()
            .map(|snapshot| cards(snapshot, &agent_id))
            .unwrap_or_default();
        let still_here = left.iter().any(|card| card.id == id);
        if still_here && self.bruh_refusal(&agent_id).is_none() {
            // Accepted, and the snapshot without the card is on its way.
            return;
        }
        self.crew.imports.deciding = None;
        // A refusal, or the owner has moved to another page meanwhile.
        if still_here || self.bruh_page_agent() != Some(agent_id.as_str()) {
            return;
        }
        let next = {
            let handles = self.crew.imports.approve.borrow();
            left.iter().find_map(|card| handles.get(&card.id).cloned())
        };
        if let Some(target) = next.or_else(|| self.crew.imports.root.get().cloned()) {
            cx.defer_in(window, move |_this, window, cx| target.focus(window, cx));
        }
    }

    /// Every bruh, by id and name, for By prompt's **For bruh**; `None`
    /// until the crew has loaded, so "Make a bruh first." never flashes.
    pub(crate) fn move_in_bruhs(&self) -> Option<Vec<(String, String)>> {
        self.crew.protocol.map(|_| {
            self.crew
                .bruhs
                .iter()
                .map(|bruh| (bruh.agent_id.clone(), bruh.name.clone()))
                .collect()
        })
    }

    /// The bruh's memory as the service last reported it.
    pub(crate) fn bruh_memory(&self, agent_id: &str) -> String {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.agents.iter().find(|agent| agent.id == agent_id))
            .map(|agent| agent.memory.clone())
            .unwrap_or_default()
    }

    /// Propose `memory` as the bruh's memory. It waits behind any command
    /// already on its way; a refusal shows on the bruh's own error line.
    pub(crate) fn propose_bruh_memory(
        &mut self,
        agent_id: &str,
        memory: String,
        cx: &mut Context<'_, Self>,
    ) {
        self.send_all(
            vec![propose_memory(agent_id.to_string(), memory)],
            Scope::Agent(agent_id.to_string()),
            cx,
        );
        cx.notify();
    }

    /// A command is on its way or waiting: a proposal hasn't answered yet.
    pub(crate) fn crew_waiting(&self) -> bool {
        self.busy || !self.queued.is_empty()
    }

    /// Why the service last refused a command for this bruh, if it did.
    pub(crate) fn bruh_refusal(&self, agent_id: &str) -> Option<String> {
        self.scoped_error
            .as_ref()
            .filter(|(scope, _)| *scope == Scope::Agent(agent_id.to_string()))
            .map(|(_, message)| message.clone())
    }

    /// Remember that the bruh asked for as `agent_id` came from `prefill`.
    pub(super) fn expect_import_follow_up(&mut self, agent_id: String, prefill: ImportPrefill) {
        self.crew.imports.follow_up = Some((agent_id, prefill));
    }

    /// Bruh `agent_id` exists: send its imported memory as a proposal,
    /// once, when one waits for it. `send_all` waits behind a busy queue
    /// instead of dropping the proposal (F28).
    pub(super) fn send_import_follow_up(&mut self, agent_id: &str, cx: &mut Context<'_, Self>) {
        let waited = self
            .crew
            .imports
            .follow_up
            .as_ref()
            .is_some_and(|(id, _)| id == agent_id);
        if !waited {
            return;
        }
        let Some((_, prefill)) = self.crew.imports.follow_up.take() else {
            return;
        };
        if let Some(command) = bruhs_models::imports::after_created(&prefill, agent_id) {
            self.send_all(vec![command], Scope::Agent(agent_id.to_string()), cx);
        }
    }

    /// The create the follow-up waited for was refused: nothing follows.
    pub(super) fn drop_import_follow_up(&mut self) {
        self.crew.imports.follow_up = None;
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE`: a crew of one bruh, Pinch, whose
    /// memory has a pending proposal (with `stale`, written over since),
    /// and no server reads over it. Returns its id.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_memory_fixture(
        &mut self,
        stale: bool,
        cx: &mut Context<'_, Self>,
    ) -> String {
        use bruhs_models::crew::{BruhAccess, BruhLook, BruhState, BruhStatus, CREW_PROTOCOL};
        let id = "smoke-pinch".to_string();
        let proposed = "## OpenClaw: USER.md\n\nLikes short answers.\nWorks on Bruhs & friends.\n\n## Claude.ai export\n\nPrefers mornings for deep work.";
        let snapshot = ReactiveSnapshot {
            protocol: CREW_PROTOCOL,
            agents: vec![ReactiveAgent {
                id: id.clone(),
                name: "Pinch".into(),
                instructions: "## SOUL.md\n\nBe kind and brief.".into(),
                memory: if stale {
                    "Written after the proposal.".into()
                } else {
                    String::new()
                },
                model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
                project_path: String::new(),
                enabled: true,
            }],
            bruhs: vec![BruhState {
                agent_id: id.clone(),
                name: "Pinch".into(),
                look: BruhLook::default(),
                access: BruhAccess::default(),
                effort: None,
                status: BruhStatus::Asleep,
                next_wake_ms: None,
                unread: 0,
                computer: false,
                vault: Vec::new(),
                apps: Vec::new(),
                mail: Vec::new(),
            }],
            proposals: vec![ReactiveProposal {
                id: "smoke-proposal".into(),
                change: ReactiveChange::SetMemory {
                    agent_id: id.clone(),
                    memory: proposed.into(),
                },
                status: ProposalStatus::Pending,
                created_at: bruhs_models::EpochMs::saturating_from_i64(1_700_000_000_000),
            }],
            ..ReactiveSnapshot::default()
        };
        self.loaded = true;
        self.set_snapshot(snapshot, cx);
        cx.notify();
        id
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE=bruh-memory-proposed`: the bruh's Memory tab.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_show_bruh_memory(&mut self, agent_id: &str, cx: &mut Context<'_, Self>) {
        self.show_bruh(Some(agent_id.to_string()), cx);
        self.set_bruh_tab(super::bruh_page::BruhTab::Memory, cx);
    }

    /// The service's answer to the queue's last command: drop the busy
    /// hold and send what waits, as a real answer would.
    #[cfg(test)]
    pub(crate) fn answer_queue_for_test(&mut self, cx: &mut Context<'_, Self>) {
        self.busy = false;
        if let Some((next, scope)) = self.queued.pop_front() {
            self.send_in(next, scope, cx);
        }
        cx.notify();
    }

    /// Refuse the command waiting for bruh `agent_id`, as the service would.
    #[cfg(test)]
    pub(crate) fn refuse_for_test(
        &mut self,
        agent_id: &str,
        error: &str,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = false;
        self.command_failed(Scope::Agent(agent_id.to_string()), error.to_string());
        cx.notify();
    }
}

/// Draws the bruh page's Memory tab card by itself.
#[cfg(test)]
pub(super) struct ProposedHost(pub(super) Entity<ReactiveAutomations>, pub(super) String);

#[cfg(test)]
impl Render for ProposedHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let agent = self.1.clone();
        let body = self.0.update(cx, |panel, cx| {
            panel.settle_memory_decision(_window, cx);
            panel.render_proposed_memory(&agent, cx)
        });
        div().w(px(640.0)).children(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{cards, ProposedHost, ReactiveAutomations};
    use crate::theme;
    use bruhs_models::crew::CREW_PROTOCOL;
    use bruhs_models::imports::{
        APPROVE_MEMORY, DECLINE, MEMORY_CHANGED, PROPOSED_HINT, PROPOSED_MEMORY,
    };
    use bruhs_models::reactive::{
        ProposalStatus, ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveProposal,
        ReactiveSnapshot,
    };
    use bruhs_models::EpochMs;
    use gpui::{AppContext as _, Entity, FocusHandle, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn agent(id: &str, memory: &str) -> ReactiveAgent {
        ReactiveAgent {
            id: id.into(),
            name: format!("Bruh {id}"),
            instructions: String::new(),
            memory: memory.into(),
            model: String::new(),
            project_path: String::new(),
            enabled: true,
        }
    }

    fn proposal(id: &str, agent_id: &str, memory: &str, at: i64) -> ReactiveProposal {
        ReactiveProposal {
            id: id.into(),
            change: ReactiveChange::SetMemory {
                agent_id: agent_id.into(),
                memory: memory.into(),
            },
            status: ProposalStatus::Pending,
            created_at: EpochMs::saturating_from_i64(at),
        }
    }

    /// The web's fixture (Task 11).
    const TEXT: &str = "## Claude.ai export\n\nLikes short answers.\nWorks on Bruhs & friends.";

    fn snapshot() -> ReactiveSnapshot {
        ReactiveSnapshot {
            protocol: CREW_PROTOCOL,
            agents: vec![agent("a", ""), agent("b", "")],
            proposals: vec![
                proposal("p1", "a", TEXT, 10),
                proposal("p2", "b", "Another bruh's memory", 20),
            ],
            ..ReactiveSnapshot::default()
        }
    }

    fn open(
        cx: &mut gpui::TestAppContext,
        snapshot: ReactiveSnapshot,
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
            let host = cx.new(|_| ProposedHost(panel, "a".into()));
            bruhs_ui::Root::new(host, window, cx)
        });
        let panel = holder.borrow().clone().unwrap();
        panel.update(cx, |panel, cx| {
            panel.show_crew_for_test(Vec::new(), CREW_PROTOCOL, cx);
            panel.show_snapshot_for_test(snapshot, cx);
            panel.show_bruh(Some("a".into()), cx);
        });
        cx.run_until_parked();
        (panel, cx)
    }

    fn drawn(cx: &mut VisualTestContext, selector: String) -> bool {
        cx.debug_bounds(Box::leak(selector.into_boxed_str()))
            .is_some()
    }

    #[test]
    fn the_newest_proposal_comes_first_and_only_this_bruhs() {
        let mut snapshot = snapshot();
        snapshot
            .proposals
            .push(proposal("p3", "a", "Newer text", 30));
        let ids: Vec<String> = cards(&snapshot, "a").into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["p3", "p1"]);
        let card = cards(&snapshot, "a").pop().unwrap();
        assert_eq!(card.memory, TEXT, "the whole text");
        assert!(!card.stale);
    }

    #[gpui::test]
    fn proposed_memory_shows_full_text_and_warns_when_stale(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, snapshot());
        for selector in [
            "desktop-proposed-memory".to_string(),
            "desktop-proposed-card-p1".to_string(),
            format!("desktop-proposed-text:{TEXT}"),
            "desktop-proposed-approve-p1".to_string(),
            "desktop-proposed-decline-p1".to_string(),
        ] {
            assert!(drawn(cx, selector.clone()), "{selector}");
        }
        assert!(
            !drawn(cx, "desktop-proposed-card-p2".into()),
            "another bruh's"
        );
        assert!(!drawn(cx, "desktop-proposed-stale-p1".into()));
        // The heading, the hint and both buttons draw the shared copy.
        for label in [PROPOSED_MEMORY, PROPOSED_HINT, APPROVE_MEMORY, DECLINE] {
            assert!(
                drawn(cx, format!("desktop-proposed-copy:{label}")),
                "{label}"
            );
        }
        assert!(!drawn(
            cx,
            format!("desktop-proposed-copy:{MEMORY_CHANGED}")
        ));

        // The memory changes while the card shows: it warns at once.
        panel.update(cx, |panel, cx| {
            let mut stale = snapshot();
            stale.agents[0].memory = "Written after the proposal.".into();
            panel.show_snapshot_for_test(stale, cx);
        });
        cx.run_until_parked();
        assert!(drawn(cx, "desktop-proposed-stale-p1".into()));
        assert!(drawn(cx, format!("desktop-proposed-copy:{MEMORY_CHANGED}")));

        // Trailing spaces on the current memory aren't a change (F31).
        panel.update(cx, |panel, cx| {
            let mut spaced = snapshot();
            spaced.agents[0].memory = "## Claude.ai export  \n".into();
            panel.show_snapshot_for_test(spaced, cx);
        });
        cx.run_until_parked();
        assert!(!drawn(cx, "desktop-proposed-stale-p1".into()));

        // Decided: no card.
        panel.update(cx, |panel, cx| {
            let mut decided = snapshot();
            decided.proposals[0].status = ProposalStatus::Approved;
            panel.show_snapshot_for_test(decided, cx);
        });
        cx.run_until_parked();
        assert!(!drawn(cx, "desktop-proposed-memory".into()));
    }

    #[gpui::test]
    fn a_long_proposal_wraps_and_scrolls_inside_its_card(cx: &mut gpui::TestAppContext) {
        let long = "Likes short answers and long walks. ".repeat(400);
        let mut big = snapshot();
        big.proposals[0] = proposal("p1", "a", &long, 10);
        let (_panel, cx) = open(cx, big);
        let card = cx.debug_bounds("desktop-proposed-card-p1").expect("card");
        let text = cx
            .debug_bounds(Box::leak(
                format!("desktop-proposed-text:{long}").into_boxed_str(),
            ))
            .expect("text");
        assert!(text.size.width <= card.size.width, "{text:?} in {card:?}");
        // The text scrolls in a box no taller than the web's `max-h-64`:
        // the card holds it, its warning and its buttons in far less.
        assert!(card.size.height <= gpui::px(360.0), "{card:?}");
        let approve = cx
            .debug_bounds("desktop-proposed-approve-p1")
            .expect("approve");
        assert!(approve.bottom() <= card.bottom(), "{approve:?} in {card:?}");
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).expect(selector);
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    fn decide(id: &str, approve: bool) -> ReactiveCommand {
        ReactiveCommand::DecideProposal {
            id: id.into(),
            approve,
        }
    }

    fn two_cards() -> ReactiveSnapshot {
        let mut two = snapshot();
        two.proposals.push(proposal("p3", "a", "Newer text", 30));
        two
    }

    /// The service's answer: the snapshot with `decided` no longer pending.
    fn answered(panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext, decided: &[&str]) {
        panel.update(cx, |panel, cx| {
            let mut next = two_cards();
            for proposal in &mut next.proposals {
                if decided.contains(&proposal.id.as_str()) {
                    proposal.status = ProposalStatus::Approved;
                }
            }
            panel.show_snapshot_for_test(next, cx);
        });
        cx.run_until_parked();
    }

    fn sent(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
    ) -> Vec<ReactiveCommand> {
        panel.read_with(cx, |panel, _| panel.sent_for_test().to_vec())
    }

    #[gpui::test]
    fn approve_and_decline_go_through_the_owner_path(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, two_cards());
        click(cx, "desktop-proposed-approve-p3");
        // Until the service answers, the other card waits too.
        click(cx, "desktop-proposed-decline-p1");
        assert_eq!(sent(&panel, cx), [decide("p3", true)]);
        answered(&panel, cx, &["p3"]);
        click(cx, "desktop-proposed-decline-p1");
        assert_eq!(sent(&panel, cx), [decide("p3", true), decide("p1", false)]);
        // A refusal frees the card; while another command is on its way, a
        // decision still waits.
        panel.update(cx, |panel, cx| panel.refuse_for_test("a", "No.", cx));
        cx.run_until_parked();
        panel.update(cx, |panel, _| panel.set_busy_for_test(true));
        click(cx, "desktop-proposed-approve-p1");
        assert_eq!(sent(&panel, cx).len(), 2);
    }

    fn press_enter(cx: &mut VisualTestContext) {
        let keystroke = gpui::Keystroke::parse("enter").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
    }

    fn focused(cx: &mut VisualTestContext) -> Option<FocusHandle> {
        cx.update(|window, cx| window.focused(cx))
    }

    fn approve_handle(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
        id: &str,
    ) -> FocusHandle {
        panel.read_with(cx, |panel, _| {
            panel
                .crew
                .imports
                .approve
                .borrow()
                .get(id)
                .cloned()
                .expect("Approve")
        })
    }

    #[gpui::test]
    fn focus_moves_once_the_decided_card_goes(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, two_cards());
        let p3 = approve_handle(&panel, cx, "p3");
        let p1 = approve_handle(&panel, cx, "p1");
        let root = panel.read_with(cx, |panel, _| {
            panel.crew.imports.root.get().cloned().expect("root")
        });
        cx.update(|window, cx| window.focus(&p3, cx));
        press_enter(cx);
        assert_eq!(sent(&panel, cx), [decide("p3", true)]);
        assert!(
            focused(cx).as_ref() == Some(&p3),
            "focus waits for the answer"
        );
        answered(&panel, cx, &["p3"]);
        assert!(
            focused(cx).as_ref() == Some(&p1),
            "then the next card's Approve"
        );
        press_enter(cx);
        answered(&panel, cx, &["p3", "p1"]);
        assert!(focused(cx).as_ref() == Some(&root), "the region, none left");
        assert!(drawn(cx, "desktop-proposed-root".into()));
    }

    #[gpui::test]
    fn a_refused_decision_keeps_the_card_and_focus(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, two_cards());
        let p3 = approve_handle(&panel, cx, "p3");
        cx.update(|window, cx| window.focus(&p3, cx));
        press_enter(cx);
        panel.update(cx, |panel, cx| panel.refuse_for_test("a", "Too late.", cx));
        cx.run_until_parked();
        assert!(focused(cx).as_ref() == Some(&p3), "focus stays on the card");
        assert!(drawn(cx, "desktop-proposed-card-p3".into()));
        // It can be pressed again.
        press_enter(cx);
        assert_eq!(sent(&panel, cx), [decide("p3", true), decide("p3", true)]);
    }
}
