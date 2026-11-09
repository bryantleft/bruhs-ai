import unittest

from orders.api import list_orders
from orders.store import OrderStore


def all_pages(store, limit):
    ids, cursor = [], None
    for _ in range(50):
        page = list_orders(store, limit=limit, cursor=cursor)
        ids.extend(o["id"] for o in page["orders"])
        cursor = page["next_cursor"]
        if cursor is None:
            return ids
    raise AssertionError("paging did not end")


class ListOrdersTest(unittest.TestCase):
    def test_first_page_is_newest_first(self):
        store = OrderStore()
        store.add({"id": 1, "created_at": "2026-03-01T09:00:00Z"})
        store.add({"id": 2, "created_at": "2026-03-01T11:00:00Z"})
        store.add({"id": 3, "created_at": "2026-03-01T10:00:00Z"})
        page = list_orders(store, limit=2)
        self.assertEqual([o["id"] for o in page["orders"]], [2, 3])
        self.assertIsNotNone(page["next_cursor"])

    def test_paging_with_distinct_timestamps(self):
        store = OrderStore()
        for i in range(1, 8):
            store.add({"id": i, "created_at": f"2026-03-01T{i:02d}:00:00Z"})
        self.assertEqual(all_pages(store, 3), [7, 6, 5, 4, 3, 2, 1])

    def test_orders_placed_in_the_same_second(self):
        # Reported from production: some of these orders never show up.
        store = OrderStore()
        for i in range(1, 6):
            store.add({"id": i, "created_at": "2026-03-01T12:00:00Z"})
        store.add({"id": 6, "created_at": "2026-03-01T11:00:00Z"})
        ids = all_pages(store, 2)
        self.assertEqual(sorted(ids), [1, 2, 3, 4, 5, 6])

    def test_limit_is_checked(self):
        with self.assertRaises(ValueError):
            list_orders(OrderStore(), limit=0)


if __name__ == "__main__":
    unittest.main()
