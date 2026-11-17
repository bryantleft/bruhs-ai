//! Pasted finance CSV rows, moved from the web Life page's `entries.rs` so
//! both apps import the same rows and say the same thing about them.

use super::FinanceTransaction;
use crate::models::parse_amount_minor;

/// The most rows one paste imports.
pub const CSV_IMPORT_ROW_LIMIT: usize = 2_000;

/// Pasted CSV rows `merchant, category, signed amount[, note]`, at most
/// [`CSV_IMPORT_ROW_LIMIT`]; a header row and rows without a valid amount
/// are skipped. Each becomes a manual transaction stamped `now_ms`.
pub fn csv_transactions(value: &str, now_ms: i64) -> Vec<FinanceTransaction> {
    value
        .lines()
        .take(CSV_IMPORT_ROW_LIMIT)
        .filter_map(|line| {
            let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
            if fields.len() < 3 || fields[0].eq_ignore_ascii_case("merchant") {
                return None;
            }
            let amount = parse_amount_minor(fields[2]).ok()?;
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

/// What the Life page says after importing `count` rows.
pub fn csv_import_notice(count: usize) -> String {
    match count {
        0 => "No valid CSV rows found.".to_string(),
        1 => "Imported 1 transaction locally.".to_string(),
        count => format!("Imported {count} transactions locally."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_import_is_bounded_and_skips_headers() {
        let rows = csv_transactions(
            "merchant,category,amount,note\nCoffee,Food,-4.50,Meeting\nBad,row,nope",
            10,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].amount_minor, -450);
        assert_eq!(rows[0].note.as_deref(), Some("Meeting"));
        assert_eq!(rows[0].occurred_at_ms, 10);
        let many = "a,b,1\n".repeat(2_500);
        assert_eq!(csv_transactions(&many, 0).len(), CSV_IMPORT_ROW_LIMIT);
    }

    #[test]
    fn csv_rows_keep_their_order_sign_and_trimmed_fields() {
        let rows = csv_transactions(" Shop , Home , $1,234.56\nRefund, Food, +3\nShort,1", 0);
        // A comma inside an amount splits the field, so the first row is
        // read as amount "$1" with "234.56" as its note.
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].merchant.as_str(), rows[0].category.as_str()),
            ("Shop", "Home")
        );
        assert_eq!(rows[0].amount_minor, 100);
        assert_eq!(rows[0].note.as_deref(), Some("234.56"));
        assert_eq!(rows[1].amount_minor, 300);
        assert_eq!(rows[1].note, None);
    }

    #[test]
    fn the_notice_counts_what_was_imported() {
        assert_eq!(csv_import_notice(0), "No valid CSV rows found.");
        assert_eq!(csv_import_notice(1), "Imported 1 transaction locally.");
        assert_eq!(csv_import_notice(3), "Imported 3 transactions locally.");
    }
}
