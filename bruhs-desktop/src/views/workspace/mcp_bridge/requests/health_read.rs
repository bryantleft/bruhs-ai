use super::{life_read, read_page::read_page, *};
use bruhs_core::workspace_bridge::HealthReadSection;

impl Workspace {
    pub(super) fn read_bridge_health(
        &self,
        section: HealthReadSection,
        query: &str,
        offset: usize,
    ) -> Result<Value, String> {
        let query = self.authorize_life_read(AgentDomain::Health, query, offset)?;
        let health = &self.life_context.health;
        let page = match section {
            HealthReadSection::Workouts => read_page(
                health
                    .workouts
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id, &r.activity])),
                offset,
            ),
            HealthReadSection::Sleep => read_page(
                health
                    .sleep
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id])),
                offset,
            ),
            HealthReadSection::Recovery => read_page(
                health
                    .recovery
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id])),
                offset,
            ),
            HealthReadSection::Habits => read_page(
                health
                    .habits
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id, &r.title])),
                offset,
            ),
        }?;
        life_read::describe(page, section, health.updated_at_ms)
    }
}
