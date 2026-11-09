"""A TTL cache for routing lookups."""

import time

from relay.conf import Settings


class TtlCache:
    def __init__(self, settings=None, clock=time.monotonic):
        settings = settings or Settings()
        self.ttl = settings.get("cache.ttl")
        self.max_entries = settings.get("cache.max_entries")
        self._clock = clock
        self._entries = {}

    def get(self, key):
        hit = self._entries.get(key)
        if hit is None:
            return None
        value, expires = hit
        if self._clock() >= expires:
            del self._entries[key]
            return None
        return value

    def put(self, key, value):
        if len(self._entries) >= self.max_entries:
            self._entries.pop(next(iter(self._entries)))
        self._entries[key] = (value, self._clock() + self.ttl)
