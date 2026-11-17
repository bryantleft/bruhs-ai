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
