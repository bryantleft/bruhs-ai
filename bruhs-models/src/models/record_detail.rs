//! Read-only selected-record projections shared by web and native surfaces.

use super::{format_money, LifeContext, LifeSection};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedLifeRecord {
    pub section: LifeSection,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordDetail {
    pub title: String,
    pub kind: &'static str,
    pub fields: Vec<(&'static str, String)>,
}

fn optional(value: Option<&str>) -> String {
    value
        .filter(|value| !value.is_empty())
        .unwrap_or("Not set")
        .into()
}

// Only the selected record is projected; the rest of the ledger stays borrowed.
pub fn life_record_detail(
    context: &LifeContext,
    selected: &SelectedLifeRecord,
    date: impl Fn(i64) -> String,
) -> Option<RecordDetail> {
    let id = &selected.id;
    let (title, kind, fields) = match selected.section {
        LifeSection::Finance => {
            let finance = &context.finance;
            if let Some(r) = finance.accounts.iter().find(|r| &r.id == id) {
                (
                    r.name.clone(),
                    "Account",
                    vec![
                        ("Type", r.kind.label().into()),
                        (
                            "Balance",
                            format_money(i128::from(r.balance_minor), &r.currency),
                        ),
                        ("Institution", optional(r.institution.as_deref())),
                        ("Updated", date(r.updated_at_ms)),
                    ],
                )
            } else if let Some(r) = finance.transactions.iter().find(|r| &r.id == id) {
                (
                    r.merchant.clone(),
                    "Transaction",
                    vec![
                        (
                            "Amount",
                            format_money(i128::from(r.amount_minor), &r.currency),
                        ),
                        ("Category", r.category.clone()),
                        ("Date", date(r.occurred_at_ms)),
                        ("Account", optional(r.account_id.as_deref())),
                        ("Note", optional(r.note.as_deref())),
                    ],
                )
            } else if let Some(r) = finance.budgets.iter().find(|r| &r.id == id) {
                (
                    r.category.clone(),
                    "Monthly budget",
                    vec![
                        (
                            "Limit",
                            format_money(i128::from(r.monthly_limit_minor), &r.currency),
                        ),
                        ("Alert threshold", format!("{}%", r.alert_percent)),
                        (
                            "Status",
                            if r.enabled { "Enabled" } else { "Disabled" }.into(),
                        ),
                    ],
                )
            } else if let Some(r) = finance.income_streams.iter().find(|r| &r.id == id) {
                (
                    r.name.clone(),
                    "Income stream",
                    vec![
                        (
                            "Expected monthly",
                            format_money(i128::from(r.expected_monthly_minor), &r.currency),
                        ),
                        (
                            "Status",
                            if r.active { "Active" } else { "Inactive" }.into(),
                        ),
                    ],
                )
            } else if let Some(r) = finance.holdings.iter().find(|r| &r.id == id) {
                (
                    r.symbol.clone(),
                    "Holding",
                    vec![
                        ("Name", r.name.clone()),
                        ("Quantity", r.quantity.to_string()),
                        (
                            "Value",
                            format_money(i128::from(r.current_value_minor), &r.currency),
                        ),
                        (
                            "Cost basis",
                            r.cost_basis_minor.map_or_else(
                                || "Not set".into(),
                                |v| format_money(i128::from(v), &r.currency),
                            ),
                        ),
                        ("Account", optional(r.account_id.as_deref())),
                        ("Updated", date(r.updated_at_ms)),
                    ],
                )
            } else {
                return None;
            }
        }
        LifeSection::Health => {
            let health = &context.health;
            if let Some(r) = health.workouts.iter().find(|r| &r.id == id) {
                (
                    r.activity.clone(),
                    "Workout",
                    vec![
                        ("Duration", format!("{} minutes", r.duration_minutes)),
                        (
                            "Perceived exertion",
                            format!("{} / 10", r.perceived_exertion),
                        ),
                        ("Date", date(r.occurred_at_ms)),
                    ],
                )
            } else if let Some(r) = health.sleep.iter().find(|r| &r.id == id) {
                (
                    "Sleep log".into(),
                    "Sleep",
                    vec![
                        ("Duration", format!("{} minutes", r.duration_minutes)),
                        ("Quality", format!("{}%", r.quality_percent)),
                        ("Ended", date(r.ended_at_ms)),
                    ],
                )
            } else if let Some(r) = health.recovery.iter().find(|r| &r.id == id) {
                (
                    "Recovery log".into(),
                    "Recovery",
                    vec![
                        ("Score", format!("{}%", r.score_percent)),
                        (
                            "Resting heart rate",
                            r.resting_heart_rate
                                .map_or_else(|| "Not set".into(), |v| format!("{v} bpm")),
                        ),
                        ("Recorded", date(r.recorded_at_ms)),
                        ("Note", optional(r.note.as_deref())),
                    ],
                )
            } else if let Some(r) = health.habits.iter().find(|r| &r.id == id) {
                (
                    r.title.clone(),
                    "Habit",
                    vec![
                        (
                            "Target",
                            format!("{} days per week", r.target_days_per_week),
                        ),
                        ("Completed days", r.completion_days.len().to_string()),
                        (
                            "Status",
                            if r.archived { "Archived" } else { "Active" }.into(),
                        ),
                    ],
                )
            } else {
                return None;
            }
        }
    };
    Some(RecordDetail {
        title,
        kind,
        fields,
    })
}
