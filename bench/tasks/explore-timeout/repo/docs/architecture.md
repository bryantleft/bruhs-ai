# Architecture

- `relay.worker` polls queues and hands messages to `relay.pipeline`.
- `relay.pipeline` runs filters, then delivers through `relay.transport`.
- `relay.cache` memoises routing lookups.
- `relay.db` stores delivery receipts.
- `relay.metrics` counts what happened.
