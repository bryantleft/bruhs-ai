use super::*;
use crate::{RelationshipItem, RelationshipItemKind};

fn person(id: &str, name: &str) -> PersonProfile {
    let mut person = PersonProfile::new(name);
    person.id = id.into();
    person
}

fn none_relationship(_: &str) -> Option<&'static RelationshipProfile> {
    None
}

#[test]
fn rename_and_tracking_undo_require_the_record_to_be_unchanged() {
    let renamed = person("ada", "Ada L.");
    let undo = PeopleUndo::Rename {
        person_id: "ada".into(),
        before: "Ada".into(),
        after: "Ada L.".into(),
    };
    assert_eq!(undo.summary(), "Renamed Ada to Ada L.");
    assert_eq!(undo.action_label(), "Undo rename");
    assert!(undo
        .check(|id| (id == "ada").then_some(&renamed), none_relationship)
        .is_ok());
    let edited_again = person("ada", "Countess");
    assert_eq!(
        undo.check(|_| Some(&edited_again), none_relationship),
        Err(PeopleUndoError::ProfileChanged)
    );
    assert_eq!(
        undo.check(|_| None, none_relationship),
        Err(PeopleUndoError::ProfileChanged)
    );

    let mut untracked = person("ada", "Ada");
    untracked.tracked = false;
    let tracking = PeopleUndo::Tracked {
        person_id: "ada".into(),
        name: "Ada".into(),
        before: true,
        after: false,
    };
    assert_eq!(tracking.summary(), "Stopped tracking Ada.");
    assert!(tracking
        .check(|_| Some(&untracked), none_relationship)
        .is_ok());
    let retracked = person("ada", "Ada");
    assert_eq!(
        tracking.check(|_| Some(&retracked), none_relationship),
        Err(PeopleUndoError::ProfileChanged)
    );
}

#[test]
fn merge_undo_refuses_when_the_survivor_relationship_changed() {
    let target = person("target", "Taylor");
    let mut after = RelationshipProfile::new("target", 1);
    after.add_item(
        RelationshipItem::new(RelationshipItemKind::Reminder, "Call", 1),
        1,
    );
    let undo = PeopleUndo::Merge {
        source_id: "source".into(),
        source_name: "Sam".into(),
        target_id: "target".into(),
        target_name: "Taylor".into(),
        before_source: None,
        before_target: None,
        after_relationship: Some(after.clone()),
        links: PersonLinkSet::default(),
    };
    assert_eq!(undo.summary(), "Merged Sam into Taylor.");
    let find = |id: &str| (id == "target").then_some(&target);
    assert!(undo
        .check(find, |id| (id == "target").then_some(&after))
        .is_ok());
    let mut edited = after.clone();
    edited.notes = "Edited after the merge".into();
    assert_eq!(
        undo.check(find, |id| (id == "target").then_some(&edited)),
        Err(PeopleUndoError::RelationshipChanged)
    );
    let source = person("source", "Sam");
    assert_eq!(
        undo.check(
            |id| match id {
                "source" => Some(&source),
                "target" => Some(&target),
                _ => None,
            },
            |id| (id == "target").then_some(&after)
        ),
        Err(PeopleUndoError::AlreadyRestored)
    );
}

#[test]
fn delete_undo_refuses_when_the_contact_is_back() {
    let deleted = person("ada", "Ada");
    let undo = PeopleUndo::Delete {
        person: deleted.clone(),
        relationship: None,
        links: PersonLinkSet::default(),
        local_only: false,
    };
    assert_eq!(undo.summary(), "Deleted Ada.");
    let initialed = PeopleUndo::Delete {
        person: person("ada", "Ada L."),
        relationship: None,
        links: PersonLinkSet::default(),
        local_only: false,
    };
    assert_eq!(initialed.summary(), "Deleted Ada L.");
    assert_eq!(undo.person_ids(), ["ada"]);
    assert!(undo.check(|_| None, none_relationship).is_ok());
    assert_eq!(
        undo.check(|_| Some(&deleted), none_relationship),
        Err(PeopleUndoError::AlreadyRestored)
    );
}

#[test]
fn messages_follow_the_copy_rules() {
    for error in [
        PeopleUndoError::ProfileChanged,
        PeopleUndoError::RelationshipChanged,
        PeopleUndoError::AlreadyRestored,
        PeopleUndoError::Busy,
    ] {
        let message = error.message();
        assert!(message.ends_with("No records were changed."), "{message}");
        assert!(!message.contains('—') && !message.contains(" - ") && !message.contains('·'));
    }
}

#[test]
fn wire_records_round_trip_and_reject_unknown_fields() {
    let deletion = PersonDeletion {
        person: person("ada", "Ada"),
        speakers: vec![MeetingSpeakerRef {
            meeting_id: "m1".into(),
            speaker_id: "them_1".into(),
        }],
        action_items: vec![ActionItemRef {
            meeting_id: "m1".into(),
            index: 0,
            text: "Write notes".into(),
        }],
        deleted_at: 7,
    };
    let json = serde_json::to_value(&deletion).unwrap();
    assert_eq!(
        serde_json::from_value::<PersonDeletion>(json.clone()).unwrap(),
        deletion
    );
    let mut extra = json;
    extra["surprise"] = serde_json::json!(true);
    assert!(serde_json::from_value::<PersonDeletion>(extra).is_err());

    let undo = PeopleUndo::Rename {
        person_id: "ada".into(),
        before: "Ada".into(),
        after: "Ada L.".into(),
    };
    let json = serde_json::to_value(&undo).unwrap();
    assert_eq!(json["kind"], "rename");
    assert_eq!(serde_json::from_value::<PeopleUndo>(json).unwrap(), undo);
}

#[test]
fn undo_summary_preserves_terminal_punctuation_and_unicode() {
    for (name, expected) in [
        ("Zoë", "Deleted Zoë."),
        ("Ada L.", "Deleted Ada L."),
        ("Zoë!", "Deleted Zoë!"),
        ("Zoë?", "Deleted Zoë?"),
        ("Zoë...", "Deleted Zoë..."),
    ] {
        let undo = PeopleUndo::Delete {
            person: person("copy", name),
            relationship: None,
            links: PersonLinkSet::default(),
            local_only: false,
        };
        assert_eq!(undo.summary(), expected);
    }
}
