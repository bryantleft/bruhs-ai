//! What the Life page's finance forms turn into, moved from the web Life
//! page's `entries.rs` so both apps read the same fields, check them in the
//! same order and say the same thing: each Add builds a record or says what
//! is missing. Each form's words are in [`finance_copy`].

use super::{FinanceAccount, FinanceBudget, FinanceTransaction, Holding, IncomeStream};
use crate::models::{parse_amount_minor, MoneyError};

/// Why a finance form could not be saved; the text is shown as the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FinanceEntryError {
    /// The amount is not a number with at most two decimals.
    #[error("{}", MoneyError::InvalidAmount)]
    InvalidAmount,
    /// The name field is blank; the message says which one.
    #[error("{0}")]
    Unnamed(&'static str),
}

fn amount(value: &str) -> Result<i64, FinanceEntryError> {
    parse_amount_minor(value).map_err(|_| FinanceEntryError::InvalidAmount)
}

fn named<'a>(name: &'a str, missing: &'static str) -> Result<&'a str, FinanceEntryError> {
    if name.trim().is_empty() {
        Err(FinanceEntryError::Unnamed(missing))
    } else {
        Ok(name)
    }
}

const UNNAMED_RECORD: &str = "Name the finance record first.";

/// The Accounts form. The amount is checked before the name, as the form
/// always did.
pub fn account(
    name: &str,
    balance: &str,
    now_ms: i64,
) -> Result<FinanceAccount, FinanceEntryError> {
    let balance = amount(balance)?;
    let name = named(name, "Name the account first.")?;
    Ok(FinanceAccount::manual(name, balance, now_ms))
}

/// The Transactions form: a spend is stored negative whichever sign was
/// typed, and a blank category is "Other".
pub fn spend(
    merchant: &str,
    amount_text: &str,
    category: &str,
    now_ms: i64,
) -> Result<FinanceTransaction, FinanceEntryError> {
    let value = amount(amount_text)?;
    let merchant = named(merchant, "Name the transaction first.")?;
    let category = if category.trim().is_empty() {
        "Other"
    } else {
        category
    };
    let spent = if value < 0 { value } else { -value };
    Ok(FinanceTransaction::manual(
        merchant, category, spent, now_ms,
    ))
}

/// The Budgets & income form's Budget row.
pub fn budget(category: &str, limit: &str) -> Result<FinanceBudget, FinanceEntryError> {
    let limit = amount(limit)?;
    Ok(FinanceBudget::monthly(
        named(category, UNNAMED_RECORD)?,
        limit,
    ))
}

/// The Budgets & income form's Income row.
pub fn income(name: &str, monthly: &str) -> Result<IncomeStream, FinanceEntryError> {
    let monthly = amount(monthly)?;
    Ok(IncomeStream::monthly(named(name, UNNAMED_RECORD)?, monthly))
}

/// The Holdings form.
pub fn holding(symbol: &str, value: &str, now_ms: i64) -> Result<Holding, FinanceEntryError> {
    let value = amount(value)?;
    Ok(Holding::manual(
        named(symbol, UNNAMED_RECORD)?,
        value,
        now_ms,
    ))
}

/// A finance form field's words: the caption shown above it, if any, its
/// accessible name and its placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinanceField {
    pub caption: Option<&'static str>,
    pub label: &'static str,
    pub placeholder: &'static str,
}

/// App-owned Finance copy both surfaces render, so web and desktop match word
/// for word. Follows `docs/ui-standards.md`: short sentences, no dashes.
pub mod finance_copy {
    use super::FinanceField;

    /// Above the Finance totals.
    pub const USD_NOTE: &str = "Manual amounts are USD. Other currencies are listed separately in minor units; no exchange rate is applied.";
    /// The Finance privacy card's line.
    pub const PRIVACY_NOTE: &str = "Records remain local. Agent access is off until you enable it.";

    pub const ACCOUNT_NAME: FinanceField = FinanceField {
        caption: Some("Name"),
        label: "Account name",
        placeholder: "Checking",
    };
    pub const ACCOUNT_BALANCE: FinanceField = FinanceField {
        caption: Some("Balance"),
        label: "Account balance",
        placeholder: "2500.00",
    };
    pub const ADD_ACCOUNT: &str = "Add";

    pub const MERCHANT: FinanceField = FinanceField {
        caption: Some("Merchant"),
        label: "Transaction merchant",
        placeholder: "Groceries",
    };
    pub const SPEND_AMOUNT: FinanceField = FinanceField {
        caption: Some("Spend amount"),
        label: "Transaction spend amount",
        placeholder: "42.00",
    };
    pub const SPEND_CATEGORY: FinanceField = FinanceField {
        caption: Some("Category"),
        label: "Transaction category",
        placeholder: "Food",
    };
    pub const ADD_SPEND: &str = "Add spend";

    pub const BUDGET_CATEGORY: FinanceField = FinanceField {
        caption: None,
        label: "Budget category",
        placeholder: "Food",
    };
    pub const BUDGET_LIMIT: FinanceField = FinanceField {
        caption: None,
        label: "Monthly budget limit",
        placeholder: "500",
    };
    pub const ADD_BUDGET: &str = "Budget";
    pub const INCOME_NAME: FinanceField = FinanceField {
        caption: None,
        label: "Income stream name",
        placeholder: "Salary",
    };
    pub const INCOME_AMOUNT: FinanceField = FinanceField {
        caption: None,
        label: "Expected monthly income",
        placeholder: "5000",
    };
    pub const ADD_INCOME: &str = "Income";

    pub const HOLDING_SYMBOL: FinanceField = FinanceField {
        caption: None,
        label: "Holding symbol",
        placeholder: "Symbol",
    };
    pub const HOLDING_VALUE: FinanceField = FinanceField {
        caption: None,
        label: "Holding current value",
        placeholder: "Value",
    };
    pub const ADD_HOLDING: &str = "Add";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The amount is checked before the name, as the form always did, and
    /// each message is the one the notice shows.
    #[test]
    fn finance_entries_check_the_amount_then_the_name() {
        assert_eq!(
            account("", "x", 0).unwrap_err(),
            FinanceEntryError::InvalidAmount
        );
        assert_eq!(
            account(" ", "10", 0).unwrap_err().to_string(),
            "Name the account first."
        );
        let checking = account("Checking", "2,500.00", 7).unwrap();
        assert_eq!(
            (checking.name.as_str(), checking.balance_minor),
            ("Checking", 250_000)
        );
        assert_eq!(
            budget("", "500").unwrap_err().to_string(),
            "Name the finance record first."
        );
        assert_eq!(budget("Food", "500").unwrap().monthly_limit_minor, 50_000);
        assert_eq!(
            income("Salary", "5000").unwrap().expected_monthly_minor,
            500_000
        );
        assert_eq!(
            holding("VTI", "12.5", 0).unwrap().current_value_minor,
            1_250
        );
        assert_eq!(
            FinanceEntryError::InvalidAmount.to_string(),
            MoneyError::InvalidAmount.to_string()
        );
    }

    #[test]
    fn a_spend_is_negative_and_defaults_its_category() {
        let coffee = spend("Coffee", "4.50", " ", 0).unwrap();
        assert_eq!(coffee.amount_minor, -450);
        assert_eq!(coffee.category, "Other");
        let refund = spend("Shop", "-3", "Home", 0).unwrap();
        assert_eq!(refund.amount_minor, -300);
        assert_eq!(refund.category, "Home");
        assert_eq!(
            spend("", "1", "", 0).unwrap_err().to_string(),
            "Name the transaction first."
        );
    }
}
