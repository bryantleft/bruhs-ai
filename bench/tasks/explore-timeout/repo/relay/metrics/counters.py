"""In-memory counters flushed on an interval."""

from collections import Counter

from relay.conf import Settings


class Counters:
    def __init__(self, settings=None):
        settings = settings or Settings()
        self.flush_interval = settings.get("metrics.flush_interval")
        self._counts = Counter()

    def incr(self, name, by=1):
        self._counts[name] += by

    def flush(self):
        counts, self._counts = dict(self._counts), Counter()
        return counts
