//! `bruhs_read_finance` and `bruhs_read_health`: each read rechecks the
//! domain's Read grant and its separate Life agent-context opt-in. Explicit
//! projections keep newly added private model fields out by default, and
//! Finance amounts stay exact across JSON clients, including values > 2^53.
use super::page::read_matching_page;
use super::{
    require_read, FinanceReadSection, HealthReadSection, LowercaseQuery, MAX_OFFSET,
    MAX_QUERY_BYTES,
};
use crate::{AgentDomain, AgentPermissionPolicy, FinanceAccountKind, LifeContext};
use serde::Serialize;
use serde_json::Value;

/// The grant, the Life opt-in and the bounds, in that order, before any
/// record is read. Returns the prepared query.
pub fn authorize_life_read(
    policy: &AgentPermissionPolicy,
    life: &LifeContext,
    domain: AgentDomain,
    query: &str,
    offset: usize,
) -> Result<LowercaseQuery, String> {
    require_read(policy, domain)?;
    let enabled = match domain {
        AgentDomain::Finance => life.finance.privacy.allow_agent_context,
        AgentDomain::Health => life.health.privacy.allow_agent_context,
        _ => false,
    };
    if !enabled {
        return Err("Life agent context is disabled for this domain. Enable it in the app before reading records.".into());
    }
    if query.len() > MAX_QUERY_BYTES || offset > MAX_OFFSET {
        return Err("Query or offset is too large.".into());
    }
    Ok(LowercaseQuery::new(query))
}

fn describe(mut page: Value, section: impl Serialize, updated_at_ms: i64) -> Result<Value, String> {
    page["section"] = serde_json::to_value(section).map_err(|_| "Cannot encode Life section.")?;
    page["source"] = "live_workspace".into();
    page["updated_at_ms"] = updated_at_ms.into();
    page["durable_receipt"] = false.into();
    Ok(page)
}

#[derive(Serialize)]
struct Account<'a> {
    id: &'a str,
    name: &'a str,
    kind: FinanceAccountKind,
    currency: &'a str,
    institution: &'a Option<String>,
    updated_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    balance_minor: Option<String>,
}
#[derive(Serialize)]
struct Transaction<'a> {
    id: &'a str,
    account_id: &'a Option<String>,
    merchant: &'a str,
    category: &'a str,
    currency: &'a str,
    occurred_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_minor: Option<String>,
}
#[derive(Serialize)]
struct Budget<'a> {
    id: &'a str,
    category: &'a str,
    currency: &'a str,
    alert_percent: u8,
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    monthly_limit_minor: Option<String>,
}
#[derive(Serialize)]
struct Income<'a> {
    id: &'a str,
    name: &'a str,
    currency: &'a str,
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_monthly_minor: Option<String>,
}
#[derive(Serialize)]
struct Holding<'a> {
    id: &'a str,
    account_id: &'a Option<String>,
    symbol: &'a str,
    name: &'a str,
    currency: &'a str,
    updated_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_quantity")]
    quantity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_value_minor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_basis_minor: Option<String>,
}

fn serialize_quantity<S: serde::Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) if !value.is_finite() => {
            Err(serde::ser::Error::custom("Invalid holding quantity"))
        }
        _ => value.serialize(serializer),
    }
}

/// One Finance section. Amounts and holding quantities appear only with the
/// exact-amount opt-in; otherwise they are omitted, never zeroed. Notes are
/// never exposed or searched.
pub fn read_finance(
    policy: &AgentPermissionPolicy,
    life: &LifeContext,
    section: FinanceReadSection,
    query: &str,
    offset: usize,
) -> Result<Value, String> {
    let query = authorize_life_read(policy, life, AgentDomain::Finance, query, offset)?;
    let mut matcher = query.matcher();
    let finance = &life.finance;
    let exact = finance.privacy.reveal_exact_amounts_to_agent;
    let page = match section {
        FinanceReadSection::Accounts => read_matching_page(
            finance.accounts.iter(),
            query.is_empty(),
            |r| matcher.matches_fields(&[&r.id, &r.name]),
            |r| Account {
                id: &r.id,
                name: &r.name,
                kind: r.kind,
                currency: &r.currency,
                institution: &r.institution,
                updated_at_ms: r.updated_at_ms,
                balance_minor: exact.then(|| r.balance_minor.to_string()),
            },
            offset,
        ),
        FinanceReadSection::Transactions => read_matching_page(
            finance.transactions.iter(),
            query.is_empty(),
            |r| matcher.matches_fields(&[&r.id, &r.merchant, &r.category]),
            |r| Transaction {
                id: &r.id,
                account_id: &r.account_id,
                merchant: &r.merchant,
                category: &r.category,
                currency: &r.currency,
                occurred_at_ms: r.occurred_at_ms,
                amount_minor: exact.then(|| r.amount_minor.to_string()),
            },
            offset,
        ),
        FinanceReadSection::Budgets => read_matching_page(
            finance.budgets.iter(),
            query.is_empty(),
            |r| matcher.matches_fields(&[&r.id, &r.category]),
            |r| Budget {
                id: &r.id,
                category: &r.category,
                currency: &r.currency,
                alert_percent: r.alert_percent,
                enabled: r.enabled,
                monthly_limit_minor: exact.then(|| r.monthly_limit_minor.to_string()),
            },
            offset,
        ),
        FinanceReadSection::IncomeStreams => read_matching_page(
            finance.income_streams.iter(),
            query.is_empty(),
            |r| matcher.matches_fields(&[&r.id, &r.name]),
            |r| Income {
                id: &r.id,
                name: &r.name,
                currency: &r.currency,
                active: r.active,
                expected_monthly_minor: exact.then(|| r.expected_monthly_minor.to_string()),
            },
            offset,
        ),
        FinanceReadSection::Holdings => read_matching_page(
            finance.holdings.iter(),
            query.is_empty(),
            |r| matcher.matches_fields(&[&r.id, &r.symbol, &r.name]),
            |r| Holding {
                id: &r.id,
                account_id: &r.account_id,
                symbol: &r.symbol,
                name: &r.name,
                currency: &r.currency,
                updated_at_ms: r.updated_at_ms,
                quantity: exact.then_some(r.quantity),
                current_value_minor: exact.then(|| r.current_value_minor.to_string()),
                cost_basis_minor: exact
                    .then_some(r.cost_basis_minor)
                    .flatten()
                    .map(|amount| amount.to_string()),
            },
            offset,
        ),
    }?;
    let mut page = describe(page, section, finance.updated_at_ms)?;
    page["amounts_redacted"] = (!exact).into();
    page["notes_excluded"] = true.into();
    Ok(page)
}

/// One Health section. `query` matches ID and activity or title, never
/// recovery notes. No diagnosis is made.
pub fn read_health(
    policy: &AgentPermissionPolicy,
    life: &LifeContext,
    section: HealthReadSection,
    query: &str,
    offset: usize,
) -> Result<Value, String> {
    let query = authorize_life_read(policy, life, AgentDomain::Health, query, offset)?;
    let mut matcher = query.matcher();
    let health = &life.health;
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
    describe(page, section, health.updated_at_ms)
}
