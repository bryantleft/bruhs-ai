use super::{life_read, read_page::read_matching_page, *};
use bruhs_core::workspace_bridge::HealthReadSection;

impl Workspace {
    pub(super) fn read_bridge_health(
        &self,
        section: HealthReadSection,
        query: &str,
        offset: usize,
    ) -> Result<Value, String> {
        let query = self.authorize_life_read(AgentDomain::Health, query, offset)?;
        let mut matcher = query.matcher();
        let health = &self.life_context.health;
        let page = match section {
            HealthReadSection::Workouts => read_matching_page(
                health.workouts.iter(),
                query.is_empty(),
                |r| matcher.matches_fields(&[&r.id, &r.activity]),
                std::convert::identity,
                offset,
            ),
            HealthReadSection::Sleep => read_matching_page(
                health.sleep.iter(),
                query.is_empty(),
                |r| matcher.matches(&r.id),
                std::convert::identity,
                offset,
            ),
            HealthReadSection::Recovery => read_matching_page(
                health.recovery.iter(),
                query.is_empty(),
                |r| matcher.matches(&r.id),
                std::convert::identity,
                offset,
            ),
            HealthReadSection::Habits => read_matching_page(
                health.habits.iter(),
                query.is_empty(),
                |r| matcher.matches_fields(&[&r.id, &r.title]),
                std::convert::identity,
                offset,
            ),
        }?;
        life_read::describe(page, section, health.updated_at_ms)
    }
}
