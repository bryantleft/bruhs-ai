import unittest

from relay.cache.ttl_cache import TtlCache


class CacheTest(unittest.TestCase):
    def test_expiry(self):
        now = [0.0]
        cache = TtlCache(clock=lambda: now[0])
        cache.put("a", 1)
        self.assertEqual(cache.get("a"), 1)
        now[0] = 31
        self.assertIsNone(cache.get("a"))
