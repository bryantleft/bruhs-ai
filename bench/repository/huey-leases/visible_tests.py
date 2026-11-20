"""Public smoke checks shipped with the coding task."""

import tempfile
import unittest
from pathlib import Path

from huey.storage import SqliteStorage


class LeasedDeliverySmokeTest(unittest.TestCase):
    def test_expired_reservation_redelivers_and_fences_the_old_token(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            storage = SqliteStorage("smoke", filename=str(Path(temporary) / "work.db"))
            try:
                storage.enqueue(b"payload")
                old, _ = storage.reserve(5, now=10)
                self.assertIsNone(storage.reserve(5, now=14))
                new, data = storage.reserve(5, now=15)
                self.assertEqual(data, b"payload")
                self.assertNotEqual(old, new)
                self.assertFalse(storage.ack(old, now=16))
                self.assertTrue(storage.ack(new, now=16))
            finally:
                storage.close()

    def test_release_leaves_the_task_queued(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            storage = SqliteStorage("smoke", filename=str(Path(temporary) / "work.db"))
            try:
                storage.enqueue(b"payload")
                token, _ = storage.reserve(5, now=10)
                self.assertTrue(storage.release(token, now=11))
                self.assertEqual(storage.queue_size(), 1)
                self.assertEqual(storage.reserve(5, now=11)[1], b"payload")
            finally:
                storage.close()
