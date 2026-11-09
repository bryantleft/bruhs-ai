# Configuration

Settings come from three layers, later layers winning:

1. built-in defaults,
2. an optional INI file passed to `Settings.load(path)`,
3. keyword overrides given to the component itself.

Keys are dotted: `cache.ttl`, `worker.poll_interval`, `db.connect_timeout`
and so on. Every timeout is in seconds.
