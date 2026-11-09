import unittest
from decimal import Decimal

from fees import late_fee


class FeesTest(unittest.TestCase):
    def test_active(self):
        self.assertEqual(late_fee("100.00", "active"), Decimal("5.00"))
        self.assertEqual(late_fee("10.00", "active"), Decimal("2.00"))
        self.assertEqual(late_fee("5000", "active"), Decimal("40.00"))

    def test_closed(self):
        self.assertEqual(late_fee("100.00", "closed"), Decimal("0.00"))

    def test_unknown_status(self):
        with self.assertRaises(ValueError):
            late_fee("1", "frozen")


if __name__ == "__main__":
    unittest.main()
