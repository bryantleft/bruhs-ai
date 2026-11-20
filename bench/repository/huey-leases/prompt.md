Add durable leased delivery to Huey's standard-library SQLite backend in this repository. Today dequeue removes work before a consumer can confirm completion. A crashed consumer should be able to leave a reservation that expires and can be delivered again.

Implement the following contract. Keep this opt-in: existing users that never reserve tasks retain their current behavior. Use no new runtime dependency.

Storage API on `huey.storage.SqliteStorage`:

* `reserve(lease_for, now=None)` returns `(token, data)` for one eligible task, or `None` if none is eligible. Data is the unchanged bytes payload. Eligible tasks follow the existing priority-descending, id-ascending ordering. The task stays in the queue until acknowledged. A leased task is ineligible until its deadline; a deadline equal to now is expired.
* `ack(token, now=None)` atomically deletes that token's live reservation and its task and returns `True`. Unknown, foreign-queue, expired or superseded tokens return `False` without changing current work.
* `renew(token, lease_for, now=None)` moves a live reservation's deadline to `now + lease_for`, keeps the token, and returns `True`. The same invalid/stale cases return `False`. Shortening a lease is permitted.
* `release(token, now=None)` makes a live reserved task eligible immediately, without removing or reordering it, and returns `True`. Invalid/stale cases return `False`.

For reserve and renew, lease_for must be a positive finite int or float. Every now argument, when supplied, must be a finite int or float. Booleans and other types are invalid. Raise ValueError for invalid values, including a non-finite resulting deadline, before mutating storage. Validate numerical arguments even if the queue is empty or the token is unknown. With now omitted, use epoch seconds from the wall clock. This contract uses real-number epoch seconds, not datetime objects.

Tokens must be nonempty strings; malformed token arguments return False after numerical validation. Each successful reservation gets a fresh opaque nonempty string token, distinct even across repeated delivery of identical payloads, flush/refill and different storage instances. Every operation is scoped to the storage's queue name. Reservation and acknowledgment must be atomic across independent connections and threads/processes, not only one Python object. Persist both tasks and reservations in the SQLite database; reopening the same file preserves live leases and permits expiry-based recovery. If an acknowledgment or queue-flush transaction fails, roll back task and reservation changes together.

Integrate existing behavior:

* Legacy dequeue must skip live reservations and may dequeue expired ones. Its removal invalidates any old reservation token.
* queue_size and enqueued_items include all unacknowledged work, including leased tasks, in existing order. A reservation or renewal does not reorder tasks.
* flush_queue removes this queue's tasks and leases, invalidates all its old tokens, and preserves other queues, result values, counters and scheduled work.
* Opening a database created by the unmodified revision must enable leased delivery without losing or changing existing data. Concurrent instances must remain safe. Keep create_tables=False semantics and existing strict_fifo behavior. With create_tables=False, initialization stays explicit; do not implicitly create tables from the new methods.

High-level API on SqliteHuey:

* reserve_task(lease_for, now=None) returns `(token, deserialized_task)` or None.
* ack_task(token, now=None), renew_task(token, lease_for, now=None), and release_task(token, now=None) forward the storage contract.
* These new methods reject immediate=True with ValueError. They do not execute tasks or acknowledge them automatically. If reserve_task cannot deserialize its payload, release its newly acquired reservation before reraising the error; leave the payload available for inspection/recovery.

Add tests and documentation. Preserve existing APIs and repository behavior. Scope this feature to SqliteStorage and SqliteHuey; no Redis, PostgreSQL, cysqlite, File or Memory backend lease API is required. Delivery is at least once: token fencing prevents a stale acknowledgment from deleting newer work, not duplicate external task side effects.

Work inside this repository without delegation for this single-attempt evaluation. Do not publish, merge, contact anyone, access credentials, or read other evaluation workspaces. Verify your change and report what remains.
