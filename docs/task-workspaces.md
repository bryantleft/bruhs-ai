# Local task workspaces

Task workspaces add server-owned agent runs beside the existing client-owned
chat threads. Open Workbench and choose Tasks to see them on the board. The server keeps running after
the view closes; completed output, provider session identity, specification
revisions and pending owner decisions are stored in the private task store.
Existing chats and life data are not copied into this store.

## The board

Tasks opens on a board with four lanes: Building, Validating, In review and
Ready. Nobody moves a card by hand. Its lane comes from what the work is
doing, so the board cannot disagree with the work:

| Lane | A card is here when |
| --- | --- |
| Building | No reviewable pull request yet, a draft, or new work after verification |
| Validating | A verification child is running or has reported, or the pull request's checks are running or failing |
| In review | The pull request waits on review, has changes requested, has a conflict, or the forge is still computing mergeability |
| Ready | Every open pull request can merge now: checks pass and review is approved or not required |

The least advanced open pull request holds a card back. A card whose pull
requests all merged leaves the lanes for Done and archived at the foot of the
board. Its status line shows the agent first (Working, Needs approval,
Failed, Interrupted, Stopped) and otherwise what the pull request waits on.
Cards that need you sort first in their lane, and the header counts what is
working, what needs you and what is ready to merge.

Delegates and forks get their own cards and are counted on their parent.
Verification runs and side chats fold into their parent's card; open the card
to reach them under Related tasks.

Pull requests link to a card two ways. A task in its own worktree is matched by
the branch checked out there, so choose New worktree for work you want tracked
to merge. An approved Resume from pull request events watch also links its pull
request, which covers tasks in the project folder. The host reads branches with
`git` and pull requests with `gh` or `glab` in the background: a branch at
most every 20 seconds, open pull requests every minute and settled ones every
15 minutes, and only while the board is open. The board answers from that
cache and never waits on a forge. A failed lookup shows its error above the
board and keeps what was read last. Guests see their one task without paths,
branches or pull requests. Branch names get reused, so a pull request that
merged or closed before the task existed is treated as earlier work and left
off its card; an open one always counts.

Agents see the board too: `workspace_status` gives each task summary its
lane, status, branch and pull requests (number, state, checks, review and
mergeability), without adding a tool to every turn.

Archive files a finished task away from the lanes; it keeps its record,
transcript, worktree and branch, and Restore or a new message brings it back.
Running or queued work cannot be archived.

Arrow keys move between cards, Enter opens one, Escape closes the panel and
returns to the card, and N starts a new task. The rules live in
`bruhs_models::coordination::board`, the lookups in
`bruhs_core::coordination::BoardLinks`, and the routes are
`GET /api/coordination/board` locally and `GET /api/board` on the paired
listener.

## Working on a task

Choose a project directory, model and access mode, then start a task. Choose
New worktree when its edits need a separate branch. A new worktree starts with the project
checkout's warm `target/debug` and `node_modules` where the filesystem can
clone them; see [Worktree disk use](worktree-disk.md). Model comparisons require
separate worktrees and use the same prompt for each selected model. The host
admits at most one comparison model per logical CPU and runs at most that many
active provider turns at once; queued work holds no CLI process. A parent awaiting
a child explicitly yields its worker slot, so nested delegation can progress even
on a single-core host. Its paused provider process remains alive while it waits.

Delegate starts a child with its own worktree. Fork starts a related exploration.
Side chat and Verify use the parent's directory in enforced Plan mode. Every
child retains its parent relationship. Bring back answer sends the completed
child answer to its parent as a new prompt; it does not merge Git changes.
Stop task and children cancels that task's active subtree. Worktrees and files
are retained for inspection when task records are deleted.

The host owns the provider event stream, so disconnecting a browser does not
cancel an agent. Restarting the host marks active and queued work Interrupted;
it does not replay commands. Send a message to explicitly resume a saved
provider session. Pending tool approvals remain with the task and are never
available to guests. Task workspaces decline private credential requests;
use ordinary chat for interactive personal credential workflows.

## Specifications and verification

Save an editable specification with acceptance criteria, then attach it to a
task. Each turn reads the current revision and records exactly which revision
it used. Editing a specification does not replace instructions inside an
already-running turn. The task shows a newer available revision until its next
message. Concurrent edits require the current revision, so stale browser saves
cannot overwrite a newer specification.

The Verify action starts a separate read-only agent against that specification
and the task's current context. Its answer is evidence to review, not an
automatic claim that the implementation is correct or permission to merge.

## Usage reset recovery

Resume at reported reset schedules a continuation only when the provider reports
an exhausted quota and a future reset timestamp. All exhausted windows must
refill, and multiple account identities require manual resumption rather than
guessing which account owns a run. Automatic quota recovery is an explicit
per-task choice and requires a provider rejection event.

Reset schedules persist across restarts. The reset worker waits for the exact
provider timestamp and resumes overdue scheduled work when the host is running.
Sending a message, stopping a task, deleting it, or cancelling its scheduled
resume removes that continuation. No fallback timestamp or model switch is
inferred.

## Pull request event continuations

In the local Tasks view, select a task and open Resume from pull request events.
Choose an existing Delivery watch and the event types the owner approves, then
approve task continuation. Every subscription pins the task's project, model
and access. Changing any of those requires a newly reviewed subscription; the
form clears pending choices when the selected identity changes. Agent MCP tools
and guest task links cannot create this approval.

Matching events become durable queued receipts. Watches in the task's own
canonical worktree are accepted as aliases of its pinned project; unrelated
checkouts are blocked. Notification metadata is bounded untrusted reference
text. It cannot authorize a publish, merge, approval or broader access mode.
The agent must inspect the current checkout and current PR state before acting.

A busy task retains the event in its queue. Once that task is completed or
failed and its previous worker has released ownership, Bruhs atomically stores
the prompt and a reservation before dispatching another turn. The receipt
deduplicates delivery retries even if acknowledgement fails. This guarantees
one reservation per approved event, not exactly-once execution of external
side effects. A host restart marks uncertain reserved turns Unconfirmed and
does not dispatch them again. Interrupted tasks require manual resumption.

Stopping a task tree disables its subscriptions and dismisses queued receipts.
Disabling or revising a subscription also dismisses older queued events; a new
approval does not inherit that backlog. Dismissed receipts stay for deduplication.
The journal and subscription list reuse the existing 50-record visible review
capacity. They fail actionably when full rather than evicting a receipt into a
possible replay. Deleting an old task removes its subscriptions and receipts.
If a reservation cannot be stored, the wake worker stops and the owner sees an
actionable error until the task store is repaired and Bruhs is restarted. This
runtime failure detail is omitted from guest snapshots.

## Paired phone and SSH access

Self-hosted access uses a separate listener. Set `BRUHS_TASK_BIND` to the explicit
address and port you want the host to use, for example `127.0.0.1:45000` behind
an SSH tunnel or your own HTTPS proxy. The default companion listener remains
loopback-only. The task listener contains only task APIs, not capture, life,
personal credentials, directory browsing or the ordinary agent API.

In the local Tasks view, open Phone and guest access. Choose Owner for your
device or Guest for the selected task, and choose an expiry in minutes. Paste
the generated token into the paired page. Tokens stay in that browser tab's
session storage; the host saves only their hashes. Revoke a grant locally to
stop future requests and its live stream. There is no public relay or managed
cloud service.

The isolated listener serves a local manifest and the existing Bruhs app icon.
Use Add to Home Screen or Install app in a phone browser over HTTPS. There is
no service worker and no offline cache of task data or credentials. If the
browser ends the installed app's session, pair it again. This is a self-hosted
phone client, not an App Store binary or a cloud execution service.

Workbench → Hosts also manages owned foreground tunnels with connect, disconnect
and restart status; see [managed local SSH tunnels](remote-hosts.md). The SSH
section prepares a validated `ssh -N -L` command for a host or SSH alias
and the ports you choose. Run it in a terminal, open the forwarded address,
and pair the browser. Phones can use your own HTTPS reverse proxy or Tailscale
Serve. Keep the host awake and preserve the original Host header in a proxy.

Guest access exposes only the chosen task's conversation and live answer.
Working paths, provider sessions and approvals are removed. A guest message is
a proposal: the local owner must send or dismiss it. Guests cannot approve
tools, create tasks, change models, edit specifications or use another task ID.

The store lives at `server/task-workspaces.json` under the app data directory,
or `task-workspaces.json` inside `BRUHS_WORKBENCH_DIR` when configured. A process
lock prevents a second server from taking ownership. Completed results use an
atomic replacement with filesystem synchronization. Streaming text is live
state until the next durable checkpoint or completion.

## Agent tools and result handoff

Host-owned tasks receive turn-scoped Workbench MCP tools. `task_work` creates
children linked to the caller, and model fanout keeps that lineage for every
isolated worktree. Tasks can control only their own subtree in the same project;
child access cannot exceed the parent's selected mode. `task_read` reads a child
answer or waits using host events. It returns when the child completes, needs
owner approval, or reaches the existing request observation deadline, after the
parent has regained a worker slot. Provider-side MCP timeouts or a disconnected
wait stop the task tree with an actionable error rather than resume it without
capacity. Review existing child output before explicitly retrying.

Delivery tools prepare a review, and browser/device tools require a grant whose
principal is that task's ID. Agents cannot mint grants, approve their own tools,
accept guest messages or execute a prepared delivery receipt. Existing ordinary
chats can use **Continue in workbench** from the command palette; this copies a
bounded transcript with the chosen project, model and access mode, and leaves
the original conversation in place. Finish a running chat turn before handing
it off so two writers do not share the same checkout.
