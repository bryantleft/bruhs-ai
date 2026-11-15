# Mail recovery before send queueing, 2026-11-15 UTC

Definite authorization or sender-profile failures before queueing now leave a
durable `Rejected` status for an exact saved review. The existing status GET can
recover that result after a lost HTTP error response. The original HTTP error is
preserved. No provider dispatch capability is created by recording a rejection.

The cache mutex and an IMMEDIATE SQLite transaction serialize the insert-only
negative record with queueing. Host, mailbox, connection, canonical draft,
revision, digest and frozen prepared content must match. Existing operation rows
remain authoritative. A different rejected operation cannot hide a queued,
dispatching or uncertain attempt, or an accepted attempt for the same review,
through the draft's latest-send projection. Storage failure and invalid or stale
bindings remain conservative; an absent status never proves that mail was unsent.

The [compact evidence](evidence/mail-prequeue-recovery-2026-11-15.json) records
source commit `1be2e94038cef74bde7235cdef40cea18c519413` and the frozen nine-file
patch. Both original selected router regressions first failed on recovery 404,
then passed. A separate selected regression reproduced rejected B hiding uncertain
A before its guard fix.

Validation on the bounded backend patch passed:

- Eight selected Core pre-queue controls, then all 178 Core mail tests.
- All 16 actual mail-router tests; the two new tests exercise both Gmail and
  Outlook, revoked consent, profile 401, lost-error recovery, preserved saved-file
  metadata and zero provider send POSTs in the negative scenarios.
- Core/Server all-target/all-feature Clippy with warnings denied, targeted
  formatting and whitespace checks.

With meeting commit `15e178eeaad1e0b078191d27b0b8c31cc73ed8ed` and the exact same
mail patch, 198 focused Core tests, 17 Server tests, strict Core/Server Clippy and
a server debug build passed. The meeting feature's 11 live checks used that
combined server and real Haiku rounds, but an older meeting-specific web bundle
that omits newer mail/agent web code. Those checks are scoped feature evidence.

This patch has no UI, schema, wire, dependency, provider-transport or asset
changes. It has no new default bundle, native UI/browser acceptance or full
14-step gate. The [earlier full gate](mail-finalization-2026-11-15.md) keeps its
original source scope. Raw sources, logs, RED archives, patch and cleanup receipts
remain locally under ignored `target/mail-slot3-prequeue-recovery-20261115/`;
the compact evidence does not upload them.

Mail and Inbox remain partial. Sent correlation/reconciliation, the full recovery,
accessibility/platform/reply matrix, live consent, provider acceptance and
recipient delivery remain open. Always-On UI/service findings and failed private
performance candidates are outside this change.
