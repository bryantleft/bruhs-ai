use super::*;

#[test]
fn deleting_project_locks_tasks_even_when_snapshot_has_no_children() {
    let guard = ProjectGuard::new(&[Write {
        kind: Kind::Project,
        id: "project",
        expected: None,
        draft: None,
    }]);
    assert_eq!(guard.deleted, BTreeSet::from(["project".into()]));
    assert_eq!(
        guard.stores().collect::<Vec<_>>(),
        vec!["projects", "work_items"]
    );
}

#[test]
fn only_new_or_changed_project_links_lock_and_validate_parent() {
    let mut original = WorkItem::new_at("Original", 1);
    original.project_id = Some("old".into());
    let mut draft = original.clone();
    draft.title = "Latest".into();
    let expected = Record::Task(original);
    let unchanged = Record::Task(draft.clone());
    let write = |draft| Write {
        kind: Kind::Task,
        id: "task",
        expected: Some(&expected),
        draft: Some(draft),
    };
    let guard = ProjectGuard::new(&[write(&unchanged)]);
    assert!(guard.added.is_empty());
    assert_eq!(guard.stores().count(), 0);
    draft.project_id = Some("new".into());
    let changed = Record::Task(draft.clone());
    let guard = ProjectGuard::new(&[write(&changed)]);
    assert_eq!(guard.added, BTreeSet::from(["new".into()]));
    assert_eq!(guard.stores().collect::<Vec<_>>(), vec!["projects"]);
    let guard = ProjectGuard::new(&[Write {
        kind: Kind::Task,
        id: "task",
        expected: None,
        draft: Some(&changed),
    }]);
    assert_eq!(guard.added, BTreeSet::from(["new".into()]));
    draft.project_id = None;
    let cleared = Record::Task(draft);
    assert_eq!(ProjectGuard::new(&[write(&cleared)]).stores().count(), 0);
}
