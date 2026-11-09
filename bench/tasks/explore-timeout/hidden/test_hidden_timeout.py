import os
import tempfile
import unittest

from relay.cache.ttl_cache import TtlCache
from relay.conf import Settings
from relay.conf.defaults import DEFAULTS
from relay.db.pool import Pool
from relay.metrics.counters import Counters
from relay.transport import backoff
from relay.transport.http_client import HttpClient
from relay.worker import shutdown
from relay.worker.poller import Poller


class TimeoutTest(unittest.TestCase):
    def test_default_is_45(self):
        self.assertEqual(DEFAULTS["http.timeout"], 45)
        self.assertEqual(Settings().get("http.timeout"), 45)
        self.assertEqual(HttpClient().timeout, 45)

    def test_explicit_and_file_values_win(self):
        self.assertEqual(HttpClient(timeout=7).timeout, 7)
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "relay.ini")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write("[http]\ntimeout = 12\n")
            self.assertEqual(HttpClient(Settings.load(path)).timeout, 12)

    def test_other_settings_unchanged(self):
        expected = {
            "cache.ttl": 30,
            "cache.max_entries": 1024,
            "db.connect_timeout": 30,
            "db.pool_size": 4,
            "http.retries": 3,
            "http.user_agent": "relay/0.4",
            "metrics.flush_interval": 30,
            "worker.batch_size": 50,
            "worker.poll_interval": 30,
        }
        for key, value in expected.items():
            self.assertEqual(DEFAULTS[key], value, key)
        self.assertEqual(set(DEFAULTS), set(expected) | {"http.timeout"})
        self.assertEqual(TtlCache().ttl, 30)
        self.assertEqual(Pool(":memory:").connect_timeout, 30)
        self.assertEqual(Poller([]).interval, 30)
        self.assertEqual(Counters().flush_interval, 30)
        self.assertEqual(backoff.MAX_DELAY, 30)
        self.assertEqual(shutdown.GRACE_SECONDS, 30)


if __name__ == "__main__":
    unittest.main()
