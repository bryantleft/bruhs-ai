import asyncio
import random
import unittest

from jobs import KeyedSerializer


async def settle():
    for _ in range(30):
        await asyncio.sleep(0)


def run(coro):
    asyncio.run(asyncio.wait_for(coro, 3))


class Jobs:
    """Gated jobs that record when they start, finish and overlap."""

    def __init__(self):
        self.running = {}
        self.overlaps = []
        self.started = []
        self.cancelled = []
        self.gates = {}

    def job(self, key, name, *, fail=None, cleanup=None):
        gate = asyncio.Event()
        self.gates[name] = gate

        async def body():
            self.started.append(name)
            self.running[key] = self.running.get(key, 0) + 1
            if self.running[key] > 1:
                self.overlaps.append(name)
            try:
                await gate.wait()
                if fail is not None:
                    raise fail
                return name
            except asyncio.CancelledError:
                self.cancelled.append(name)
                if cleanup is not None:
                    await cleanup.wait()
                raise
            finally:
                self.running[key] -= 1

        return body

    def release(self, name):
        self.gates[name].set()


def submit(s, jobs, key, *names, **kw):
    return [asyncio.ensure_future(s.run(key, jobs.job(key, n, **kw))) for n in names]


class OrderTest(unittest.TestCase):
    def test_strict_order_without_overlap(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            tasks = submit(s, jobs, "k", "A", "B", "C", "D")
            for name in "ABCD":
                await settle()
                self.assertEqual(jobs.started[-1], name)
                jobs.release(name)
            self.assertEqual(await asyncio.gather(*tasks), list("ABCD"))
            self.assertEqual(jobs.overlaps, [])

        run(main())

    def test_keys_run_concurrently(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            tasks = [submit(s, jobs, key, key.upper())[0] for key in ("x", "y", "z")]
            await settle()
            self.assertEqual(sorted(jobs.started), ["X", "Y", "Z"])
            for name in "ZYX":
                jobs.release(name)
            self.assertEqual(await asyncio.gather(*tasks), ["X", "Y", "Z"])

        run(main())

    def test_next_job_does_not_overtake_on_hand_over(self):
        # A finishes while B is waiting; C arrives in the same event-loop
        # step. C must still wait for B.
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            ta, tb = submit(s, jobs, "k", "A", "B")
            await settle()
            jobs.release("A")
            (tc,) = submit(s, jobs, "k", "C")
            await settle()
            self.assertEqual(jobs.started, ["A", "B"])
            self.assertEqual(jobs.overlaps, [])
            jobs.release("B")
            await settle()
            self.assertEqual(jobs.started, ["A", "B", "C"])
            jobs.release("C")
            self.assertEqual(await asyncio.gather(ta, tb, tc), ["A", "B", "C"])
            self.assertEqual(jobs.overlaps, [])
            self.assertEqual(s.pending_keys(), [])

        run(main())


class DropTest(unittest.TestCase):
    def test_dropped_job_does_not_let_the_next_one_in(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            ta, tb, tc = submit(s, jobs, "k", "A", "B", "C")
            await settle()
            tb.cancel()
            await settle()
            self.assertEqual(jobs.started, ["A"], "C started while A was running")
            jobs.release("A")
            await settle()
            self.assertEqual(jobs.started, ["A", "C"])
            jobs.release("C")
            self.assertEqual(await ta, "A")
            self.assertEqual(await tc, "C")
            self.assertTrue(tb.cancelled())
            self.assertEqual(jobs.overlaps, [])

        run(main())

    def test_a_drop_hurts_no_other_caller(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            ta, tb, tc, td = submit(s, jobs, "k", "A", "B", "C", "D")
            await settle()
            tc.cancel()
            await settle()
            for name in "ABD":
                jobs.release(name)
                await settle()
            results = await asyncio.gather(ta, tb, td, return_exceptions=True)
            self.assertEqual(results, ["A", "B", "D"])
            self.assertTrue(tc.cancelled())
            self.assertEqual(jobs.started, ["A", "B", "D"])
            self.assertEqual(jobs.overlaps, [])

        run(main())

    def test_several_drops_in_a_row(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            ta, tb, tc, td = submit(s, jobs, "k", "A", "B", "C", "D")
            await settle()
            tc.cancel()
            tb.cancel()
            await settle()
            self.assertEqual(jobs.started, ["A"])
            jobs.release("A")
            await settle()
            self.assertEqual(jobs.started, ["A", "D"])
            jobs.release("D")
            self.assertEqual(await asyncio.gather(ta, td), ["A", "D"])
            self.assertEqual(jobs.overlaps, [])

        run(main())

    def test_dropping_the_last_waiting_job(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            ta, tb = submit(s, jobs, "k", "A", "B")
            await settle()
            tb.cancel()
            await settle()
            self.assertEqual(s.pending_keys(), ["k"])
            (tc,) = submit(s, jobs, "k", "C")
            await settle()
            self.assertEqual(jobs.started, ["A"], "C started while A was running")
            jobs.release("A")
            await settle()
            jobs.release("C")
            self.assertEqual(await asyncio.gather(ta, tc), ["A", "C"])
            self.assertEqual(jobs.overlaps, [])
            self.assertEqual(s.pending_keys(), [])

        run(main())

    def test_random_drops_keep_order(self):
        async def main():
            rng = random.Random(42)
            for _ in range(15):
                s, jobs = KeyedSerializer(), Jobs()
                names = [f"j{i}" for i in range(8)]
                tasks = dict(zip(names, submit(s, jobs, "k", *names)))
                await settle()
                dropped = set(rng.sample(names[1:], rng.randint(1, 5)))
                for name in sorted(dropped, key=lambda _: rng.random()):
                    tasks[name].cancel()
                    await asyncio.sleep(0)
                await settle()
                kept = [n for n in names if n not in dropped]
                for name in kept:
                    await settle()
                    self.assertEqual(jobs.started[-1], name)
                    jobs.release(name)
                await settle()
                self.assertEqual(jobs.started, kept)
                self.assertEqual(jobs.overlaps, [])
                for name in kept:
                    self.assertEqual(await tasks[name], name)
                for name in dropped:
                    self.assertTrue(tasks[name].cancelled())
                self.assertEqual(s.pending_keys(), [])

        run(main())


class RunningCancelTest(unittest.TestCase):
    def test_next_job_waits_for_the_cancelled_jobs_cleanup(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            cleanup = asyncio.Event()
            (ta,) = submit(s, jobs, "k", "A", cleanup=cleanup)
            (tb,) = submit(s, jobs, "k", "B")
            await settle()
            ta.cancel()
            await settle()
            self.assertEqual(jobs.cancelled, ["A"])
            self.assertEqual(jobs.started, ["A"], "B started during A's cleanup")
            cleanup.set()
            await settle()
            self.assertEqual(jobs.started, ["A", "B"])
            jobs.release("B")
            self.assertEqual(await tb, "B")
            self.assertTrue(ta.cancelled())

        run(main())

    def test_failure_behind_a_drop(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            (ta,) = submit(s, jobs, "k", "A", fail=KeyError("A"))
            tb, tc = submit(s, jobs, "k", "B", "C")
            await settle()
            tb.cancel()
            await settle()
            jobs.release("A")
            await settle()
            jobs.release("C")
            with self.assertRaises(KeyError):
                await ta
            self.assertEqual(await tc, "C")
            self.assertEqual(jobs.started, ["A", "C"])

        run(main())


class DrainTest(unittest.TestCase):
    def test_drain_waits_for_earlier_jobs_only(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            ta, tb = submit(s, jobs, "k1", "A", "B")
            (tc,) = submit(s, jobs, "k2", "C")
            await settle()
            d = asyncio.ensure_future(s.drain())
            await settle()
            (td,) = submit(s, jobs, "k1", "D")
            jobs.release("A")
            jobs.release("C")
            await settle()
            self.assertFalse(d.done())
            jobs.release("B")
            await settle()
            self.assertTrue(d.done(), "drain waited for a job submitted after it")
            self.assertIn("D", jobs.started)
            jobs.release("D")
            self.assertEqual(await asyncio.gather(ta, tb, tc, td), ["A", "B", "C", "D"])

        run(main())

    def test_drain_is_not_fooled_by_a_drop(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            ta, tb = submit(s, jobs, "k", "A", "B")
            await settle()
            d = asyncio.ensure_future(s.drain())
            await settle()
            tb.cancel()
            await settle()
            self.assertFalse(d.done(), "drain returned while A was still running")
            jobs.release("A")
            await settle()
            self.assertTrue(d.done())
            self.assertEqual(await ta, "A")

        run(main())

    def test_cancelling_drain_affects_no_job(self):
        async def main():
            s, jobs = KeyedSerializer(), Jobs()
            ta, tb = submit(s, jobs, "k", "A", "B")
            await settle()
            d = asyncio.ensure_future(s.drain())
            await settle()
            d.cancel()
            await settle()
            self.assertFalse(ta.done())
            jobs.release("A")
            await settle()
            jobs.release("B")
            self.assertEqual(await asyncio.gather(ta, tb), ["A", "B"])
            self.assertEqual(jobs.overlaps, [])
            self.assertEqual(s.pending_keys(), [])

        run(main())

    def test_drain_when_idle(self):
        async def main():
            s = KeyedSerializer()
            await asyncio.wait_for(s.drain(), 0.5)

        run(main())


if __name__ == "__main__":
    unittest.main()
