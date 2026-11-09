import unittest

from kvstore.cache import QueryCache
from kvstore.follower import Follower
from kvstore.store import Store


def seeded():
    store = Store()
    store.put_many({"user:ann": 1, "user:bob": 2, "team:x": 3})
    return store


class HiddenRenameTest(unittest.TestCase):
    def test_renames_and_counts(self):
        store = seeded()
        self.assertEqual(store.rename_prefix("user:", "member:"), 2)
        self.assertEqual(store.snapshot(), {"member:ann": 1, "member:bob": 2, "team:x": 3})

    def test_prefixes_are_case_insensitive(self):
        store = seeded()
        self.assertEqual(store.rename_prefix("USER:", "Member:"), 2)
        self.assertEqual(store.get("member:ann"), 1)
        self.assertIsNone(store.get("user:ann"))
        self.assertEqual(store.scan("member:"), [("member:ann", 1), ("member:bob", 2)])

    def test_one_commit_per_call(self):
        store = seeded()
        version, entries = store.version, len(store.log)
        store.rename_prefix("user:", "member:")
        self.assertEqual(store.version, version + 1)
        self.assertEqual(len(store.log), entries + 1)

    def test_cache_is_invalidated(self):
        store = seeded()
        cache = QueryCache(store)
        self.assertEqual(len(cache.scan("user:")), 2)
        self.assertEqual(cache.scan("member:"), [])
        store.rename_prefix("user:", "member:")
        self.assertEqual(cache.scan("user:"), [])
        self.assertEqual(cache.scan("member:"), [("member:ann", 1), ("member:bob", 2)])

    def test_follower_matches(self):
        store = seeded()
        follower = Follower()
        follower.catch_up(store.log)
        store.rename_prefix("user:", "member:")
        follower.catch_up(store.log)
        self.assertEqual(follower.data, store.snapshot())
        fresh = Follower()
        fresh.catch_up(store.log)
        self.assertEqual(fresh.data, store.snapshot())

    def test_overlapping_prefixes(self):
        # "a1" -> "ab1" lands on a key that is itself renamed ("ab1" -> "abb1").
        store = Store()
        store.put_many({"a1": "first", "ab1": "second", "b": "other"})
        self.assertEqual(store.rename_prefix("a", "ab"), 2)
        expected = {"ab1": "first", "abb1": "second", "b": "other"}
        self.assertEqual(store.snapshot(), expected)
        follower = Follower()
        follower.catch_up(store.log)
        self.assertEqual(follower.data, expected)

    def test_collision_changes_nothing(self):
        store = seeded()
        store.put("member:bob", 99)
        before, version, entries = store.snapshot(), store.version, len(store.log)
        with self.assertRaises(KeyError):
            store.rename_prefix("user:", "member:")
        self.assertEqual(store.snapshot(), before)
        self.assertEqual(store.version, version)
        self.assertEqual(len(store.log), entries)

    def test_no_match_changes_nothing(self):
        store = seeded()
        version, entries = store.version, len(store.log)
        self.assertEqual(store.rename_prefix("nobody:", "x:"), 0)
        self.assertEqual(store.version, version)
        self.assertEqual(len(store.log), entries)

    def test_existing_behaviour_kept(self):
        store = seeded()
        store.delete("team:x")
        self.assertEqual(store.scan(""), [("user:ann", 1), ("user:bob", 2)])
        with self.assertRaises(KeyError):
            store.delete("team:x")


if __name__ == "__main__":
    unittest.main()
