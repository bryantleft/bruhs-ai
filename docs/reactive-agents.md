# Reactive agents

Bruhs can wake a persistent agent when a file arrives, a website or command output
changes, a webhook is received, a schedule is due, or a meeting finishes processing.
The agent keeps its own provider conversation and owner-approved memory. You can
chat with it between events and change its instructions without maintaining a
separate automation script.

The local companion server owns this work. Closing a browser tab or leaving
Workflows does not stop it. Stopping the server does. The native app's bundled
server may stop when the app quits; run `bruhs-server` separately if watches must
outlive the app. This feature does not install an operating-system service.

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
  `bruhs-models/src/reactive.rs`.
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
source checkpoints, completion markers, setup proposals and run receipts in
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
