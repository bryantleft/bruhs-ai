import datetime
import unittest

from bizdays import add_business_days, add_months, is_business_day, roll, schedule

D = datetime.date


class HiddenRollTest(unittest.TestCase):
    def test_business_day_unchanged(self):
        for convention in ("following", "preceding", "modified_following", "modified_preceding"):
            self.assertEqual(roll(D(2027, 1, 27), convention), D(2027, 1, 27))

    def test_unknown_convention(self):
        with self.assertRaises(ValueError):
            roll(D(2027, 1, 27), "nearest")  # a business day: still rejected
        with self.assertRaises(ValueError):
            roll(D(2027, 1, 30), "Following")

    def test_following_preceding(self):
        self.assertEqual(roll(D(2027, 1, 16), "following"), D(2027, 1, 18))
        self.assertEqual(roll(D(2027, 1, 16), "preceding"), D(2027, 1, 15))
        self.assertEqual(roll(D(2027, 1, 30), "following"), D(2027, 2, 1))

    def test_following_skips_holidays(self):
        self.assertEqual(roll(D(2027, 4, 10), "following", {D(2027, 4, 12)}), D(2027, 4, 13))
        self.assertEqual(roll(D(2027, 4, 12), "preceding", {D(2027, 4, 12), D(2027, 4, 9)}), D(2027, 4, 8))

    def test_modified_following(self):
        self.assertEqual(roll(D(2027, 1, 16), "modified_following"), D(2027, 1, 18))
        self.assertEqual(roll(D(2027, 1, 30), "modified_following"), D(2027, 1, 29))
        self.assertEqual(roll(D(2027, 1, 31), "modified_following"), D(2027, 1, 29))
        self.assertEqual(roll(D(2027, 4, 30), "modified_following", {D(2027, 4, 30)}), D(2027, 4, 29))

    def test_modified_preceding(self):
        self.assertEqual(roll(D(2027, 1, 16), "modified_preceding"), D(2027, 1, 15))
        self.assertEqual(roll(D(2027, 5, 1), "modified_preceding"), D(2027, 5, 3))
        self.assertEqual(roll(D(2027, 5, 2), "modified_preceding"), D(2027, 5, 3))
        self.assertEqual(roll(D(2027, 5, 3), "modified_preceding", {D(2027, 5, 3)}), D(2027, 5, 4))


class HiddenAddBusinessDaysTest(unittest.TestCase):
    def test_forward_over_weekend(self):
        self.assertEqual(add_business_days(D(2027, 1, 29), 1), D(2027, 2, 1))
        self.assertEqual(add_business_days(D(2027, 1, 30), 1), D(2027, 2, 1))
        self.assertEqual(add_business_days(D(2027, 1, 31), 1), D(2027, 2, 1))
        self.assertEqual(add_business_days(D(2027, 1, 30), 2), D(2027, 2, 2))
        self.assertEqual(add_business_days(D(2027, 2, 1), 5), D(2027, 2, 8))

    def test_backward(self):
        self.assertEqual(add_business_days(D(2027, 2, 1), -1), D(2027, 1, 29))
        self.assertEqual(add_business_days(D(2027, 1, 31), -1), D(2027, 1, 29))
        self.assertEqual(add_business_days(D(2027, 1, 30), -1), D(2027, 1, 29))
        self.assertEqual(add_business_days(D(2027, 2, 17), -3, {D(2027, 2, 15)}), D(2027, 2, 11))

    def test_holidays(self):
        self.assertEqual(add_business_days(D(2027, 2, 12), 1, {D(2027, 2, 15)}), D(2027, 2, 16))
        holidays = {D(2027, 2, 15), D(2027, 3, 1)}
        self.assertEqual(add_business_days(D(2027, 1, 29), 25, holidays), D(2027, 3, 9))

    def test_zero_rolls_following(self):
        self.assertEqual(add_business_days(D(2027, 1, 27), 0), D(2027, 1, 27))
        self.assertEqual(add_business_days(D(2027, 1, 30), 0), D(2027, 2, 1))
        self.assertEqual(add_business_days(D(2027, 2, 15), 0, {D(2027, 2, 15)}), D(2027, 2, 16))

    def test_holidays_not_mutated(self):
        holidays = {D(2027, 2, 15)}
        add_business_days(D(2027, 2, 12), 3, holidays)
        self.assertEqual(holidays, {D(2027, 2, 15)})

    def test_is_business_day(self):
        self.assertFalse(is_business_day(D(2027, 1, 31)))
        self.assertTrue(is_business_day(D(2027, 2, 1), frozenset()))


class HiddenAddMonthsTest(unittest.TestCase):
    def test_clamp(self):
        self.assertEqual(add_months(D(2027, 1, 31), 1), D(2027, 2, 28))
        self.assertEqual(add_months(D(2028, 1, 31), 1), D(2028, 2, 29))
        self.assertEqual(add_months(D(2027, 3, 31), -1), D(2027, 2, 28))
        self.assertEqual(add_months(D(2027, 5, 31), 1), D(2027, 6, 30))

    def test_gregorian_leap_rule(self):
        self.assertEqual(add_months(D(1900, 1, 31), 1), D(1900, 2, 28))
        self.assertEqual(add_months(D(2000, 1, 31), 1), D(2000, 2, 29))
        self.assertEqual(add_months(D(2100, 3, 29), -1), D(2100, 2, 28))

    def test_year_wrap_and_negative(self):
        self.assertEqual(add_months(D(2027, 11, 15), 3), D(2028, 2, 15))
        self.assertEqual(add_months(D(2027, 2, 15), -14), D(2025, 12, 15))
        self.assertEqual(add_months(D(2027, 1, 15), -1), D(2026, 12, 15))
        self.assertEqual(add_months(D(2027, 12, 31), 0), D(2027, 12, 31))
        self.assertEqual(add_months(D(2027, 12, 31), 12), D(2028, 12, 31))

    def test_end_of_month(self):
        self.assertEqual(add_months(D(2027, 2, 28), 1, end_of_month=True), D(2027, 3, 31))
        self.assertEqual(add_months(D(2027, 4, 30), 1, end_of_month=True), D(2027, 5, 31))
        self.assertEqual(add_months(D(2027, 4, 30), -1, end_of_month=True), D(2027, 3, 31))
        self.assertEqual(add_months(D(2027, 2, 28), 12, end_of_month=True), D(2028, 2, 29))
        self.assertEqual(add_months(D(2028, 2, 29), 12, end_of_month=True), D(2029, 2, 28))

    def test_end_of_month_only_on_month_end(self):
        # Feb 28 2028 is not the last day of February 2028 (leap year).
        self.assertEqual(add_months(D(2028, 2, 28), 1, end_of_month=True), D(2028, 3, 28))
        self.assertEqual(add_months(D(2027, 3, 30), 1, end_of_month=True), D(2027, 4, 30))
        self.assertEqual(add_months(D(2027, 1, 30), 1, end_of_month=True), D(2027, 2, 28))
        self.assertEqual(add_months(D(2027, 4, 30), -1), D(2027, 3, 30))


class HiddenScheduleTest(unittest.TestCase):
    def test_no_drift_from_month_end(self):
        self.assertEqual(
            schedule(D(2027, 1, 31), 1, 4, "following"),
            [D(2027, 3, 1), D(2027, 3, 31), D(2027, 4, 30), D(2027, 5, 31)],
        )

    def test_default_is_modified_following(self):
        self.assertEqual(
            schedule(D(2027, 1, 31), 1, 4),
            [D(2027, 2, 26), D(2027, 3, 31), D(2027, 4, 30), D(2027, 5, 31)],
        )

    def test_end_of_month_flag(self):
        self.assertEqual(
            schedule(D(2027, 2, 28), 1, 3, end_of_month=True),
            [D(2027, 3, 31), D(2027, 4, 30), D(2027, 5, 31)],
        )
        self.assertEqual(
            schedule(D(2027, 2, 28), 1, 3),
            [D(2027, 3, 29), D(2027, 4, 28), D(2027, 5, 28)],
        )

    def test_holidays_and_step(self):
        self.assertEqual(
            schedule(D(2026, 10, 30), 6, 2, "modified_following", {D(2027, 4, 30)}),
            [D(2027, 4, 29), D(2027, 10, 29)],
        )

    def test_bounds(self):
        self.assertEqual(schedule(D(2027, 1, 15), 1, 0), [])
        with self.assertRaises(ValueError):
            schedule(D(2027, 1, 15), 0, 3)
        with self.assertRaises(ValueError):
            schedule(D(2027, 1, 15), 1, -1)


if __name__ == "__main__":
    unittest.main()
