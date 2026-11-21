# Pause and resume individual cron entries

Add concurrent-safe per-entry pause and resume to github.com/robfig/cron/v3.
The existing scheduler, parser, wrappers and public behavior must remain
compatible. This is an in-memory control feature, with no persistence or new
external dependencies.

## Public API

Add an exported Paused bool field to Entry and two methods to Cron:

    Pause(id EntryID) bool
    Resume(id EntryID) bool

Pause returns true only when an existing active entry changes to paused.
Resume returns true only when an existing paused entry changes to active.
Unknown IDs, zero IDs and requests for an already-matching state return false.
Removing a paused entry permanently removes it; Resume cannot recreate it.

## State and execution

1. A paused entry stays in Entry and Entries snapshots with its original ID,
   Schedule, Job, WrappedJob and Prev. Its Next must be the zero time. Snapshots
   remain copies; changing a returned Paused value cannot change the scheduler.
2. Pause is synchronous: its state change is complete when it returns. A job
   already dispatched may finish normally, and Stop must still wait for it.
   Dispatch means the scheduler has committed to launching the job goroutine;
   that goroutine may begin executing after Pause returns.
   Pausing neither cancels that job nor creates another wrapper or job instance.
3. While stopped, Resume clears Paused but keeps Next zero. Starting or running
   the scheduler computes Next only for active entries. Paused entries must not
   call Schedule.Next on startup or dispatch jobs.
4. While running, Resume computes the next occurrence from a fresh current time
   in the Cron location, using the existing Schedule. It never replays missed
   occurrences and never resets Prev. A zero Next from an unsatisfiable schedule
   remains valid and dormant.
5. Pausing or resuming one entry must not intentionally suppress, remove, pause or
   reschedule another entry. Normal serialized command processing is permitted;
   this is not a zero-latency requirement. The scheduler must remain responsive when every entry is paused.
6. Pause and Resume are safe alongside Entries, Entry, Schedule, AddFunc, Remove,
   Start, Run and Stop. A job may pause itself and resume another entry. The race
   detector must remain clean. Concurrent controls and lifecycle operations
   must admit one consistent serial order. A Resume linearized while stopped
   leaves Next zero; a later Start or Run may then compute it. Existing Stop
   and wrapper semantics are unchanged.

For this task, stopping preserves each entry's existing state. A later Resume
while stopped explicitly clears Next, as described above. All control methods
apply to the next dispatch decisions, not retroactively to work already started.

## Deliverables and validation

Implement the feature, add relevant tests and update user documentation and API
comments. Keep the dependency-free module and its existing tests. The supplied
pause_smoke_test.go covers only basic API behavior; add coverage for runtime
control, lifecycle and concurrency cases. Run go test -race -count=1 . and report
what passed or could not be run. The evaluation environment provides an offline
Go toolchain and private per-attempt build caches.

Work directly without delegation. Do not publish, merge, contact anyone, access
credentials or read other evaluation workspaces. Do not alter this contract.
