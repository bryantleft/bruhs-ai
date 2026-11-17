# Bruhs crew design

Recorded 2026-11-16. Status: proposed (owner answered access and vault questions 2026-11-16).

## Intent

[dusk bot](https://dusk.bot/) (a "Grok bot alternative") presents AI agents as
contacts you text. Each has a name, a look, its own memory, routines, a
browser, saved logins and app connections, and several can share a room and
hand work to each other. The owner wants Bruhs to cover everything that
product does, inside the one bruhs-ai app, local-first, built by us rather
than through hosted brokers. Bruhs' existing customizable mascot face becomes
each agent's identity.

dusk bot's code is PolyForm Noncommercial (source-available, not open
source). Bruhs is public. **Build from its public docs only; never read or
copy its code.**

## Goals

1. **Crew.** Every always-on agent is a *bruh*: a contact with a name, a
   customizable face and colour, one job, memory, routines, an access level
   and a chat.
2. **Faces.** The Bruh face can turn, recolour and show state (asleep,
   working, needs you) on both apps.
3. **Rooms.** Group threads where @mentions hand work to bruhs and bruhs hand
   work to each other.
4. **Approvals and stop.** Per-bruh access from Read only to Full, approval
   cards for risky steps, a separate "send as me" switch, and ⌘. to stop
   everything.
5. **Computer.** Each bruh has its own persistent browser, watched live, which
   the owner can take over.
6. **Vault.** Saved logins scoped to a bruh, used without the bruh seeing them;
   1Password references.
7. **Backup.** One encrypted file holding all user data, restorable on another
   machine.
8. **Imports.** ChatGPT, Claude.ai and Grok chat exports become threads;
   OpenClaw and Hermes Agent personas become bruhs.
9. **Apps.** Token-based connectors for dusk's app list, reachable by agents
   through one tool, plus app-event triggers.
10. **Notch.** On macOS the desktop shows crew presence at the notch.

Every goal ships on web and desktop (the parity goal), except where the
platform makes it impossible; those exceptions are named below and recorded in
`docs/web-desktop-parity-goal-2026-11-15.md`.

## Non-goals

- Hosted cloud, relays, Composio, or any third-party broker that sees user
  data or tokens.
- A full Linux desktop/VM per bruh (dusk's Docker desktop). Browser only.
- Usage analytics or crash reporting.
- Commercial licensing, billing, seats.
- Windows/Linux notch equivalents (no notch; the in-app crew rail covers it).

## Anti-goals (failure even if it works)

- A bruh sees a saved password, token or 1Password value in its context.
- A cosmetic change (face, colour) cancels a bruh's pending runs or resets its
  session.
- A bruh writes or sends anything above the owner's chosen access without an
  approval.
- A backup is readable without the password, or restore silently overwrites
  data without confirmation.
- Web and desktop drift: one app shows a bruh state, room message or approval
  the other can't act on. (Crew data is server-owned, so both apps see the
  same bruhs and rooms.)
- Face animation that delays input or ignores reduced motion.
- Persimmon used for a bruh colour (it's reserved for the brand).

## Constraints from the code today

- Always-on agents (`bruhs_models::reactive::ReactiveAgent`) live in
  server SQLite `server/reactive.sqlite3`; types use `deny_unknown_fields`;
  there is no migration system (`CREATE TABLE IF NOT EXISTS`). The installed
  background service may run an older copied release, so new rows/commands can
  be rejected by an old binary.
- `SaveAgent` cancels pending runs on *any* payload change
  (`bruhs-core/src/reactive/store.rs:628`) and renaming resets the session
  (name is in the instructions stamp, `worker.rs:70`).
- Reactive turns run in Plan mode with thread access off (`worker.rs:96-107`).
- Chat threads and roles are client-owned (web IndexedDB / localStorage,
  desktop `desktop-state.json`). The server can't read roles.
- An agent's conversation is runs (`prompt`/`result`) shown as cards; there is
  no transcript view.
- `@` in the composer only inserts thread/file context chips; it routes no work.
- The mascot (`bruhs_models::logic::dither::bruh_dots`) is forward-only with
  fixed colours; `bruhs-site/src/scripts/braille.js` already has continuous
  `lookX/lookY` and custom colours to port.
- Browser tools (`bruhs-core/src/local_tools/browser.rs`): hand-written CDP,
  headed Chrome, throwaway temp profile, JS-based click/fill, in-memory grants
  keyed by thread. Device video (`routes/local_tools.rs`, one-frame-in-flight
  WebSocket) is the live-view precedent.
- Secrets: `keyring` v3, project-scoped only, created only from in-thread
  requests; per-turn `bruhs_secrets` MCP broker injects values into approved
  commands' env.
- Connections: OAuth-only enum (Google, Microsoft, GitHub, Gmail, Outlook);
  agents reach them only via `request_connection` into a command's env.
- No `aes-gcm`/`argon2` in the lockfile; no full backup; two data roots
  (desktop `…/bruhs-ai`, core/server `…/dev.bruhs.bruhs-ai`) plus keychain
  items plus web IndexedDB (21 stores) and `bruhs.*` localStorage.
- Desktop runs agent turns in-process; no feed publishes per-thread status.
  `MobileThreadState` is the closest presence projection.
- Meeting pill (`views/meeting_dock.rs`) is a non-activating NSPanel via
  objc2; no notch geometry, tray or `LSUIElement` code yet.
- Workspace lints: no truncating `as` casts, `#[expect]` not `#[allow]`,
  `// SAFETY:` on unsafe; gate is `scripts/gate.sh`.

## Approaches considered

**What a bruh is.**
- **A. Every always-on agent becomes a bruh (chosen).** It already has name,
  job, memory, routines and between-event chat — everything a dusk bot has.
  Add look, access and status.
- B. A new Bruh record wrapping agents: two concepts to keep in sync.
- C. Roles become bruhs: roles have no memory, routines or server presence.

**Rooms.**
- **Server-owned rooms beside bruh chats (chosen, revised after review).**
  Bruhs and their chats already live on the server; rooms are mostly
  bruh-to-bruh, so they live there too. That keeps handing off while the app is
  closed (background service) and shows the same room in web and desktop,
  which client-owned threads can't (web and desktop never share threads).
- Client-owned group thread (first pick): stalls hand-offs while the owning app
  is closed, and a desktop room never appears on the web.

**Computer.**
- **Persistent headless Chrome profile per bruh, live screencast, take control
  by forwarding input through CDP (chosen).** Works identically in the web
  page and the desktop, so parity is free; also gives agents real input.
- Relaunch headed Chrome on take-over: native-only, loses page state, no web
  parity. Kept as a secondary "Open in a window" escape hatch.
- Docker Linux desktop (dusk's way): adds Docker; non-goal.

**Apps.**
- **Token connectors + one generic `app_request` tool (chosen).** Every dusk
  app offers personal/API tokens, so no OAuth client registration, no broker.
  One tool keeps per-turn token overhead flat (harness goal: fewer tokens).
- One MCP tool per endpoint: hundreds of schemas in every turn.
- Composio: third-party broker; non-goal.

**Backup.**
- **Server-built streaming AES-256-GCM archive, Argon2id key (chosen).** Web
  contributes its IndexedDB dump; one file covers both apps.
- Per-app backups: two files, and the web can't read server/keychain data.

## Design

### 1. Crew and faces

**Data.**
- New `bruhs_models::crew` module:
  - `BruhLook { look: Look, face: BruhColor, mouth: MouthColor }` where
    `Look` is `Left | Front | Right` (the `@bruhs/theme` face variants; the
    renderer interpolates for animation) and `BruhColor` is a fruit token
    from an allow-list that excludes persimmon. Default = today's yellow face,
    red mouth, Front.
  - `BruhAccess { level: AccessLevel, send_as_me: bool }`,
    `AccessLevel = ReadOnly | Ask | Full`. Default `ReadOnly` (today's
    behaviour). `send_as_me` defaults off.
  - `BruhStatus = Asleep | Working | NeedsYou | Paused | Off` with
    `next_wake_ms: Option<EpochMs>`.
- Store look and access in a **separate table** `reactive_bruhs(agent_id,
  payload)` with `#[serde(default)]` fields and **without**
  `deny_unknown_fields`, so (a) look edits never touch `ReactiveAgent` and so
  never cancel runs or reset sessions, (b) older service binaries ignore the
  table. Access changes *do* cancel pending runs (they change what a run may
  do) but keep the session: the instructions stamp must not include the access
  level or the name; the current level and name are sent per run instead
  (today the stamp's "All work in this runtime is read-only" and the name
  would reset the session).
- New `ReactiveCommand::SetBruh { agent_id, look, access }` and
  `ReactiveCommand::StopAll`. To avoid an old background service rejecting
  them: the app checks the service's version on connect and restarts/upgrades
  the service when it's older (builder confirms how service upgrades work
  today in `docs/reactive-agents.md` "Keep agents running after quit").
- `ReactiveSnapshot` gains `bruhs: Vec<BruhState { agent_id, look, access,
  status, unread }>` (`#[serde(default)]`). The SSE `reactive` frame carries
  statuses so rails update without the 2 s poll.

**Access in runs.**
- `ReadOnly` = today (Plan mode, no app tools).
- **Write tools** (owner-approved list): file edits and shell commands in the
  bruh's own working folder, its browser (Computer), `app_request` writes,
  and creating tasks and calendar entries. Tasks/calendar records are
  client-owned, so the bruh proposes them through the existing draft/workflow
  path and the app that owns the records applies them (no server writes into
  IndexedDB).
- `Ask` = write tools available; each call becomes an approval. Because bruh
  runs are unattended, the approval goes to the bruh's outbox as an `ask` item
  with approve/deny (also in the notch and inline in rooms); the run blocks on
  it (bounded by a timeout that ends the run with a "waiting on you" note and
  lets `AnswerOutbox` resume it).
- `Full` = write tools without approval.
- **Code delivery** (workbench push, merge, open PR) needs `Full` **and** a
  per-bruh `can_deliver` switch (default off).
- **Sending in the owner's name** (mail send, Slack post, app writes flagged
  `sends_as_owner`) needs the `send_as_me` switch at any level; without it
  the action becomes an approval. A new `send_mail` agent tool (through the
  existing Gmail/Outlook connections and the mail store's send path) is in
  scope; it is the first `sends_as_owner` action.
- Codex can't ask for approval, so a Codex bruh offers Read only and Full
  only; Ask is shown disabled with the reason, matching how the Default mode
  treats Codex today.
- Subscriptions' `auto_run` stays read-only unless the bruh's level is `Ask`
  or `Full` (the subscription opt-in remains required).

**Faces.**
- Extend `logic::dither` with `bruh_dots_styled(size, style: &FaceStyle)`
  where `FaceStyle { look_x, look_y, face_rgb, mouth_rgb, eyes: Open|Closed }`.
  Port `lookX/lookY` geometry from `bruhs-site/src/scripts/braille.js`.
  Keep `bruh_dots` as the Front/default wrapper.
- State expressions: Asleep = eyes closed, still; Working = slow look sweep
  left↔right; NeedsYou = looks at the viewer (Front) with eyes open and a
  ring in the bruh colour; Paused = Front, dimmed; Off = desaturated.
- Motion: sweep ≤ 1 cycle per 2 s, drawn only for visible faces; reduced
  motion removes the sweep and keeps the static state. Uses the shared motion
  preference on desktop.
- Web draws on `<canvas>` (`ui/braille.rs`), desktop via `views/braille.rs`.

**Surfaces (both apps).**
- **Crew rail** in the sidebar, above the thread list, outside
  `SidebarSection` (so agents never mix into thread drag/drop): one row per
  bruh — small face, name, status line (e.g. "wakes 8:00", "needs you"),
  unread pill. `+` creates a bruh. Collapsible; ⌘⇧B focuses it.
- **Bruh page** (replaces opening Automations for a bruh): header with face,
  name, job, status, Stop. Tabs: **Chat**, **Computer**, **Memory**,
  **Routines**, **Settings**.
  - Chat renders runs as a transcript (prompt → owner bubble, result → bruh
    bubble with face; trigger label for non-chat runs; outbox asks and
    approvals inline as cards). Composer sends `ReactiveCommand::Chat`.
  - Memory, Routines reuse the existing panels (`reactive_automations/*`).
  - Settings: name, job/instructions, model, effort, access, send-as-me,
    computer on/off, vault grants, app grants, **face editor**.
- **Face editor:** live preview, Left/Front/Right, colour swatches from the
  allow-list, mouth colour, preview of each state.
- **Create bruh** dialog: name, face, colour, one-line job; optional "start
  from role" copies a role's model/effort/instructions (roles stay
  client-side; the copy is a snapshot).
- Existing Automations page keeps working; agents there show their face.

**Stop.** ⌘. stops the focused thread's turn (today) **and** sends `StopAll`
(cancels running/pending bruh runs, closes their browsers' pending actions,
pauses bruhs until resumed). The bruh page and notch have per-bruh Stop.

### 2. Rooms

- Server tables `reactive_rooms(id, payload)` and `reactive_room_messages(
  room_id, seq, payload)`. `Room { id, name, members: Vec<agent_id>, lead:
  Option<agent_id> }`; `RoomMessage { id, author: Owner | Bruh(agent_id) |
  System, text, run_id?, created_at }`. Commands `CreateRoom`, `UpdateRoom`,
  `DeleteRoom`, `PostRoom { room_id, text }`; rooms ride `ReactiveSnapshot`
  (paged messages) and the `reactive` SSE frame.
- Routing (server, shared rules in `bruhs_models::crew::room`):
  - An owner post with @mentions queues a run for each mentioned member with
    `RunTrigger::Room { room_id, message_id }`. The prompt carries the new
    message and the last N room messages (bounded chars). No mention → the
    lead; no lead → nobody (it sits, like a human group chat).
  - A finished room run posts its result as a `Bruh` message (keyed by run id,
    idempotent).
  - A bruh message that @mentions another member routes again. A hop count on
    the trigger stops loops (limit: reuse `thread_link`'s); past it the room
    gets a `System` "stopped handing off" message.
  - A bruh uses **one provider session** for its chat and its rooms (it's one
    person); the room envelope says which room it's speaking in.
  - Approvals a room run raises show inline in the room and in the bruh's chat.
- Both apps: rooms listed in the crew rail under the bruhs; "New room" picks
  members; the room view is a transcript with faces and a composer whose `@`
  lists members first (`ChipKind::Bruh`).
- Regular thread agents can hand work to a bruh: new `bruhs_threads`
  operation `message_bruh { name_or_id, message, wait }` mapping to
  `ReactiveCommand::Chat` (reads-only/Plan rules as for `message_thread`).

### 3. Computer (per-bruh browser)

- `local_tools` principals gain `Principal::Bruh(agent_id)` beside thread ids.
  Bruh grants persist in the reactive store (thread grants stay in memory).
- A bruh's browser: Chrome with `--headless=new`, persistent
  `--user-data-dir` under the core data root `bruhs/<agent_id>/browser/`,
  started on first use and closed after idle (builder picks the timeout).
  Deleting the bruh deletes the profile.
- Real input: add `Input.dispatchMouseEvent`/`dispatchKeyEvent`/
  `insertText` actions to the browser host; agents use them for click/type
  (JS click stays as fallback).
- Reactive turns for a bruh with Computer on get the workbench MCP's
  `local_browser` tool bound to that bruh's grant (today reactive turns get no
  workbench MCP).
- **Live view:** `Page.startScreencast` JPEG frames over a WebSocket
  `/api/local-tools/browser/{session}/screencast` with the device video's
  one-frame-in-flight ack. Web renders into `<img>`/canvas; desktop decodes to
  a gpui image. The Computer tab shows URL, live frames and status.
- **Take control:** pauses the bruh (its run waits at the next tool call),
  then forwards the owner's pointer/keyboard/scroll from the live view as CDP
  input. "Hand back" resumes the run with a note ("owner took over; now on
  <url>"). For captchas/passkeys that need a real window, desktop offers
  "Open in a window": closes headless, relaunches headed on the same profile
  and URL, and relaunches headless on hand back. Web can't open native
  windows; it offers take-control via forwarding only (documented parity
  exception).
- Vault fill: `local_browser` `fill` accepts `secret: { login_id, field }`;
  the server fills from the keychain and never returns the value.

### 4. Vault

- Extend secrets scope: `CredentialScope = Project(path) | Global |
  Bruh(agent_id)` (catalog field `scope`, default `Project` from existing
  `project_path`). Storage key includes the scope.
- New entry kinds: `Value` (today), `Login { url, username, password,
  totp_secret? }`, `OnePassword { reference: "op://…" }`.
- Settings → **Vault** (rename of Credentials, both apps) can now *create*
  entries, not only replace them. Bruh Settings grants entries to a bruh.
- Use: `request_secret`/browser `fill` resolve `OnePassword` at use time via
  `op read <ref>` (the `op` CLI, signed in by the owner; errors surface as
  "1Password needs you"). TOTP codes generated server-side at fill time.
- **Grants are standing approval for login fill only** (owner decision:
  "whatever is best"; this is the call). A login granted to a bruh may be
  filled into a page whose origin matches the entry's `url` without asking,
  at any access level that has Computer on, so overnight routines can work.
  A fill on any other origin is refused. Every other use (a value or password
  injected into a command's environment, an app token outside `app_request`)
  still asks each time. App tokens granted to a bruh are used by
  `app_request` under the bruh's access level.
- Values never enter model context, transcripts, logs or backups' plaintext.

### 5. Backup

- `bruhs-core::backup` + routes `POST /api/backup` (create, streams the file)
  and `POST /api/backup/restore` (upload, verify, stage).
- Contents: core data root and desktop data root minus caches (`logs/`,
  `models/`, `worktrees/`, `updates/`, browser profiles optional); SQLite
  databases copied consistently with `VACUUM INTO` (or the backup API);
  keychain items (secrets, connection tokens, calendar tokens) — included by
  default, with a checkbox to exclude; web IndexedDB + `bruhs.*`
  localStorage, dumped by the web client and sent with the create request.
  Meeting audio included by default with its size shown; can be excluded.
- Format: magic `BRUHSBAK` + version, Argon2id params + salt, then a
  tar stream encrypted with AES-256-GCM in the STREAM construction (chunked,
  so large audio doesn't need memory). Password ≥ 12 characters, entered
  twice. Crates: `aes-gcm`, `argon2` (RustCrypto; check `cargo deny`).
- Another machine: the manifest records the source home directory and data
  roots. Restore rewrites that home prefix in agent working folders, project
  paths and the credential catalog, then re-keys keychain items (their ids
  hash the project path). Paths outside the old home are kept and flagged in
  the restore summary.
- Web data is included when the backup is made from the web app. A backup made
  from the desktop says web data isn't in it and links to making one from the
  web; restore on the web reads its part from any backup that has one.
- Restore: verify the password and integrity first, show what's inside, require
  explicit confirmation that current data will be replaced, stage to a
  temp dir, and swap on restart (desktop and server); the web client clears
  and rewrites IndexedDB from the web part. Keep the replaced data as
  `pre-restore-<date>` until the owner deletes it.
- Both apps: Settings → Backup.

### 6. Imports

- New upload route `POST /api/import/upload` (multipart, size cap) returning
  `ImportCandidate`s; `GET /api/import/thread` gains the uploaded source.
  `ImportSource` gains `ChatGpt | ClaudeAi | Grok | OpenClaw | Hermes`.
- ChatGPT: the data-export ZIP's `conversations.json` (message tree via
  `mapping` + `current_node`; follow the current branch). Enable the `zip`
  crate's `deflate` feature.
- Claude.ai: the export's `conversations.json` (`chat_messages`).
- Grok: the account data export's conversation JSON. **Format unverified:**
  the builder must obtain a real export before writing the parser and record
  a sanitized fixture.
- Imported chats become threads via the existing `ImportedThread` +
  transcript-context path (no resumable session).
- OpenClaw / Hermes Agent: read their local persona and memory files (paths
  and formats to verify against real installs; sanitized fixtures) and create
  a bruh with that job and memory (memory goes through the owner-approval
  path).
- "Move in by prompt": a copyable prompt asking ChatGPT/Claude/Grok to list
  what it knows about the owner; pasting the answer proposes memory for a
  chosen bruh.

### 7. Apps

- `bruhs_models::apps`: a connector catalog `AppConnector { id, name,
  base_url (or per-account template), auth: Bearer | Basic | Header(name) |
  Query(name), token_help_url, scopes_hint, guide }` for: GitHub (PAT),
  Notion, Linear, Airtable, Todoist, Trello (key+token), ClickUp, HubSpot
  (private app), Jira (email + API token + site), Shopify (admin token +
  shop), Stripe (restricted key), Cloudflare, Slack (user/bot token), Figma.
  Gmail/Outlook/Calendar stay on their existing OAuth connections.
- Tokens are vault entries of kind `AppToken { app, account }` scoped Global
  or per bruh.
- Agents get one per-turn MCP tool `app_request { app, method, path, query,
  body }` (on the existing workbench MCP) plus `list_apps`. The server
  resolves the token, enforces the base URL (no host override), caps response
  size, redacts auth headers, and returns JSON. `GET` is a read; other methods
  are writes and follow the access level; endpoints flagged `sends_as_owner`
  per connector (e.g. Slack `chat.postMessage`) need `send_as_me` or approval.
  Each connector's `guide` is a short skill text loaded on demand, not in
  every turn.
- Settings → **Apps** (both apps): list, connect (paste token, test call),
  disconnect, grant to bruhs.
- App-event triggers: `SourceKind::AppPoll { app, query, poll_ms }` with
  built-in queries for GitHub new issues/PRs and new mail (existing mail
  store), so routines can start on "new GitHub issue" / "new email".

### 8. Notch (macOS desktop)

- A second non-activating panel (pattern from `meeting_dock.rs`) anchored to
  the notch using `NSScreen.safeAreaInsets` / `auxiliaryTopLeftArea` /
  `auxiliaryTopRightArea`; on displays without a notch, top-centre below the
  menu bar.
- Collapsed: faces of bruhs that are Working or NeedsYou beside the notch
  (nothing when all are asleep). Hover/click expands: each active bruh's
  status, latest line, approve/deny for pending asks, Stop, open bruh page.
- Coexists with the meeting pill (pill keeps its own place and priority).
- Setting: Settings → Crew → "Show crew at the notch" (default on, macOS).
- Parity: web shows the same presence in the crew rail and uses browser
  notifications for NeedsYou when permitted. Recorded as a platform exception.

## Delivery order

Each project lands separately: its own plan, tests, `scripts/gate.sh`,
web/desktop acceptance (behaviour + same-size screenshots), docs, and parity
doc entry. Then the next starts.

1. Crew + faces + access/approvals + Stop (foundation for all others)
2. Rooms
3. Computer
4. Vault (Computer's login fill depends on it; land vault before fill)
5. Apps
6. Backup (last data-shape change lands before it, so it covers everything)
7. Imports
8. Notch

## Docs to add or update

`docs/crew.md` (bruhs, faces, access, rooms), `docs/reactive-agents.md`,
`docs/thread-messages.md` (`message_bruh`), `docs/secure-credential-requests.md`
(vault), `docs/service-connections.md` (apps), new `docs/backup.md`,
import docs, `docs/web-desktop-parity-goal-2026-11-15.md` (exceptions),
`CHANGELOG.md`.

## Left to the builder

- Exact table/column names, idle timeouts, transcript window size N, hop limit
  value (reuse `thread_link`'s if sensible), colour allow-list membership.
- Rail/page layout details within `docs/ui-standards.md` and the bruhs design
  rules (all-mono, no shadows, ring + tone).
- Whether the bruh page is a new `AppView` or a mode of Automations.
- Screencast frame rate/quality within the device-video flow control.
- Connector-specific `sends_as_owner` lists and guides.
