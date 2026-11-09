//! Exact local money arithmetic. Never infer exchange rates or currency scales.
use std::{collections::BTreeMap, fmt};

pub(crate) mod serde_minor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoneyError {
    InvalidAmount,
    InvalidCurrency,
    Overflow,
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidAmount => {
                "Enter an amount with at most two decimal places, within the supported range."
            }
            Self::InvalidCurrency => "Amount unavailable: a record has an invalid currency code.",
            Self::Overflow => "Amount unavailable: the total exceeds the supported range.",
        })
    }
}
impl std::error::Error for MoneyError {}

/// Parse a USD amount without rounding, float conversion, or saturating casts.
pub fn parse_amount_minor(value: &str) -> Result<i64, MoneyError> {
    let value = value.trim();
    if value.len() > 64 {
        return Err(MoneyError::InvalidAmount);
    }
    let (negative, value) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let value = value.strip_prefix('$').unwrap_or(value);
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if fractional.len() > 2 || !fractional.bytes().all(|b| b.is_ascii_digit()) {
        return Err(MoneyError::InvalidAmount);
    }
    let groups: Vec<_> = whole.split(',').collect();
    if groups.len() > 1
        && (groups[0].is_empty() || groups[0].len() > 3 || groups[1..].iter().any(|g| g.len() != 3))
    {
        return Err(MoneyError::InvalidAmount);
    }
    let whole = groups.concat();
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(MoneyError::InvalidAmount);
    }
    let whole: i128 = whole.parse().map_err(|_| MoneyError::InvalidAmount)?;
    let fraction: i128 = if fractional.is_empty() {
        0
    } else {
        fractional.parse().map_err(|_| MoneyError::InvalidAmount)?
    };
    let minor = whole
        .checked_mul(100)
        .and_then(|v| v.checked_add(fraction * if fractional.len() == 1 { 10 } else { 1 }))
        .ok_or(MoneyError::InvalidAmount)?;
    i64::try_from(if negative { -minor } else { minor }).map_err(|_| MoneyError::InvalidAmount)
}

pub fn normalized_currency(value: &str) -> Result<String, MoneyError> {
    let value = value.trim();
    if value.len() != 3 || !value.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(MoneyError::InvalidCurrency);
    }
    Ok(value.to_ascii_uppercase())
}

pub fn format_money(minor: i128, currency: &str) -> String {
    let Ok(currency) = normalized_currency(currency) else {
        return MoneyError::InvalidCurrency.to_string();
    };
    // Only USD input currently has a defined two-decimal contract. Unknown
    // scales stay explicitly in minor units, never silently divided by 100.
    if currency != "USD" {
        return format!("{currency} {minor} minor units");
    }
    let absolute = minor.unsigned_abs();
    let sign = if minor < 0 { "−" } else { "" };
    format!("{sign}${}.{:02} USD", absolute / 100, absolute % 100)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoneyTotals(BTreeMap<String, i128>);

impl MoneyTotals {
    pub fn add(&mut self, currency: &str, minor: i128) -> Result<(), MoneyError> {
        let entry = self.0.entry(normalized_currency(currency)?).or_default();
        *entry = entry.checked_add(minor).ok_or(MoneyError::Overflow)?;
        Ok(())
    }
    /// The total for `currency`, normalized the way [`MoneyTotals::add`]
    /// normalizes it (`" usd "` reads the `USD` total).
    pub fn minor(&self, currency: &str) -> Option<i128> {
        // An invalid code was never added, so it has no total.
        let currency = normalized_currency(currency).ok()?;
        self.0.get(&currency).copied()
    }
}

impl fmt::Display for MoneyTotals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("No entries");
        }
        for (index, (currency, minor)) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(" · ")?;
            }
            f.write_str(&format_money(*minor, currency))?;
        }
        Ok(())
    }
}

pub fn format_money_totals(totals: Result<MoneyTotals, MoneyError>) -> String {
    match totals {
        Ok(totals) => totals.to_string(),
        Err(error) => error.to_string(),
    }
}
