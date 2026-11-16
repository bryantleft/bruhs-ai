//! `bruhs_read_people` and `bruhs_read_person`: cached contact identities and
//! one person's context. Voice fingerprints are never projected, and a
//! person who opted out (`tracked == false`) has every stored observation
//! withheld, not only new extraction.
use super::page::{read_matching_page, read_page};
use super::{
    require_read, LowercaseQuery, PersonReadSection, MAX_ID_BYTES, MAX_OFFSET, MAX_QUERY_BYTES,
};
use crate::{AgentDomain, AgentPermissionPolicy, PersonProfile, RelationshipProfile};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize)]
struct PersonSummary<'a> {
    id: &'a str,
    display_name: &'a str,
    tracked: bool,
    created_at: i64,
    updated_at: i64,
    alias_count: usize,
    /// Withheld (null) for opted-out people, like their observations.
    observation_count: Option<usize>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    observations_withheld: bool,
}

impl<'a> From<&'a PersonProfile> for PersonSummary<'a> {
    fn from(person: &'a PersonProfile) -> Self {
        Self {
            id: &person.id,
            display_name: &person.display_name,
            tracked: person.tracked,
            created_at: person.created_at,
            updated_at: person.updated_at,
            alias_count: person.aliases.len(),
            observation_count: person.tracked.then_some(person.observations.len()),
            observations_withheld: !person.tracked,
        }
    }
}

#[derive(Serialize)]
struct RelationshipDetails<'a> {
    company: &'a str,
    role: &'a str,
    email: &'a str,
    phone: &'a str,
    location: &'a str,
    notes: &'a str,
    last_contacted_at_ms: Option<i64>,
    updated_at_ms: i64,
    item_count: usize,
}

impl<'a> From<&'a RelationshipProfile> for RelationshipDetails<'a> {
    fn from(profile: &'a RelationshipProfile) -> Self {
        Self {
            company: &profile.company,
            role: &profile.role,
            email: &profile.email,
            phone: &profile.phone,
            location: &profile.location,
            notes: &profile.notes,
            last_contacted_at_ms: profile.last_contacted_at_ms,
            updated_at_ms: profile.updated_at_ms,
            item_count: profile.items.len(),
        }
    }
}

#[derive(Serialize)]
struct PersonDetails<'a> {
    #[serde(flatten)]
    person: PersonSummary<'a>,
    aliases: &'a [String],
    relationship: Option<RelationshipDetails<'a>>,
}

/// Identity summaries. `query` matches ID, display name and aliases only:
/// discovery never scans or copies histories, notes or fingerprints. The
/// caller has checked People Read (the generic read does so for every domain).
pub fn read_people<'a>(
    people: impl Iterator<Item = &'a PersonProfile>,
    query: &str,
    offset: usize,
) -> Result<Value, String> {
    if query.len() > MAX_QUERY_BYTES || offset > MAX_OFFSET {
        return Err("Query or offset is too large.".into());
    }
    let query = LowercaseQuery::new(query);
    let mut matcher = query.matcher();
    read_matching_page(
        people,
        query.is_empty(),
        |person| {
            query.is_empty()
                || [&person.id, &person.display_name]
                    .into_iter()
                    .chain(&person.aliases)
                    .any(|field| matcher.matches(field))
        },
        PersonSummary::from,
        offset,
    )
}

/// One person's profile or a page of one history section. Checks People
/// Read before resolving the ID, and refuses ambiguous duplicates instead of
/// choosing one.
pub fn read_person<'a>(
    policy: &AgentPermissionPolicy,
    people: impl Iterator<Item = &'a PersonProfile>,
    relationships: impl Iterator<Item = &'a RelationshipProfile>,
    person_id: &str,
    section: PersonReadSection,
    offset: usize,
) -> Result<Value, String> {
    require_read(policy, AgentDomain::People)?;
    if person_id.trim().is_empty()
        || person_id.len() > MAX_ID_BYTES
        || offset > MAX_OFFSET
        || (section == PersonReadSection::Profile && offset != 0)
    {
        return Err("Invalid person ID or offset. Profile reads require offset 0.".into());
    }
    let mut people = people.filter(|person| person.id == person_id);
    let person = people
        .next()
        .ok_or("Person was not found in the current workspace.")?;
    if people.next().is_some() {
        return Err("Duplicate person IDs prevent an unambiguous read.".into());
    }
    let mut profiles = relationships.filter(|profile| profile.person_id == person_id);
    let profile = profiles.next();
    if profiles.next().is_some() {
        return Err("Duplicate relationship profiles prevent an unambiguous read.".into());
    }
    let mut page = match section {
        PersonReadSection::Profile => read_page(
            std::iter::once(PersonDetails {
                person: person.into(),
                aliases: &person.aliases,
                relationship: profile.map(RelationshipDetails::from),
            }),
            0,
        ),
        // A per-person opt-out withholds every stored observation from
        // agent context, not only new extraction.
        PersonReadSection::Observations if !person.tracked => {
            Ok(json!({"records": [], "next_offset": null, "observations_withheld": true}))
        }
        PersonReadSection::Observations => read_page(person.observations.iter(), offset),
        PersonReadSection::RelationshipItems => read_page(
            profile.into_iter().flat_map(|profile| &profile.items),
            offset,
        ),
    }?;
    page["person_id"] = person_id.into();
    page["section"] = json!(section);
    Ok(page)
}
