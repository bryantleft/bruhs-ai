//! What the Life page's forms turn into: each "Add" reads the typed text
//! and either builds a record or says what is missing. Pure, so the parsing
//! is tested on the host.

use bruhs_models::num::ToIntSaturating;
use bruhs_models::{
    FinanceAccount, FinanceBudget, FinanceTransaction, Habit, Holding, IncomeStream, MoneyError,
    RecoveryLog, SleepLog, WorkoutLog,
};

/// Why a finance form could not be saved; the text is shown as the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    /// The amount is not a number with at most two decimals.
    #[error("{}", MoneyError::InvalidAmount)]
    InvalidAmount,
    /// The name field is blank; the message says which one.
    #[error("{0}")]
    Unnamed(&'static str),
}

/// Minor units (cents) of a typed amount like `$1,234.56` or `-5.25`.
pub fn parse_amount_minor(value: &str) -> Option<i64> {
    bruhs_models::parse_amount_minor(value).ok()
}

fn amount(value: &str) -> Result<i64, EntryError> {
    parse_amount_minor(value).ok_or(EntryError::InvalidAmount)
}

fn named(name: String, missing: &'static str) -> Result<String, EntryError> {
    if name.trim().is_empty() {
        Err(EntryError::Unnamed(missing))
    } else {
        Ok(name)
    }
}

const UNNAMED_RECORD: &str = "Name the finance record first.";

pub fn account(name: String, balance: &str, now_ms: i64) -> Result<FinanceAccount, EntryError> {
    let balance = amount(balance)?;
    let name = named(name, "Name the account first.")?;
    Ok(FinanceAccount::manual(name, balance, now_ms))
}

/// A spend: the amount is stored negative whichever sign was typed; a blank
/// category is "Other".
pub fn spend(
    merchant: String,
    amount_text: &str,
    category: String,
    now_ms: i64,
) -> Result<FinanceTransaction, EntryError> {
    let value = amount(amount_text)?;
    let merchant = named(merchant, "Name the transaction first.")?;
    let category = if category.trim().is_empty() {
        "Other".to_string()
    } else {
        category
    };
    let spent = if value < 0 { value } else { -value };
    Ok(FinanceTransaction::manual(
        merchant, category, spent, now_ms,
    ))
}

pub fn budget(category: String, limit: &str) -> Result<FinanceBudget, EntryError> {
    let limit = amount(limit)?;
    Ok(FinanceBudget::monthly(
        named(category, UNNAMED_RECORD)?,
        limit,
    ))
}

pub fn income(name: String, monthly: &str) -> Result<IncomeStream, EntryError> {
    let monthly = amount(monthly)?;
    Ok(IncomeStream::monthly(named(name, UNNAMED_RECORD)?, monthly))
}

pub fn holding(symbol: String, value: &str, now_ms: i64) -> Result<Holding, EntryError> {
    let value = amount(value)?;
    Ok(Holding::manual(
        named(symbol, UNNAMED_RECORD)?,
        value,
        now_ms,
    ))
}

/// Pasted CSV rows `merchant, category, signed amount[, note]`, at most
/// 2,000; a header row and rows without a valid amount are skipped.
pub fn csv_transactions(value: &str, now_ms: i64) -> Vec<FinanceTransaction> {
    value
        .lines()
        .take(2_000)
        .filter_map(|line| {
            let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
            if fields.len() < 3 || fields[0].eq_ignore_ascii_case("merchant") {
                return None;
            }
            let amount = parse_amount_minor(fields[2])?;
            let mut transaction = FinanceTransaction::manual(
                fields[0],
                fields.get(1).copied().unwrap_or("Other"),
                amount,
                now_ms,
            );
            transaction.note = fields.get(3).map(|note| (*note).to_string());
            Some(transaction)
        })
        .collect()
}

/// A workout needs an activity and whole minutes (0–65535).
pub fn workout(activity: String, minutes: &str, now_ms: i64) -> Option<WorkoutLog> {
    let minutes = minutes.parse::<u16>().ok()?;
    (!activity.trim().is_empty()).then(|| WorkoutLog::manual(activity, minutes, now_ms))
}

/// Sleep needs positive hours and a quality percent (0–255, as typed).
pub fn sleep(hours: &str, quality: &str, now_ms: i64) -> Option<SleepLog> {
    let hours = hours.parse::<f64>().ok()?;
    let quality = quality.parse::<u8>().ok()?;
    (hours.is_finite() && hours > 0.0)
        .then(|| SleepLog::manual((hours * 60.0).round().to_u16_saturating(), quality, now_ms))
}

pub fn recovery(score: &str, now_ms: i64) -> Option<RecoveryLog> {
    Some(RecoveryLog::manual(score.parse::<u8>().ok()?, now_ms))
}

/// A habit with a weekly target of five days.
pub fn habit(title: String) -> Option<Habit> {
    (!title.trim().is_empty()).then(|| Habit::weekly(title, 5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_parser_handles_currency_and_rejects_non_numbers() {
        assert_eq!(parse_amount_minor("$1,234.56"), Some(123_456));
        assert_eq!(parse_amount_minor("-5.25"), Some(-525));
        assert_eq!(parse_amount_minor("nope"), None);
    }

    #[test]
    fn csv_import_is_bounded_and_skips_headers() {
        let rows = csv_transactions(
            "merchant,category,amount,note\nCoffee,Food,-4.50,Meeting\nBad,row,nope",
            10,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].amount_minor, -450);
        assert_eq!(rows[0].note.as_deref(), Some("Meeting"));
        let many = "a,b,1\n".repeat(2_500);
        assert_eq!(csv_transactions(&many, 0).len(), 2_000);
    }

    /// The amount is checked before the name, as the form always did, and
    /// each message is the one the notice shows.
    #[test]
    fn finance_entries_check_the_amount_then_the_name() {
        assert_eq!(
            account(String::new(), "x", 0).unwrap_err(),
            EntryError::InvalidAmount
        );
        assert_eq!(
            account(" ".into(), "10", 0).unwrap_err().to_string(),
            "Name the account first."
        );
        let checking = account("Checking".into(), "2,500.00", 7).unwrap();
        assert_eq!(
            (checking.name.as_str(), checking.balance_minor),
            ("Checking", 250_000)
        );
        assert_eq!(
            budget(String::new(), "500").unwrap_err().to_string(),
            "Name the finance record first."
        );
        assert_eq!(
            budget("Food".into(), "500").unwrap().monthly_limit_minor,
            50_000
        );
        assert_eq!(
            income("Salary".into(), "5000")
                .unwrap()
                .expected_monthly_minor,
            500_000
        );
        assert_eq!(
            holding("VTI".into(), "12.5", 0)
                .unwrap()
                .current_value_minor,
            1_250
        );
        assert_eq!(
            EntryError::InvalidAmount.to_string(),
            MoneyError::InvalidAmount.to_string()
        );
    }

    #[test]
    fn a_spend_is_negative_and_defaults_its_category() {
        let coffee = spend("Coffee".into(), "4.50", " ".into(), 0).unwrap();
        assert_eq!(coffee.amount_minor, -450);
        assert_eq!(coffee.category, "Other");
        let refund = spend("Shop".into(), "-3", "Home".into(), 0).unwrap();
        assert_eq!(refund.amount_minor, -300);
        assert_eq!(refund.category, "Home");
        assert_eq!(
            spend(String::new(), "1", String::new(), 0)
                .unwrap_err()
                .to_string(),
            "Name the transaction first."
        );
    }

    #[test]
    fn health_entries_need_whole_numbers_and_a_name() {
        assert_eq!(workout("Run".into(), "30", 0).unwrap().duration_minutes, 30);
        assert!(workout("Run".into(), "30.5", 0).is_none());
        assert!(workout(" ".into(), "30", 0).is_none());
        let night = sleep("7.5", "80", 0).unwrap();
        assert_eq!((night.duration_minutes, night.quality_percent), (450, 80));
        assert!(sleep("0", "80", 0).is_none());
        assert!(sleep("inf", "80", 0).is_none());
        assert!(sleep("7", "300", 0).is_none());
        assert_eq!(recovery("64", 0).unwrap().score_percent, 64);
        assert!(recovery("-1", 0).is_none());
        assert!(habit("Walk".into()).is_some());
        assert!(habit("  ".into()).is_none());
    }
}
