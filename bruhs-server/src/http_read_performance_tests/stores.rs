use super::*;
use bruhs_core::meetings::SqliteMeetingStore;
use bruhs_models::{
    Meeting, MeetingStatus, MeetingSummary, ObservationKind, PersonObservation, PersonProfile,
    PersonVersion, PersonVersionPage, PERSON_VERSION_PAGE_SIZE,
};

pub(super) async fn run(
    result: &mut Vec<Measurement>,
    backend: &'static str,
    records: usize,
    large: bool,
    reverse: bool,
    warmups: usize,
    samples: usize,
) {
    let directory = tempfile::tempdir().expect("private store read fixture");
    let store: Arc<dyn MeetingStore> = match backend {
        "sqlite" => Arc::new(
            SqliteMeetingStore::open(directory.path().join("meetings.sqlite3"))
                .await
                .expect("private SQLite store"),
        ),
        "memory" => Arc::new(InMemoryMeetingStore::new()),
        _ => unreachable!("declared store backend"),
    };
    let mut people = Vec::new();
    let mut meetings = Vec::new();
    for index in 0..records {
        let ordinal = u32::try_from(index).expect("fixture ordinal");
        let mut person = PersonProfile::new(format!("Contact {index:06} Ω"));
        person.id = format!("p{index:06}");
        person.aliases = vec![format!("alias {index} 😀")];
        person.voice_embedding = Some(vec![0.25; 64]);
        person.created_at = i64::from(ordinal);
        person.updated_at = i64::from(ordinal) + 10;
        person.tracked = index % 2 == 0;
        person.observations = vec![PersonObservation {
            at_meeting_id: format!("m{index:06}"),
            kind: ObservationKind::Quote,
            text: if large {
                "Ω".repeat(32768)
            } else {
                "quoted \"agenda\"\r\n😀".to_owned()
            },
            created_at: i64::from(ordinal),
        }];
        assert_eq!(
            store
                .upsert_person(person.clone())
                .await
                .expect("seed contact"),
            person
        );
        people.push(person);
        let mut meeting = Meeting::new(
            format!("Meeting {index:06} 😀"),
            f64::from(ordinal) + 1000.0,
        );
        meeting.id = format!("m{index:06}");
        meeting.status = MeetingStatus::Ended;
        meeting.ended_at_ms = Some(meeting.started_at_ms + 1.0);
        meeting.language = Some("en".to_owned());
        meeting.summary = Some(MeetingSummary {
            tldr: if large {
                "Ω".repeat(32768)
            } else {
                "summary \"agenda\"\n😀".to_owned()
            },
            action_items: Vec::new(),
            decisions: vec!["keep the complete payload".to_owned()],
            topics: Vec::new(),
            generated_by: "fixture".to_owned(),
            generated_at: i64::from(ordinal),
        });
        assert_eq!(
            store
                .create_meeting(meeting.clone())
                .await
                .expect("seed meeting"),
            meeting
        );
        meetings.push(meeting);
    }
    let host = Host::new(directory.path(), Some(store.clone())).await;
    let mut cases = cases(&people, &meetings);
    assert_eq!(cases.len(), 12);
    if reverse {
        cases.reverse();
    }
    for case in cases {
        result.push(
            measure(
                &host,
                case,
                backend,
                records,
                if large { "large-text" } else { "ordinary" },
                warmups,
                samples,
            )
            .await,
        );
    }
    host.close().await;
    drop(store);
    let path = directory.path().to_owned();
    directory.close().expect("remove private store fixture");
    assert!(!path.exists(), "store fixture cleanup");
}

fn cases(people: &[PersonProfile], meetings: &[Meeting]) -> Vec<ReadCase> {
    let missing_person = json!({"error":"person not found: missing-person"});
    let first_person = people.first().map_or_else(
        || json!({"error":"person not found: p000000"}),
        |p| json!(p),
    );
    let first_meeting = meetings.first().map_or_else(
        || json!({"error":"meeting not found: m000000"}),
        |m| json!(m),
    );
    let batch: Vec<_> = (0..64).rev().map(|index| format!("p{index:06}")).collect();
    let profiles: Vec<_> = batch
        .iter()
        .filter_map(|id| people.iter().find(|p| &p.id == id))
        .collect();
    let ordered_meetings: Vec<_> = meetings.iter().rev().collect();
    vec![
        ReadCase::get(
            "people-all",
            "GET /api/people",
            "/api/people".to_owned(),
            json!(people),
        ),
        ReadCase::get(
            "person-first",
            "GET /api/people/{id}",
            "/api/people/p000000".to_owned(),
            first_person,
        )
        .error(if people.is_empty() { 404 } else { 200 }),
        ReadCase::get(
            "person-missing",
            "GET /api/people/{id}",
            "/api/people/missing-person".to_owned(),
            missing_person,
        )
        .error(404),
        ReadCase::get(
            "versions-first",
            "GET /api/people/versions",
            "/api/people/versions".to_owned(),
            page(people, ""),
        ),
        ReadCase::get(
            "versions-after",
            "GET /api/people/versions",
            "/api/people/versions?after=p000255".to_owned(),
            page(people, "p000255"),
        ),
        ReadCase::post(
            "profiles-batch",
            "POST /api/people/profiles",
            json!({"ids":batch}),
            json!(profiles),
        ),
        ReadCase::post(
            "profiles-missing",
            "POST /api/people/profiles",
            json!({"ids":["missing-person"]}),
            json!([]),
        ),
        ReadCase::post(
            "profiles-duplicate",
            "POST /api/people/profiles",
            json!({"ids":["p000000","p000000"]}),
            json!({"error":"Contact lookup requires at most 64 unique, nonempty contact IDs."}),
        )
        .error(400),
        ReadCase::post(
            "resolve-unmerged",
            "POST /api/people/resolve-merges",
            json!({"ids":batch}),
            json!([]),
        ),
        ReadCase::get(
            "meetings-all",
            "GET /api/meetings",
            "/api/meetings".to_owned(),
            json!(ordered_meetings),
        ),
        ReadCase::get(
            "meeting-first",
            "GET /api/meetings/{id}",
            "/api/meetings/m000000".to_owned(),
            first_meeting,
        )
        .error(if meetings.is_empty() { 404 } else { 200 }),
        ReadCase::get(
            "meeting-missing",
            "GET /api/meetings/{id}",
            "/api/meetings/missing-meeting".to_owned(),
            json!({"error":"meeting not found: missing-meeting"}),
        )
        .error(404),
    ]
}

fn page(people: &[PersonProfile], after: &str) -> Value {
    let available: Vec<_> = people
        .iter()
        .filter(|person| person.id.as_str() > after)
        .collect();
    let entries: Vec<_> = available
        .iter()
        .take(PERSON_VERSION_PAGE_SIZE)
        .map(|person| PersonVersion {
            id: person.id.clone(),
            updated_at: person.updated_at,
        })
        .collect();
    let next_after = (available.len() > PERSON_VERSION_PAGE_SIZE)
        .then(|| entries.last().expect("full page").id.clone());
    json!(PersonVersionPage {
        entries,
        next_after
    })
}
