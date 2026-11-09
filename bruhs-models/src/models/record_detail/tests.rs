use super::*;
use crate::{
    FinanceAccount, FinanceBudget, FinanceTransaction, Habit, Holding, IncomeStream, RecoveryLog,
    SleepLog, WorkoutLog,
};

fn fixture() -> LifeContext {
    let mut context = LifeContext::default();
    context
        .finance
        .accounts
        .push(FinanceAccount::manual("Account", i64::MIN, 11));
    context
        .finance
        .transactions
        .push(FinanceTransaction::manual(
            "Merchant",
            "Category",
            i64::MAX,
            12,
        ));
    let mut budget = FinanceBudget::monthly("Budget", 1);
    budget.monthly_limit_minor = u64::MAX;
    context.finance.budgets.push(budget);
    let mut income = IncomeStream::monthly("Income", 1);
    income.expected_monthly_minor = u64::MAX;
    context.finance.income_streams.push(income);
    context
        .finance
        .holdings
        .push(Holding::manual("Holding", i64::MIN, 13));
    context
        .health
        .workouts
        .push(WorkoutLog::manual("Workout", 37, 14));
    context.health.sleep.push(SleepLog::manual(421, 84, 15));
    context.health.recovery.push(RecoveryLog::manual(73, 16));
    context.health.habits.push(Habit::weekly("Habit", 4));
    context
}

#[test]
fn projects_every_life_record_with_exact_values_without_mutating_sources() {
    let context = fixture();
    let before = context.clone();
    let records = [
        (
            LifeSection::Finance,
            &context.finance.accounts[0].id,
            "Account",
            "Balance",
            "−$92233720368547758.08 USD",
        ),
        (
            LifeSection::Finance,
            &context.finance.transactions[0].id,
            "Transaction",
            "Amount",
            "$92233720368547758.07 USD",
        ),
        (
            LifeSection::Finance,
            &context.finance.budgets[0].id,
            "Monthly budget",
            "Limit",
            "$184467440737095516.15 USD",
        ),
        (
            LifeSection::Finance,
            &context.finance.income_streams[0].id,
            "Income stream",
            "Expected monthly",
            "$184467440737095516.15 USD",
        ),
        (
            LifeSection::Finance,
            &context.finance.holdings[0].id,
            "Holding",
            "Value",
            "−$92233720368547758.08 USD",
        ),
        (
            LifeSection::Health,
            &context.health.workouts[0].id,
            "Workout",
            "Duration",
            "37 minutes",
        ),
        (
            LifeSection::Health,
            &context.health.sleep[0].id,
            "Sleep",
            "Duration",
            "421 minutes",
        ),
        (
            LifeSection::Health,
            &context.health.recovery[0].id,
            "Recovery",
            "Score",
            "73%",
        ),
        (
            LifeSection::Health,
            &context.health.habits[0].id,
            "Habit",
            "Target",
            "4 days per week",
        ),
    ];
    for (section, id, kind, label, expected) in records {
        let detail = life_record_detail(
            &context,
            &SelectedLifeRecord {
                section,
                id: id.clone(),
            },
            |at| format!("Date {at}"),
        )
        .expect("selected record");
        assert_eq!(detail.kind, kind);
        assert!(
            detail
                .fields
                .iter()
                .any(|(key, value)| (*key, value.as_str()) == (label, expected)),
            "{kind}: {:?}",
            detail.fields
        );
    }
    assert_eq!(context, before);
}

#[test]
fn missing_and_wrong_domain_ids_do_not_select_a_different_record() {
    let context = fixture();
    for (section, id) in [
        (LifeSection::Finance, "missing"),
        (LifeSection::Health, "missing"),
        (LifeSection::Finance, context.health.workouts[0].id.as_str()),
        (LifeSection::Health, context.finance.accounts[0].id.as_str()),
    ] {
        assert!(life_record_detail(
            &context,
            &SelectedLifeRecord {
                section,
                id: id.into()
            },
            |at| at.to_string()
        )
        .is_none());
    }
}

#[test]
fn historical_records_beyond_the_overview_limit_keep_their_own_dates_and_notes() {
    let mut context = fixture();
    context.finance.transactions = (0..100)
        .map(|i| {
            let mut record = FinanceTransaction::manual("Repeated merchant", "Category", -i, i);
            record.note = Some(format!("Receipt {i}"));
            record
        })
        .collect();
    let selected = SelectedLifeRecord {
        section: LifeSection::Finance,
        id: context.finance.transactions[99].id.clone(),
    };
    let detail =
        life_record_detail(&context, &selected, |at| format!("Date {at}")).expect("old record");
    assert!(detail.fields.contains(&("Date", "Date 99".into())));
    assert!(detail.fields.contains(&("Note", "Receipt 99".into())));
}
