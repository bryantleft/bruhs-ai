import datetime
import unittest

from bizdays import add_business_days, add_months, is_business_day, roll, schedule

D = datetime.date


class BizdaysTest(unittest.TestCase):
    def test_is_business_day(self):
        self.assertTrue(is_business_day(D(2027, 3, 3)))  # Wednesday
        self.assertFalse(is_business_day(D(2027, 3, 6)))  # Saturday
        self.assertFalse(is_business_day(D(2027, 3, 3), {D(2027, 3, 3)}))

    def test_roll_following(self):
        self.assertEqual(roll(D(2027, 3, 6), "following"), D(2027, 3, 8))

    def test_add_business_days_simple(self):
        self.assertEqual(add_business_days(D(2027, 3, 1), 1), D(2027, 3, 2))
        self.assertEqual(add_business_days(D(2027, 3, 1), 4), D(2027, 3, 5))

    def test_add_months_simple(self):
        self.assertEqual(add_months(D(2027, 1, 15), 1), D(2027, 2, 15))

    def test_schedule_quarterly(self):
        self.assertEqual(
            schedule(D(2027, 1, 15), 3, 2),
            [D(2027, 4, 15), D(2027, 7, 15)],
        )


if __name__ == "__main__":
    unittest.main()
