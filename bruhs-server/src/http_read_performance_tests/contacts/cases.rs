use super::*;
use bruhs_models::{
    MeetingEvent, ObservationKind, PersonMergeResolution, PersonObservation, PersonProfile,
};
use std::collections::BTreeMap;

const VERSION: i64 = 4_000_000_000_000;

#[derive(Clone, Copy)]
pub(super) enum Case {
    Create,
    CreateRetry,
    CreateEmpty,
    Patch,
    PatchEmpty,
    PatchMissing,
    Tracked,
    TrackedMissing,
    TrackedExhausted,
    Rename,
    RenameEmpty,
    RenameMissing,
    RenameExhausted,
    Merge,
    MergeRetry,
    MergeSelf,
    MergeMissing,
    MergeConflict,
    MergeExhausted,
    EnrollMissing,
}

impl Case {
    pub(super) const ALL: [Self; 20] = [
        Self::Create,
        Self::CreateRetry,
        Self::CreateEmpty,
        Self::Patch,
        Self::PatchEmpty,
        Self::PatchMissing,
        Self::Tracked,
        Self::TrackedMissing,
        Self::TrackedExhausted,
        Self::Rename,
        Self::RenameEmpty,
        Self::RenameMissing,
        Self::RenameExhausted,
        Self::Merge,
        Self::MergeRetry,
        Self::MergeSelf,
        Self::MergeMissing,
        Self::MergeConflict,
        Self::MergeExhausted,
        Self::EnrollMissing,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Create => "create-fresh",
            Self::CreateRetry => "create-retry",
            Self::CreateEmpty => "create-empty",
            Self::Patch => "patch-name",
            Self::PatchEmpty => "patch-empty",
            Self::PatchMissing => "patch-missing",
            Self::Tracked => "tracked-toggle",
            Self::TrackedMissing => "tracked-missing",
            Self::TrackedExhausted => "tracked-exhausted",
            Self::Rename => "rename",
            Self::RenameEmpty => "rename-empty",
            Self::RenameMissing => "rename-missing",
            Self::RenameExhausted => "rename-exhausted",
            Self::Merge => "merge-fresh",
            Self::MergeRetry => "merge-retry",
            Self::MergeSelf => "merge-self",
            Self::MergeMissing => "merge-missing",
            Self::MergeConflict => "merge-conflict",
            Self::MergeExhausted => "merge-exhausted",
            Self::EnrollMissing => "enroll-missing",
        }
    }

    pub(super) fn action(self) -> &'static str {
        match self {
            Self::Create | Self::CreateRetry | Self::CreateEmpty => "POST /api/people",
            Self::Patch | Self::PatchEmpty | Self::PatchMissing => "PATCH /api/people/{id}",
            Self::Tracked | Self::TrackedMissing | Self::TrackedExhausted => {
                "PATCH /api/people/{id}/tracked"
            }
            Self::Rename | Self::RenameEmpty | Self::RenameMissing | Self::RenameExhausted => {
                "PATCH /api/people/{id}/rename"
            }
            Self::EnrollMissing => "POST /api/people/{id}/enroll",
            _ => "POST /api/people/{source_id}/merge_into/{target_id}",
        }
    }
}

#[derive(Serialize)]
pub(super) struct Model {
    pub(super) people: BTreeMap<String, PersonProfile>,
    redirects: BTreeMap<String, String>,
}

fn profile(id: &str, source: bool, shape: &str) -> PersonProfile {
    let mut value = PersonProfile::new(if source { "Source Ω" } else { "Target 😀" });
    value.id = id.to_owned();
    value.created_at = 42;
    value.updated_at = VERSION;
    value.tracked = !source;
    value.aliases = if source {
        vec!["ALIAS shared".into(), "K".into(), "new alias 😀".into()]
    } else {
        vec!["alias shared".into(), "K".into()]
    };
    if shape == "alias-heavy" && !source {
        value
            .aliases
            .extend((0..1001).map(|n| format!("alias-{n:06} Ω")));
    }
    value.voice_embedding = source.then(|| vec![0.25; 64]);
    value.observations = vec![PersonObservation {
        at_meeting_id: "fixture-meeting".into(),
        kind: ObservationKind::Quote,
        text: if shape == "observation-heavy" {
            "Ω".repeat(32768)
        } else {
            "quoted \"agenda\"\r\n😀".into()
        },
        created_at: 42,
    }];
    value
}

pub(super) struct Expected {
    pub(super) method: Method,
    pub(super) path: String,
    pub(super) body: Value,
    pub(super) status: u16,
    pub(super) response: Value,
    pub(super) event: Option<MeetingEvent>,
}

impl Model {
    pub(super) async fn seed(
        case: Case,
        background: usize,
        shape: &str,
        iterations: usize,
        store: &Arc<dyn MeetingStore>,
    ) -> Self {
        assert!(
            chrono::Utc::now().timestamp_millis() < VERSION,
            "fixture seed must stay ahead of real clock"
        );
        let mut people = BTreeMap::new();
        for n in 0..background {
            let id = format!("background-{n:06}");
            people.insert(id.clone(), profile(&id, false, "ordinary"));
        }
        for (id, source) in [("target", false), ("source", true), ("other", false)] {
            people.insert(id.into(), profile(id, source, shape));
        }
        if matches!(
            case,
            Case::TrackedExhausted | Case::RenameExhausted | Case::MergeExhausted
        ) {
            people.get_mut("target").expect("target").updated_at = i64::MAX;
        }
        if matches!(case, Case::Merge) {
            for n in 0..iterations {
                for (prefix, source) in [("target", false), ("source", true)] {
                    let id = format!("{prefix}-{n:06}");
                    people.insert(id.clone(), profile(&id, source, shape));
                }
            }
        }
        for value in people.values() {
            assert_eq!(
                store
                    .upsert_person(value.clone())
                    .await
                    .expect("seed private contact"),
                *value
            );
        }
        let mut model = Self {
            people,
            redirects: BTreeMap::new(),
        };
        if matches!(case, Case::MergeRetry | Case::MergeConflict) {
            let expected = model.merge("source", "target");
            assert_eq!(
                store
                    .merge_person_into(
                        &"source".parse().expect("source id"),
                        &"target".parse().expect("target id")
                    )
                    .await
                    .expect("seed committed merge"),
                expected
            );
        }
        model.verify(store).await;
        model
    }

    fn merge(&mut self, source_id: &str, target_id: &str) -> PersonProfile {
        let source = self.people.remove(source_id).expect("known source");
        let target = self.people.get_mut(target_id).expect("known target");
        target.updated_at = target.updated_at.max(source.updated_at) + 1;
        target
            .aliases
            .extend(["Source Ω".into(), "new alias 😀".into()]);
        target.observations.extend(source.observations);
        target.voice_embedding = source.voice_embedding;
        target.tracked = false;
        self.redirects.insert(source_id.into(), target_id.into());
        target.clone()
    }

    pub(super) async fn verify(&self, store: &Arc<dyn MeetingStore>) {
        assert_eq!(
            store.list_people().await.expect("all committed contacts"),
            self.people.values().cloned().collect::<Vec<_>>(),
            "full store state"
        );
        let ids: Vec<_> = self.redirects.keys().cloned().collect();
        let expected: Vec<_> = self
            .redirects
            .iter()
            .map(|(source, target)| PersonMergeResolution {
                source_id: source.clone(),
                target: self.people[target].clone(),
            })
            .collect();
        assert_eq!(
            store
                .resolve_person_merges(&ids)
                .await
                .expect("saved merge redirects"),
            expected
        );
    }

    pub(super) fn next(&mut self, case: Case, index: usize, backend: &str) -> Expected {
        let mut result = Expected {
            method: Method::PATCH,
            path: "/api/people/target".into(),
            body: json!({}),
            status: 200,
            response: Value::Null,
            event: None,
        };
        let missing = "person not found: missing";
        let invalid = |text: &str| json!({"error":text});
        let exhausted = json!({"error":"Record version is exhausted; no change was saved."});
        match case {
            Case::Create | Case::CreateRetry | Case::CreateEmpty => {
                result.method = Method::POST;
                result.path = "/api/people".into();
                let id = if matches!(case, Case::Create) {
                    format!("created-{index:06}")
                } else {
                    "target".into()
                };
                result.body = json!({"id":id,"display_name":if matches!(case, Case::CreateEmpty) { " " } else { "  Created Ω  " },"created_at":42,"updated_at":VERSION});
                if matches!(case, Case::CreateEmpty) {
                    result.status = 400;
                    result.response = json!({"error":"id and display_name must not be empty"});
                } else if matches!(case, Case::CreateRetry) {
                    result.response = json!(self.people[&id]);
                } else {
                    let mut person = PersonProfile::new("Created Ω");
                    person.id = id.clone();
                    person.created_at = 42;
                    person.updated_at = VERSION;
                    result.status = 201;
                    result.response = json!(person);
                    result.event = Some(MeetingEvent::PersonChanged {
                        person_id: id.clone(),
                        updated_at: VERSION,
                    });
                    self.people.insert(id, person);
                }
            }
            Case::Merge
            | Case::MergeRetry
            | Case::MergeSelf
            | Case::MergeMissing
            | Case::MergeConflict
            | Case::MergeExhausted => {
                result.method = Method::POST;
                let (source, target) = match case {
                    Case::Merge => (format!("source-{index:06}"), format!("target-{index:06}")),
                    Case::MergeSelf => ("target".into(), "target".into()),
                    Case::MergeMissing => ("missing".into(), "target".into()),
                    Case::MergeConflict => ("source".into(), "other".into()),
                    _ => ("source".into(), "target".into()),
                };
                result.path = format!("/api/people/{source}/merge_into/{target}");
                match case {
                    Case::MergeSelf => {
                        result.status = 400;
                        result.response = invalid(if backend == "sqlite" {
                            "Source and target must be different contacts."
                        } else {
                            "source and target must be different profiles"
                        });
                    }
                    Case::MergeMissing => {
                        result.status = 404;
                        result.response = json!({"error":missing});
                    }
                    Case::MergeConflict => {
                        result.status = if backend == "sqlite" { 409 } else { 400 };
                        result.response = if backend == "sqlite" {
                            json!({"error":"Source was already merged into another contact."})
                        } else {
                            invalid("This source was already merged into another contact.")
                        };
                    }
                    Case::MergeExhausted => {
                        result.status = 409;
                        result.response = exhausted;
                    }
                    _ => {
                        let person = if matches!(case, Case::MergeRetry) {
                            self.people[&target].clone()
                        } else {
                            self.merge(&source, &target)
                        };
                        result.response = json!(person);
                        result.event = Some(MeetingEvent::PersonMerged { source_id: source });
                    }
                }
            }
            Case::EnrollMissing => {
                result.method = Method::POST;
                result.path = "/api/people/missing/enroll".into();
                result.status = 404;
                result.response = json!({"error":missing});
            }
            Case::PatchMissing | Case::TrackedMissing | Case::RenameMissing => {
                result.path = format!(
                    "/api/people/missing{}",
                    match case {
                        Case::TrackedMissing => "/tracked",
                        Case::RenameMissing => "/rename",
                        _ => "",
                    }
                );
                result.body = match case {
                    Case::TrackedMissing => json!({"tracked":false}),
                    _ => json!({"display_name":"Renamed"}),
                };
                result.status = 404;
                result.response = json!({"error":missing});
            }
            Case::RenameEmpty => {
                result.path.push_str("/rename");
                result.body = json!({"display_name":" "});
                result.status = 400;
                result.response = invalid(if backend == "sqlite" {
                    "Contact identity, name and voice values must be valid."
                } else {
                    "display_name must not be empty"
                });
            }
            Case::TrackedExhausted | Case::RenameExhausted => {
                result
                    .path
                    .push_str(if matches!(case, Case::TrackedExhausted) {
                        "/tracked"
                    } else {
                        "/rename"
                    });
                result.body = if matches!(case, Case::TrackedExhausted) {
                    json!({"tracked":false})
                } else {
                    json!({"display_name":"Renamed"})
                };
                result.status = 409;
                result.response = exhausted;
            }
            Case::Patch | Case::PatchEmpty | Case::Tracked | Case::Rename => {
                let person = self.people.get_mut("target").expect("known target");
                if !matches!(case, Case::PatchEmpty) {
                    person.updated_at += 1;
                    if matches!(case, Case::Tracked) {
                        result.path.push_str("/tracked");
                        person.tracked = !index.is_multiple_of(2);
                        result.body = json!({"tracked":person.tracked});
                    } else {
                        if matches!(case, Case::Rename) {
                            result.path.push_str("/rename");
                        }
                        person.display_name = if index.is_multiple_of(2) {
                            "Renamed Ω"
                        } else {
                            "Renamed 😀"
                        }
                        .into();
                        result.body = json!({"display_name":format!("  {}  ",person.display_name)});
                    }
                    result.event = Some(MeetingEvent::PersonChanged {
                        person_id: person.id.clone(),
                        updated_at: person.updated_at,
                    });
                }
                result.response = json!(person);
            }
        }
        result
    }
}
