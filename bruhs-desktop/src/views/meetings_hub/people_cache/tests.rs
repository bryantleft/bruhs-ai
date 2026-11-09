use super::*;
use gpui::{AppContext, TestAppContext};

fn fixture(cx: &mut TestAppContext) -> gpui::Entity<MeetingsHub> {
    cx.executor().allow_parking();
    let runtime = Arc::new(tokio::runtime::Runtime::new().expect("runtime"));
    cx.new(|cx| {
        MeetingsHub::new(
            ServerClient::new("http://127.0.0.1:0", "http://127.0.0.1:0").unwrap(),
            runtime,
            MeetingPrefs::default(),
            cx,
        )
    })
}

#[gpui::test]
fn retired_contact_cannot_return_from_old_refresh_or_create_ack_after_restore(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        let source = PersonProfile::new("Source");
        let mut target = PersonProfile::new("Target");
        target.updated_at = 10;
        hub.people = vec![source.clone(), target.clone()];
        hub.pending_person_creates.insert(source.id.clone());
        hub.commit_person_merge(
            source.id.clone(),
            PeopleMergeArchive {
                source: Some(source.clone()),
                target: Some(target.clone()),
                source_was_pending: true,
                source_relationship: None,
                target_relationship: None,
                target_id: target.id.clone(),
            },
            target.clone(),
            cx,
        );
        let saved = serde_json::to_string(&hub.people_snapshot()).unwrap();
        hub.restore_people(serde_json::from_str(&saved).unwrap());
        let mut stale_target = target.clone();
        stale_target.display_name = "Stale target name".into();
        stale_target.updated_at = 1;
        hub.merge_people_cache(vec![source.clone(), stale_target], cx);
        hub.apply_person_create(&source, Ok(source.clone()), cx);
        assert_eq!(hub.people, vec![target]);
        assert_eq!(hub.pending_people_count(), 0);
        assert!(hub.person_is_retired(&source.id));
    });
}

#[gpui::test]
fn refresh_and_late_create_ack_do_not_replace_pending_local_contact(cx: &mut TestAppContext) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        let person = PersonProfile::new("Local");
        hub.restore_people(PeopleSnapshot {
            records: vec![person.clone()],
            pending_creates: HashSet::from([person.id.clone()]),
            ..Default::default()
        });
        let mut remote = person.clone();
        remote.display_name = "Old server name".into();
        let other = PersonProfile::new("Unrelated server contact");
        hub.merge_people_cache(vec![remote, other.clone()], cx);
        assert_eq!(hub.people, vec![person.clone(), other.clone()]);
        hub.people[0].display_name = "Newer local name".into();
        hub.apply_person_create(&person, Ok(person.clone()), cx);
        assert_eq!(hub.people[0].display_name, "Newer local name");
        assert_eq!(hub.people[1], other);
        assert_eq!(hub.pending_people_count(), 1);
    });
}

#[gpui::test]
fn mismatched_create_response_preserves_pending_identity_and_matching_ack_clears_it(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        let person = PersonProfile::new("Local");
        hub.restore_people(PeopleSnapshot {
            records: vec![person.clone()],
            pending_creates: HashSet::from([person.id.clone()]),
            ..Default::default()
        });
        hub.apply_person_create(&person, Ok(PersonProfile::new("Local")), cx);
        assert_eq!(hub.people, vec![person.clone()]);
        assert_eq!(hub.pending_people_count(), 1);
        hub.apply_person_create(&person, Ok(person.clone()), cx);
        assert_eq!(hub.people, vec![person]);
        assert_eq!(hub.pending_people_count(), 0);
    });
}

#[gpui::test]
fn retry_schedules_at_most_four_creates_and_does_not_duplicate_in_flight_requests(
    cx: &mut TestAppContext,
) {
    let hub = fixture(cx);
    hub.update(cx, |hub, cx| {
        let records: Vec<_> = (0..10)
            .map(|i| PersonProfile::new(format!("Local {i}")))
            .collect();
        let pending_creates = records.iter().map(|person| person.id.clone()).collect();
        hub.restore_people(PeopleSnapshot {
            records,
            pending_creates,
            ..Default::default()
        });
        hub.sync_pending_people(cx);
        assert_eq!(hub.person_creates_in_flight.len(), 4);
        let first = hub.person_creates_in_flight.clone();
        hub.sync_pending_people(cx);
        assert_eq!(hub.person_creates_in_flight, first);
        assert_eq!(hub.pending_people_count(), 10);
    });
}
