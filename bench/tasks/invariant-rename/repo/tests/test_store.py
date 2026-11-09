import unittest

from kvstore.cache import QueryCache
from kvstore.follower import Follower
from kvstore.store import Store


class StoreTest(unittest.TestCase):
    def test_put_get_is_case_insensitive(self):
        store = Store()
        store.put(" User:Ann ", 1)
        self.assertEqual(store.get("user:ann"), 1)

    def test_scan(self):
        store = Store()
        store.put_many({"user:b": 2, "user:a": 1, "team:x": 3})
        self.assertEqual(store.scan("USER:"), [("user:a", 1), ("user:b", 2)])

    def test_cache_sees_writes(self):
        store = Store()
        cache = QueryCache(store)
        store.put("user:a", 1)
        self.assertEqual(cache.scan("user:"), [("user:a", 1)])
        store.put("user:b", 2)
        self.assertEqual(cache.scan("user:"), [("user:a", 1), ("user:b", 2)])

    def test_follower_replays_log(self):
        store = Store()
        store.put_many({"a": 1, "b": 2})
        store.delete("a")
        follower = Follower()
        follower.catch_up(store.log)
        self.assertEqual(follower.data, store.snapshot())


if __name__ == "__main__":
    unittest.main()
