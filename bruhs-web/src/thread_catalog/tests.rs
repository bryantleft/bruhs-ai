use super::*;
use bruhs_models::SidebarTab::{Chat, Code};

#[test]
fn unresolved_catalog_does_not_claim_the_library_is_empty() {
    assert_eq!(
        Status::Pending.empty_message(),
        Some("Loading saved threads…")
    );
    assert_eq!(
        Status::Loading.empty_message(),
        Some("Loading saved threads…")
    );
    assert_eq!(
        Status::Failed.empty_message(),
        Some("Saved threads couldn’t be loaded.")
    );
    assert_eq!(Status::Ready.empty_message(), None);
}

fn thread(id: &str, project: &str) -> Thread {
    Thread {
        id: id.into(),
        title: id.into(),
        project_name: project.into(),
        updated_at: bruhs_models::EpochMs::new(1).unwrap(),
        project_path: None,
        session_id: None,
        fresh_session: false,
        branch: None,
        model: None,
        pinned: false,
        settled: false,
        pin_rank: None,
        active_rank: None,
        snoozed_until_ms: None,
        goal: None,
        worktree_path: None,
        effort: None,
        run_locked: false,
        role: None,
        instructions_stamp: None,
        import_context: None,
    }
}

#[test]
fn exact_link_selects_its_mode_instead_of_default() {
    let rows = vec![thread("chat", ""), thread("code", "Project")];
    assert_eq!(
        initial_selection(&rows, Some("code"), Chat),
        Some(("code".into(), Some(Code)))
    );
}

#[test]
fn missing_link_uses_saved_workspace_mode() {
    let rows = vec![thread("chat", ""), thread("code", "Project")];
    assert_eq!(
        initial_selection(&rows, Some("missing"), Code),
        Some(("code".into(), None))
    );
}

#[test]
fn empty_catalog_does_not_select_an_unknown_link() {
    assert_eq!(initial_selection(&[], Some("missing"), Chat), None);
}

#[test]
fn snapshot_merge_retains_latest_created_thread_fields() {
    let mut edits = InitialEdits::default();
    let first = thread("local", "");
    edits.edit(&first.id, Some(&first));
    let mut latest = first.clone();
    latest.title = "Latest title".into();
    latest.pinned = true;
    latest.session_id = Some("synthetic-session".into());
    edits.edit(&latest.id, Some(&latest));
    let merged = edits.merge_by(vec![thread("saved", ""), first], |row| &row.id);
    assert_eq!(
        serde_json::to_value(&merged[1]).unwrap(),
        serde_json::to_value(latest).unwrap()
    );
}

#[test]
fn deleted_snapshot_record_is_not_resurrected() {
    let mut edits = InitialEdits::default();
    let deleted = thread("deleted", "");
    edits.edit(&deleted.id, Some(&deleted));
    edits.edit(&deleted.id, None);
    assert!(edits.merge_by(vec![deleted], |row| &row.id).is_empty());
}
