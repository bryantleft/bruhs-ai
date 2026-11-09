import os
import tempfile
import unittest

from relay.conf import Settings


class SettingsTest(unittest.TestCase):
    def test_file_overrides_defaults(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "relay.ini")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write("[cache]\nttl = 5\n")
            self.assertEqual(Settings.load(path).get("cache.ttl"), 5)

    def test_overrides(self):
        self.assertEqual(Settings().with_overrides(db__pool_size=9).get("db.pool_size"), 9)
