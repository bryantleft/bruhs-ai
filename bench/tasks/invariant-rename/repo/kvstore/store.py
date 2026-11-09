"""The store."""

import copy

from .keys import normalize_key, normalize_prefix
from .ops import apply_entry


class Store:
    def __init__(self):
        self._data = {}
        self._version = 0
        self._log = []

    @property
    def version(self):
        return self._version

    @property
    def log(self):
        return copy.deepcopy(self._log)

    def snapshot(self):
        return dict(self._data)

    def get(self, key, default=None):
        return self._data.get(normalize_key(key), default)

    def scan(self, prefix):
        """(key, value) pairs whose key starts with `prefix`, sorted by key."""
        prefix = normalize_prefix(prefix)
        return sorted((k, v) for k, v in self._data.items() if k.startswith(prefix))

    def put(self, key, value):
        self._commit({"op": "put", "key": normalize_key(key), "value": value})

    def put_many(self, mapping):
        ops = [{"op": "put", "key": normalize_key(k), "value": v} for k, v in mapping.items()]
        if ops:
            self._commit({"op": "batch", "ops": ops})

    def delete(self, key):
        key = normalize_key(key)
        if key not in self._data:
            raise KeyError(key)
        self._commit({"op": "delete", "key": key})

    def _commit(self, entry):
        apply_entry(self._data, entry)
        self._version += 1
        self._log.append(copy.deepcopy(entry))
