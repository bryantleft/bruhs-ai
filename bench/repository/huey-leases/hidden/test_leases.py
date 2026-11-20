"""Independent public-contract checks against real SQLite files."""

from __future__ import annotations

import json
import math
import os
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path

from huey import SqliteHuey
from huey.storage import SqliteStorage


class LeaseStorageTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(dir=os.getcwd())
        self.filename = str(Path(self.temporary.name) / "queue.db")
        self.instances = []
        self.s = self.store()

    def tearDown(self) -> None:
        for instance in self.instances:
            instance.close()
        self.temporary.cleanup()

    def store(self, name: str = "primary", **kwargs: object) -> SqliteStorage:
        storage = SqliteStorage(name, filename=self.filename, **kwargs)
        self.instances.append(storage)
        return storage

    def test_reservation_keeps_queue_and_priority_order(self) -> None:
        for data, priority in [(b"a", 0), (b"b", 3), (b"c", 3)]:
            self.s.enqueue(data, priority)
        first = self.s.reserve(10, now=100)
        second = self.s.reserve(10, now=100)
        self.assertEqual((first[1], second[1]), (b"b", b"c"))
        self.assertIsInstance(first[0], str)
        self.assertTrue(first[0])
        self.assertNotEqual(first[0], second[0])
        self.assertEqual(self.s.queue_size(), 3)
        self.assertEqual(self.s.enqueued_items(), [b"b", b"c", b"a"])
        self.assertEqual(self.s.enqueued_items(2), [b"b", b"c"])
        self.assertTrue(self.s.ack(first[0], now=101))
        self.assertEqual(self.s.enqueued_items(), [b"c", b"a"])

    def test_exact_expiry_and_new_token_fence_every_old_operation(self) -> None:
        self.s.enqueue(b"same")
        old, data = self.s.reserve(5, now=10)
        self.assertEqual(data, b"same")
        self.assertIsNone(self.store().reserve(5, now=14.999))
        newer, data = self.store().reserve(7, now=15)
        self.assertEqual(data, b"same")
        self.assertNotEqual(old, newer)
        self.assertFalse(self.s.ack(old, now=16))
        self.assertFalse(self.s.release(old, now=16))
        self.assertFalse(self.s.renew(old, 100, now=16))
        self.assertIsNone(self.s.reserve(1, now=16))
        self.assertTrue(self.s.ack(newer, now=16))
        self.assertEqual(self.s.queue_size(), 0)

    def test_expired_token_cannot_act_even_before_redelivery(self) -> None:
        self.s.enqueue(b"x")
        token, _ = self.s.reserve(2, now=5)
        self.assertFalse(self.s.ack(token, now=7))
        self.assertFalse(self.s.release(token, now=7))
        self.assertFalse(self.s.renew(token, 10, now=7))
        self.assertEqual(self.s.queue_size(), 1)
        self.assertEqual(self.s.reserve(2, now=7)[1], b"x")

    def test_renewal_can_extend_or_shorten_without_reordering(self) -> None:
        self.s.enqueue(b"a")
        self.s.enqueue(b"b")
        token, _ = self.s.reserve(2, now=1)
        self.assertTrue(self.store().renew(token, 20, now=2))
        other, data = self.s.reserve(10, now=3)
        self.assertEqual(data, b"b")
        self.assertTrue(self.s.renew(token, 1, now=4))
        new, data = self.s.reserve(2, now=5)
        self.assertEqual(data, b"a")
        self.assertNotEqual(new, token)
        self.assertEqual(self.s.enqueued_items(), [b"a", b"b"])
        self.assertTrue(self.s.ack(other, now=5))

    def test_release_redelivers_in_original_order_with_a_new_token(self) -> None:
        self.s.enqueue(b"same")
        self.s.enqueue(b"same")
        one, _ = self.s.reserve(10, now=0)
        two, _ = self.s.reserve(10, now=0)
        self.assertNotEqual(one, two)
        self.assertTrue(self.store().release(one, now=1))
        newer, data = self.s.reserve(10, now=1)
        self.assertEqual(data, b"same")
        self.assertNotIn(newer, [one, two])
        self.assertFalse(self.s.ack(one, now=2))
        self.assertTrue(self.s.ack(newer, now=2))
        self.assertTrue(self.s.ack(two, now=2))

    def test_real_clock_and_legacy_dequeue_respect_a_live_lease(self) -> None:
        self.s.enqueue(b"reserved", 4)
        self.s.enqueue(b"available", 1)
        token, _ = self.s.reserve(60)
        self.assertEqual(self.store().dequeue(), b"available")
        self.assertIsNone(self.s.dequeue())
        self.assertTrue(self.s.ack(token))

    def test_legacy_dequeue_can_remove_expired_work_and_invalidates_token(self) -> None:
        self.s.enqueue(b"expired")
        old, _ = self.s.reserve(1, now=0)
        self.assertEqual(self.store().dequeue(), b"expired")
        self.s.enqueue(b"new")
        self.assertFalse(self.s.ack(old))
        self.assertFalse(self.s.release(old))
        self.assertEqual(self.s.reserve(60)[1], b"new")

    def test_reopen_preserves_live_reservation_and_expiry_recovery(self) -> None:
        self.s.enqueue(b"durable")
        old, _ = self.s.reserve(5, now=100)
        self.s.close()
        reopened = self.store()
        self.assertIsNone(reopened.reserve(2, now=104))
        new, data = reopened.reserve(2, now=105)
        self.assertEqual(data, b"durable")
        self.assertNotEqual(old, new)
        self.assertFalse(reopened.ack(old, now=105))
        self.assertTrue(reopened.ack(new, now=105))

    def test_namespaces_and_flush_preserve_other_queues_and_other_data(self) -> None:
        other = self.store("other")
        self.s.enqueue(b"own")
        other.enqueue(b"foreign")
        self.s.put_data("result", b"saved")
        self.s.incr("count", 7)
        when = datetime(2030, 1, 1)
        self.s.add_to_schedule(b"later", when)
        own, _ = self.s.reserve(10, now=1)
        foreign, _ = other.reserve(10, now=1)
        self.assertFalse(self.s.ack(foreign, now=2))
        self.assertFalse(self.s.release(foreign, now=2))
        self.assertFalse(self.s.renew(foreign, 100, now=2))
        self.s.flush_queue()
        self.assertFalse(self.s.ack(own, now=2))
        self.assertEqual(other.queue_size(), 1)
        self.assertIsNone(other.reserve(10, now=2))
        self.assertTrue(other.ack(foreign, now=2))
        self.assertEqual(self.s.peek_data("result"), b"saved")
        self.assertEqual(self.s.incr("count", 0), 7)
        self.assertEqual(self.s.scheduled_items(), [b"later"])

    def test_flush_refill_never_reuses_a_reservation_token(self) -> None:
        seen = set()
        for _ in range(4):
            self.s.enqueue(b"identical")
            token, _ = self.s.reserve(100, now=10)
            self.assertNotIn(token, seen)
            for old in seen:
                self.assertFalse(self.s.ack(old, now=11))
            seen.add(token)
            self.s.flush_queue()

    def test_bad_numbers_fail_before_mutation_including_unknown_tokens(self) -> None:
        self.s.enqueue(b"payload")
        bad = [True, False, "1", None, math.nan, math.inf, -math.inf, 10**1000]
        for value in bad + [0, -1]:
            with self.subTest(duration=repr(value)):
                with self.assertRaises(ValueError):
                    self.s.reserve(value, now=1)
                with self.assertRaises(ValueError):
                    self.s.renew("unknown", value, now=1)
        for value in [True, False, "1", math.nan, math.inf, -math.inf, 10**1000]:
            with self.subTest(now=repr(value)):
                for operation in (
                    lambda value=value: self.s.reserve(1, now=value),
                    lambda value=value: self.s.ack("unknown", now=value),
                    lambda value=value: self.s.renew("unknown", 1, now=value),
                    lambda value=value: self.s.release("unknown", now=value),
                ):
                    with self.assertRaises(ValueError):
                        operation()
        with self.assertRaises(ValueError):
            self.s.reserve(1e308, now=1e308)
        for token in (None, "", b"unknown", False, object()):
            self.assertFalse(self.s.ack(token, now=1))
            self.assertFalse(self.s.release(token, now=1))
            self.assertFalse(self.s.renew(token, 1, now=1))
        self.assertEqual(self.s.queue_size(), 1)
        self.assertEqual(self.s.reserve(0.5, now=-1)[1], b"payload")

    def test_empty_queue_and_empty_binary_payload_are_distinct(self) -> None:
        self.assertIsNone(self.s.reserve(1, now=1))
        self.s.enqueue(b"")
        token, data = self.s.reserve(1, now=1)
        self.assertEqual(data, b"")
        self.assertTrue(self.s.ack(token, now=1))
        self.assertFalse(self.s.ack(token, now=1))

    def test_failed_ack_and_flush_roll_back_reservations_together(self) -> None:
        for operation in ("ack", "flush"):
            with self.subTest(operation=operation):
                self.s.flush_queue()
                self.s.enqueue(b"must survive")
                token, _ = self.s.reserve(10, now=10)
                connection = sqlite3.connect(self.filename)
                connection.execute(
                    "create trigger deny_task_delete before delete on task begin select raise(abort, 'owned failure'); end"
                )
                connection.commit()
                try:
                    with self.assertRaises(sqlite3.DatabaseError):
                        if operation == "ack":
                            self.s.ack(token, now=11)
                        else:
                            self.s.flush_queue()
                finally:
                    connection.execute("drop trigger deny_task_delete")
                    connection.commit()
                    connection.close()
                self.assertEqual(self.s.queue_size(), 1)
                self.assertIsNone(self.store().reserve(5, now=11))
                self.assertTrue(self.s.ack(token, now=11))

    def test_two_connections_claim_each_task_once_under_contention(self) -> None:
        other = self.store()
        for i in range(12):
            self.s.enqueue(str(i).encode())
        barrier = threading.Barrier(3)

        def claim(storage: SqliteStorage) -> list[tuple[str, bytes]]:
            barrier.wait(timeout=60)
            result = []
            while True:
                item = storage.reserve(100, now=1)
                if item is None:
                    return result
                result.append(item)

        with ThreadPoolExecutor(max_workers=2) as pool:
            a = pool.submit(claim, self.s)
            b = pool.submit(claim, other)
            barrier.wait(timeout=60)
            results = a.result(timeout=60) + b.result(timeout=60)
        self.assertEqual(
            sorted(data for _, data in results),
            sorted(str(i).encode() for i in range(12)),
        )
        self.assertEqual(len({token for token, _ in results}), 12)
        self.assertEqual(self.s.queue_size(), 12)
        for token, _ in results:
            self.assertTrue(other.ack(token, now=2))

    def test_crashed_consumer_reservation_is_visible_to_another_process(self) -> None:
        self.s.enqueue(b"recover")
        code = "import os,sys,json;sys.path.insert(0,sys.argv[1]);from huey.storage import SqliteStorage;s=SqliteStorage('primary',filename=sys.argv[2]);token,data=s.reserve(5,now=10);print(json.dumps([token,data.decode()]),flush=True);os._exit(0)"
        child = subprocess.run(
            [
                sys.executable,
                "-I",
                "-S",
                "-c",
                code,
                str(Path(__file__).resolve().parents[1]),
                self.filename,
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertEqual(child.returncode, 0, child.stderr)
        token, data = json.loads(child.stdout)
        self.assertEqual(data, "recover")
        self.assertIsNone(self.store().reserve(3, now=14))
        new, payload = self.s.reserve(3, now=15)
        self.assertEqual(payload, b"recover")
        self.assertNotEqual(token, new)
        self.assertFalse(self.s.ack(token, now=16))
        self.assertTrue(self.s.ack(new, now=16))


class LeaseApiTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(dir=os.getcwd())
        self.h = SqliteHuey(
            "api", filename=str(Path(self.temporary.name) / "api.db"), immediate=False
        )
        self.addCleanup(self.temporary.cleanup)
        self.addCleanup(self.h.storage.close)

    def test_high_level_reserve_roundtrip_and_explicit_acknowledgment(self) -> None:
        @self.h.task()
        def add(a: int, b: int) -> int:
            return a + b

        expected = add(3, 4)
        token, task = self.h.reserve_task(10, now=1)
        self.assertEqual(task.id, expected.id)
        self.assertEqual(task.args, (3, 4))
        self.assertTrue(self.h.renew_task(token, 20, now=2))
        self.assertEqual(self.h.pending_count(), 1)
        self.assertTrue(self.h.release_task(token, now=3))
        newer, task = self.h.reserve_task(10, now=3)
        self.assertFalse(self.h.ack_task(token, now=4))
        self.assertEqual(self.h.execute(task), 7)
        self.assertEqual(self.h.pending_count(), 1)
        self.assertTrue(self.h.ack_task(newer, now=4))
        self.assertEqual(self.h.pending_count(), 0)
        self.assertIsNone(self.h.reserve_task(1, now=4))

    def test_bad_serialization_releases_the_new_reservation(self) -> None:
        self.h.storage.enqueue(b"not a serialized task")
        with self.assertRaises(Exception):  # noqa: B017 - serializer exception type is not part of the contract.
            self.h.reserve_task(10, now=1)
        token, payload = self.h.storage.reserve(10, now=2)
        self.assertEqual(payload, b"not a serialized task")
        self.assertTrue(self.h.ack_task(token, now=2))

    def test_immediate_mode_rejects_all_new_methods(self) -> None:
        self.h.immediate = True
        for operation in (
            lambda: self.h.reserve_task(1),
            lambda: self.h.ack_task("x"),
            lambda: self.h.renew_task("x", 1),
            lambda: self.h.release_task("x"),
        ):
            with self.assertRaises(ValueError):
                operation()


class LeaseDatabaseTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(dir=os.getcwd())
        self.filename = str(Path(self.temporary.name) / "legacy.db")
        self.addCleanup(self.temporary.cleanup)

    def seed_legacy(self) -> None:
        connection = sqlite3.connect(self.filename)
        connection.executescript("""
            create table task (id integer primary key, queue text not null,
                               data blob not null, priority real not null default 0);
            create table kv (queue text, key text, value blob, primary key(queue,key));
            create table schedule (id integer primary key, queue text, data blob, timestamp real);
            create table counter (queue text, key text, value integer, primary key(queue,key));
            insert into task values (41,'primary',X'6c6567616379',7);
            insert into task values (42,'other',X'6f74686572',2);
            insert into kv values ('primary','result',X'7361766564');
            insert into schedule values (51,'primary',X'6c61746572',1900000000);
            insert into counter values ('primary','counter',9);
        """)
        connection.commit()
        connection.close()

    def test_old_database_preserves_its_data_and_can_use_leases(self) -> None:
        self.seed_legacy()
        storage = SqliteStorage("primary", filename=self.filename)
        self.addCleanup(storage.close)
        token, data = storage.reserve(10, now=1)
        self.assertEqual(data, b"legacy")
        self.assertEqual(storage.peek_data("result"), b"saved")
        self.assertEqual(storage.scheduled_items(), [b"later"])
        self.assertEqual(storage.incr("counter", 0), 9)
        self.assertTrue(storage.ack(token, now=2))
        other = SqliteStorage("other", filename=self.filename)
        self.addCleanup(other.close)
        self.assertEqual(other.dequeue(), b"other")

    def test_create_tables_false_requires_explicit_initialization(self) -> None:
        storage = SqliteStorage("primary", filename=self.filename, create_tables=False)
        self.addCleanup(storage.close)
        with self.assertRaises(sqlite3.OperationalError):
            storage.reserve(5, now=1)
        connection = sqlite3.connect(self.filename)
        self.assertEqual(
            connection.execute(
                "select name from sqlite_master where type='table'"
            ).fetchall(),
            [],
        )
        connection.close()
        storage.initialize_schema()
        storage.enqueue(b"ready")
        token, data = storage.reserve(5, now=1)
        self.assertEqual(data, b"ready")
        self.assertTrue(storage.ack(token, now=1))

    def test_parallel_open_of_an_old_database_reserves_each_item_once(self) -> None:
        self.seed_legacy()
        barrier = threading.Barrier(3)

        def open_and_reserve() -> object:
            barrier.wait(timeout=60)
            storage = SqliteStorage("primary", filename=self.filename)
            try:
                return storage.reserve(10, now=1)
            finally:
                storage.close()

        with ThreadPoolExecutor(max_workers=2) as pool:
            futures = [pool.submit(open_and_reserve) for _ in range(2)]
            barrier.wait(timeout=60)
            results = [f.result(timeout=60) for f in futures]
        claimed = [item for item in results if item is not None]
        self.assertEqual(len(claimed), 1)
        self.assertEqual(claimed[0][1], b"legacy")

    def test_strict_fifo_keeps_its_autoincrement_schema(self) -> None:
        storage = SqliteStorage("primary", filename=self.filename, strict_fifo=True)
        self.addCleanup(storage.close)
        storage.enqueue(b"first")
        first_id = storage.sql("select id from task", results=True)[0][0]
        token, _ = storage.reserve(10, now=1)
        self.assertTrue(storage.ack(token, now=1))
        storage.enqueue(b"second")
        second_id = storage.sql("select id from task", results=True)[0][0]
        self.assertGreater(second_id, first_id)
        self.assertEqual(storage.reserve(10, now=1)[1], b"second")
