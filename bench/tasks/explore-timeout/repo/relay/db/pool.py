"""A tiny connection pool around sqlite3."""

import sqlite3

from relay.conf import Settings


class Pool:
    def __init__(self, path, settings=None):
        settings = settings or Settings()
        self.path = path
        self.connect_timeout = settings.get("db.connect_timeout")
        self.size = settings.get("db.pool_size")
        self._free = []

    def acquire(self):
        if self._free:
            return self._free.pop()
        return sqlite3.connect(self.path, timeout=self.connect_timeout)

    def release(self, connection):
        if len(self._free) < self.size:
            self._free.append(connection)
        else:
            connection.close()
