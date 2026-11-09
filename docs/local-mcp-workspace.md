# Local MCP workspace connection

This connection is opt-in and currently supports the native Unix app only.
It reads the running workspace, not the MCP process's separate in-memory state.
Connected reads are Chat summaries/completed history, Tasks, local Calendar
events, Capture, Knowledge documents, Inbox conversations, People context,
loaded Meeting identities/summaries/transcripts, local text Meeting notes, and
independently opted-in Finance/Health sections.
Writes additionally support empty local Chat threads,
manual Finance transactions, Health workout logs, undated People reminders,
and local text Meeting notes.
Knowledge clips/attachments are not separate tools. Captured-meeting writes and uncached
Meeting reads, additional provider adapters,
externally synchronized calendar events, web connections, agent-approved
execution, and direct agent-executed rollback are not implemented. Native
clients can request an in-app undo review as described below.

## Connect a trusted local client

1. Build the app and MCP executable (`cargo build -p bruhs-desktop -p bruhs-mcp`).
2. Open **Agent workflows** in the native app. Review the domain Read/Write
   grants, then select **Enable for this session**.
3. Select **Copy MCP config** and add it to your trusted client's MCP settings.
   The copied command is `bruhs-mcp`; replace it with the absolute path to
   `target/debug/bruhs-mcp` if that executable is not on your client's PATH.
4. Restart that MCP client connection after applying the configuration.

No HTTP listener or browser port is used. The web verification server remains
on port **8087**; the MCP connection uses a private Unix socket instead.

Each MCP stdio JSON-RPC frame is limited to **64 KiB including its newline**,
separately from the private socket request limit. Oversized input receives an
invalid-request error without executing any part of that frame. The server
discards the rest through the next newline and then accepts new requests; it
does not buffer an arbitrarily large line or treat its tail as a new command.
Invalid UTF-8 receives a parse error without closing the session. These limits
apply before tool argument validation, including to otherwise valid JSON.

The copied configuration contains `BRUHS_WORKSPACE_BRIDGE`, a path to a private
temporary connection file. The session token is not copied to the clipboard.
Do not share the file or enable access for an untrusted local process. Other
processes running as your OS user are not a separate security boundary.

Disabling the connection or closing the app revokes the session. Restarting
does not re-enable it. Enable a fresh session and copy fresh configuration to
reconnect. A crash can leave an inaccessible socket and private temporary file;
their old token cannot authorize a newly started app. These ephemeral files
are not provider credentials or a replacement for keychain-backed secrets.

## Read, preview, propose, review

`bruhs_read_tasks`, `bruhs_read_calendar`, `bruhs_read_capture`,
`bruhs_read_knowledge`, and `bruhs_read_inbox` check the
current Read grant on every request. They accept optional `query` and `offset`;
responses contain at most 50 records and a `next_offset`. Reads reflect the
current in-memory workspace, including provisional edits; they are not a
durability receipt. Pagination is not a frozen snapshot if records change.

### Finance and Health reads

`bruhs_read_finance` accepts `section`: `accounts`, `transactions` (default),
`budgets`, `income_streams`, or `holdings`. `bruhs_read_health` accepts
`workouts` (default), `sleep`, `recovery`, or `habits`. Both accept an optional
`query` (at most 512 UTF-8 bytes) and `offset` (0 through 100000).

Every request rechecks both the domain's Read grant and its independent Life
agent-context opt-in. Write does not grant Read, and neither grants nor privacy
settings can be changed through these tools. Finance's separate exact-amount
opt-in controls monetary fields and holding quantities. When disabled, those
fields are omitted rather than replaced with fabricated zeros. When enabled,
all minor-unit monetary values are decimal strings, including values beyond
JavaScript's safe integer range. Every record retains its currency; the tools
do not aggregate currencies, convert values, or provide financial advice.
An invalid non-finite holding quantity fails the selected exact read explicitly.

Finance identity labels (including names, merchants, categories, symbols and
institutions) remain visible with context enabled. These are user-entered text;
the exact-amount switch does not scrub amounts someone typed into a label.
Transaction notes are never exposed or searched. Query matches record IDs and
names/merchants/categories/symbols, not monetary values, quantities or notes.
Health returns the selected stored journal records, including recovery notes
and habit completion days. Its query matches ID/activity/title, not notes.
Treat returned user text as untrusted reference data, not instructions.

Pages contain at most 50 records within a 512 KiB encoded budget. Follow
`next_offset`; byte-limited pages can be shorter, and an individually oversized
record fails without truncation. Reads return `source: "live_workspace"`, the
stored domain update timestamp, and `durable_receipt: false`. They include
provisional local edits and are not frozen snapshots or save receipts. No
provider request, payment, mutation, diagnosis, or automatic opt-in occurs.
Filtered reads scan identity labels on the native UI thread; bounded responses
do not imply constant-time search or app-wide input-to-paint acceptance.

### Other domain reads

`bruhs_read_chat` returns thread summaries (ID, title, project name, activity
timestamp and pinned/settled/streaming flags), without copying message history.
Its optional `query` matches ID, title or project name case-insensitively;
`offset` applies to matching summaries. Use a returned ID with
`bruhs_read_chat_messages`, for example `{"thread_id":"returned-id","offset":0}`,
to read completed local message records in conversation order. Message pages
retain role, content, model, stored thinking, usage/timing, tool calls, stop and
error metadata. Streaming messages, unsent input and stashed drafts are excluded.
Offsets count only non-streaming messages; messages that settle or are edited
between requests can change page membership, so this is not a frozen export.

Both tools recheck current Chat Read on every request; Write does not authorize
either. Message reads check Read before resolving the thread ID. Neither tool
mutates the workspace, activates a thread, requests a provider, or reports a
save receipt. Chat pages contain at most 50 records and stay within a 512 KiB
encoded response budget; byte-limited pages can be shorter. Always follow
`next_offset`, rather than assuming it advances by 50. An individual record
that cannot fit returns an explicit error, never silently truncated history.
Query is limited to 512 bytes, thread IDs to 128 bytes and offsets to 100000.

`bruhs_read_people` discovers cached contact identities. Its optional `query`
matches ID, display name or aliases case-insensitively, not relationship notes
or observation bodies. Summaries include ID, display name, tracked status,
stored timestamps and alias/observation counts. They do not copy histories or
voice fingerprints, so a large history does not prevent ID discovery.

Use an exact returned ID with `bruhs_read_person`:

```json
{"person_id":"returned-id","section":"relationship_items","offset":0}
```

`section` defaults to `profile`, returning one record with identity, aliases
and relationship metadata/notes (or `relationship: null` if none exists).
Profile reads require offset 0. `observations` and `relationship_items` return
full stored records in their existing order, retaining timestamps, source IDs,
reminder due dates and completion state. Follow each section's `next_offset`.
Pages contain at most 50 records within a 512 KiB encoded budget; byte-limited
pages may be shorter. A single oversized record fails explicitly, never
truncates, and identity discovery remains separate. Query is bounded to 512
bytes, IDs to 128 bytes, and offsets to 100000. Reads reject ambiguous duplicate
IDs/profiles rather than selecting an arbitrary match.

Both tools require the current People Read grant independently of Write;
one-person reads check it before resolving the ID. Voice fingerprints are
never exposed. Reads use current local state, including provisional edits;
they are neither frozen exports nor save receipts and do not refresh providers,
mutate records, or alter grants.

`bruhs_read_meetings` discovers meeting identities currently loaded in the native
workspace. Its optional `query` matches ID or title, not summaries, transcripts,
or audio paths. Records contain ID, title, status, start/end timestamps, the
stored update version, and whether a summary is available. Discovery does not
copy summary bodies or transcript history.

Use an exact returned ID with `bruhs_read_meeting`:

```json
{"meeting_id":"returned-id","section":"transcript","offset":0}
```

`section` defaults to `summary`: one record containing meeting identity,
speakers, and the stored summary (or `summary: null`). Summary requires offset 0.
`transcript` pages loaded segments in their existing order, preserving timestamps,
speaker IDs, confidence fields, text, and explicit `partial` flags. An unloaded
transcript returns an error, not a fabricated empty transcript; open the meeting
in the native app first. Invalid or foreign transcript records and ambiguous
meeting IDs fail explicitly. Neither tool exposes audio paths or audio contents.

Both tools recheck Meetings Read independently of Write. The single-meeting
read checks permission before resolving identity. Pages contain at most 50
records within a 512 KiB encoded response budget; follow `next_offset`, since
byte-limited pages can be shorter. An oversized individual summary/segment
fails instead of silently truncating. Query is bounded to 512 bytes, IDs to 128
bytes, and offsets to 100000. Responses identify `source: "workspace_cache"`.
They do not fetch providers, mutate records, or report save receipts. Native
workspace saves now include validated, server-acknowledged meeting metadata and
fetched transcript snapshots. Restoring that file makes saved context available
before networking; the app's global save status, not an MCP read, determines
whether a pending cache update reached disk. Unconfirmed speaker edits remain
excluded. Received stream fragments now have a separate saved checkpoint; restore
overlays them on the last fetched transcript and preserves their `partial` flags.
These fragments do not prove the transcript is complete, and an MCP read is still
not a durability receipt. Capture never restarts merely because fragments restore.
An unloaded meeting
or transcript still requires opening/refreshing it in the app. Live cache changes
between requests can change page membership: this is not a frozen transcript export.

Local text notes are a separate record type from captured meetings and generated
summaries. `bruhs_read_meeting_notes` discovers note IDs/titles (optional identity-only
`query` and `offset`, at most 50 records per bounded page); `bruhs_read_meeting_note`
requires an exact `note_id` and returns the full note. Both independently recheck
Meetings Read before looking up records; Write does not grant Read. These are live
workspace reads, not durable save receipts.

`bruhs_write_meetings` accepts `create_meeting_note` with a required title and an
optional body (default empty for compatibility). The app reviews the exact title
and body, requires Meetings Write and user approval, and saves the created note
with its workflow checkpoint. Limits are 4096 title bytes and 65536 body bytes;
the entire encoded MCP request must still fit the 64 KiB transport limit.
This does not record audio, call a model/provider, or alter captured meetings.
Web and native Meetings pages provide a paged title list and selected plaintext
body. Undo preflights exact unchanged records and all current Write grants before
removing any mixed-domain targets. Manual note editing, captured-meeting linking,
and universal exact-record search integration are not completed by this adapter.

Typed writes default to preview only. For example, the arguments to
`bruhs_write_tasks` can be:

```json
{
  "operation": {"type": "create_task", "title": "Draft the launch brief"},
  "dry_run": true
}
```

Knowledge writes support `create_knowledge_note` with `title` and `body`.
Inbox writes support `draft_inbox_message` with `recipient` and `body`: the
app creates a new local import-source conversation containing one draft.
It does not infer an existing recipient/account, send externally, or claim
remote delivery. Both operations use the existing app-controlled workflow
checkpoint and unchanged-record undo path, on web and native.

Native `bruhs_write_chat` supports `create_chat_thread` with `title`, also
advertised in the cross-domain proposal schema. App-approved execution creates
an empty general thread with the exact reviewed title, without opening it,
sending a message, or requesting model generation. It retains the native app's
current model, permission mode and normal local working directory; it does not
grant Chat Read. The window-bound callback rechecks authorization after it is
scheduled, and repeated activation cannot queue duplicate creations. The thread
and workflow checkpoint share the acknowledged whole-file save. Failed saves
remain provisional; Retry saves that same thread rather than recreating it.

Chat undo requires the complete creation snapshot to remain unchanged. Active
threads, messages, unsent input, stashed drafts, streaming, running tasks and
pending permission requests block deletion of the entire reviewed set. The web
Chat adapter additionally checks separately stored message records in the same
IndexedDB transaction as the thread/checkpoint. Native GPUI fixture tests cover
checkpoint restoration, not a hardware restart or crash-recovery certification.

Native `bruhs_write_people` supports `create_person_reminder` with `person_id`,
`person` (the exact current display name), and `reminder`. The ID and full text
appear in the in-app review. Execution requires exactly one cached contact with
that ID and name; duplicate names never select another contact. Missing, renamed,
or duplicate-ID contacts fail closed and require a new review. Legacy name-only
operations can still load, but cannot be previewed or executed. This creates an
**undated local reminder**, not a notification, outbound message, or contact
edit. People Read is independent: an authorized client can discover the ID/name
with `bruhs_read_people`, but Write never grants Read.

Reminder execution saves the relationship profile and checkpoint together.
Undo validates an exact before/after receipt and the entire workflow before
changing any domain. It restores prior notes, commitments and timestamps, or
removes only the relationship profile newly created by that workflow—not the
contact. Several reminders for one person are checked in reverse order.
Later relationship edits or reminder completion block the whole undo. Save
failure remains provisional, and Retry persists the same reminder ID. The web
People workflow adapter and hardware crash/restart acceptance remain open.

Native `bruhs_write_finance` supports `add_finance_transaction` with `merchant`,
`category`, and `amount_minor` as an exact decimal **string** of signed USD cents
(negative means spend). Values must fit -9223372036854775807 through
9223372036854775807. `bruhs_write_health` supports `log_workout` with `activity`
and `minutes` (1–1440). Both create manual local records dated at execution;
workouts initially use perceived exertion 5/10. These are not payments,
provider writes, or medical actions. Unknown operation fields are rejected:
for example, supplying `currency: "EUR"` cannot silently create a USD record.
Legacy integer amounts remain readable; new workflow payloads persist strings
so a JavaScript round trip does not round cents. Callers should always send
strings rather than already-rounded JavaScript numbers.

Each domain requires its own Write grant and in-app approval. Neither enables
Read access, changes Life privacy settings, or discloses existing Life records.
The same native whole-file save and exact-record undo checks apply. Web workflow
execution and undo also support these manual Finance/Health operations, using
one IndexedDB transaction for the Life metadata, workflow checkpoint, and any
separate records removed by a mixed-domain undo. Both metadata revisions are
checked before writing; stale data and transaction aborts cannot partly save a
record or checkpoint. Life edits are frozen only while a Life transaction is
in flight, and committed records appear in the existing Life surface.
Finance/Health reads and the web MCP connection remain unimplemented. These
local execution adapters do not establish all-domain or hardware/provider parity.

To submit that same operation for in-app review, set `dry_run` to `false` and
include a stable `request_id` of 1–128 bytes. This requires the current Write
grant. Reuse the same ID and identical payload if a response is lost; changing
the payload under that ID is rejected. Idempotency is scoped to the enabled
session, not to a new connection session after restart. Do not blindly resubmit
after reconnecting: inspect the saved workflows and records first.

In native **Workflow review**, use **Previous workflow** and **Next workflow**
to inspect older saved plans. The heading identifies the displayed title,
status, and position in history. Run, background, pause, resume, and undo act
on that displayed workflow, not whichever job happens to be newest. Navigating
history does not approve or execute anything. The app-wide pending MCP notice
reopens its specific proposal even if another workflow was being inspected.

## Cross-domain workflows

Use `bruhs_propose_workflow` to submit one ordered review unit across connected
domains. It accepts `title`, `operations` (1–128 typed operations), optional
`dry_run` (defaults to `true`), and a `request_id` when submitting. For example:

```json
{
  "title": "Prepare the launch brief",
  "operations": [
    {"type": "create_task", "title": "Finish the launch brief"},
    {"type": "create_knowledge_note", "title": "Launch brief", "body": "Review the draft outline."},
    {"type": "draft_inbox_message", "recipient": "Ada", "body": "The outline is ready for review."}
  ],
  "dry_run": false,
  "request_id": "launch-brief-plan-1"
}
```

The entire encoded IPC request must fit 64 KiB. The app validates every
operation and adapter, and requires current Write grants for every participating
domain before accepting the proposal. A rejected batch creates no workflow or
records. Preview reports blocked domains without submitting anything. Reusing
an ID requires the same title, operations, and order; a changed title or reordered
payload is not a retry. Single-operation tools use the same session submission
and approval boundary.

One approval starts one job. Each step still waits for the previous local save,
rechecks current grants and the complete reviewed payload, and has a creation
snapshot for user-controlled undo. This is an ordered, checkpointed workflow,
not a single all-or-nothing transaction spanning every step. Users can inspect
partial progress and undo completed, unchanged records. Background execution
remains an explicit user opt-in; agents cannot enable it through the protocol.

Only one unfinished workflow can be present when a new MCP proposal is
submitted. Review or decline it before submitting another. A session accepts
at most 100 distinct proposals. Repeated delivery of an existing ID returns
that proposal's status instead of creating a duplicate.

In the app, inspect the complete payload and choose **Start transaction** or
**Decline proposal**. Approval is bound to the displayed workflow ID, not
whichever proposal was added most recently. Start creates a job, not its
records. **Run next step** or explicit background opt-in uses the existing
permission-rechecking executor. Decline is terminal and retains an audit entry.

`bruhs_workflow_status` accepts the returned `workflow_id`. It reports only a
proposal belonging to this connection session and still requires its Write
grant. `executed: true` requires both a completed workflow and an acknowledged
local save. Saving, failure, conflict, and recovery states never count as
durable completion. A timeout means status is unknown, not successful.

Status includes `progress.total_steps`, `completed_steps`, and `reverted_steps`.
These are provisional in-memory counts when `durable` is false. The corresponding
`durable_completed_steps` and `durable_reverted_steps` are null until saving
succeeds. A started job includes its status, zero-based `next_step`, background
opt-in state, and `can_advance`; the latter is false while saving or when the
reviewed payload, grants, or job state no longer permit execution. Responses do
not expose record snapshots or other workflows. Revoking any participating
domain's Write grant makes status unavailable to that session; review in the app.

The protocol has no grant, approve, execute, or direct rollback command. A
disconnected app returns an error; connected calls do not fall back to
fabricated data. A domain without an adapter in the app still returns an
explicit unavailable response, so callers must handle one.

## Tool catalog and errors

`tools/list` is mode-aware and is generated from the same registry that
dispatches `tools/call`, so it never advertises a tool the active mode cannot
run. With `BRUHS_WORKSPACE_BRIDGE` set (the connected mode) it lists every
typed `bruhs_read_*` and `bruhs_write_*` tool, the specific readers, and the
workflow tools (`bruhs_propose_workflow`, `bruhs_workflow_status`,
`bruhs_request_workflow_rollback`). Without it (standalone mode, which serves
only this process's empty in-memory state) it lists `bruhs_read_chat`, the
dry-run `bruhs_write_*` previews, `bruhs_list_threads`, `bruhs_list_tasks` and
`bruhs_agent_audit`; the other domains' data lives in the app. The legacy
`bruhs_create_thread`, `bruhs_send_message`, `bruhs_create_work_item`,
`bruhs_rollback_workflow` and `bruhs_set_agent_capability` tools were removed:
the default policy grants no Write capability and nothing can change it, so
they could only fail. In particular, the MCP process starts no agent turns, so
it cannot block on a provider permission prompt.

Errors follow one rule. A frame that is not JSON is a parse error (-32700).
JSON that is not a JSON-RPC 2.0 request (a `jsonrpc` other than `"2.0"`, no
`method`, a batch array) is invalid request (-32600). A `tools/call` without a
string `name`, or naming a tool the active mode does not list, is invalid
params (-32602). Everything about the arguments of a listed tool (missing,
mistyped or oversized fields, unknown fields where the schema forbids them)
and every execution failure is a normal tool result with `isError: true`, so
the model can read the reason and retry. All identifiers are 1-128 bytes,
`request_id` included; `query` is at most 512 bytes and `offset` at most
100000 on every read tool, generic or specific.

If the server cannot write to stdout (for example the client closed the pipe)
or cannot read stdin, it stops and exits with status 1 rather than discarding
responses.

## Request an undo review

`bruhs_request_workflow_rollback` accepts a `workflow_id` from the current
connection session and an optional `dry_run` (defaults to `true`). Preview
checks the saved checkpoint, all participating Write grants, and full creation
snapshots without changing records or opening a review. With `dry_run: false`,
it asks the user to review undo in the native app; it still deletes nothing.

```json
{"workflow_id":"the-proposed-workflow-id","dry_run":false}
```

The app identifies the workflow and offers **Keep changes** to decline, or
the existing **Undo transaction** action to remove its unchanged created
records. Undo rechecks current grants and snapshots at click time. A running
background job must first be paused by the user; the agent cannot pause it.
Partial workflows can then undo their completed, unchanged steps.

Requests are idempotent per workflow/session. Repeating a declined request
does not reopen it or change the current review. Pending undo requests cannot
be displaced by new MCP proposals. Request/decline/approval review state is
ephemeral: disabling or restarting the connection clears it, without deleting
records or replaying undo. Existing workflows remain available in app history,
but a new session cannot claim ownership of an old session's proposal.

The request response distinguishes `checkpoint_durable` from the session-only
`rollback_review` (`null`, `pending`, `declined`, or `approved`). It is never a
receipt for deleting records. In `bruhs_workflow_status`, `rolled_back` becomes
true only after the user-controlled rollback's record deletions and workflow
checkpoint are saved. While saving or failed, it stays false and
`progress.durable_reverted_steps` is null. The completed rollback remains in
the saved workflow/audit history; the transient request itself is not a
durable audit entry. There is no direct rollback tool.
