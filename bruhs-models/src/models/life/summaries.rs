//! The Life page's short record summaries, so both apps say the same thing.

use super::{FinanceLedger, HealthJournal, Holding};
use crate::models::format_money;

/// `count` and the noun for it: "1 budget", "2 budgets".
fn counted(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

impl FinanceLedger {
    /// The Budgets & income card: "2 budgets, 1 income stream".
    pub fn budgets_and_income_summary(&self) -> String {
        format!(
            "{}, {}",
            counted(self.budgets.len(), "budget", "budgets"),
            counted(self.income_streams.len(), "income stream", "income streams")
        )
    }
}

impl HealthJournal {
    /// The Sleep & recovery card: "3 sleep logs, 1 recovery log".
    pub fn sleep_and_recovery_summary(&self) -> String {
        format!(
            "{}, {}",
            counted(self.sleep.len(), "sleep log", "sleep logs"),
            counted(self.recovery.len(), "recovery log", "recovery logs")
        )
    }
}

impl Holding {
    /// A holding's chip: its symbol and current value, "VTI, $12.50 USD".
    pub fn label(&self) -> String {
        format!(
            "{}, {}",
            self.symbol,
            format_money(i128::from(self.current_value_minor), &self.currency)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FinanceBudget, IncomeStream, RecoveryLog, SleepLog};

    #[test]
    fn summaries_count_each_record_kind() {
        let mut finance = FinanceLedger::default();
        assert_eq!(
            finance.budgets_and_income_summary(),
            "0 budgets, 0 income streams"
        );
        finance.budgets.push(FinanceBudget::monthly("Food", 500));
        finance
            .income_streams
            .push(IncomeStream::monthly("Salary", 1));
        finance
            .income_streams
            .push(IncomeStream::monthly("Rent", 1));
        assert_eq!(
            finance.budgets_and_income_summary(),
            "1 budget, 2 income streams"
        );
        let mut health = HealthJournal::default();
        assert_eq!(
            health.sleep_and_recovery_summary(),
            "0 sleep logs, 0 recovery logs"
        );
        health.sleep.push(SleepLog::manual(480, 80, 1));
        health.sleep.push(SleepLog::manual(420, 70, 2));
        health.recovery.push(RecoveryLog::manual(64, 1));
        assert_eq!(
            health.sleep_and_recovery_summary(),
            "2 sleep logs, 1 recovery log"
        );
    }

    #[test]
    fn a_holding_chip_names_its_symbol_and_value() {
        assert_eq!(Holding::manual("vti", 1_250, 0).label(), "VTI, $12.50 USD");
        assert_eq!(
            Holding::manual("bnd", -9_007_199_254_740_993, 0).label(),
            "BND, \u{2212}$90071992547409.93 USD"
        );
    }
}
