import copy
import math
import random
import unittest
from decimal import Decimal
from fractions import Fraction

from invoice import compute_invoice


def line(qty, price, rate, pct=None, **extra):
    out = {"quantity": qty, "unit_price": price, "tax_rate": rate, **extra}
    if pct is not None:
        out["discount_percent"] = pct
    return out


def inv(lines, currency="EUR", discount=None, inclusive=None):
    out = {"currency": currency, "lines": lines}
    if discount is not None:
        out["invoice_discount"] = discount
    if inclusive is not None:
        out["prices_include_tax"] = inclusive
    return out


class LineRoundingTest(unittest.TestCase):
    def amounts(self, invoice):
        return [l["amount"] for l in compute_invoice(invoice)["lines"]]

    def test_half_away_from_zero_without_float_error(self):
        lines = [
            line("1", "1.005", "0"),
            line("3", "0.335", "0"),
            line("1.5", "19.99", "0"),
            line("1", "2.675", "0"),
            line("-1", "2.345", "0"),
            line("-3", "0.335", "0"),
            line("0.001", "4.4449", "0"),
        ]
        self.assertEqual(
            self.amounts(inv(lines)),
            ["1.01", "1.01", "29.99", "2.68", "-2.35", "-1.01", "0.00"],
        )

    def test_line_discount_uses_the_rounded_amount(self):
        # amount 0.125 -> 0.13; 50 % of 0.13 = 0.065 -> 0.07 (not 50 % of 0.125)
        result = compute_invoice(inv([line("1", "0.125", "0", "50")]))
        self.assertEqual(result["lines"][0]["amount"], "0.13")
        self.assertEqual(result["lines"][0]["line_discount"], "0.07")
        self.assertEqual(result["lines"][0]["taxable"], "0.06")

    def test_fractional_percent_and_full_discount(self):
        result = compute_invoice(inv([line("1", "9.99", "0", "12.5"), line("2", "4.00", "0", "100")]))
        self.assertEqual(result["lines"][0]["line_discount"], "1.25")  # 1.24875
        self.assertEqual(result["lines"][0]["taxable"], "8.74")
        self.assertEqual(result["lines"][1]["line_discount"], "8.00")
        self.assertEqual(result["lines"][1]["taxable"], "0.00")

    def test_discounted_return(self):
        result = compute_invoice(inv([line("-2", "5.00", "0.1", "10"), line("1", "20.00", "0.1")]))
        self.assertEqual(
            result["lines"][0],
            {"amount": "-10.00", "line_discount": "-1.00", "allocated": "0.00", "taxable": "-9.00",
             "tax": "-0.90"},
        )
        self.assertEqual(result["taxes"], [{"rate": "0.1", "base": "11.00", "tax": "1.10"}])


class TaxTest(unittest.TestCase):
    def test_group_tax_rounds_half_to_even(self):
        result = compute_invoice(inv([line("1", "12.25", "0.1"), line("1", "0.35", "0.2")]))
        self.assertEqual(
            result["taxes"],
            [
                {"rate": "0.2", "base": "0.35", "tax": "0.07"},
                {"rate": "0.1", "base": "12.25", "tax": "1.22"},
            ],
        )
        self.assertEqual(result["tax_total"], "1.29")
        self.assertEqual(result["total"], "13.89")

    def test_tax_is_per_group_not_per_line(self):
        result = compute_invoice(inv([line("1", "0.05", "0.1")] * 3))
        self.assertEqual(result["taxes"], [{"rate": "0.1", "base": "0.15", "tax": "0.02"}])

    def test_numerically_equal_rates_are_one_group(self):
        result = compute_invoice(
            inv([line("1", "1.00", "0.2"), line("1", "2.00", "0.20"), line("1", "3.00", "0.200")])
        )
        self.assertEqual(result["taxes"], [{"rate": "0.2", "base": "6.00", "tax": "1.20"}])

    def test_rate_format_order_and_zero_group(self):
        result = compute_invoice(
            inv([line("1", "1.00", "0.075"), line("1", "1.00", "0.00"), line("1", "1.00", "0.190")])
        )
        self.assertEqual(
            result["taxes"],
            [
                {"rate": "0.19", "base": "1.00", "tax": "0.19"},
                {"rate": "0.075", "base": "1.00", "tax": "0.08"},  # 0.075 -> 0.08 (even)
                {"rate": "0", "base": "1.00", "tax": "0.00"},
            ],
        )

    def test_negative_base_rounds_half_to_even_and_has_no_negative_zero(self):
        result = compute_invoice(
            inv([line("-1", "0.25", "0.1"), line("-1", "0.04", "0.2"), line("1", "0.04", "0.05")])
        )
        self.assertEqual(
            result["taxes"],
            [
                {"rate": "0.2", "base": "-0.04", "tax": "-0.01"},  # -0.008
                {"rate": "0.1", "base": "-0.25", "tax": "-0.02"},  # -0.025, half to even
                {"rate": "0.05", "base": "0.04", "tax": "0.00"},  # 0.002
            ],
        )
        self.assertEqual(result["tax_total"], "-0.03")
        self.assertEqual(result["total"], "-0.28")
        zero = compute_invoice(inv([line("-1", "0.04", "0.1")]))
        self.assertEqual(zero["taxes"][0]["tax"], "0.00")  # -0.004 -> "0.00", never "-0.00"
        self.assertEqual(zero["tax_total"], "0.00")
        self.assertEqual(zero["total"], "-0.04")
        balanced = compute_invoice(inv([line("1", "3.00", "0.1"), line("-1", "3.00", "0.1")]))
        self.assertEqual(
            (balanced["net_total"], balanced["tax_total"], balanced["total"]),
            ("0.00", "0.00", "0.00"),
        )


class InclusiveTest(unittest.TestCase):
    def test_tax_is_taken_out_of_the_gross_base(self):
        result = compute_invoice(
            inv([line("1", "11.90", "0.19"), line("1", "10.00", "0.07")], inclusive=True)
        )
        self.assertEqual(
            result["taxes"],
            [
                {"rate": "0.19", "base": "11.90", "tax": "1.90"},
                {"rate": "0.07", "base": "10.00", "tax": "0.65"},  # 0.6542...
            ],
        )
        self.assertEqual(
            (result["net_total"], result["tax_total"], result["total"]),
            ("19.35", "2.55", "21.90"),
        )

    def test_inclusive_half_to_even(self):
        # 0.12 * 0.6 / 1.6 = 0.045 exactly -> 0.04
        result = compute_invoice(inv([line("1", "0.12", "0.6")], inclusive=True))
        self.assertEqual(result["taxes"], [{"rate": "0.6", "base": "0.12", "tax": "0.04"}])
        self.assertEqual((result["net_total"], result["total"]), ("0.08", "0.12"))

    def test_inclusive_with_invoice_discount(self):
        result = compute_invoice(
            inv([line("2", "5.95", "0.19"), line("1", "3.00", "0")], discount="1.49", inclusive=True)
        )
        # nets 1190 and 300 cents, D = 149: shares 119.0 and 30.0 exactly
        self.assertEqual([l["allocated"] for l in result["lines"]], ["1.19", "0.30"])
        self.assertEqual(
            result["taxes"],
            [
                {"rate": "0.19", "base": "10.71", "tax": "1.71"},  # 1.7100...
                {"rate": "0", "base": "2.70", "tax": "0.00"},
            ],
        )
        self.assertEqual(
            (result["net_total"], result["tax_total"], result["total"]),
            ("11.70", "1.71", "13.41"),
        )


class LineTaxTest(unittest.TestCase):
    def line_taxes(self, invoice):
        return [l["tax"] for l in compute_invoice(invoice)["lines"]]

    def test_worked_example(self):
        invoice = inv(
            [
                line("3", "3.335", "0.19"),
                line("1", "10.00", "0.07", "15"),
                line("-1", "2.50", "0.19"),
            ],
            discount="1.00",
        )
        self.assertEqual(self.line_taxes(invoice), ["1.80", "0.56", "-0.48"])

    def test_negative_exact_taxes_round_toward_negative_infinity(self):
        # exact -2.3, -2.3, 10 cents; floors -3, -3, 10; group tax 5.4 -> 5
        lines = [line("-1", "0.23", "0.1"), line("-1", "0.23", "0.1"), line("1", "1.00", "0.1")]
        self.assertEqual(self.line_taxes(inv(lines)), ["-0.02", "-0.03", "0.10"])

    def test_leftover_units_go_to_the_largest_fractions(self):
        lines = [line("1", "0.13", "0.1"), line("1", "0.11", "0.1"), line("1", "0.17", "0.1")]
        self.assertEqual(self.line_taxes(inv(lines)), ["0.01", "0.01", "0.02"])
        ties = [line("1", "0.05", "0.1")] * 3  # exact 0.5 each, group tax 1.5 -> 2
        self.assertEqual(self.line_taxes(inv(ties)), ["0.01", "0.01", "0.00"])

    def test_groups_are_split_separately(self):
        lines = [
            line("1", "0.05", "0.1"),
            line("1", "0.13", "0.2"),
            line("1", "0.05", "0.1"),
            line("1", "0.14", "0.2"),
        ]
        # 0.1: exact 0.5, 0.5, group 0.01 -> first line; 0.2: exact 2.6, 2.8, group 5.4 -> 5
        self.assertEqual(self.line_taxes(inv(lines)), ["0.01", "0.02", "0.00", "0.03"])

    def test_inclusive_split(self):
        lines = [line("1", "1.00", "0.07"), line("1", "2.00", "0.07"), line("1", "3.00", "0.07")]
        # exact 6.54.., 13.08.., 19.62..; group tax 39.25.. -> 39; leftover to line 3
        self.assertEqual(self.line_taxes(inv(lines, inclusive=True)), ["0.06", "0.13", "0.20"])
        lines = [line("-1", "1.00", "0.07"), line("1", "3.00", "0.07")]
        # exact -6.54.. and 19.62..; floors -7, 19; group tax 13.08.. -> 13
        self.assertEqual(self.line_taxes(inv(lines, inclusive=True)), ["-0.07", "0.20"])

    def test_line_taxes_add_up_to_the_group_tax(self):
        rng = random.Random(1108)
        rates = ["0", "0.05", "0.1", "0.19", "0.2"]
        for _ in range(80):
            lines = [
                line(
                    str(rng.choice([-3, -1, 1, 1, 2, 5])),
                    f"{rng.randint(1, 999) / 100:.2f}",
                    rng.choice(rates),
                    rng.choice([None, "10", "33"]),
                )
                for _ in range(rng.randint(1, 8))
            ]
            result = compute_invoice(inv(lines))
            for group in result["taxes"]:
                rate = Fraction(group["rate"])
                members = [
                    i for i, l in enumerate(lines) if Fraction(l["tax_rate"]) == rate
                ]
                exact = [Fraction(result["lines"][i]["taxable"]) * 100 * rate for i in members]
                want = [math.floor(x) for x in exact]
                order = sorted(range(len(members)), key=lambda k: (-(exact[k] - want[k]), k))
                left = int(Fraction(group["tax"]) * 100) - sum(want)
                self.assertGreaterEqual(left, 0)
                for k in order[:left]:
                    want[k] += 1
                got = [int(Fraction(result["lines"][i]["tax"]) * 100) for i in members]
                self.assertEqual(got, want, lines)


class InvoiceDiscountTest(unittest.TestCase):
    def allocated(self, invoice):
        return [l["allocated"] for l in compute_invoice(invoice)["lines"]]

    def test_ties_go_to_the_earlier_line(self):
        three = [line("1", "1.00", "0")] * 3
        self.assertEqual(self.allocated(inv(three, discount="0.01")), ["0.01", "0.00", "0.00"])
        self.assertEqual(self.allocated(inv(three, discount="0.02")), ["0.01", "0.01", "0.00"])
        mixed = [line("1", "1.00", "0"), line("1", "2.00", "0"), line("1", "1.00", "0")]
        self.assertEqual(self.allocated(inv(mixed, discount="0.02")), ["0.01", "0.01", "0.00"])

    def test_largest_fraction_wins_regardless_of_position(self):
        lines = [line("1", "1.00", "0"), line("1", "1.00", "0"), line("1", "5.00", "0")]
        # D = 10: shares 1.428.., 1.428.., 7.142.. -> 1, 1, 7 + leftover 1 to an earlier tie
        self.assertEqual(self.allocated(inv(lines, discount="0.10")), ["0.02", "0.01", "0.07"])
        lines = [line("1", "3.00", "0"), line("1", "1.00", "0"), line("1", "3.00", "0")]
        # D = 5: shares 2.142.., 0.714.., 2.142.. -> 2, 0, 2 + leftover 1 to line 2 (.714)
        self.assertEqual(self.allocated(inv(lines, discount="0.05")), ["0.02", "0.01", "0.02"])

    def test_returns_and_zero_lines_get_nothing(self):
        lines = [
            line("-1", "4.00", "0"),
            line("1", "2.00", "0", "100"),
            line("1", "1.00", "0"),
            line("1", "1.00", "0"),
        ]
        self.assertEqual(self.allocated(inv(lines, discount="0.03")), ["0.00", "0.00", "0.02", "0.01"])

    def test_allocation_always_sums_to_the_discount(self):
        rng = random.Random(20261108)
        for _ in range(60):
            lines = [
                line(str(rng.choice([-2, 1, 1, 2, 3])), f"{rng.randint(1, 5000) / 100:.2f}", "0")
                for _ in range(rng.randint(1, 7))
            ]
            nets = [int(round(Decimal(l["quantity"]) * Decimal(l["unit_price"]) * 100)) for l in lines]
            s = sum(n for n in nets if n > 0)
            if s == 0:
                continue
            d = rng.randint(0, s)
            got = [
                int(Decimal(a) * 100)
                for a in self.allocated(inv(lines, discount=f"{d // 100}.{d % 100:02d}"))
            ]
            # reference: largest remainder, ties by position
            shares = [Fraction(d * n, s) if n > 0 else Fraction(0) for n in nets]
            want = [int(x) for x in shares]
            order = sorted(
                (i for i, n in enumerate(nets) if n > 0), key=lambda i: (-(shares[i] - want[i]), i)
            )
            for i in order[: d - sum(want)]:
                want[i] += 1
            self.assertEqual(got, want, (lines, d))

    def test_discount_bigger_than_eligible_lines_raises(self):
        with self.assertRaises(ValueError):
            compute_invoice(inv([line("1", "1.00", "0"), line("-1", "5.00", "0")], discount="1.01"))
        with self.assertRaises(ValueError):
            compute_invoice(inv([line("-1", "5.00", "0")], discount="0.01"))
        ok = compute_invoice(inv([line("1", "1.00", "0.1")], discount="1.00"))
        self.assertEqual(ok["total"], "0.00")
        only_returns = compute_invoice(inv([line("-1", "5.00", "0")], discount="0"))
        self.assertEqual(only_returns["total"], "-5.00")

    def test_discount_must_be_whole_minor_units(self):
        with self.assertRaises(ValueError):
            compute_invoice(inv([line("1", "10.00", "0")], discount="0.005"))
        with self.assertRaises(ValueError):
            compute_invoice(inv([line("1", "1000", "0")], currency="JPY", discount="100.5"))
        result = compute_invoice(inv([line("1", "10.00", "0")], discount="0.500"))
        self.assertEqual(result["lines"][0]["allocated"], "0.50")


class CurrencyTest(unittest.TestCase):
    def test_jpy_has_no_decimals(self):
        result = compute_invoice(inv([line("3", "333.5", "0.1")], currency="JPY", discount="1"))
        self.assertEqual(
            result["lines"],
            [{"amount": "1001", "line_discount": "0", "allocated": "1", "taxable": "1000", "tax": "100"}],
        )
        self.assertEqual(result["taxes"], [{"rate": "0.1", "base": "1000", "tax": "100"}])
        self.assertEqual(result["total"], "1100")
        krw = compute_invoice(inv([line("1", "1005", "0.1")], currency="KRW"))
        self.assertEqual(krw["taxes"][0]["tax"], "100")  # 100.5 -> 100 (even)

    def test_kwd_has_three_decimals(self):
        result = compute_invoice(inv([line("1", "1.2345", "0.05")], currency="KWD"))
        self.assertEqual(result["lines"][0]["amount"], "1.235")
        self.assertEqual(result["taxes"], [{"rate": "0.05", "base": "1.235", "tax": "0.062"}])
        self.assertEqual(result["total"], "1.297")
        bhd = compute_invoice(inv([line("1", "0.125", "0.1")], currency="BHD"))
        self.assertEqual(bhd["taxes"][0]["tax"], "0.012")  # 0.0125 -> 0.012

    def test_unknown_currency(self):
        with self.assertRaises(ValueError):
            compute_invoice(inv([line("1", "1.00", "0")], currency="XYZ"))


class InputTest(unittest.TestCase):
    def test_non_strings_raise_type_error(self):
        bad = [
            inv([line(1, "1.00", "0")]),
            inv([line("1", 1.5, "0")]),
            inv([line("1", "1.00", Decimal("0.1"))]),
            inv([line("1", "1.00", "0", 10)]),
            inv([line("1", "1.00", "0")], discount=0.5),
        ]
        for invoice in bad:
            with self.assertRaises(TypeError, msg=repr(invoice)):
                compute_invoice(invoice)

    def test_every_value_has_its_tables_type(self):
        good = [line("1", "1.00", "0")]
        bad = [
            [good],
            {"currency": None, "lines": good},
            {"currency": 978, "lines": good},
            {"currency": "EUR", "lines": good, "prices_include_tax": "false"},
            {"currency": "EUR", "lines": good, "prices_include_tax": 1},
            {"currency": "EUR", "lines": good, "prices_include_tax": None},
            {"currency": "EUR", "lines": tuple(good)},
            {"currency": "EUR", "lines": None},
            {"currency": "EUR", "lines": [["1", "1.00", "0"]]},
        ]
        for invoice in bad:
            with self.assertRaises(TypeError, msg=repr(invoice)):
                compute_invoice(invoice)
        ignored = line("1", "1.00", "0", sku=123, description=None, tags=["a"])
        self.assertEqual(compute_invoice(inv([ignored]))["total"], "1.00")
        self.assertEqual(compute_invoice(inv(good, inclusive=True))["total"], "1.00")

    def test_bad_values_raise_value_error(self):
        bad = [
            inv([line("abc", "1.00", "0")]),
            inv([line("1", "12,50", "0")]),
            inv([line("1", "", "0")]),
            inv([line("1", "-1.00", "0")]),
            inv([line("1", "1.00", "1")]),
            inv([line("1", "1.00", "-0.1")]),
            inv([line("1", "1.00", "0", "100.5")]),
            inv([line("1", "1.00", "0", "-1")]),
            inv([line("1", "1.00", "0")], discount="-0.01"),
            inv([line("1", "1.00", "0")], discount="x"),
        ]
        for invoice in bad:
            with self.assertRaises(ValueError, msg=repr(invoice)):
                compute_invoice(invoice)

    def test_empty_invoice(self):
        self.assertEqual(
            compute_invoice({"currency": "GBP", "lines": []}),
            {
                "currency": "GBP",
                "lines": [],
                "taxes": [],
                "net_total": "0.00",
                "tax_total": "0.00",
                "total": "0.00",
            },
        )

    def test_extra_keys_ignored_and_input_not_mutated(self):
        invoice = inv(
            [line("2", "1.50", "0.1", "10", description="pen", sku="P-1")],
            currency="CHF",
            discount="0.10",
            inclusive=False,
        )
        before = copy.deepcopy(invoice)
        result = compute_invoice(invoice)
        self.assertEqual(invoice, before)
        self.assertEqual(
            result,
            {
                "currency": "CHF",
                "lines": [
                    {"amount": "3.00", "line_discount": "0.30", "allocated": "0.10", "taxable": "2.60",
                     "tax": "0.26"}
                ],
                "taxes": [{"rate": "0.1", "base": "2.60", "tax": "0.26"}],
                "net_total": "2.60",
                "tax_total": "0.26",
                "total": "2.86",
            },
        )


if __name__ == "__main__":
    unittest.main()
