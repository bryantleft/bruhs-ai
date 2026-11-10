//! Warn while this tab owns unsent drafts, pending writes or a live response.
//! Browsers may suppress this prompt; it does not provide crash-safe recovery.
use leptos::prelude::*;

use crate::relationship_store::RelationshipStore;
use crate::state::{AutomationStateSignal, ComposerDraft, LifeContextSignal};
use crate::task_saves::TaskSaves;

pub(crate) fn install() {
    let saves = expect_context::<TaskSaves>();
    let relationships = expect_context::<RelationshipStore>();
    let turns = expect_context::<crate::state::Turns>();
    let LifeContextSignal(life) = expect_context();
    let AutomationStateSignal(automation) = expect_context();
    let ComposerDraft(composer) = expect_context();
    let at_risk = Memo::new(move |_| {
        let pending = saves.pending();
        // A turn running in any thread, not only the one on screen.
        let streaming = turns.any_running();
        let life_pending = life.pending();
        let automation_pending = automation.pending();
        let relationships_pending = relationships.pending();
        let composer_pending = composer.with(|draft| !draft.trim().is_empty());
        pending
            || streaming
            || life_pending
            || automation_pending
            || relationships_pending
            || composer_pending
    });
    Effect::new(move |_| {
        if !at_risk.get() {
            return;
        }
        let listener = leptos::leptos_dom::helpers::window_event_listener(
            leptos::ev::beforeunload,
            move |event| {
                // A commit can finish before the effect detaches this listener.
                // Never warn for already-saved data or use raw callback context.
                // An untracked Memo read still refreshes dirty dependencies.
                if at_risk.try_get_untracked().unwrap_or(false) {
                    event.prevent_default();
                    event.set_return_value("");
                }
            },
        );
        on_cleanup(move || listener.remove());
    });
}
