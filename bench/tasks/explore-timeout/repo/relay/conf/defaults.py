"""Built-in defaults for every setting. Values are seconds unless named
otherwise."""

DEFAULTS = {
    "cache.ttl": 30,
    "cache.max_entries": 1024,
    "db.connect_timeout": 30,
    "db.pool_size": 4,
    "http.retries": 3,
    "http.timeout": 30,
    "http.user_agent": "relay/0.4",
    "metrics.flush_interval": 30,
    "worker.batch_size": 50,
    "worker.poll_interval": 30,
}
