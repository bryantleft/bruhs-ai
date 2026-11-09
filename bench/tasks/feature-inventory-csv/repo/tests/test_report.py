import unittest
from decimal import Decimal

from inventory.models import Item
from inventory.report import low_stock, text_report


ITEMS = [
    Item("B-200", "bolts", 12, Decimal("4.25")),
    Item("A-100", "anchor", 3, Decimal("19.99")),
]


class ReportTest(unittest.TestCase):
    def test_text_report_sorted_by_name(self):
        lines = text_report(ITEMS).splitlines()
        self.assertTrue(lines[0].startswith("A-100"))
        self.assertTrue(lines[1].startswith("B-200"))

    def test_low_stock(self):
        self.assertEqual([item.sku for item in low_stock(ITEMS)], ["A-100"])


if __name__ == "__main__":
    unittest.main()
