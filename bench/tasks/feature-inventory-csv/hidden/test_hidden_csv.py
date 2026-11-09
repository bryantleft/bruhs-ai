import contextlib
import csv
import io
import os
import unittest
from decimal import Decimal

from inventory import cli
from inventory.models import Item
from inventory.report import csv_report


class CsvTest(unittest.TestCase):
    def test_total_value(self):
        self.assertEqual(Item("x", "x", 3, Decimal("19.99")).total_value, Decimal("59.97"))
        # 250 * 0.015 = 3.75; 3 * 0.005 = 0.015 -> 0.02 (half up)
        self.assertEqual(Item("x", "x", 250, Decimal("0.015")).total_value, Decimal("3.75"))
        self.assertEqual(Item("x", "x", 3, Decimal("0.005")).total_value, Decimal("0.02"))
        self.assertIsInstance(Item("x", "x", 1, Decimal("1")).total_value, Decimal)
        with self.assertRaises(AttributeError):
            Item("x", "x", 1, Decimal("1")).total_value = Decimal("2")

    def test_csv_report(self):
        items = [
            Item("B-200", "Bolts, zinc (100)", 12, Decimal("4.25")),
            Item("A-100", 'Anchor "heavy"', 3, Decimal("19.99")),
            Item("C-300", "Cable tie", 250, Decimal("0.015")),
        ]
        text = csv_report(items)
        self.assertNotIn("\r", text)
        self.assertTrue(text.endswith("\n"))
        rows = list(csv.reader(io.StringIO(text)))
        self.assertEqual(rows[0], ["sku", "name", "quantity", "unit_price", "total_value"])
        self.assertEqual(rows[1], ["A-100", 'Anchor "heavy"', "3", "19.99", "59.97"])
        self.assertEqual(rows[2], ["B-200", "Bolts, zinc (100)", "12", "4.25", "51.00"])
        self.assertEqual(rows[3], ["C-300", "Cable tie", "250", "0.02", "3.75"])
        self.assertEqual(len(rows), 4)
        self.assertIn('"Bolts, zinc (100)"', text)

    def test_empty(self):
        self.assertEqual(csv_report([]), "sku,name,quantity,unit_price,total_value\n")

    def test_cli_flag(self):
        here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = cli.main(["--csv", os.path.join(here, "stock.json")])
        self.assertIn(code, (0, None))
        self.assertTrue(out.getvalue().startswith("sku,name,quantity,unit_price,total_value\n"))
        self.assertIn("A-100", out.getvalue())
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            cli.main([os.path.join(here, "stock.json")])
        self.assertFalse(out.getvalue().startswith("sku,"))


if __name__ == "__main__":
    unittest.main()
