# Messages between threads

Each chat thread runs its own agent with its own context. An agent can see
the other threads, read one, and send one a message, so work can be split
across threads and the results can come back without the user copying text
between them.

## Flow: one thread asks another

```mermaid
flowchart LR
    A[Agent in thread A: calls a bruhs_threads tool]
    Broker[Per-turn broker: checks the token, relays]
    App[App that owns the threads: web or desktop]
    Queue[Thread B: prompt queue]
    B[Agent in thread B: runs the message as a turn]
    A -->|list, read or message| Broker
    Broker -->|thread_request event on A's turn| App
    App -->|answer text| Broker
    Broker -->|tool result| A
    App -->|message with sender header| Queue
    Queue -->|when B is free| B
    B -->|reply, or message_thread back to A| App
```

## The tools

A turn gets a private MCP server, `bruhs_threads` (the host executable in
`--thread-mcp` mode):

| Tool | What it does |
| --- | --- |
| `list_threads` | Names up to 30 threads, most recent first: short id, title, project, whether each is running or has prompts queued, its model and its role. Then the roles a new thread can start with. |
| `read_thread` | Returns the last messages of one thread (6 by default, 20 at most), each clipped, inside a fixed size budget. |
| `message_thread` | Sends a message to a thread. Without `thread_id` it starts a new thread where the sender runs: same project, worktree and model. `wait: true` holds the call until that thread has replied and returns its reply. `role` starts a new thread on a role's model and instructions (see `docs/model-roles.md`). |

Thread ids are accepted whole or as a unique prefix. Replies print the
shortest prefix that is unique.

Which tools a turn gets:

| Turn | Tools |
| --- | --- |
| Messages between threads is Off | None |
| Messages between threads is Read only | `list_threads`, `read_thread` |
| Plan mode | `list_threads`, `read_thread` |
| Codex in ask-first mode (it cannot ask) | `list_threads`, `read_thread` |
| Anything else | All three |

## Settings

Settings → General has two rows, the same on web and desktop:

- **Messages between threads**: Read and message (default), Read only, or
  Off. The web sends the choice with each turn (`thread_access` on
  `POST /api/agent`); the desktop puts it on the turn it builds.
- **Agent message limit**: 2, 4, 6 (default) or 10 messages in a row.

The web stores them as `bruhs.threadAccess` and `bruhs.threadHops`. The
desktop state file gets `thread_access` and `thread_hops`, written only when
they differ from the defaults.

## Decisions

- **The app answers, not the server.** The web keeps threads in IndexedDB and
  the desktop in its own files, so the core process cannot list or read them.
  The broker streams a `thread_request` event on the calling turn and waits up
  to 60 seconds for the app's answer (`POST /api/agent/thread-reply` on the
  web, `ThreadLinks::resolve` on desktop). The rules and the reply text are in
  `bruhs_models::thread_link`, so both apps answer identically.
- **A message is a queued prompt.** It joins the recipient's existing prompt
  queue and runs as an ordinary turn there, with that thread's own session,
  permission mode, approvals and history. The user can edit or remove it while
  it waits, and a held queue stays held.
- **Sending does not wait unless asked.** `message_thread` returns as soon as
  the message is handed over. An answer is a second message, sent by the
  recipient's agent with the same tool, that arrives as a new turn in the
  sender.
- **A sender may wait.** With `wait: true` the call is held, for up to an
  hour, until the recipient's turn for that message ends; the tool result is
  that turn's reply, or why there is none (stopped, failed, removed from the
  queue, thread deleted). The delivered header tells the recipient its reply
  is returned and not to message back. Both apps refuse to wait on a thread
  that is itself waiting on another, which is what a deadlock would need, and
  do not hold a call nothing could answer soon: the recipient's queue is
  paused, or (on the web) no lane is free or the chat page is not mounted.
  The message is then delivered without waiting and the reply says so.
- **Chains stop at the user's limit.** A delivered message opens with a header
  line that names the sender and a hop count
  (`[Message from thread "Title" (id 1a2b3c4d), hop 2/6. …]`). The count is
  read back from the prompt of the turn that sends the next message; one past
  the limit is refused and the agent is told to hand back to the user. A
  prompt the user types counts from zero again.
- **Reads are free, writes follow the permission mode.** `list_threads` and
  `read_thread` never ask, and are marked read-only so a Plan turn may call
  them. On Claude, `message_thread` goes through the turn's permission mode
  like any other tool: Ask and Accept edits show an approval, bypass does not.
- **One capability per turn.** The broker listens on a loopback port with a
  random token that is revoked when the turn ends. The helper can only relay
  the three operations; it cannot answer one.
- **The header is for the model, not the reader.** Both apps show an
  agent-sent message as its body under a caption, `From thread “Title”`
  (blueberry, the information color), with ` · waiting for this reply` when
  the sender waits. Copy and Edit from here keep the whole prompt, and
  `read_thread` shows it whole to other agents.

## Threads run side by side

Both apps run several threads at once, so a message to a free thread starts
its turn right away, the view stays where it is, and a sender may wait.

On the web each thread's turn lives in `state::Turns`: its settled messages
and, apart from them, the reply it is writing, so a token re-renders only
that reply. Three things are specific to the web:

- **Three lanes.** At most three turns run at once (`MAX_RUNNING_TURNS`). A
  turn holds one connection to the server while it streams, and a browser
  allows a host six. A fourth prompt waits in its thread's queue and starts
  when a turn ends.
- **Queues drain off screen.** A prompt queued in a thread starts when that
  thread is idle and a lane is free, whether or not the thread is open. Its
  saved messages load first. Stop and a reload still pause every queue until
  Resume or the next send.
- **The chat page starts turns.** It owns the turn lifecycle, so while another
  view is open a queued message waits, and a call that asked to wait is not
  held. A turn that is already running keeps running.

Approvals belong to the thread that asked: each thread shows only its own,
and the sidebar marks every thread that is working or waiting on you.

## Verification

```sh
cargo test -p bruhs-models thread_link
cargo test -p bruhs-core thread_link
cargo test -p bruhs-server --test thread_mcp
cargo test -p bruhs-web thread_link
cargo test -p bruhs-desktop thread_link
```

Two live checks spend real tokens and are not in the gate:

```sh
# Web: both settings, a round trip between two threads run in the background,
# a call that waits for a reply, a read, a new thread, and queue and Stop.
cargo build -p bruhs-server
(cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43021 trunk build --dist /tmp/bruhs-thread-link-dist)
BRUHS_PLAYWRIGHT_PATH=/path/to/playwright \
  node scripts/verify-thread-link.playwright.mjs "$PWD" /tmp/bruhs-thread-link-dist --live

# Desktop (debug build): one thread starts another and gets a message back,
# then starts a third and waits for its reply inside the call.
BRUHS_DESKTOP_THREADS_SMOKE=1 BRUHS_DESKTOP_STATE_FILE=/tmp/bruhs-threads-smoke.json \
  cargo run -p bruhs-desktop
```
