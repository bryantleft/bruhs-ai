import datetime
import unittest

from ledger.accounts import Ledger
from ledger.money import parse_amount
from ledger.report import format_cents, statement_lines

D = datetime.date


class HiddenMoneyTest(unittest.TestCase):
    def test_docstring_examples(self):
        cases = {
            "12.34": 1234, "7": 700, "1.5": 150, ".25": 25, "+3.25": 325,
            "-0.50": -50, "-12.3": -1230, "1,234.56": 123456, " 4 ": 400,
            "-0.05": -5, "-3": -300, "0.07": 7,
        }
        for text, cents in cases.items():
            with self.subTest(text=text):
                self.assertEqual(parse_amount(text), cents)

    def test_rejects(self):
        for text in ("", "   ", "1.234", "abc", "1.x"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                parse_amount(text)


class HiddenFormatTest(unittest.TestCase):
    def test_docstring_examples(self):
        cases = {123456: "1,234.56", 5: "0.05", 0: "0.00", -1234: "-12.34",
                 -5: "-0.05", -100: "-1.00", -123456: "-1,234.56"}
        for cents, text in cases.items():
            with self.subTest(cents=cents):
                self.assertEqual(format_cents(cents), text)


class HiddenLedgerTest(unittest.TestCase):
    def setUp(self):
        self.ledger = Ledger()
        self.ledger.post(D(2026, 3, 1), "cash", "100.00", "salary")
        self.ledger.post(D(2026, 3, 31), "cash", "-20.00", "zebra fee")
        self.ledger.post(D(2026, 3, 31), "cash", "-0.50", "atm refund reversal")

    def test_balance_as_of_is_inclusive(self):
        self.assertEqual(self.ledger.balance("cash", D(2026, 3, 31)), 7950)
        self.assertEqual(self.ledger.balance("cash", D(2026, 3, 30)), 10000)
        self.assertEqual(self.ledger.balance("cash", D(2026, 3, 1)), 10000)
        self.assertEqual(self.ledger.balance("cash", D(2026, 2, 28)), 0)
        self.assertEqual(self.ledger.balance("cash"), 7950)

    def test_same_day_entries_keep_posting_order(self):
        memos = [e.memo for e in self.ledger.entries_between("cash", D(2026, 3, 1), D(2026, 3, 31))]
        self.assertEqual(memos, ["salary", "zebra fee", "atm refund reversal"])

    def test_entries_sorted_by_date(self):
        self.ledger.post(D(2026, 3, 15), "cash", 1, "late booking")
        days = [e.day for e in self.ledger.entries_between("cash", D(2026, 3, 1), D(2026, 3, 31))]
        self.assertEqual(days, sorted(days))
        self.assertEqual(len(days), 4)

    def test_transfer(self):
        self.ledger.transfer(D(2026, 3, 5), "cash", "savings", "10.5", "move")
        self.assertEqual(self.ledger.balance("savings", D(2026, 3, 5)), 1050)
        self.assertEqual(self.ledger.balance("cash", D(2026, 3, 5)), 8950)

    def test_statement(self):
        self.ledger.post(D(2026, 2, 28), "cash", "5", "opening")
        lines = statement_lines(self.ledger, "cash", D(2026, 3, 1), D(2026, 3, 31))
        self.assertEqual(lines, [
            "Opening balance  5.00",
            "2026-03-01  salary  100.00  105.00",
            "2026-03-31  zebra fee  -20.00  85.00",
            "2026-03-31  atm refund reversal  -0.50  84.50",
            "Closing balance  84.50",
        ])

    def test_statement_negative_running_balance(self):
        ledger = Ledger()
        ledger.post(D(2026, 4, 2), "card", "-0.05", "fee")
        lines = statement_lines(ledger, "card", D(2026, 4, 1), D(2026, 4, 2))
        self.assertEqual(lines[1], "2026-04-02  fee  -0.05  -0.05")
        self.assertEqual(lines[2], "Closing balance  -0.05")


if __name__ == "__main__":
    unittest.main()
