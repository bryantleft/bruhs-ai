# kvstore

A small in-memory key-value store used by the settings service.

- `kvstore.store.Store` — the store: `get`, `scan`, `put`, `put_many`, `delete`.
- `kvstore.cache.QueryCache` — memoises prefix scans for the request path.
- `kvstore.follower.Follower` — a replica fed from the store's change log.
- `kvstore.keys` — key normalisation.
