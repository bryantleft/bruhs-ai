//! The **Proposed memory** card on a bruh's Memory tab (D12): every
//! pending `SetMemory` for the bruh, newest first, with its full text, a
//! warning when approving would undo a later change, and Approve or
//! Decline. Approving goes through the owner-approval path, never around it.

use bruhs_models::imports::{
    memory_proposal_drops, pending_memory_proposals, APPROVE_MEMORY, DECLINE, MEMORY_CHANGED,
    PROPOSED_HINT, PROPOSED_MEMORY,
};
use bruhs_models::reactive::{ReactiveCommand, ReactiveSnapshot};
use leptos::{html, prelude::*};

use super::data::CrewData;

const BUTTON: &str = "bruhs-control min-h-control shrink-0 cursor-pointer rounded-seed px-3 text-label-sm text-rambutan-200 inset-ring inset-ring-longan-700 transition-colors duration-150 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 aria-disabled:cursor-not-allowed aria-disabled:opacity-50";

/// One pending proposal as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Card {
    id: String,
    memory: String,
    /// Approving would replace a change made after it was proposed.
    stale: bool,
}

/// The bruh's pending memory proposals, newest first.
fn cards(snapshot: &ReactiveSnapshot, agent_id: &str) -> Vec<Card> {
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

/// The bruh's cards, following the snapshot.
fn proposal_cards(
    agent: StoredValue<String>,
    snapshot: RwSignal<Option<ReactiveSnapshot>>,
) -> Memo<Vec<Card>> {
    Memo::new(move |_| {
        snapshot.with(|snapshot| {
            snapshot
                .as_ref()
                .map(|snapshot| agent.with_value(|id| cards(snapshot, id)))
                .unwrap_or_default()
        })
    })
}

/// Whether card `id` is stale now. A card stays drawn while its proposal
/// waits, so the warning reads the list on each draw, not the card's first
/// value: memory approved or edited after it showed flags it at once (I1).
fn card_stale(
    list: Memo<Vec<Card>>,
    id: String,
) -> impl Fn() -> bool + Clone + Send + Sync + 'static {
    move || list.with(|cards| cards.iter().any(|card| card.id == id && card.stale))
}

#[component]
pub fn ProposedMemory(
    agent_id: String,
    snapshot: RwSignal<Option<ReactiveSnapshot>>,
    send: Callback<ReactiveCommand>,
) -> impl IntoView {
    let agent = StoredValue::new(agent_id);
    // The page's queue: a decision waits while another command is on its way.
    let busy = use_context::<CrewData>().map(|data| data.busy);
    let waiting = move || busy.is_some_and(|busy| busy.get());
    let list = proposal_cards(agent, snapshot);
    let root: NodeRef<html::Div> = NodeRef::new();
    // The proposal just decided, until it leaves the list.
    let deciding = RwSignal::new(None::<String>);

    // Once a decided card goes, focus moves to the next card's Approve, or
    // to the region, never to the page behind.
    Effect::new(move |_| {
        let Some(id) = deciding.get() else {
            return;
        };
        let still_here = list.with(|cards| cards.iter().any(|card| card.id == id));
        if still_here {
            // A refused decision leaves the card: let it be pressed again.
            if !waiting() {
                deciding.set(None);
            }
            return;
        }
        deciding.set(None);
        focus_after_decision(root);
    });

    let decide = move |id: String, approve: bool| {
        if untrack(waiting) || deciding.get_untracked().is_some() {
            return;
        }
        deciding.set(Some(id.clone()));
        send.run(ReactiveCommand::DecideProposal { id, approve });
    };
    let card_view = move |card: Card| {
        let stale = card_stale(list, card.id.clone());
        let warning_id = format!("proposed-stale-{}", card.id);
        let approve_id = card.id.clone();
        let decline_id = card.id.clone();
        let locked = move || waiting() || deciding.with(Option::is_some);
        view! {
            <article data-proposed-card="" class="min-w-0 space-y-2 rounded-grape bg-longan-900 p-3 inset-ring inset-ring-longan-700/50">
                {
                    let (stale, warning_id) = (stale.clone(), warning_id.clone());
                    move || stale().then(|| view! {
                        <p id=warning_id.clone() data-proposed-stale="" class="text-label-sm text-durian-300">{MEMORY_CHANGED}</p>
                    })
                }
                <div data-proposed-text="" role="region" tabindex="0" aria-label=PROPOSED_MEMORY class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words select-text rounded-seed bg-longan-950/60 p-2 text-body-sm text-rambutan-200 focus-visible:outline-2 focus-visible:outline-persimmon-400">{card.memory}</div>
                <div class="flex flex-wrap gap-2">
                    <button
                        type="button"
                        data-proposed-approve=""
                        class=BUTTON
                        aria-describedby=move || stale().then(|| warning_id.clone())
                        aria-disabled=move || locked().then_some("true")
                        on:click=move |_| decide(approve_id.clone(), true)
                    >{APPROVE_MEMORY}</button>
                    <button
                        type="button"
                        data-proposed-decline=""
                        class=BUTTON
                        aria-disabled=move || locked().then_some("true")
                        on:click=move |_| decide(decline_id.clone(), false)
                    >{DECLINE}</button>
                </div>
            </article>
        }
    };

    view! {
        <div node_ref=root data-proposed-root="" tabindex="-1" class="min-w-0 focus:outline-none">
            <Show when=move || list.with(|cards| !cards.is_empty())>
                <section aria-labelledby="proposed-memory-title" class="mb-4 min-w-0 space-y-2">
                    <h4 id="proposed-memory-title" class="text-label text-rambutan-100">{PROPOSED_MEMORY}</h4>
                    <p class="text-label-sm text-rambutan-400">{PROPOSED_HINT}</p>
                    <For each=move || list.get() key=|card| card.id.clone() children=card_view />
                </section>
            </Show>
        </div>
    }
}

/// Focuses the first card's Approve once drawn, or the region when no card
/// is left. A native test build has no page.
fn focus_after_decision(root: NodeRef<html::Div>) {
    if cfg!(not(target_arch = "wasm32")) {
        return;
    }
    request_animation_frame(move || {
        use wasm_bindgen::JsCast as _;
        let Some(Some(root)) = root.try_get_untracked() else {
            return;
        };
        let next = root
            .query_selector("[data-proposed-approve]")
            .ok()
            .flatten()
            .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
        let _ = match next {
            Some(button) => button.focus(),
            None => root.focus(),
        };
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::imports::{
        APPROVE_MEMORY, DECLINE, MEMORY_CHANGED, PROPOSED_HINT, PROPOSED_MEMORY,
    };
    use bruhs_models::reactive::{ProposalStatus, ReactiveAgent, ReactiveChange, ReactiveProposal};
    use bruhs_models::EpochMs;
    use leptos::tachys;

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

    fn render(agent_id: &str, snapshot: ReactiveSnapshot) -> String {
        let _ = any_spawner::Executor::init_futures_executor();
        let agent_id = agent_id.to_string();
        let snapshot = RwSignal::new(Some(snapshot));
        untrack(|| {
            tachys::view::RenderHtml::to_html(
                view! { <ProposedMemory agent_id=agent_id snapshot=snapshot send=Callback::new(|_| {}) /> },
            )
        })
    }

    /// The opening tag of the element carrying `marker`.
    fn tag<'a>(html: &'a str, marker: &str) -> &'a str {
        let at = html
            .find(marker)
            .unwrap_or_else(|| panic!("{marker} in {html}"));
        let start = html[..at].rfind('<').unwrap();
        let end = at + html[at..].find('>').unwrap();
        &html[start..=end]
    }

    #[test]
    fn proposed_memory_shows_full_text_and_warns_when_stale() {
        Owner::new().with(|| {
            let text = "## Claude.ai export\n\nLikes short answers.\nWorks on Bruhs & friends.";
            let snapshot = ReactiveSnapshot {
                agents: vec![agent("a", ""), agent("b", "")],
                proposals: vec![
                    proposal("p1", "a", text, 10),
                    proposal("p2", "b", "Another bruh's memory", 20),
                ],
                ..ReactiveSnapshot::default()
            };
            let html = render("a", snapshot.clone());
            for copy in [PROPOSED_MEMORY, PROPOSED_HINT, APPROVE_MEMORY, DECLINE] {
                assert!(html.contains(copy), "{copy} in {html}");
            }
            // The whole text, wrapped and scrollable.
            assert!(
                html.contains("Likes short answers.\nWorks on Bruhs &amp; friends."),
                "{html}"
            );
            let block = tag(&html, "data-proposed-text");
            assert!(block.contains("role=\"region\""), "{block}");
            for class in ["whitespace-pre-wrap", "overflow-y-auto", "max-h-"] {
                assert!(block.contains(class), "{class} in {block}");
            }
            assert!(!html.contains(MEMORY_CHANGED), "{html}");
            assert!(!html.contains("Another bruh"), "{html}");

            // The bruh's memory changed after the proposal: approving would
            // drop that change, so the card says so first.
            let mut stale = snapshot.clone();
            stale.agents[0].memory = "Written after the proposal.".into();
            let html = render("a", stale);
            assert!(html.contains(MEMORY_CHANGED), "{html}");
            assert!(tag(&html, "data-proposed-stale").contains("text-durian"));

            // Trailing spaces on the current memory aren't a change (F31).
            let mut spaced = snapshot.clone();
            spaced.agents[0].memory = "## Claude.ai export  \n".into();
            assert!(!render("a", spaced).contains(MEMORY_CHANGED));

            // Decided proposals and a bruh with none show no card.
            let mut decided = snapshot;
            decided.proposals[0].status = ProposalStatus::Approved;
            let html = render("a", decided);
            assert!(!html.contains(PROPOSED_MEMORY), "{html}");
        });
    }

    #[test]
    fn a_card_on_screen_warns_once_the_memory_changes() {
        Owner::new().with(|| {
            let snapshot = RwSignal::new(Some(ReactiveSnapshot {
                agents: vec![agent("a", "")],
                proposals: vec![
                    proposal("p1", "a", "First text", 10),
                    proposal("p2", "a", "Second text", 20),
                ],
                ..ReactiveSnapshot::default()
            }));
            let list = proposal_cards(StoredValue::new("a".to_string()), snapshot);
            let stale = card_stale(list, "p1".into());
            assert!(!stale());
            // The owner approves the other proposal while this card shows:
            // approving this one now would drop that memory.
            snapshot.update(|snapshot| {
                let snapshot = snapshot.as_mut().unwrap();
                snapshot.agents[0].memory = "Second text".into();
                snapshot.proposals[1].status = ProposalStatus::Approved;
            });
            assert!(stale(), "the card still shows as current");
            // Back to a prefix of it: the warning goes again.
            snapshot
                .update(|snapshot| snapshot.as_mut().unwrap().agents[0].memory = "First".into());
            assert!(!stale());
        });
    }

    #[test]
    fn the_newest_proposal_comes_first() {
        Owner::new().with(|| {
            let snapshot = ReactiveSnapshot {
                agents: vec![agent("a", "")],
                proposals: vec![
                    proposal("old", "a", "Older text", 10),
                    proposal("new", "a", "Newer text", 20),
                ],
                ..ReactiveSnapshot::default()
            };
            let html = render("a", snapshot);
            let newer = html.find("Newer text").expect("newer");
            let older = html.find("Older text").expect("older");
            assert!(newer < older, "{html}");
            // The heading and hint show once above the cards.
            assert_eq!(html.matches(PROPOSED_HINT).count(), 1, "{html}");
            assert_eq!(html.matches(APPROVE_MEMORY).count(), 2, "{html}");
        });
    }
}
