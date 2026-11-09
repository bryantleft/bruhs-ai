use super::*;
use crate::{RelationshipItem, RelationshipItemKind};

#[test]
fn conflicting_fields_and_notes_are_preserved_without_replacing_target_values() {
    let mut source = RelationshipProfile::new("source", 1);
    source.company = "Source company".into();
    source.notes = "Source private notes 🗒".into();
    source.phone = "123".into();
    let mut target = RelationshipProfile::new("target", 2);
    target.company = "Target company".into();
    target.notes = "Target private notes".into();
    let merged = merge_relationship_context("source", "target", Some(&source), Some(&target), 3)
        .unwrap()
        .unwrap();
    assert_eq!(merged.company, target.company);
    assert_eq!(merged.phone, source.phone);
    assert!(
        merged.notes.contains(&target.notes)
            && merged.notes.contains(&source.notes)
            && merged.notes.contains(&source.company)
    );
}

#[test]
fn original_item_identity_completion_and_due_date_survive_transfer() {
    let mut source = RelationshipProfile::new("source", 1);
    let mut item = RelationshipItem::new(RelationshipItemKind::Reminder, "Call back", 1);
    item.completed_at_ms = Some(2);
    item.due_at_ms = Some(3);
    source.items.push(item.clone());
    let merged = merge_relationship_context("source", "target", Some(&source), None, 4)
        .unwrap()
        .unwrap();
    assert_eq!(merged.items, vec![item]);
    assert_eq!(merged.person_id, "target");
}

#[test]
fn identical_shared_items_are_not_duplicated() {
    let mut source = RelationshipProfile::new("source", 1);
    source.items.push(RelationshipItem::new(
        RelationshipItemKind::Note,
        "Same note",
        1,
    ));
    let mut target = source.clone();
    target.person_id = "target".into();
    assert_eq!(
        merge_relationship_context("source", "target", Some(&source), Some(&target), 2)
            .unwrap()
            .unwrap()
            .items,
        source.items
    );
}

#[test]
fn conflicting_item_identity_refuses_the_whole_merge() {
    let mut source = RelationshipProfile::new("source", 1);
    source.items.push(RelationshipItem::new(
        RelationshipItemKind::Note,
        "Source",
        1,
    ));
    let mut target = source.clone();
    target.person_id = "target".into();
    target.items[0].text = "Different".into();
    assert!(
        merge_relationship_context("source", "target", Some(&source), Some(&target), 2).is_err()
    );
}

#[test]
fn missing_context_does_not_create_an_empty_profile() {
    assert_eq!(
        merge_relationship_context("source", "target", None, None, 1).unwrap(),
        None
    );
}

#[test]
fn mismatched_profile_identity_is_rejected() {
    let source = RelationshipProfile::new("unexpected", 1);
    assert!(merge_relationship_context("source", "target", Some(&source), None, 2).is_err());
}

#[test]
fn duplicate_target_items_are_rejected_even_without_source_context() {
    let mut target = RelationshipProfile::new("target", 1);
    let item = RelationshipItem::new(RelationshipItemKind::Note, "Repeated", 1);
    target.items = vec![item.clone(), item];
    assert!(merge_relationship_context("source", "target", None, Some(&target), 2).is_err());
}
