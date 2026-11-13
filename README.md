# bruhs

A local-first, keyboard-native agent for your whole day. Coding and chat
agents, tasks, calendar, meetings, people, notes, money and health share one
window, and your data stays on your machine.

bruhs runs on macOS as a native app (GPUI) and in the browser (a Leptos WASM
app served by a local Rust server). Agents run through the Claude Code and
Codex CLIs you already have, on your own logins.

## What it does

- **Agent threads.** Chat or code with Claude or Codex in a project, with
  diffs, a terminal drawer (`⌘J`), a preview pane (`⌘⇧J`), worktrees,
  branch and PR context, a model picker, prompt stash (`⌘S`), and threads
  that keep running in the background. Threads import from Claude Code,
  Codex, T3 Code and Cursor, and continue in Claude Code, Codex or T3 Code
  (or copy out as Markdown).
- **Today.** `⌘⇧Y` combines the schedule, ready focus work, overdue triage,
  waiting-on items, relationship follow-ups and recent meeting decisions.
  "Plan my day" proposes conflict-aware focus blocks, previews every change,
  and applies or undoes them as one local transaction.
- **Tasks and calendar.** Status, priority, cycles, dependencies, estimates,
  recurrence and waiting-on context. The calendar's Schedule view turns
  estimates into protected focus blocks; Google Calendar syncs read-only.
- **Meetings.** Local recording of your microphone and, on macOS, the other
  side of the call; on-device Whisper transcription, live previews,
  speaker labels, voice identity across meetings, summaries, and a chat
  thread per meeting.
- **People.** Profiles built from meetings and capture: observations,
  reminders, aliases, merges and per-person opt-outs.
- **Knowledge and capture.** Documents, source clips and notes. `⌘⇧Space`
  captures a task, note, event, contact or message draft from anywhere;
  `⌘⇧F` searches threads, tasks, events, meetings, people, transcripts and
  notes.
- **Life.** A local finance ledger (accounts, budgets, income streams,
  holdings, CSV import) and health journal (workouts, sleep, recovery,
  habits). Neither reaches an agent unless you opt in, and exact amounts
  have a second gate.
- **Workflows.** Cross-domain agent plans are previewed field by field, run
  step by step with permission checks, and can be undone. Every domain has
  separate read and write grants.
- **Private credentials.** When an agent needs a secret, it asks through a
  separate form; the value goes to the OS keychain and never into the chat.
  See [credential requests](docs/secure-credential-requests.md).
- **MCP.** `bruhs-mcp` exposes the workspace to MCP clients as typed tools
  that propose changes for you to approve. See
  [local MCP setup](docs/local-mcp-workspace.md).

Every action is reachable from the command bar (`⌘K`), and most are one
shortcut away.

The standing [configurability goal](docs/configurability-goal-2026-11-09.md)
tracks user control over models, project profiles, summaries, privacy,
recording, automation, memory, and appearance across web and desktop.

## Install

Download **Bruhs** for macOS (13 or later, Apple silicon) from the
[latest release](https://github.com/bryantleft/bruhs-ai/releases/latest):
open `Bruhs-macOS-arm64.dmg` and drag Bruhs to Applications. It is signed
with a Developer ID and notarized, and it updates itself. What changed in each
release: [CHANGELOG.md](CHANGELOG.md) or [bruhs.ai/changelog](https://bruhs.ai/changelog).

The app starts its own server (which also serves the web app on
`http://127.0.0.1:3001`) and capture daemon. Sign in to Claude Code
(`claude auth login`) or Codex (`codex login`) first: bruhs runs the CLIs you
have.

## Run from source

```bash
scripts/dev.sh
```

That starts the capture daemon, the API server and the web app with unified
logs; Ctrl-C stops all three. It serves the app at
`https://bruhs.localhost` through [portless](https://www.npmjs.com/package/portless)
(or `http://bruhs.localhost:1355` if you skip the sudo prompt).
`PORTLESS=0 scripts/dev.sh` uses plain `127.0.0.1` ports instead.

Or run the pieces yourself:

```bash
cd bruhs-server && cargo run     # API + web app on :3001
cd bruhs-web && trunk serve      # optional: hot reload on :8080
cargo run -p bruhs-desktop       # the native app
```

You need:

- Rust (the pinned toolchain in `rust-toolchain.toml`, with the
  `wasm32-unknown-unknown` target) and [Trunk](https://trunkrs.dev)
  (`cargo install trunk`).
- Python 3.9+ for Trunk's asset hook. Release builds keep
  originals and add smaller gzip sidecars; debug builds skip compression. The
  companion server negotiates those sidecars without compressing API or SSE
  replies. Run `python3 -m unittest discover -s bruhs-web/tests -p 'test_precompress.py'`
  to check the release artifact contract.
- [portless](https://www.npmjs.com/package/portless) for the named-URL flow
  (`npm i -g portless`).
- The `claude` CLI, signed in with `claude login`. Claude runs use that login:
  an `ANTHROPIC_API_KEY` in the environment is removed from the CLI's
  environment (it would switch to API billing) unless you set
  `BRUHS_CLAUDE_USE_API_KEY=1`.
- The `codex` CLI, signed in with `codex login`. Codex has no approval
  prompt, so the **Default** permission mode runs it in a read-only sandbox;
  **Auto-accept** lets it write the workspace and **YOLO** runs it
  unsandboxed.

The desktop app keeps its state, attachments and logs in
`~/Library/Application Support/bruhs-ai` on macOS, `$XDG_DATA_HOME/bruhs-ai`
on Linux and `%APPDATA%\bruhs-ai` on Windows. `BRUHS_DESKTOP_STATE_FILE`
moves the state file; the log is `logs/bruhs-desktop.log` (rotated past 5 MB),
and `BRUHS_DESKTOP_LOG` sets its level.

### Ports

| Service       | Env var              | Default |
|---------------|----------------------|---------|
| bruhs-server  | `BRUHS_SERVER_PORT`  | `3001`  |
| bruhs-capture | `BRUHS_CAPTURE_PORT` | `3002`  |
| bruhs-web     | `trunk serve --port` | `8080`  |

When the server or daemon move, point the others at them:
`BRUHS_CAPTURE_URL` (server and desktop → daemon), `BRUHS_SERVER_URL`
(desktop → server), and `BRUHS_API_URL` for the web app (baked in at build
time). The web app gets live updates over one stream per tab,
`/api/events/stream`. The browser never calls the daemon directly; it goes
through the server's `/api/capture/*` proxy.

## Privacy and security

- **Data stays local.** The web app stores threads, tasks, events, notes,
  contacts and the rest in IndexedDB; the desktop keeps the same records in
  its data directory. The server keeps meetings, transcripts and contacts in
  a private SQLite file (`server/meetings.sqlite3` in the app data
  directory, or `BRUHS_MEETING_DB`). Back it up with a SQLite-aware tool.
- **The local server only answers local pages.** It checks Host and Origin
  before routing and accepts loopback origins plus the ones you configure
  (`BRUHS_API_URL`, `BRUHS_WEB_ORIGIN`). See
  [local server access](docs/local-server-access.md).
- **The capture daemon needs a token.** It writes a fresh bearer token at
  every launch (`capture/daemon-token` in the app data directory,
  owner-only); the server and the desktop read it, the browser never does.
- **The microphone opens only when you ask.** Recording is explicit;
  audio-driven auto-record needs a separate, expiring opt-in each session.
  See [microphone activity consent](docs/microphone-activity-consent.md).
- **Update checks are optional.** Settings → Updates → **Check for updates**
  turns off every online lookup.

## Staying up to date

**Settings → Updates** (web and desktop) checks the provider CLIs and bruhs
itself.

- **Provider CLIs.** bruhs reads which installer owns `claude` and `codex`
  (mise, Claude's native installer, Codex's standalone install, Vite+, Bun,
  pnpm, Yarn, Volta, npm or Homebrew), asks that installer's source for the
  latest version, and warns when an installed version is outside the range
  bruhs supports (Claude Code `>=2.1.280`, Codex `>=0.156.0`). **Update now**
  runs the owner's update command detached, so closing a window never stops
  it halfway; updates through the same installer queue. A CLI bruhs cannot
  update gets the command to run yourself.
- **The app.** Bruhs.app checks the latest GitHub release, downloads the
  update in the background, and verifies it against the release's
  `SHA256SUMS` and a code requirement pinning the same Apple Developer ID
  team. The sidebar then offers **Restart to update**; the swap checks the
  signature again before it moves the new app in.
- **A source checkout.** The build records its commit and compares it with
  `origin`'s default branch (`git ls-remote`, read-only). bruhs never pulls;
  Settings shows the command to pull and rebuild.

## Google Calendar

bruhs connects to Google Calendar with OAuth2 + PKCE and your own Google
Cloud OAuth client. The client secret never leaves bruhs-server; tokens live
in the OS keychain.

1. In the [Google Cloud console](https://console.cloud.google.com/apis/credentials),
   create or pick a project and enable the **Google Calendar API**.
2. Configure the OAuth consent screen (External; scopes
   `.../auth/calendar.events.readonly`, `.../auth/calendar.readonly`,
   `openid`, `email`).
3. Create an OAuth 2.0 Client ID (Web application) with the redirect URI
   `http://127.0.0.1:3001/api/calendar/google/oauth/callback`.
4. Start the server with the credentials:

   ```bash
   export BRUHS_GOOGLE_CLIENT_ID=<id>.apps.googleusercontent.com
   export BRUHS_GOOGLE_CLIENT_SECRET=<secret>
   cd bruhs-server && cargo run
   ```

5. In Settings → Google Calendar, click **Connect**.

bruhs syncs the next 14 days of every calendar you enable (your primary
calendar by default) every 5 minutes, and the calendar list every hour;
**Refresh** syncs now. Google events are read-only in bruhs. Disconnecting
revokes the refresh token and clears the keychain entry.

## Meetings

The `bruhs-capture` daemon (`cd bruhs-capture && cargo run`, or started by
Bruhs.app) records on `127.0.0.1:3002`. Audio, models and transcripts live
under `~/Library/Application Support/dev.bruhs.bruhs-ai/`. While the daemon
is down, **+ Record** is disabled.

- **Audio.** The first recording asks for microphone access. On macOS the
  daemon also records system audio through ScreenCaptureKit, so transcripts
  cover both sides of a call; grant Screen Recording when asked (System
  Settings → Privacy & Security → Screen Recording if you missed it). Linux
  and Windows record the microphone only. Each meeting keeps
  `<meeting>.mic.wav`, `<meeting>.sys.wav` and the mixed file Whisper reads.
- **Transcription.** Whisper runs on device by default (Metal on Apple silicon). While
  recording, both channels are transcribed in ~5-second batches as previews;
  at stop, a full pass replaces them. Pick the models in Settings: Tiny (75
  MB), Base (150 MB, default), Small (470 MB) or Large v3 Turbo (1.6 GB,
  multilingual), with a separate, smaller **Live preview model** if you
  like. Models download on first use. Settings → Transcription also supports a
  configured **OpenAI-compatible server** and its model for the final pass;
  selecting it uploads recorded audio to that server after Stop. Live previews
  stay on device. See [provider setup](docs/transcription-providers.md) and the
  standing [quality improvement goal](docs/transcription-goal-2026-11-09.md).
- **Speakers.** Microphone segments are **Me** and system-audio segments
  **Them**. sherpa-onnx voice embeddings split **Them** into Speaker 1,
  Speaker 2…, and match voices to People profiles across meetings (cosine
  threshold 0.5–0.95, default 0.75; auto-linking can be turned off). Rename
  or link a speaker from its chip. Enrolling a person's voice (five seconds
  on their profile) gives the best matches. Optionally, Apple's Vision
  framework reads participant names from Zoom, Meet or Teams on device
  while you record. Segments under 85% confidence show a badge.
- **Summary and chat.** After transcription, Claude summarizes the meeting
  (TL;DR, action items, decisions, topics; `claude-sonnet-4-5` by default)
  and bruhs opens a chat thread that carries the transcript, so you can ask
  follow-ups. Codex turns cannot take that context, so use Claude for
  meeting chat.
- **People observations.** Summaries also add topics, decisions, action
  items, tone and quotes to the speakers' People profiles. Turn this off
  globally, per meeting, or per person (*untracked*); **Re-extract** reruns
  it after you rename speakers.
- **Auto-record.** bruhs can offer to record Google Calendar events with at
  least N attendees (default 2): **Record**, **Skip once** or **Always skip
  this event**, or start silently if you turn off the prompt. It stops at
  the event's end plus a delay (default 60 s). Separately opted in, it can
  notice a minute of continuous speech and offer to record, starting after a
  30-second countdown. Everything here is off by default.

## Architecture

| Crate | What it does |
|---|---|
| [`bruhs-web/`](bruhs-web/) | Leptos WASM app, the browser UI |
| [`bruhs-server/`](bruhs-server/) | Axum API; serves the web app and proxies the capture daemon |
| [`bruhs-models/`](bruhs-models/) | Types, wire formats and pure logic shared by every surface (WASM-safe) |
| [`bruhs-core/`](bruhs-core/) | Native app logic: agent runs, threads, tasks, meetings, git, terminal |
| [`bruhs-capture/`](bruhs-capture/) | Capture daemon: audio, transcription, diarization |
| [`bruhs-desktop/`](bruhs-desktop/) | Native GPUI app |
| [`bruhs-mcp/`](bruhs-mcp/) | MCP server over stdio |
| [`claude-sdk-rs/`](claude-sdk-rs/) | Claude Code CLI wrapper |
| [`codex-sdk-rs/`](codex-sdk-rs/) | Codex CLI wrapper |
| [`bruhs-site/`](bruhs-site/) | The bruhs.ai landing page |
| [`bench/`](bench/) | A task benchmark across agent harnesses; see [results](bench/RESULTS.md) |

Styled with the [bruhs design system](https://design.bruhs.dev/llms.txt):
Monaspace type, fruit-named tokens, warm dark surfaces, no drop shadows.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the checks, code rules and how
releases are cut.

## License

[MIT](LICENSE). Third-party components keep their own licenses; the app
ships them in `THIRD_PARTY_LICENSES`.
