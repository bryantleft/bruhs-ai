import unittest

from relay.transport.backoff import delays
from relay.transport.http_client import HttpClient
from relay.transport.signing import sign, verify


class TransportTest(unittest.TestCase):
    def test_explicit_timeout_wins(self):
        self.assertEqual(HttpClient(timeout=3).timeout, 3)

    def test_backoff_caps(self):
        self.assertEqual(delays(8)[-1], 30)

    def test_signing(self):
        self.assertTrue(verify("k", b"x", sign("k", b"x")))
