import unittest

from invoice import compute_invoice


class InvoiceTest(unittest.TestCase):
    def test_worked_example(self):
        result = compute_invoice({
            "currency": "EUR",
            "lines": [
                {"quantity": "3", "unit_price": "3.335", "tax_rate": "0.19"},
                {"quantity": "1", "unit_price": "10.00", "tax_rate": "0.07",
                 "discount_percent": "15"},
                {"quantity": "-1", "unit_price": "2.50", "tax_rate": "0.19"},
            ],
            "invoice_discount": "1.00",
        })
        self.assertEqual(
            [line["allocated"] for line in result["lines"]], ["0.54", "0.46", "0.00"]
        )
        self.assertEqual(
            result["taxes"],
            [
                {"rate": "0.19", "base": "6.97", "tax": "1.32"},
                {"rate": "0.07", "base": "8.04", "tax": "0.56"},
            ],
        )
        self.assertEqual(
            (result["net_total"], result["tax_total"], result["total"]),
            ("15.01", "1.88", "16.89"),
        )

    def test_single_line(self):
        result = compute_invoice({
            "currency": "USD",
            "lines": [{"quantity": "2", "unit_price": "5.00", "tax_rate": "0.10"}],
        })
        self.assertEqual(result["lines"], [
            {"amount": "10.00", "line_discount": "0.00", "allocated": "0.00", "taxable": "10.00",
             "tax": "1.00"}
        ])
        self.assertEqual(result["total"], "11.00")


if __name__ == "__main__":
    unittest.main()
