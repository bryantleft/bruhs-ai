"""Polls a queue on an interval."""

from relay.conf import Settings


class Poller:
    def __init__(self, queue, settings=None):
        settings = settings or Settings()
        self.queue = queue
        self.interval = settings.get("worker.poll_interval")
        self.batch_size = settings.get("worker.batch_size")

    def poll_once(self):
        batch = []
        while self.queue and len(batch) < self.batch_size:
            batch.append(self.queue.pop(0))
        return batch
