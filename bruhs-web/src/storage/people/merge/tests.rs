use super::*;

fn prepared() -> Record {
    let source = PersonProfile::new("Source");
    let target = PersonProfile::new("Target");
    Record {
        key: KEY.into(),
        id: "merge-test".into(),
        source,
        target,
        before_source: None,
        before_target: None,
        after_relationship: None,
        created_at_ms: 1,
        phase: Phase::Prepared,
    }
}

#[test]
fn prepared_record_round_trips_and_locks_only_its_two_contacts() {
    let record = prepared();
    let decoded: Record = serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
    decoded.validate().unwrap();
    assert_eq!(record, decoded);
    assert!(record.locks(&record.source.id) && record.locks(&record.target.id));
    assert!(!record.locks("unrelated"));
}

#[test]
fn recovery_rejects_a_relationship_result_that_does_not_match_the_originals() {
    let mut record = prepared();
    record.after_relationship = Some(RelationshipProfile::new(&record.target.id, 1));
    assert!(record.validate().is_err());
}

#[test]
fn recovery_rejects_a_confirmed_reply_for_another_contact() {
    let mut record = prepared();
    record.phase = Phase::Confirmed {
        reply: Box::new(record.source.clone()),
    };
    assert!(record.validate().is_err());
}

#[test]
fn recovery_rejects_an_older_confirmed_reply() {
    let mut record = prepared();
    let mut reply = record.target.clone();
    reply.updated_at -= 1;
    record.phase = Phase::Confirmed {
        reply: Box::new(reply),
    };
    assert!(record.validate().is_err());
}

#[test]
fn recovery_rejects_an_equal_version_conflicting_reply() {
    let mut record = prepared();
    let mut reply = record.target.clone();
    reply.display_name = "Conflicting target".into();
    record.phase = Phase::Confirmed {
        reply: Box::new(reply),
    };
    assert!(record.validate().is_err());
}

#[test]
fn completed_record_retains_originals_without_locking_contacts() {
    let mut record = prepared();
    let originals = (record.source.clone(), record.target.clone());
    let mut reply = record.target.clone();
    reply.updated_at += 1;
    record.phase = Phase::Completed {
        reply: Box::new(reply),
    };
    record.validate().unwrap();
    assert!(!record.pending() && !record.locks(&record.source.id));
    assert_eq!((record.source, record.target), originals);
}
