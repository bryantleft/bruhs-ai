use super::*;
use bruhs_models::{AutomationState, FinanceTransaction, LifeContext, WorkflowOperation};

#[test]
fn normal_metadata_and_exact_money_strings_use_the_batch_path() {
    let mut life = LifeContext::default();
    life.finance
        .transactions
        .push(FinanceTransaction::manual("Exact", "Manual", i64::MIN, 42));
    assert!(life.serialize(JsonSafe).is_ok());
    assert!(AutomationState::default().serialize(JsonSafe).is_ok());
    assert!(WorkflowOperation::AddFinanceTransaction {
        merchant: "Exact".into(),
        category: "Manual".into(),
        amount_minor: i64::MAX
    }
    .serialize(JsonSafe)
    .is_ok());
}

#[test]
fn nested_unsafe_integer_requires_the_existing_codec() {
    assert!(Some(vec![9_007_199_254_740_992_i64])
        .serialize(JsonSafe)
        .is_err());
    assert!([i64::MIN].serialize(JsonSafe).is_err());
    assert!([u64::MAX].serialize(JsonSafe).is_err());
    assert!([9_007_199_254_740_991_i64, -9_007_199_254_740_991]
        .serialize(JsonSafe)
        .is_ok());
}

#[test]
fn special_numeric_representations_keep_the_structured_codec() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(Some(value).serialize(JsonSafe).is_err());
    }
    assert!(0.1_f32.serialize(JsonSafe).is_err());
    assert!(1_i128.serialize(JsonSafe).is_err());
    assert!(1_u128.serialize(JsonSafe).is_err());
    assert!([0.1_f64, -0.0, f64::MAX].serialize(JsonSafe).is_ok());
}

#[test]
fn metadata_timestamps_cannot_silently_round() {
    let mut life = LifeContext::default();
    life.health.updated_at_ms = 9_007_199_254_740_992;
    assert!(life.serialize(JsonSafe).is_err());
}

#[test]
fn map_keys_keep_the_existing_string_only_contract() {
    assert!(std::collections::BTreeMap::from([(1_u32, "value")])
        .serialize(JsonSafe)
        .is_err());
    assert!(std::collections::BTreeMap::from([("key", "value")])
        .serialize(JsonSafe)
        .is_ok());
}

#[test]
fn float_roundtrip_keeps_finite_binary_values_exact() {
    for bits in [
        1_u64,
        0x3fb999999999999a,
        0x8000000000000000,
        0x7fefffffffffffff,
        0x0010000000000001,
        0x3ff199999999999a,
    ] {
        let value = f64::from_bits(bits);
        let decoded: f64 = serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
        assert_eq!(decoded.to_bits(), bits);
    }
}
