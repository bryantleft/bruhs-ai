# Reactive agents

Bruhs can wake a persistent agent when a file arrives, a website or command output
changes, a webhook is received, a schedule is due, or a meeting finishes processing.
The agent keeps its own provider conversation and owner-approved memory. You can
chat with it between events and change its instructions without maintaining a
separate automation script.

The local companion server owns this work. Closing a browser tab or leaving
Workflows does not stop it. Stopping the server does. The native app's bundled
server stops when the app quits unless the background service is on (see
[Keep agents running after quit](#keep-agents-running-after-quit)).

An always-on agent is not literally always running. It sleeps, wakes for a
**run** (an event, a follow-up it scheduled, or your message), does the work,
saves what it learned, and goes back to sleep. Each run ends in one of three
ways: it tells you something, it stays quiet, or it asks you a question.

## Watch scanned bills

1. Open **Agent workflows → Automations → New automation**. Give it a name,
   choose a local Claude or Codex model, enter an existing absolute working folder,
   and describe the task. For example:

   > Extract the vendor, amount, currency, due date, payment reference and bank
   > details from each scan. Flag uncertainty and possible duplicates. Prepare a
   > reviewable note. Never submit a payment.

2. Choose a folder trigger pointing at the scanner's output directory. `*.pdf`
   matches PDF names regardless of ASCII case. A source waits for a file to remain
   unchanged before reporting it. **Advanced options** includes persistent memory,
   event instructions, timing and the option to review every event before running.
3. Select **Review automation**. Check its purpose, complete source settings,
   working folder, model, memory and read-only access. Nothing is saved or enabled
   while composing or reviewing. **Enable automation** saves and activates the
   complete setup in one transaction. The reviewed default automatically produces
   read-only drafts; writes still need workflow approval.
4. Inspect activity and drafts on the automation's card. **Chat** continues its
   persistent conversation. **Edit automation** changes its setup and memory through
   the same review. **Pause automation** stops event work; **Resume automation**
   restores it without changing the event-review policy. Chat remains available.

There are no active automations by default. Each new automation requires its own
explicit activation. **Advanced settings** retains the separate agent, source and
subscription controls for existing configurations and shared components. Existing
automations keep their saved automatic or per-event review policy.

First activation reconciles all matching files already in the folder, not just
new arrivals. Subsequent scans use durable checkpoints, including files that
arrived while the server was stopped. Pause and rename preserve these checkpoints.
Changing a source's definition starts a new observation generation.

## Turn drafts into actions

Reactive runs use read-only tools. An event cannot silently inherit the access,
credentials, integrations or thread tools of an interactive chat.

For tasks that need new app records, the agent can return a structured draft:

```json
{
  "summary": "Follow up after the review",
  "operations": [
    { "type": "create_task", "title": "Send the revised proposal" },
    {
      "type": "create_knowledge_note",
      "title": "Review decisions",
      "body": "The team agreed to review the revised proposal on Friday."
    }
  ]
}
```

Completed, valid drafts offer **Preview changes**. That creates a proposal in the
existing workflow system. Independent domain Write grants, full payload review,
explicit transaction approval, durable checkpoints and undo still apply. Importing
a draft starts no job and creates no domain record. A completed agent run is not
a receipt that its proposed actions were applied. Plain answers remain plain text.

The existing workflow operation types cover chat threads, tasks, calendar events,
meeting and knowledge notes, inbox drafts, person reminders, capture, finance
transactions and workouts. A finance record is a local ledger entry, not a bank
transfer. There is no built-in banking connector or automatic payment submission.

## Updates, questions and notifications

Agents report outcomes, not every step. Each run's answer ends with a
`bruhs-report` block (taught by the runtime's fixed instructions, see
`bruhs_models::reactive::REPORT_PROTOCOL`):

```bruhs-report
{"outcome":"notify","title":"Invoice due Friday","message":"ACME, 420 EUR, account ends 9999 (new).",
 "remember":[{"key":"acme","kind":"note","text":"ACME bills monthly"}],
 "follow_ups":[{"key":"acme-paid","in_minutes":2880,"note":"Check the invoice was paid"}]}
```

- `notify` and `ask` become items under **Automations → Updates**, with an
  unread count on the navigation entry and an in-app banner when one arrives.
  `silent` keeps the result in the run's activity only. Agents are told to start
  quiet and never repeat what they already told you.
- A run without a block stays silent, except that a workflow draft always asks
  for review. A malformed block is ignored and its reason shown on the run as a
  report note; nothing in it is applied.
- **Reply** to a question (or an update) resumes the agent's conversation with
  your answer. **Dismiss** resolves it without a run.
- Everything a report asks for (memory, follow-ups, the update itself) commits
  in the same transaction as the run's completion receipt. Failed, cancelled and
  replaced runs apply nothing.

**Notifications** settings add system notifications for updates and/or
questions. They are off by default. The local server sends them (macOS
Notification Center via `osascript`, Linux `notify-send`), so they arrive while
no app window is open. Quiet hours (start, end and an IANA time zone) hold them
until the window ends. A burst becomes one digest. Items read in the app first
are never sent. A refused notification is recorded on the item.

## Memory

Each agent has two kinds of memory:

- **Owner-approved memory** is part of the agent's fixed instructions. Changing
  it is a reviewed setup change and starts a new provider conversation.
- **Agent memory** is written by the agent through its report: `note` entries
  for preferences, corrections and facts it cannot look up again, and
  `progress` entries for where a multi-run task stands. Entries are keyed;
  reusing a key replaces the entry. Each entry records the run that wrote it.

Agent memory is bounded (40 entries, 600 characters each, 8,000 in total). A
report that would overflow it saves nothing new, still applies its `forget`
list, and queues a memory tidy-up run (at most daily). **Tidy memory** queues
one on demand: the agent merges duplicates and drops stale entries. In
**Agent memory** you can edit, forget or copy entries; the next run sees the
change.

Memory and the last few updates ride in the run's own message, not the system
instructions, so the instructions stay identical between runs and providers can
cache them. A resumed conversation gets memory again only after someone other
than the agent changed it. When a conversation's context passes 120,000 tokens,
the next run starts a fresh conversation, and its memory, recent updates and
pending follow-ups are repeated to it. The run shows "Started a new
conversation; memory carried over."

Providers keep a resumed conversation's original instructions, so each saved
session records a fingerprint of the instructions it started with. When they
change (an owner edit, or an app update to the report protocol), the next run
starts a fresh conversation the same way.

## Follow-ups and calendar schedules

An agent can schedule its own follow-ups (`in_minutes` or an RFC 3339 `at`,
up to a year ahead, 20 pending per agent). When one comes due the agent wakes
with its note. A follow-up from an automation that reviews each event waits for
review too. Agent workflows lists pending follow-ups per agent with **Cancel
follow-up**. A paused agent's follow-ups wait until it is resumed.

A **Calendar** source fires at wall-clock times in a named time zone, such as
weekdays at 08:00 in `Europe/Paris`. Daylight-saving gaps fire at the first
valid time after the jump. After downtime it emits one event with a
`missed_slots` count. Weekly report presets can use a fixed weekday and time
instead of "seven days after enabling".

## Grouping, lanes and restarts

Events never call an agent directly; they queue durable runs. When an agent's
turn starts, all of its queued, due runs (up to 20) are handled together in one
turn, and the others are marked **Handled with another run**. Each agent runs
one turn at a time, and up to three agents run at once, so a slow weekly report
does not hold up a bill watcher. Weekly report runs are never grouped.

A subscription's **While the agent is busy** setting chooses what a new event
does mid-run: wait and be handled with the next batch (the default), or
**Restart with the new event**, which stops the current turn (keeping its
partial output) and starts one turn with both.

## Turn drafts into actions: the decision comes back

When you preview a run's draft, the workflow remembers which run it came from.
Once it is applied, declined, fails or is rolled back, the app reports the
decision and the agent wakes with it ("Your draft … was declined"), so it can
update its memory or follow-ups. Each decision is reported once.

## Keep agents running after quit

**Automations → Background service → Keep agents running after quit** (or
`bruhs-server service install`) registers this server binary as a login
service: a launchd agent `dev.bruhs.server` on macOS
(`~/Library/LaunchAgents/dev.bruhs.server.plist`) or a systemd user unit
`bruhs-server.service` on Linux. It starts at login and restarts only after a
failure. It carries the installing process's `BRUHS_*` settings and `PATH`
(agents need `PATH` to find their CLIs) in a definition file only you can read.
While the app's own server is running, the service's start fails on the shared
automation database lock and retries every 30 seconds, so it takes over when the
app quits. The desktop app uses an already running server from the same build.
`bruhs-server service status` and `service uninstall` report or remove it. Logs
go to `logs/bruhs-server.service.log` in the app data directory. Reinstall after
moving or rebuilding to a different binary path.

Loaded is not the same as working. Status reports the service as serving only
when the process the service manager started is the one answering on the port:
a running server reports its own process ID in `GET /api/service`
(`responder_pid`), and `service status` makes one short loopback request to
compare it with the job's (`service_pid`). A job that is loaded but has no
process, a process that does not answer, or an answer that does not name its
process (an older server, say) is reported as such, never as running. The
status includes the log path when there is one.

Runs never hold credentials. Approved writes are executed by the app through
workflow grants, so the agent describes an action and the host performs it with
your accounts.

## Configure through chat

Interactive Claude and Codex turns receive a private `bruhs_reactive` MCP server:

- `reactive_status` lists agents, source/subscription state, recent runs and
  pending proposals using compact navigation data.
- `reactive_inspect` reads one exact configuration, memory, event or run by ID.
- `reactive_propose` saves a typed setup change for owner review in Workflows.

For example, ask the interactive agent to create a scanner assistant and watch a
specific folder. It proposes one complete automation, including the agent, source
and subscription. Reviewing and approving that single proposal activates the
whole setup atomically. An agent cannot approve its own proposal through these
tools. Plan-mode chat turns only get the read tools; event-triggered runs get none
of these administrative tools. Memory replacement is also an explicit reviewed
change. Ordinary provider conversation history persists without replacing that
memory.

The helper connects only to the local companion server. It uses
`BRUHS_SERVER_URL`, or loopback plus `BRUHS_SERVER_PORT` (default `3001`). Remote
control-server addresses, redirects and proxy forwarding are refused.

## Event sources

| Source | Behavior |
| --- | --- |
| Folder | Reconciles matching files, optionally recursively, with persisted settling and file observations. Skips symlinks and common dependency/build folders. Masks support `*`, `?`, and comma-separated alternatives. |
| Website | Polls HTTP or HTTPS and reports the first nonempty successful body and later changes. Requests have a deadline and bounded response size. |
| Command | Runs an explicitly configured shell command in its configured folder, with a deadline and bounded output. Reports nonempty output changes. Commands run with the server account's rights; review the exact command before enabling it. |
| Schedule | A repeating interval with a persisted due time. After downtime it emits one catch-up event with a missed-interval count, instead of flooding the queue. |
| Calendar | `HH:MM` times on chosen ISO weekdays (empty means every day) in an IANA time zone. One catch-up event with a `missed_slots` count after downtime. |
| Webhook | Accepts JSON with a source-specific Bearer token. Tokens are revealed separately in the owner UI and are never included in snapshots or model tools. |
| App event | `meeting.completed`, `meeting.transcribed`, or `meeting.summary`. Uses durable app records and completion receipts, with background reconciliation. |
| Custom event | An adapter publishes a JSON observation with a stable event key. |

Only enabled sources with an enabled subscription to an enabled agent are polled.
Polling is independent for each source; a slow website or command does not block
other watches. Debouncing replaces a pending observation with the latest one and
requires review again; throttling suppresses repeated runs within its window.

MQTT and Zigbee can feed Custom/Webhook sources or an installed command adapter.
An FTP server can write uploads into a watched folder. Bruhs does not bundle an
MQTT client, Zigbee coordinator, Telegram bot, or FTP server. These adapters reuse
the same persistent event and subscription contracts.

`meeting.completed` means recording stopped cleanly. `meeting.transcribed` waits
for a durable completion marker written on final transcription or capture recovery,
not merely recording stop or an interim transcript chunk. Its context excludes
partial segments. `meeting.summary` reports generated-summary changes. App event
payloads never include local audio paths. Oversized transcript/summary context is
explicitly marked as truncated, with a digest of the full content. Transcription
completion recovery applies to recordings observed by this runtime or replayed by
capture's recovery feed; older recordings without such a receipt are not assumed
to be finalized.

## API and adapters

Both interfaces use the same local server API:

- `GET /api/reactive`: configurations, all unresolved runs/proposals, recent
  finished activity and source health.
- `POST /api/reactive/commands`: a shared typed `ReactiveCommand` from
  `bruhs-models/src/reactive.rs`. Besides setup and run commands it covers the
  outbox (`mark_outbox_read`, `answer_outbox`, `dismiss_outbox`), agent memory
  (`edit_memory`, `forget_memory`, `consolidate_memory`), `cancel_follow_up`,
  `set_notification_settings` and `draft_outcome`.
- `GET /api/events/stream` frames named `reactive` carry an `OutboxSignal`
  (unread count, open questions, newest unread update) on open and on change.
- `GET`/`POST /api/service`: background service status, install and uninstall.
- `GET /api/reactive/sources/{id}/token`: separately reveal a webhook's private
  credential to the local owner UI. Responses are not cached.
- `POST /api/reactive/hooks/{id}`: JSON event ingress with
  `Authorization: Bearer <source token>`.

Webhook producers should send `X-Event-Id`, or GitHub's `X-GitHub-Delivery`, so
retries have a stable identity. Repeated identities with the same payload reuse
the receipt; conflicting payloads fail. Without a delivery header, each request
has a new identity and may create another run. Bodies are limited to 64 KiB.

A Custom source adapter posts this command:

```json
{
  "action": "publish",
  "source_id": "printer-status",
  "key": "device-report-1042",
  "payload": { "state": "finished", "job": "bracket" }
}
```

The control API retains the companion server's local Host/Origin checks. Those
checks protect against foreign browser pages, not hostile local processes. An
internet webhook needs a reverse proxy or tunnel exposing only its hook route,
preserving Bearer authentication and rewriting the upstream Host to loopback.
Do not expose the local configuration or credential routes through that proxy.

## Persistence and recovery

The server stores configuration, memory, provider session references, events,
source checkpoints, completion markers, setup proposals, run receipts, the
outbox, agent memory, follow-ups, notification settings and draft decisions in
`server/reactive.sqlite3` in Bruhs' local data directory. `BRUHS_REACTIVE_DB`
selects a different file. It has a separate lifetime lock so two server processes
cannot recover or execute the same queue. Back it up with SQLite-aware tooling.

Events, routed work and source checkpoints commit together. Duplicate deliveries
do not create duplicate work. Queued work survives restarts. Work interrupted
mid-turn is marked **Interrupted** and requires an explicit retry; it is never
silently replayed. Source definition changes or disabling an agent/subscription
cancel affected pending and running work. Failed/cancelled runs retain their
available output; late completion cannot overwrite cancellation.

The global unresolved-work and pending-setup queues are bounded at 50 each and
remain visible. Folder backlogs are ingested in capacity-aware batches; remaining
files keep their pending observations until capacity becomes available. Source
errors and output/response-size limits are shown explicitly, not treated as empty
successful observations. A truncated model answer is marked failed and cannot be
imported as a completed workflow draft.

Event identities and stored history are retained locally for deduplication and
review. This implementation does not automatically prune or expire that database.
Use a dedicated small source folder or an extracting command/endpoint for large
archives and feeds.

The design is inspired by [PINE](https://github.com/pmndrs/pine). Bruhs retains its
existing Rust architecture, Claude/Codex logins, model selection, capability grants
and workflow execution instead of adopting a second coding-agent harness.

## Notification volume eval

`reactive_outcome_eval` sends synthetic events through the real worker and a
signed-in provider and scores each run: a routine success stays silent, a
failure notifies, the same failure is not repeated, changed bank details ask
first, a stated preference is remembered, and a promised reply gets a follow-up.

```sh
cargo run -p bruhs-core --example reactive_outcome_eval -- \
  --run-live --out target/reactive-outcome-eval/results.json
```

The default model is `claude-haiku-4-5`; `--models a,b` selects others. It
makes real model requests and is excluded from ordinary tests.

## Live acceptance check

The explicit live check uses the actual worker and signed-in local Claude and
Codex CLIs with disposable fixture files and a temporary database:

```sh
BRUHS_CODEX_LEAN_SESSIONS_FILE=target/reactive-smoke-state/lean-sessions.json \
  cargo run -p bruhs-core --example reactive_smoke -- \
  --run-live --out target/reactive-live-smoke/results.json
```

It checks one-review activation, automatic file handling, unchanged inputs, saved
memory, provider conversation recovery after restarting the runtime, Pause/Resume,
and catch-up without duplicate events. The default models are `claude-haiku-4-5`
and `gpt-6-luna`; `--models model-one,model-two` selects others. This check makes
real model requests and is excluded from ordinary tests. Its fixture contains no
personal documents and requests no writes or payments. Provider CLIs retain their
normal local session history.
