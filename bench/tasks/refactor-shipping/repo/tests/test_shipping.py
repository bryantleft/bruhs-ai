import unittest

from shipping import cheapest_country, invoice_line, quote


class ShippingTest(unittest.TestCase):
    def test_quote(self):
        self.assertEqual(quote("US", 2), 740)
        self.assertEqual(quote("US", 2, express=True), 1110)
        self.assertEqual(quote("JP", 0.5), 1330)

    def test_invoice_line(self):
        self.assertEqual(invoice_line("DE", 1.5), "Germany standard 1.5kg: $12.15")

    def test_cheapest(self):
        self.assertEqual(cheapest_country(["JP", "CA", "US"], 3), "US")

    def test_errors(self):
        with self.assertRaises(KeyError):
            quote("ZZ", 1)
        with self.assertRaises(ValueError):
            quote("US", 0)


if __name__ == "__main__":
    unittest.main()
