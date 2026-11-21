"""Finite-int contract checks added before the second campaign."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from huey.storage import SqliteStorage


class FiniteIntegerContract(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.storage = SqliteStorage(
            "review", filename=str(Path(self.temporary.name) / "queue.db")
        )
        self.storage.enqueue(b"work")

    def tearDown(self) -> None:
        self.storage.close()
        self.temporary.cleanup()

    def test_finite_integer_duration_above_sqlite_int64(self) -> None:
        token, data = self.storage.reserve(2**80, now=1)
        self.assertEqual(data, b"work")
        self.assertTrue(self.storage.ack(token, now=2))

    def test_finite_integer_clock_above_sqlite_int64(self) -> None:
        now = 2**80
        token, data = self.storage.reserve(now, now=now)
        self.assertEqual(data, b"work")
        self.assertTrue(self.storage.ack(token, now=now))

    def test_finite_integer_renewal_above_sqlite_int64(self) -> None:
        token, _ = self.storage.reserve(10, now=1)
        self.assertTrue(self.storage.renew(token, 2**80, now=2))
        self.assertTrue(self.storage.ack(token, now=3))
