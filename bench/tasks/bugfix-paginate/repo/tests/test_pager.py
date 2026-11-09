import unittest

from pager import page_count, paginate, page_window


class PagerTest(unittest.TestCase):
    def test_exact_multiple(self):
        self.assertEqual(page_count(9, 3), 3)
        self.assertEqual(paginate(list(range(9)), 2, 3), [3, 4, 5])

    def test_window(self):
        self.assertEqual(page_window(1, 10), [1, 2, 3, 4, 5])
        self.assertEqual(page_window(10, 10), [6, 7, 8, 9, 10])


if __name__ == "__main__":
    unittest.main()
