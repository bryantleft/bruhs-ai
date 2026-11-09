use super::*;

impl Workspace {
    pub(super) fn authorize_life_read(
        &self,
        domain: AgentDomain,
        query: &str,
        offset: usize,
    ) -> Result<String, String> {
        if !self
            .automation_state
            .permissions
            .allows(domain, AgentAccess::Read)
        {
            return Err("Read access is not granted for this domain.".into());
        }
        let enabled = match domain {
            AgentDomain::Finance => self.life_context.finance.privacy.allow_agent_context,
            AgentDomain::Health => self.life_context.health.privacy.allow_agent_context,
            _ => false,
        };
        if !enabled {
            return Err("Life agent context is disabled for this domain. Enable it in the app before reading records.".into());
        }
        if query.len() > 512 || offset > 100_000 {
            return Err("Query or offset is too large.".into());
        }
        Ok(query.to_lowercase())
    }
}

pub(super) fn matches(query: &str, fields: &[&str]) -> bool {
    query.is_empty()
        || fields
            .iter()
            .any(|field| field.to_lowercase().contains(query))
}

pub(super) fn describe(
    mut page: Value,
    section: impl Serialize,
    updated_at_ms: i64,
) -> Result<Value, String> {
    page["section"] = serde_json::to_value(section).map_err(|_| "Cannot encode Life section.")?;
    page["source"] = "live_workspace".into();
    page["updated_at_ms"] = updated_at_ms.into();
    page["durable_receipt"] = false.into();
    Ok(page)
}
