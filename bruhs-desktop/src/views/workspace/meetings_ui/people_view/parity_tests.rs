//! Web parity for the People page's recovery views (PM-31): the merge
//! recovery data viewer and the contact refresh status.

use super::super::super::performance_tests::fixture;
use super::super::super::AppView;
use crate::views::meetings_hub::{PeopleMergeArchive, PeopleSnapshot};
use bruhs_models::{PersonProfile, RelationshipProfile};
use gpui::VisualTestContext;
use std::sync::Arc;

fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.debug_bounds(selector).is_some()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let target = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(target.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

fn profile(id: &str, name: &str) -> PersonProfile {
    let mut person = PersonProfile::new(name);
    person.id = id.into();
    person
}

/// Originals kept after a conflicting recovery of the merge of `source`,
/// with the source's private relationship notes.
fn kept(source: &str, notes: &str) -> Arc<PeopleMergeArchive> {
    let mut relationship = RelationshipProfile::new(source, 1);
    relationship.notes = notes.into();
    Arc::new(PeopleMergeArchive {
        source: Some(profile(source, "Unsynced source name")),
        target: Some(profile("target", "Surviving name")),
        source_was_pending: true,
        source_relationship: Some(relationship),
        target_relationship: None,
        target_id: "target".into(),
        links: Default::default(),
    })
}

fn snapshot(recoveries: Vec<Arc<PeopleMergeArchive>>) -> PeopleSnapshot {
    PeopleSnapshot {
        records: vec![Arc::new(profile("target", "Surviving name"))],
        merge_archives: [("source".to_string(), kept("source", "First archive"))]
            .into_iter()
            .collect(),
        merge_recoveries: [("source".to_string(), recoveries)].into_iter().collect(),
        ..Default::default()
    }
}

/// Like the web's `ContactMergeStatus`, kept originals can be viewed on
/// request, read only, and the view hides once they change instead of
/// serializing private data again unasked.
#[gpui::test]
fn kept_merge_originals_can_be_viewed_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.restore_people(snapshot(vec![kept("source", "Kept private note")]));
        });
        workspace.active_view = AppView::People;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(shown(cx, "desktop-people-merge-status"));
    assert!(!shown(cx, "desktop-people-merge-recovery-data"));

    click(cx, "desktop-people-view-merge-recovery");
    assert!(shown(cx, "desktop-people-merge-recovery-data"));
    workspace.read_with(cx, |workspace, _| {
        let (count, json) = workspace
            .people_page
            .merge_recovery
            .as_ref()
            .expect("the requested recovery data");
        assert_eq!(*count, 1);
        let value: serde_json::Value = serde_json::from_str(json).expect("pretty JSON");
        assert_eq!(
            value["source"][0]["source_relationship"]["notes"],
            "Kept private note"
        );
        assert_eq!(
            value["source"][0]["source"]["display_name"],
            "Unsynced source name"
        );
        assert_eq!(value["source"][0]["target_id"], "target");
    });

    workspace.update(cx, |workspace, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.restore_people(snapshot(vec![
                kept("source", "Kept private note"),
                kept("source", "Another kept note"),
            ]));
        });
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        !shown(cx, "desktop-people-merge-recovery-data"),
        "changed originals hide the old view"
    );
    click(cx, "desktop-people-view-merge-recovery");
    assert!(shown(cx, "desktop-people-merge-recovery-data"));
    workspace.read_with(cx, |workspace, _| {
        let (count, json) = workspace.people_page.merge_recovery.as_ref().unwrap();
        assert_eq!(*count, 2);
        assert!(json.contains("Another kept note"));
    });
}

/// Like the web, a running contact refresh says so while the cached
/// contacts stay listed.
#[gpui::test]
fn a_contact_refresh_is_announced_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.restore_people(PeopleSnapshot {
                records: vec![Arc::new(profile("ada", "Ada"))],
                ..Default::default()
            });
            hub.set_people_refreshing_for_test(true);
        });
        workspace.active_view = AppView::People;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(shown(cx, "desktop-people-refreshing"));
    assert!(
        shown(cx, "desktop-person-name-ada"),
        "cached contacts remain available"
    );
    workspace.update(cx, |workspace, cx| {
        workspace
            .meetings_hub
            .update(cx, |hub, _| hub.set_people_refreshing_for_test(false));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!shown(cx, "desktop-people-refreshing"));
}

fn people_with_relationships(
    workspace: &gpui::Entity<super::super::super::Workspace>,
    cx: &mut VisualTestContext,
) {
    workspace.update(cx, |workspace, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.restore_people(PeopleSnapshot {
                records: vec![
                    Arc::new(profile("ada", "Ada")),
                    Arc::new(profile("grace", "Grace")),
                ],
                ..Default::default()
            });
        });
        let mut ada = RelationshipProfile::new("ada", 1);
        ada.notes = "Met at the conference".into();
        ada.company = "Analytical Engines".into();
        let mut grace = RelationshipProfile::new("grace", 1);
        grace.company = "Navy".into();
        workspace.relationships = vec![ada, grace];
    });
}

fn field(input: &gpui::Entity<gpui_component::input::InputState>, cx: &gpui::App) -> String {
    input.read(cx).value().to_string()
}

/// `field` for the multi-line notes.
fn notes(input: &gpui::Entity<gpui_component::input::TextareaState>, cx: &gpui::App) -> String {
    input.read(cx).value().to_string()
}

/// Web: Notes is a four-row field, so Enter starts a new line in it (the
/// desktop's one-line field could not hold line breaks typed here).
#[gpui::test]
fn notes_take_more_than_one_line_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    people_with_relationships(&workspace, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_person_profile("ada", window, cx);
        w.relationship_notes_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        gpui::Focusable::focus_handle(w.relationship_notes_input.read(cx), cx).focus(window, cx);
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_input("first");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("second");
    workspace.update(cx, |w, cx| {
        assert_eq!(notes(&w.relationship_notes_input, cx), "first\nsecond");
    });
}

/// PM-37: like the web, each contact keeps its unsubmitted relationship
/// edits while another contact is open, People lists them, and Discard
/// returns the fields to the saved details.
#[gpui::test]
fn relationship_edits_survive_opening_another_contact_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    people_with_relationships(&workspace, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_person_profile("ada", window, cx);
        assert_eq!(
            notes(&w.relationship_notes_input, cx),
            "Met at the conference"
        );
        w.relationship_notes_input.update(cx, |input, cx| {
            input.set_value("Prefers mornings", window, cx)
        });
        w.relationship_item_input
            .update(cx, |input, cx| input.set_value("Send the deck", window, cx));

        w.open_person_profile("grace", window, cx);
        assert_eq!(notes(&w.relationship_notes_input, cx), "");
        assert_eq!(field(&w.relationship_company_input, cx), "Navy");
        assert_eq!(field(&w.relationship_item_input, cx), "");
        let drafts = w.relationship_drafts_now(cx);
        assert_eq!(drafts.len(), 1);
        assert!(drafts.contains("ada"));
        assert!(drafts.recovery_json().contains("Prefers mornings"));

        w.open_person_profile("ada", window, cx);
        assert_eq!(notes(&w.relationship_notes_input, cx), "Prefers mornings");
        assert_eq!(field(&w.relationship_item_input, cx), "Send the deck");
        cx.notify();
    });
    cx.run_until_parked();
    assert!(shown(cx, "desktop-people-drafts"));
    click(cx, "desktop-people-drafts-private");
    assert!(shown(cx, "desktop-people-draft-recovery-data"));

    let discard: &'static str = Box::leak(
        format!(
            "desktop-people-draft-discard-{}",
            super::super::super::element_key("ada")
        )
        .into_boxed_str(),
    );
    click(cx, discard);
    workspace.read_with(cx, |w, cx| {
        assert_eq!(
            notes(&w.relationship_notes_input, cx),
            "Met at the conference"
        );
        assert_eq!(field(&w.relationship_item_input, cx), "");
        assert!(w.relationship_drafts_now(cx).is_empty());
        assert_eq!(
            w.relationships[0].notes, "Met at the conference",
            "nothing saved"
        );
    });
    assert!(!shown(cx, "desktop-people-drafts"));
}

/// PM-37: Save details writes only the fields edited here. A field also
/// changed elsewhere keeps the draft for review; an unrelated newer field
/// survives the save.
#[gpui::test]
fn save_details_keeps_a_conflicting_draft_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    people_with_relationships(&workspace, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_person_profile("ada", window, cx);
        w.relationship_notes_input
            .update(cx, |input, cx| input.set_value("Mine", window, cx));
        // Saved elsewhere meanwhile, such as by an approved agent workflow.
        w.relationships[0].notes = "Theirs".into();
        w.save_relationship_details("ada", window, cx);
        assert_eq!(
            w.people_page.draft_error,
            Some(bruhs_models::people_copy::DETAILS_CHANGED)
        );
        assert_eq!(w.relationships[0].notes, "Theirs");
        assert_eq!(notes(&w.relationship_notes_input, cx), "Mine");
        assert!(w.relationship_drafts_now(cx).contains("ada"));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(shown(cx, "desktop-relationship-draft-error"));

    workspace.update_in(cx, |w, window, cx| {
        w.discard_relationship_draft("ada", window, cx);
        w.relationship_company_input.update(cx, |input, cx| {
            input.set_value(" Babbage & Co ", window, cx)
        });
        w.relationships[0].notes = "Latest".into();
        w.save_relationship_details("ada", window, cx);
        assert_eq!(w.people_page.draft_error, None);
        assert_eq!(w.relationships[0].company, "Babbage & Co");
        assert_eq!(
            w.relationships[0].notes, "Latest",
            "an unrelated field is kept"
        );
        assert_eq!(notes(&w.relationship_notes_input, cx), "Latest");
        assert!(w.relationship_drafts_now(cx).is_empty());
    });
}

/// PM-37: like the web's `begin_merge`, a merge waits until neither contact
/// has unsaved relationship edits, and nothing is sent meanwhile.
#[gpui::test]
fn a_merge_waits_for_unsaved_relationship_edits_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    people_with_relationships(&workspace, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_person_profile("ada", window, cx);
        w.relationship_notes_input
            .update(cx, |input, cx| input.set_value("Unsaved", window, cx));
        // Another contact is open; Ada's edits are kept.
        w.open_person_profile("grace", window, cx);
        w.merge = Some(super::super::PersonMergePanel {
            person_id: "grace".into(),
            target_id: Some("ada".into()),
            dropdown_open: false,
            confirming: true,
        });
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-people-merge-commit");
    workspace.read_with(cx, |w, cx| {
        assert_eq!(
            w.people_page.notice.as_deref(),
            Some(bruhs_models::people_copy::MERGE_DRAFTS_UNSAVED)
        );
        assert!(w.merge.is_some(), "the merge stays open");
        assert!(w.people_page.pending_merge.is_none());
        assert!(!w.meetings_hub.read(cx).person_merge_in_flight);
        assert!(w.relationship_drafts_now(cx).contains("ada"));
    });
}
