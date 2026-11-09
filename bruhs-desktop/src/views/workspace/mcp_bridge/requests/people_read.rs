use super::read_page::read_page;
use super::*;
use bruhs_core::workspace_bridge::PersonReadSection;
use bruhs_models::{PersonProfile, RelationshipProfile};

#[derive(Serialize)]
struct PersonSummary<'a> {
    id: &'a str,
    display_name: &'a str,
    tracked: bool,
    created_at: i64,
    updated_at: i64,
    alias_count: usize,
    observation_count: usize,
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
            observation_count: person.observations.len(),
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

impl Workspace {
    pub(super) fn read_bridge_people(
        &self,
        query: &str,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        if query.len() > 512 || offset > 100_000 {
            return Err("Query or offset is too large.".into());
        }
        let query = query.to_lowercase();
        // Discovery never serializes or scans histories, private notes or fingerprints.
        let summaries = self
            .meetings_hub
            .read(cx)
            .people
            .iter()
            .filter(|person| {
                query.is_empty()
                    || [&person.id, &person.display_name]
                        .into_iter()
                        .chain(&person.aliases)
                        .any(|field| field.to_lowercase().contains(&query))
            })
            .map(PersonSummary::from);
        read_page(summaries, offset)
    }

    pub(super) fn read_bridge_person(
        &self,
        person_id: &str,
        section: PersonReadSection,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        if !self
            .automation_state
            .permissions
            .allows(AgentDomain::People, AgentAccess::Read)
        {
            return Err("Read access is not granted for this domain.".into());
        }
        if person_id.trim().is_empty()
            || person_id.len() > 128
            || offset > 100_000
            || (section == PersonReadSection::Profile && offset != 0)
        {
            return Err("Invalid person ID or offset. Profile reads require offset 0.".into());
        }
        let mut people = self
            .meetings_hub
            .read(cx)
            .people
            .iter()
            .filter(|person| person.id == person_id);
        let person = people
            .next()
            .ok_or("Person was not found in the current workspace.")?;
        if people.next().is_some() {
            return Err("Duplicate person IDs prevent an unambiguous read.".into());
        }
        let mut profiles = self
            .relationships
            .iter()
            .filter(|profile| profile.person_id == person_id);
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
}
