"""Memoised prefix scans for the request path."""

from .keys import normalize_prefix


class QueryCache:
    def __init__(self, store):
        self._store = store
        self._entries = {}

    # A cached scan is reused for as long as Store.version is the version it
    # was computed at; nothing else invalidates it. So every change to the
    # store's data must bump the version, i.e. go through Store._commit. A
    # write that edits Store._data directly is invisible here and the cache
    # keeps serving the old result.
    def scan(self, prefix):
        prefix = normalize_prefix(prefix)
        hit = self._entries.get(prefix)
        if hit is not None and hit[0] == self._store.version:
            return list(hit[1])
        result = self._store.scan(prefix)
        self._entries[prefix] = (self._store.version, result)
        return list(result)
