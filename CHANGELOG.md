# Changelog

Notable user-facing changes to bruhs are documented here, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/). Each version
is one signed release on [GitHub](https://github.com/bryantleft/bruhs-ai/releases),
and [bruhs.ai/changelog](https://bruhs.ai/changelog) shows the same entries.

## [Unreleased]

### Added

- Added messages between threads. An agent can list your threads, read one,
  and send one a message or start a new thread; the other thread's agent can
  answer back. A message waits in that thread's queue like one you typed, so
  you can still edit or remove it, and it shows who sent it. An agent can also
  wait for the other thread's reply. Plan turns can list and read threads.
- Added two settings under General: Messages between threads (Read and
  message, Read only, or Off) and Agent message limit, which stops a chain of
  agent messages after 2, 4, 6 or 10 and hands back to you.

- Added roles under General: a named model, effort, access mode and
  instructions that an agent can start a thread with, so one thread can
  orchestrate while others write or run on the models you chose. A project
  can override a role, and a role can narrow your access mode but never widen
  it. See `docs/model-roles.md`.
- Added models by task under General: choose the model for meeting summaries
  and for each provider's second opinion.

### Changed

- The web app now runs threads side by side, up to three at once. Sending in
  one thread no longer waits for another thread's turn to end, a queued prompt
  starts when its thread is free even if that thread is not open, and each
  thread asks for its own approvals.

## [0.1.1] - 2026-11-09

This release fixes the calendar around daylight saving time and tidies the
app's pages and controls.

### Added

- Added a Release notes link to Settings → Updates that opens this changelog
  at the version on offer.

### Fixed

- Fixed the calendar showing a day twice and today in the wrong column in the
  month a daylight saving change happens. Every calendar view now steps by
  calendar date.
- Fixed imported threads showing raw model ids such as
  `claude-haiku-4-5-2025…` in the composer; they now show the model's name.
- Fixed the composer's buttons wrapping onto a second line in narrow windows.
  Attach and Stash drop their labels instead and keep their icons.
- Fixed the Today icon showing as a dot on the web, and Today and Inbox
  showing no icon on desktop.
- Fixed Agent workflows suggesting a focus block when you have no tasks.
- Fixed thread titles showing the backticks from a first message such as
  "Implement `compute_invoice`", in the sidebar, the chat header and the
  import list.

### Changed

- Gave Inbox, Knowledge, People, Meetings and Terminal the same page header
  as Today and Life, and desktop Today the same header as the web.
- Made empty Today cards a quiet line instead of a dashed box.
- Made every Settings menu use the same dropdown, with inputs at the same
  height.
- Made Inbox, Life and meeting notes controls match the rest of the app.
- Rewrote Inbox and Meetings messages that described internals, and fixed
  singular counts such as "1 local record".

## [0.1.0] - 2026-11-09

The first public release: a local-first agent for your whole day, as a
signed Mac app and a web app that both run on your machine.

### Added

- Agent threads with Claude Code and Codex, on your own logins: a live
  timeline that folds finished turns, tool approvals, Steer to redirect
  Claude mid-reply, `/compact`, queued follow-ups, and a prompt stash (⌘S).
- Project threads in the current checkout or a new worktree, with Edit from
  here to rewind a thread and its files to an earlier message.
- A diff panel with a code, tests and docs filter, viewed marks and ⌘F find;
  a terminal drawer (⌘J); a preview panel (⌘⇧J) for local dev servers; and a
  Files tab.
- Thread import from Claude Code, Codex, T3 Code and Cursor, and Continue in
  Claude Code, Codex or T3 Code, or copy a thread as Markdown.
- Today (⌘⇧Y): your schedule, focus tasks, overdue work and meeting
  decisions, with Plan my day to preview focus blocks you can undo.
- Tasks and a calendar with Smart scheduling, local booking links, and
  read-only Google Calendar sync.
- Meetings: on-device recording of your microphone and the other side of the
  call, Whisper transcription with live previews, speaker labels and voice
  identity across meetings, summaries, editable notes, and a compact live
  meeting dock.
- People, Knowledge (Markdown notes and web clips), Inbox, and Life, a
  private finance ledger and health journal hidden from agents until you opt
  in.
- Capture anything (⌘⇧Space) and search across everything (⌘⇧F).
- Agent workflows that preview a multi-step change you can undo, and an MCP
  server that lets other agents propose changes for you to approve.
- Private credential requests: agents ask for a secret through a separate
  form, and the value goes to your keychain, never into the chat.
- Settings → Updates: update checks for the Claude Code and Codex CLIs with
  one-click updates, and for bruhs itself.
- The Mac app (macOS 13 or later, Apple silicon), signed and notarized. It
  starts its own server and meeting-capture daemon, and installs its updates
  when you choose Restart to update.
- Editable keyboard shortcuts for every action, and a command bar (⌘K).

[Unreleased]: https://github.com/bryantleft/bruhs-ai/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/bryantleft/bruhs-ai/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/bryantleft/bruhs-ai/releases/tag/v0.1.0
