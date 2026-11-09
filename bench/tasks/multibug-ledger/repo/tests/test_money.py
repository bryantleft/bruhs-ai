import unittest

from ledger.money import parse_amount


class ParseAmountTest(unittest.TestCase):
    def test_two_decimals(self):
        self.assertEqual(parse_amount("12.34"), 1234)

    def test_whole_number(self):
        self.assertEqual(parse_amount("7"), 700)

    def test_one_decimal(self):
        self.assertEqual(parse_amount("1.5"), 150)

    def test_too_many_decimals(self):
        with self.assertRaises(ValueError):
            parse_amount("1.234")


if __name__ == "__main__":
    unittest.main()
