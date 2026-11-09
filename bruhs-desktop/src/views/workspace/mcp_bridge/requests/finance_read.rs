use super::{life_read, read_page::read_page, *};
use bruhs_core::workspace_bridge::FinanceReadSection;
use bruhs_models::FinanceAccountKind;

// Explicit projections prevent newly added private model fields leaking by
// default. Amounts remain exact across JSON clients, including values > 2^53.
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

impl Workspace {
    pub(super) fn read_bridge_finance(
        &self,
        section: FinanceReadSection,
        query: &str,
        offset: usize,
    ) -> Result<Value, String> {
        let query = self.authorize_life_read(AgentDomain::Finance, query, offset)?;
        let finance = &self.life_context.finance;
        let exact = finance.privacy.reveal_exact_amounts_to_agent;
        let page = match section {
            FinanceReadSection::Accounts => read_page(
                finance
                    .accounts
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id, &r.name]))
                    .map(|r| Account {
                        id: &r.id,
                        name: &r.name,
                        kind: r.kind,
                        currency: &r.currency,
                        institution: &r.institution,
                        updated_at_ms: r.updated_at_ms,
                        balance_minor: exact.then(|| r.balance_minor.to_string()),
                    }),
                offset,
            ),
            FinanceReadSection::Transactions => read_page(
                finance
                    .transactions
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id, &r.merchant, &r.category]))
                    .map(|r| Transaction {
                        id: &r.id,
                        account_id: &r.account_id,
                        merchant: &r.merchant,
                        category: &r.category,
                        currency: &r.currency,
                        occurred_at_ms: r.occurred_at_ms,
                        amount_minor: exact.then(|| r.amount_minor.to_string()),
                    }),
                offset,
            ),
            FinanceReadSection::Budgets => read_page(
                finance
                    .budgets
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id, &r.category]))
                    .map(|r| Budget {
                        id: &r.id,
                        category: &r.category,
                        currency: &r.currency,
                        alert_percent: r.alert_percent,
                        enabled: r.enabled,
                        monthly_limit_minor: exact.then(|| r.monthly_limit_minor.to_string()),
                    }),
                offset,
            ),
            FinanceReadSection::IncomeStreams => read_page(
                finance
                    .income_streams
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id, &r.name]))
                    .map(|r| Income {
                        id: &r.id,
                        name: &r.name,
                        currency: &r.currency,
                        active: r.active,
                        expected_monthly_minor: exact.then(|| r.expected_monthly_minor.to_string()),
                    }),
                offset,
            ),
            FinanceReadSection::Holdings => read_page(
                finance
                    .holdings
                    .iter()
                    .filter(|r| life_read::matches(&query, &[&r.id, &r.symbol, &r.name]))
                    .map(|r| Holding {
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
                    }),
                offset,
            ),
        }?;
        let mut page = life_read::describe(page, section, finance.updated_at_ms)?;
        page["amounts_redacted"] = (!exact).into();
        page["notes_excluded"] = true.into();
        Ok(page)
    }
}
