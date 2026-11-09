import unittest

from pager import page_count, paginate, page_window


class HiddenPagerTest(unittest.TestCase):
    def test_partial_last_page(self):
        self.assertEqual(page_count(10, 3), 4)
        self.assertEqual(paginate(list(range(10)), 4, 3), [9])
        self.assertEqual(page_count(1, 50), 1)
        self.assertEqual(paginate(["a"], 1, 50), ["a"])

    def test_empty(self):
        self.assertEqual(page_count(0, 3), 0)
        self.assertEqual(paginate([], 1, 3), [])

    def test_past_the_end(self):
        self.assertEqual(paginate(list(range(10)), 5, 3), [])

    def test_exact_multiple_still_works(self):
        self.assertEqual(page_count(9, 3), 3)
        self.assertEqual(paginate(list(range(9)), 3, 3), [6, 7, 8])
        self.assertEqual(paginate(list(range(9)), 4, 3), [])

    def test_invalid_arguments(self):
        with self.assertRaises(ValueError):
            paginate([1, 2], 0, 3)
        with self.assertRaises(ValueError):
            paginate([1, 2], 1, 0)
        with self.assertRaises(ValueError):
            page_count(5, 0)

    def test_every_item_is_shown_once(self):
        for total in range(0, 25):
            for per_page in range(1, 8):
                items = list(range(total))
                seen = []
                for page in range(1, page_count(total, per_page) + 1):
                    chunk = paginate(items, page, per_page)
                    self.assertTrue(chunk, (total, per_page, page))
                    seen.extend(chunk)
                self.assertEqual(seen, items, (total, per_page))

    def test_window_unchanged(self):
        self.assertEqual(page_window(2, 3), [1, 2, 3])
        self.assertEqual(page_window(1, 0), [])


if __name__ == "__main__":
    unittest.main()
