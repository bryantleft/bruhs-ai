"""The keyed serializer."""

import asyncio


class KeyedSerializer:
    """Runs async jobs one at a time per key.

    `await s.run(key, job)` calls `job()` (a zero-argument async function)
    and returns its result, or raises its exception. We use it so that two
    transfers on the same account never run at the same time, while
    transfers on different accounts do not wait for each other. The rules,
    all of which callers rely on:

    1. One at a time, in order. Jobs of the same key never overlap: a job
       starts only after every job of that key whose `run` was called
       earlier has finished or been dropped (rule 3), and the jobs of a key
       start in the order their `run` calls were made.

    2. Keys are independent. Jobs of different keys run concurrently; a
       slow job never delays a job of another key.

    3. Cancelling a waiting job. If the caller of `run` is cancelled while
       its job is still waiting (it has not started), the job is dropped:
       it never starts, and the caller gets `CancelledError`. The other jobs
       of the key are not affected: a job behind the dropped one still
       starts only after every job before it has finished (rule 1), and no
       other caller is cancelled or gets an error because of the drop.

    4. Cancelling a running job. If the caller is cancelled while its job
       is running, the job is cancelled (`CancelledError` is raised inside
       it). A job may catch that to clean up, and may await while doing so;
       the next job of the key starts only after the cancelled job has
       completely finished.

    5. Failures. A job that raises passes its exception to its own caller
       only; the next job of the key starts as usual.

    6. No leftovers. `pending_keys()` returns the sorted list of keys that
       have a waiting or running job. Once every job of a key has finished
       or been dropped, the serializer keeps no state for that key.

    7. Draining. `await s.drain()` returns once every job whose `run` was
       called before `drain` was called has finished or been dropped. It
       does not wait for jobs whose `run` is called after it. Cancelling a
       `drain()` affects no job.
    """

    def __init__(self):
        # key -> Future that is done once the last job queued for the key
        # is through; each new job waits on its predecessor's future. A
        # job's future is set only when the job has finished, or, for a
        # dropped job, once its predecessor is through: so the chain never
        # lets a job start before every earlier job is done.
        self._tails = {}

    async def run(self, key, job):
        loop = asyncio.get_running_loop()
        prev = self._tails.get(key)
        done = loop.create_future()
        self._tails[key] = done
        if prev is not None:
            try:
                # shield: our cancellation must not cancel prev, which the
                # job before us owns.
                await asyncio.shield(prev)
            except asyncio.CancelledError:
                # Dropped while waiting: hand the turn on only once the job
                # before us is through.
                prev.add_done_callback(lambda _f: self._finish(key, done))
                raise
        try:
            return await job()
        finally:
            self._finish(key, done)

    def _finish(self, key, done):
        if not done.done():
            done.set_result(None)
        if self._tails.get(key) is done:
            del self._tails[key]

    def pending_keys(self):
        return sorted(self._tails)

    async def drain(self):
        tails = list(self._tails.values())
        if tails:
            # shield: cancelling drain must not cancel the jobs' futures.
            await asyncio.shield(asyncio.gather(*tails))
