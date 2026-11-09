use bruhs_models::*;

#[test]
fn exact_amount_parser_preserves_every_cent_through_i64_boundaries() {
    for (input, expected) in [
        ("90071992547409.93", 9_007_199_254_740_993),
        ("92233720368547758.07", i64::MAX),
        ("-92233720368547758.08", i64::MIN),
        ("$1,234.56", 123456),
        ("-0.01", -1),
    ] {
        assert_eq!(parse_amount_minor(input), Ok(expected), "{input}");
    }
}

#[test]
fn invalid_amounts_are_rejected_without_rounding_or_saturation() {
    for input in [
        "92233720368547758.08",
        "-92233720368547758.09",
        "1e20",
        "NaN",
        "inf",
        "1.001",
        "1,2",
        "1,,000",
        "1$2",
        ".",
        "",
        "1.2.3",
        "１２",
    ] {
        assert!(parse_amount_minor(input).is_err(), "{input}");
    }
}

#[test]
fn totals_keep_currencies_separate_and_exceed_i64_without_wrapping() {
    let mut ledger = FinanceLedger::default();
    ledger
        .accounts
        .push(FinanceAccount::manual("A", i64::MAX, 0));
    ledger
        .accounts
        .push(FinanceAccount::manual("B", i64::MAX, 0));
    let mut euro = FinanceAccount::manual("Euro", 500, 0);
    euro.currency = "eur".into();
    ledger.accounts.push(euro);
    let totals = ledger.net_worth_totals().unwrap();
    assert_eq!(totals.minor("USD"), Some(2 * i128::from(i64::MAX)));
    assert_eq!(totals.minor("EUR"), Some(500));
    assert_eq!(
        totals.to_string(),
        "EUR 500 minor units · $184467440737095516.14 USD"
    );
}

#[test]
fn invalid_currency_or_impossible_sum_is_explicit_not_a_partial_total() {
    let mut totals = MoneyTotals::default();
    assert_eq!(totals.add("<USD>", 1), Err(MoneyError::InvalidCurrency));
    totals.add("USD", i128::MAX).unwrap();
    assert_eq!(totals.add("USD", 1), Err(MoneyError::Overflow));
    assert_eq!(totals.minor("USD"), Some(i128::MAX));
}

#[test]
fn spend_handles_minimum_integer_and_excludes_future_entries() {
    let mut ledger = FinanceLedger::default();
    ledger
        .transactions
        .push(FinanceTransaction::manual("past", "Food", i64::MIN, 0));
    ledger
        .transactions
        .push(FinanceTransaction::manual("future", "Food", -100, 2));
    assert_eq!(
        ledger
            .trailing_spend_totals(1, i64::MAX)
            .unwrap()
            .minor("USD"),
        Some(1_i128 << 63)
    );
    assert!(ledger
        .trailing_spend_totals(i64::MIN, i64::MAX)
        .unwrap()
        .minor("USD")
        .is_none());
}

#[test]
fn budget_evidence_only_counts_matching_currency_and_does_not_overflow() {
    let mut context = LifeContext::default();
    context
        .finance
        .budgets
        .push(FinanceBudget::monthly("Food", i64::MAX));
    let mut euro = FinanceTransaction::manual("Euro", "Food", i64::MIN, 0);
    euro.currency = "EUR".into();
    context.finance.transactions.push(euro);
    assert!(context.insights(&[], 1).is_empty());
    context
        .finance
        .transactions
        .push(FinanceTransaction::manual("USD", "Food", i64::MIN, 0));
    let insights = context.insights(&[], 1);
    assert_eq!(insights.len(), 1);
    assert!(insights[0].title.contains("100%"));
    assert!(insights[0]
        .evidence
        .contains("9223372036854775808 of 9223372036854775807 USD"));
    assert!(insights[0].evidence.contains("1 matching transaction"));
}

#[test]
fn minimum_integer_magnitudes_and_legacy_currency_defaults_round_trip() {
    let budget = FinanceBudget::monthly("Food", i64::MIN);
    let income = IncomeStream::monthly("Income", i64::MIN);
    assert_eq!(budget.monthly_limit_minor, 1_u64 << 63);
    assert_eq!(income.expected_monthly_minor, 1_u64 << 63);
    let mut value = serde_json::to_value(&budget).unwrap();
    value.as_object_mut().unwrap().remove("currency");
    let restored: FinanceBudget = serde_json::from_value(value).unwrap();
    assert_eq!(restored, budget);
}

#[test]
fn agent_context_keeps_currency_totals_and_redacts_exact_amounts() {
    let mut context = LifeContext::default();
    context
        .finance
        .accounts
        .push(FinanceAccount::manual("Private", i64::MAX, 0));
    context.finance.privacy.allow_agent_context = true;
    let redacted = life_agent_prompt(&context, &[], 1).unwrap();
    assert!(redacted.contains("[amount redacted]"));
    assert!(!redacted.contains("92233720368547758"));
    context.finance.privacy.reveal_exact_amounts_to_agent = true;
    assert!(life_agent_prompt(&context, &[], 1)
        .unwrap()
        .contains("$92233720368547758.07 USD"));
}

#[test]
fn persisted_amounts_use_exact_strings_and_accept_legacy_integers() {
    let account = FinanceAccount::manual("A", i64::MAX, 0);
    let mut json = serde_json::to_value(&account).unwrap();
    assert_eq!(json["balance_minor"], i64::MAX.to_string());
    assert_eq!(
        serde_json::from_value::<FinanceAccount>(json.clone()).unwrap(),
        account
    );
    json["balance_minor"] = serde_json::json!(i64::MAX);
    assert_eq!(
        serde_json::from_value::<FinanceAccount>(json).unwrap(),
        account
    );
    let mut holding = Holding::manual("A", i64::MIN, 0);
    holding.cost_basis_minor = Some(i64::MAX);
    assert_eq!(
        serde_json::from_str::<Holding>(&serde_json::to_string(&holding).unwrap()).unwrap(),
        holding
    );
}

#[test]
fn malformed_money_never_deserializes_as_zero_or_a_clamped_value() {
    let account = FinanceAccount::manual("A", 1, 0);
    for value in [
        serde_json::json!("9223372036854775808"),
        serde_json::json!("1e9"),
        serde_json::json!("not money"),
        serde_json::json!(1.5),
        serde_json::json!(null),
    ] {
        let mut json = serde_json::to_value(&account).unwrap();
        json["balance_minor"] = value;
        assert!(serde_json::from_value::<FinanceAccount>(json).is_err());
    }
}

#[test]
fn legacy_budget_income_and_holding_records_keep_usd_without_rewriting_values() {
    let ledger = FinanceLedger {
        budgets: vec![FinanceBudget::monthly("Food", 100)],
        income_streams: vec![IncomeStream::monthly("Pay", 200)],
        holdings: vec![Holding::manual("BRUH", 300, 0)],
        ..Default::default()
    };
    let mut json = serde_json::to_value(&ledger).unwrap();
    for (collection, field, value) in [
        ("budgets", "monthly_limit_minor", 100),
        ("income_streams", "expected_monthly_minor", 200),
        ("holdings", "current_value_minor", 300),
    ] {
        json[collection][0]
            .as_object_mut()
            .unwrap()
            .remove("currency");
        json[collection][0][field] = serde_json::json!(value);
    }
    assert_eq!(
        serde_json::from_value::<FinanceLedger>(json).unwrap(),
        ledger
    );
}

#[test]
fn totals_are_read_back_with_the_same_currency_normalization_as_they_are_added() {
    let mut totals = MoneyTotals::default();
    totals.add(" usd ", 250).unwrap();
    assert_eq!(totals.minor("usd"), Some(250));
    assert_eq!(totals.minor(" USD "), Some(250));
    assert_eq!(totals.minor("USD"), Some(250));
    assert_eq!(totals.minor("EUR"), None);
    assert_eq!(totals.minor("not-a-code"), None);
}
