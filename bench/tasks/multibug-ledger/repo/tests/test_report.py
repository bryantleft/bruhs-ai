import datetime
import unittest

from ledger.accounts import Ledger
from ledger.report import format_cents, statement_lines


class ReportTest(unittest.TestCase):
    def test_format_positive(self):
        self.assertEqual(format_cents(123456), "1,234.56")
        self.assertEqual(format_cents(5), "0.05")

    def test_statement_shape(self):
        ledger = Ledger()
        ledger.post(datetime.date(2026, 3, 2), "cash", "100.00", "deposit")
        lines = statement_lines(ledger, "cash", datetime.date(2026, 3, 1), datetime.date(2026, 3, 10))
        self.assertEqual(lines[0], "Opening balance  0.00")
        self.assertEqual(lines[1], "2026-03-02  deposit  100.00  100.00")


if __name__ == "__main__":
    unittest.main()
