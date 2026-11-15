use super::*;
use crate::{
    ActionItem, InboxParticipant, InboxSource, MeetingSummary, ObservationKind, PersonObservation,
    Speaker,
};

fn person(id: &str, name: &str, aliases: &[&str]) -> PersonProfile {
    let mut person = PersonProfile::new(name);
    person.id = id.into();
    person.aliases = aliases.iter().map(|alias| (*alias).to_string()).collect();
    person
}

#[test]
fn search_matches_names_and_aliases_case_insensitively_with_unicode() {
    let zoe = person("p1", "Zoë Ångström", &["The Captain"]);
    assert!(person_matches_query(&zoe, "zoË"));
    assert!(person_matches_query(&zoe, "  ångSTRÖM "));
    assert!(person_matches_query(&zoe, "captain"));
    assert!(person_matches_query(&zoe, ""));
    assert!(person_matches_query(&zoe, "   "));
    assert!(!person_matches_query(&zoe, "zoe"));
    assert!(!person_matches_query(&zoe, "mayor"));
}

#[test]
fn search_finds_people_beyond_the_first_page() {
    let people = (0..60)
        .map(|n| person(&format!("p{n}"), &format!("Person {n}"), &[]))
        .chain(std::iter::once(person("late", "Late Arrival", &["Needle"])))
        .collect::<Vec<_>>();
    assert_eq!(people_page_count(people.len()), 3);
    assert!(people_page(&people, 0)
        .iter()
        .all(|person| person.id != "late"));
    let found = people
        .iter()
        .filter(|person| person_matches_query(person, "NEEDLE"))
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 1);
    assert_eq!(people_page_count(found.len()), 1);
    assert_eq!(people_page(&found, 0)[0].id, "late");
}

#[test]
fn paging_is_bounded_and_never_empty_of_pages() {
    let items = (0..51).collect::<Vec<_>>();
    assert_eq!(people_page_count(0), 1);
    assert_eq!(people_page_count(25), 1);
    assert_eq!(people_page_count(26), 2);
    assert_eq!(people_page(&items, 0).len(), PEOPLE_PAGE_SIZE);
    assert_eq!(people_page(&items, 2), &[50]);
    assert!(people_page(&items, 3).is_empty());
    assert!(people_page(&items, usize::MAX).is_empty());
    assert_eq!(people_page_of(0), 0);
    assert_eq!(people_page_of(25), 1);
}

#[test]
fn unique_name_match_uses_aliases_and_refuses_ambiguity() {
    let ada = person("ada", "Ada Lovelace", &["Countess"]);
    let zoe = person("zoe", "Zoë", &[]);
    let people = [ada.clone(), zoe.clone()];
    assert_eq!(
        unique_person_named(&people, "COUNTESS").map(|p| p.id.as_str()),
        Some("ada")
    );
    assert_eq!(
        unique_person_named(&people, "ZOË").map(|p| p.id.as_str()),
        Some("zoe")
    );
    assert!(unique_person_named(&people, "Nobody").is_none());
    assert!(unique_person_named(&people, "  ").is_none());
    let twin = person("zoe-2", "zoë", &[]);
    let ambiguous = [ada, zoe, twin];
    assert!(unique_person_named(&ambiguous, "Zoë").is_none());
}

#[test]
fn alias_review_refuses_blank_existing_and_other_people_names() {
    let ada = person("ada", "Ada", &["Countess"]);
    let zoe = person("zoe", "Zoë", &["Z"]);
    let people = [ada, zoe];
    assert_eq!(
        review_alias(&people, "ada", "  Lady L  ").as_deref(),
        Ok("Lady L")
    );
    assert_eq!(
        review_alias(&people, "ada", "   "),
        Err(AliasProblem::Blank)
    );
    assert_eq!(
        review_alias(&people, "ada", "countess"),
        Err(AliasProblem::AlreadyOnProfile)
    );
    assert_eq!(
        review_alias(&people, "ada", "ADA"),
        Err(AliasProblem::AlreadyOnProfile)
    );
    let used = review_alias(&people, "ada", "zoË").unwrap_err();
    assert_eq!(
        used,
        AliasProblem::UsedBy {
            person_id: "zoe".into(),
            display_name: "Zoë".into()
        }
    );
    assert!(used.message().contains("Zoë already uses that name"));
    assert!(!used.message().contains('—') && !used.message().contains(" - "));
}

#[test]
fn normalized_aliases_drop_blanks_duplicates_and_the_display_name() {
    let aliases = vec![
        " Countess ".to_string(),
        "COUNTESS".into(),
        String::new(),
        "ada".into(),
        "Lady L".into(),
    ];
    assert_eq!(
        normalize_aliases("Ada", &aliases),
        vec!["Countess".to_string(), "Lady L".into()]
    );
}

fn meeting(id: &str, started: f64) -> Meeting {
    let mut meeting = Meeting::new(id, started);
    meeting.id = id.into();
    meeting
}

#[test]
fn person_meetings_join_speakers_action_items_and_observation_sources() {
    let mut ada = person("ada", "Ada", &[]);
    ada.observations.push(PersonObservation {
        at_meeting_id: "observed".into(),
        kind: ObservationKind::Topic,
        text: "Engines".into(),
        created_at: 1,
    });
    let mut spoke = meeting("spoke", 30.0);
    spoke.speakers.push(Speaker {
        id: "them_1".into(),
        label: "Ada".into(),
        person_id: Some("ada".into()),
        color: None,
    });
    let mut assigned = meeting("assigned", 20.0);
    assigned.summary = Some(MeetingSummary {
        tldr: String::new(),
        action_items: vec![ActionItem {
            text: "Write notes".into(),
            assignee_person_id: Some("ada".into()),
            due_hint: None,
        }],
        decisions: Vec::new(),
        topics: Vec::new(),
        generated_by: String::new(),
        generated_at: 0,
    });
    let observed = meeting("observed", 10.0);
    let unrelated = meeting("unrelated", 40.0);
    let meetings = [observed, unrelated, assigned, spoke];
    let ids = person_meetings(&ada, &meetings)
        .into_iter()
        .map(|meeting| meeting.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["spoke", "assigned", "observed"]);
}

fn conversation(id: &str, updated: i64, linked: Option<&str>) -> InboxConversation {
    let mut conversation = InboxConversation::new(
        InboxSource::Email,
        id,
        vec![InboxParticipant {
            id: format!("{id}-participant"),
            display_name: "Ada".into(),
            address: "ada@example.invalid".into(),
            person_id: linked.map(str::to_string),
        }],
        updated,
    );
    conversation.id = id.into();
    conversation
}

#[test]
fn person_conversations_list_linked_threads_newest_first() {
    let conversations = [
        conversation("old", 1, Some("ada")),
        conversation("other", 5, Some("zoe")),
        conversation("new", 9, Some("ada")),
        conversation("unlinked", 7, None),
    ];
    let ids = person_conversations("ada", &conversations)
        .into_iter()
        .map(|conversation| conversation.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["new", "old"]);
}

fn document(id: &str, links: &[(KnowledgeLinkKind, &str, &str)]) -> KnowledgeDocument {
    let mut document = KnowledgeDocument::new(id, "", 1);
    document.id = id.into();
    document.links = links
        .iter()
        .map(|(kind, target, label)| KnowledgeLink {
            kind: *kind,
            target_id: (*target).into(),
            label: (*label).into(),
        })
        .collect();
    document
}

#[test]
fn delete_cascade_unlinks_without_deleting_and_undo_relinks_exactly() {
    let mut conversations = vec![
        conversation("c1", 1, Some("ada")),
        conversation("c2", 2, Some("zoe")),
    ];
    let mut documents = vec![
        document(
            "d1",
            &[
                (KnowledgeLinkKind::Person, "ada", "Ada"),
                (KnowledgeLinkKind::Meeting, "ada", "A meeting with id ada"),
                (KnowledgeLinkKind::Person, "zoe", "Zoë"),
            ],
        ),
        document("d2", &[]),
    ];
    let original_conversations = conversations.clone();
    let original_documents = documents.clone();
    let links = unlink_person("ada", &mut conversations, &mut documents);
    assert_eq!(links.participants.len(), 1);
    assert_eq!(links.document_links.len(), 1);
    assert_eq!(conversations.len(), 2, "conversations are never deleted");
    assert_eq!(conversations[0].participants[0].person_id, None);
    assert_eq!(conversations[0].participants[0].display_name, "Ada");
    assert_eq!(documents[0].links.len(), 2, "only the person link goes");
    assert_eq!(
        links.conversation_ids().into_iter().collect::<Vec<_>>(),
        ["c1"]
    );
    assert_eq!(links.document_ids().into_iter().collect::<Vec<_>>(), ["d1"]);

    let restored = relink_person("ada", &links, &mut conversations, &mut documents);
    assert_eq!(restored, links);
    assert_eq!(conversations, original_conversations);
    // The link returns, appended after the links that stayed.
    assert_eq!(documents[0].links.len(), 3);
    assert!(documents[0].links.contains(&original_documents[0].links[0]));
    // Relinking twice changes nothing more.
    assert!(relink_person("ada", &links, &mut conversations, &mut documents).is_empty());
}

#[test]
fn relink_leaves_participants_linked_elsewhere_since_the_delete() {
    let mut conversations = vec![conversation("c1", 1, Some("ada"))];
    let mut documents = Vec::new();
    let links = unlink_person("ada", &mut conversations, &mut documents);
    conversations[0].participants[0].person_id = Some("zoe".into());
    assert!(relink_person("ada", &links, &mut conversations, &mut documents).is_empty());
    assert_eq!(
        conversations[0].participants[0].person_id.as_deref(),
        Some("zoe")
    );
}

#[test]
fn merge_cascade_retargets_and_undo_restores_only_moved_links() {
    let mut conversations = vec![
        conversation("from-source", 1, Some("source")),
        conversation("already-target", 2, Some("target")),
    ];
    let mut documents = vec![document(
        "d1",
        &[
            (KnowledgeLinkKind::Person, "source", "Sam"),
            (KnowledgeLinkKind::Person, "target", "Taylor"),
        ],
    )];
    let original_conversations = conversations.clone();
    let original_documents = documents.clone();
    let moved = retarget_person_links("source", "target", &mut conversations, &mut documents);
    assert_eq!(moved.participants.len(), 1);
    assert_eq!(moved.document_links.len(), 1);
    assert!(conversations
        .iter()
        .all(|c| c.participants[0].person_id.as_deref() == Some("target")));
    assert!(documents[0]
        .links
        .iter()
        .all(|link| link.target_id == "target"));

    let restored = restore_person_links(
        "source",
        "target",
        &moved,
        &mut conversations,
        &mut documents,
    );
    assert_eq!(restored, moved);
    assert_eq!(conversations, original_conversations);
    assert_eq!(documents, original_documents);
}

#[test]
fn deleting_a_meeting_drops_only_its_observations() {
    let mut ada = person("ada", "Ada", &[]);
    for (meeting, text) in [("gone", "one"), ("kept", "two"), ("gone", "three")] {
        ada.observations.push(PersonObservation {
            at_meeting_id: meeting.into(),
            kind: ObservationKind::Quote,
            text: text.into(),
            created_at: 1,
        });
    }
    assert_eq!(drop_meeting_observations(&mut ada, "gone"), 2);
    assert_eq!(ada.observations.len(), 1);
    assert_eq!(ada.observations[0].at_meeting_id, "kept");
    assert_eq!(drop_meeting_observations(&mut ada, "gone"), 0);
}
