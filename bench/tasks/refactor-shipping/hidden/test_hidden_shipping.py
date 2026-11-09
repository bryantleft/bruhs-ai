import ast
import inspect
import os
import unittest

import shipping
from _bench_hidden import orig_shipping as orig

COUNTRIES = ["US", "CA", "MX", "DE", "FR", "JP", "AU"]
WEIGHTS = [0.1, 0.5, 1, 1.5, 2, 2.75, 3, 7.3, 10, 25, 99.9]


def outcome(fn, *args, **kwargs):
    try:
        return ("ok", fn(*args, **kwargs))
    except Exception as error:  # noqa: BLE001 - comparing behaviour
        return ("err", type(error).__name__, str(error))


class SameBehaviourTest(unittest.TestCase):
    def test_quotes_match(self):
        for country in COUNTRIES + ["ZZ", "us", ""]:
            for weight in WEIGHTS + [0, -1]:
                for express in (False, True):
                    self.assertEqual(
                        outcome(shipping.quote, country, weight, express),
                        outcome(orig.quote, country, weight, express),
                        (country, weight, express),
                    )

    def test_invoice_lines_match(self):
        for country in COUNTRIES + ["ZZ"]:
            for weight in WEIGHTS:
                for express in (False, True):
                    self.assertEqual(
                        outcome(shipping.invoice_line, country, weight, express),
                        outcome(orig.invoice_line, country, weight, express),
                    )

    def test_cheapest_match(self):
        cases = [COUNTRIES, ["DE", "FR"], ["FR", "DE"], ["AU"], [], ["US", "ZZ"], ["JP", "AU", "MX"]]
        for countries in cases:
            for weight in WEIGHTS:
                self.assertEqual(
                    outcome(shipping.cheapest_country, countries, weight),
                    outcome(orig.cheapest_country, countries, weight),
                    (countries, weight),
                )


class StructureTest(unittest.TestCase):
    def test_rates_table(self):
        self.assertIsInstance(shipping.RATES, dict)
        self.assertEqual(set(shipping.RATES), set(COUNTRIES))
        for country in COUNTRIES:
            self.assertIs(shipping.rate_for(country), shipping.RATES[country])
        with self.assertRaises(KeyError):
            shipping.rate_for("ZZ")

    def test_each_country_code_appears_once(self):
        source = inspect.getsource(shipping)
        literals = [
            node.value
            for node in ast.walk(ast.parse(source))
            if isinstance(node, ast.Constant) and node.value in COUNTRIES
        ]
        for country in COUNTRIES:
            self.assertEqual(literals.count(country), 1, country)

    def test_no_country_chains_left(self):
        tree = ast.parse(inspect.getsource(shipping))
        compares = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Compare)
            and any(isinstance(c, ast.Constant) and c.value in COUNTRIES for c in node.comparators)
        ]
        self.assertEqual(compares, [])


if __name__ == "__main__":
    unittest.main()
