import asyncio
import unittest

from jobs import KeyedSerializer


async def settle():
    for _ in range(20):
        await asyncio.sleep(0)


class SerializerTest(unittest.TestCase):
    def test_jobs_of_a_key_run_in_order(self):
        async def main():
            s = KeyedSerializer()
            log = []

            def job(name):
                async def run():
                    log.append(f"start {name}")
                    await asyncio.sleep(0)
                    log.append(f"end {name}")
                    return name

                return run

            results = await asyncio.gather(*(s.run("acct", job(n)) for n in "abc"))
            self.assertEqual(results, ["a", "b", "c"])
            self.assertEqual(
                log, ["start a", "end a", "start b", "end b", "start c", "end c"]
            )

        asyncio.run(main())

    def test_keys_do_not_wait_for_each_other(self):
        async def main():
            s = KeyedSerializer()
            gate = asyncio.Event()

            async def slow():
                await gate.wait()
                return "slow"

            async def fast():
                return "fast"

            t = asyncio.ensure_future(s.run("a", slow))
            await settle()
            self.assertEqual(await s.run("b", fast), "fast")
            gate.set()
            self.assertEqual(await t, "slow")

        asyncio.run(main())

    def test_failure_goes_to_its_caller(self):
        async def main():
            s = KeyedSerializer()

            async def boom():
                raise LookupError("no such account")

            async def ok():
                return 1

            first = asyncio.ensure_future(s.run("a", boom))
            second = asyncio.ensure_future(s.run("a", ok))
            with self.assertRaises(LookupError):
                await first
            self.assertEqual(await second, 1)

        asyncio.run(main())

    def test_no_keys_left_when_idle(self):
        async def main():
            s = KeyedSerializer()

            async def ok():
                await asyncio.sleep(0)

            t = asyncio.ensure_future(s.run("b", ok))
            u = asyncio.ensure_future(s.run("a", ok))
            await asyncio.sleep(0)
            self.assertEqual(s.pending_keys(), ["a", "b"])
            await asyncio.gather(t, u)
            self.assertEqual(s.pending_keys(), [])
            await s.drain()

        asyncio.run(main())


if __name__ == "__main__":
    unittest.main()
