import base64
import random
import unittest
from datetime import datetime, timedelta, timezone

from orders.api import list_orders
from orders.cursor import InvalidCursor
from orders.store import OrderStore
from orders.timeutil import to_micros


def walk(store, limit, between=None):
    """Follow the cursors; `between(page_number)` runs before each later page."""
    pages, cursor = [], None
    for n in range(500):
        if n and between:
            between(n)
        page = list_orders(store, limit=limit, cursor=cursor)
        pages.append([o["id"] for o in page["orders"]])
        cursor = page["next_cursor"]
        if cursor is None:
            return pages
    raise AssertionError("paging did not end")


def flat(pages):
    return [i for page in pages for i in page]


def store_of(rows):
    store = OrderStore()
    for order_id, created in rows:
        store.add({"id": order_id, "created_at": created})
    return store


def b64(text):
    return base64.urlsafe_b64encode(text.encode()).decode()


class OrderTest(unittest.TestCase):
    def test_same_instant_is_listed_by_id_highest_first(self):
        store = store_of([(i, "2026-03-01T12:00:00Z") for i in (3, 1, 5, 2, 4)])
        store.add({"id": 9, "created_at": "2026-03-01T11:59:59Z"})
        for limit in (1, 2, 3, 6, 100):
            self.assertEqual(flat(walk(store, limit)), [5, 4, 3, 2, 1, 9], limit)

    def test_offsets_are_part_of_the_instant(self):
        store = store_of(
            [
                (1, "2026-03-01T10:15:30+01:00"),  # 09:15:30Z
                (2, "2026-03-01T09:30:00Z"),  # 09:30:00Z
                (3, "2026-03-01T20:00:00+11:00"),  # 09:00:00Z
                (4, "2026-03-01T04:20:00-05:00"),  # 09:20:00Z
                (5, "2026-03-01T10:30:00+01:00"),  # 09:30:00Z, same instant as 2
                (6, "2026-02-28T23:45:00-10:00"),  # 2026-03-01T09:45:00Z
            ]
        )
        self.assertEqual(flat(walk(store, 100)), [6, 5, 2, 4, 1, 3])
        self.assertEqual(walk(store, 2), [[6, 5], [2, 4], [1, 3]])

    def test_fractions_of_a_second(self):
        store = store_of(
            [
                (1, "2026-03-01T09:00:00.25Z"),
                (2, "2026-03-01T09:00:00.5Z"),
                (3, "2026-03-01T10:00:00.250000+01:00"),  # same instant as 1
                (4, "2026-03-01T09:00:00Z"),
                (5, "2026-03-01T09:00:00.000001Z"),
            ]
        )
        self.assertEqual(flat(walk(store, 2)), [2, 3, 1, 5, 4])

    def test_to_micros(self):
        self.assertEqual(to_micros("1970-01-01T00:00:00Z"), 0)
        self.assertEqual(to_micros("1970-01-01T01:00:00+01:00"), 0)
        self.assertEqual(to_micros("1969-12-31T23:59:59.5Z"), -500_000)
        self.assertEqual(
            to_micros("2026-03-01T20:15:30.25+11:00"), to_micros("2026-03-01T09:15:30.250Z")
        )
        self.assertEqual(to_micros("2026-03-01T09:15:30Z"), 1_772_356_530_000_000)
        self.assertEqual(to_micros("2026-03-01T04:15:30.000007-05:00"), 1_772_356_530_000_007)

    def test_matches_a_reference_listing(self):
        rng = random.Random(7)
        base = datetime(2026, 3, 1, 9, 0, tzinfo=timezone.utc)
        offsets = [timedelta(hours=h) for h in (-10, -5, 0, 1, 5.5, 11)]
        rows, expect = [], []
        ids = rng.sample(range(1, 10_000), 120)
        for order_id in ids:
            instant = base + timedelta(seconds=rng.choice(range(0, 40, 2)), microseconds=rng.choice([0, 0, 250_000]))
            off = rng.choice(offsets)
            local = instant.astimezone(timezone(off))
            text = local.isoformat()
            if off == timedelta(0) and rng.random() < 0.5:
                text = text.replace("+00:00", "Z")
            rows.append((order_id, text))
            expect.append((instant, order_id))
        store = store_of(rows)
        want = [order_id for _, order_id in sorted(expect, key=lambda x: (x[0], x[1]), reverse=True)]
        for limit in (1, 3, 7, 10, 40, 100):
            self.assertEqual(flat(walk(store, limit)), want, limit)


class NextCursorTest(unittest.TestCase):
    def test_no_cursor_after_the_last_order(self):
        store = store_of([(i, f"2026-03-01T0{i}:00:00Z") for i in range(1, 5)])
        self.assertEqual(walk(store, 2), [[4, 3], [2, 1]])
        self.assertEqual(walk(store, 4), [[4, 3, 2, 1]])
        self.assertEqual(walk(store, 1), [[4], [3], [2], [1]])
        self.assertIsNotNone(list_orders(store, limit=3)["next_cursor"])

    def test_empty_store(self):
        self.assertEqual(list_orders(OrderStore(), limit=5), {"orders": [], "next_cursor": None})

    def test_last_page_ending_in_a_tie(self):
        store = store_of([(1, "2026-03-01T09:00:00Z"), (2, "2026-03-01T09:00:00Z"), (3, "2026-03-01T10:00:00Z")])
        self.assertEqual(walk(store, 1), [[3], [2], [1]])
        self.assertEqual(walk(store, 3), [[3, 2, 1]])


class LiveChangesTest(unittest.TestCase):
    def rows(self):
        out = []
        for i in range(1, 21):
            minute = 10 + (i // 3)  # groups of up to three orders per instant
            out.append((i, f"2026-03-01T09:{minute:02d}:00Z"))
        return out

    def test_orders_added_between_pages(self):
        store = store_of(self.rows())
        original = flat(walk(store, 100))
        counter = iter(range(100, 1000))

        def add_some(_n):
            for created in ("2026-03-01T12:00:00Z", "2026-03-01T09:13:00Z", "2026-03-01T08:00:00Z"):
                store.add({"id": next(counter), "created_at": created})

        seen = flat(walk(store, 4, between=add_some))
        self.assertEqual([i for i in seen if i < 100], original)
        self.assertEqual(len(seen), len(set(seen)))

    def test_orders_removed_between_pages(self):
        store = store_of(self.rows())
        original = flat(walk(store, 100))
        removed = set()
        last_seen = []

        def remove_some(n):
            # remove the order the cursor points at, and one further down
            for victim in (last_seen[-1], original[(n * 5 + 2) % len(original)]):
                if victim not in removed:
                    store.remove(victim)
                    removed.add(victim)

        pages, cursor = [], None
        for n in range(100):
            if n:
                remove_some(n)
            page = list_orders(store, limit=4, cursor=cursor)
            pages.append([o["id"] for o in page["orders"]])
            last_seen.extend(pages[-1])
            cursor = page["next_cursor"]
            if cursor is None:
                break
        seen = flat(pages)
        self.assertEqual(len(seen), len(set(seen)))
        # every order that was never removed is seen, in order
        survivors = [i for i in original if i not in removed]
        self.assertEqual([i for i in seen if i not in removed], survivors)


class InvalidCursorTest(unittest.TestCase):
    def test_garbage_is_rejected(self):
        store = store_of([(1, "2026-03-01T09:00:00Z"), (2, "2026-03-01T10:00:00Z")])
        tokens = [
            "",
            "!!!",
            "not a cursor",
            b64("hello"),
            b64("[]"),
            b64('"abc"'),
            b64("null"),
            "é",
            b64("[1, 2, 3]"),
            b64("[7]"),
        ]
        for token in tokens:
            with self.assertRaises(InvalidCursor, msg=repr(token)):
                list_orders(store, limit=1, cursor=token)

    def test_invalid_cursor_is_a_value_error(self):
        self.assertTrue(issubclass(InvalidCursor, ValueError))

    def test_real_cursor_still_works(self):
        store = store_of([(1, "2026-03-01T09:00:00Z"), (2, "2026-03-01T10:00:00Z")])
        cursor = list_orders(store, limit=1)["next_cursor"]
        self.assertIsInstance(cursor, str)
        self.assertEqual([o["id"] for o in list_orders(store, limit=1, cursor=cursor)["orders"]], [1])


if __name__ == "__main__":
    unittest.main()
