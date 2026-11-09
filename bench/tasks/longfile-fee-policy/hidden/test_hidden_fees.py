import unittest
from decimal import Decimal

from fees import late_fee

D = Decimal


class HiddenFeesTest(unittest.TestCase):
    def test_grace_pays_half_rounded_down(self):
        self.assertEqual(late_fee("100.00", "grace"), D("2.50"))
        # 5% of 123.45 = 6.17 (half up 6.1725) -> half 3.085 -> down 3.08
        self.assertEqual(late_fee("123.45", "grace"), D("3.08"))
        # minimum applies before halving
        self.assertEqual(late_fee("10.00", "grace"), D("1.00"))
        self.assertEqual(late_fee("3.00", "grace"), D("1.00"))

    def test_grace_cap_from_amendment(self):
        # 5% of 400 = 20.00 -> 10.00, under the cap
        self.assertEqual(late_fee("400", "grace"), D("10.00"))
        # 5% of 600 = 30.00 -> 15.00, capped at 12.50
        self.assertEqual(late_fee("600", "grace"), D("12.50"))
        self.assertEqual(late_fee("5000", "grace"), D("12.50"))

    def test_write_off_applies_to_grace(self):
        self.assertEqual(late_fee("0.99", "grace"), D("0.00"))

    def test_other_statuses_unchanged(self):
        for status in ("active", "suspended"):
            self.assertEqual(late_fee("100.00", status), D("5.00"))
            self.assertEqual(late_fee("10.00", status), D("2.00"))
            self.assertEqual(late_fee("5000", status), D("40.00"))
            self.assertEqual(late_fee("0.50", status), D("0.00"))
        self.assertEqual(late_fee("600", "closed"), D("0.00"))

    def test_returns_decimal_with_cents(self):
        fee = late_fee("123.45", "grace")
        self.assertIsInstance(fee, Decimal)
        self.assertEqual(fee.as_tuple().exponent, -2)


if __name__ == "__main__":
    unittest.main()
