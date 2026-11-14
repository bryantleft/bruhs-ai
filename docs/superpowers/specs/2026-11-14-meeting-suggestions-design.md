# Live meeting suggestions: design

Date: 2026-11-14. Status: built. User guide: `docs/meeting-suggestions.md`.

## Intent

Bring the Inner Thoughts loop and the Visual Captions proactivity finding from
Bruce Liu's *Beyond Taking Turns in Human-AI Interaction* (UCLA, 2026) into
bruhs. While a meeting records, an assistant forms private notes about the
talk and decides on its own whether each is worth raising. The user asked for
the feature end to end on main without a design round, so every call below is
the builder's.

The finding that shapes the settings: in the Visual Captions study 6, 7 and 7
participants preferred automatic display, private suggestions and on-demand
suggestions respectively. No single level suits most people.

## Goals

- Private suggestions in the live meeting surface on web and desktop, with the
  same rules, copy and limits.
- Four proactivity levels: Off, When I ask, Automatic, Automatic with alerts.
- Retrieval from the user's own tasks, events and people.
- One-step outcomes: Add task, Copy, Dismiss.
- A threshold that adapts from those outcomes instead of being hand tuned.

## Non-goals

- Speaking aloud, interrupting, or showing anything to other participants.
  The thesis's interrupt threshold and assertive tone do not apply.
- Persisting suggestions with the meeting.
- Per-project or per-meeting saved overrides. Pause covers one meeting.

## Anti-goals

- Spending usage the user did not choose to spend: the default is When I
  ask, and rounds share the prompt-suggestion usage reserve.
- Acting on the user's behalf. Nothing is created without a click.
- Leaking record ids or attendee addresses to the model.

## Constraints found

- The server only relays live transcript chunks and never stores them, and
  tasks and calendar events live only in each app (IndexedDB on the web,
  `desktop-state.json` on the desktop). So the app must run the loop.
- The Claude and Codex command-line tools return no token probabilities, so
  the thesis's weighted score from log probabilities cannot be computed. The
  model returns an integer 1 to 5 after arguing both sides.
- Retrieval must run in WASM, so there are no embeddings: saliency is word
  overlap on titles, weighted by kind.

## Approaches considered

1. **Server-side worker** subscribing to meeting events, like reactive
   agents. Rejected: the server has no tasks or events, and the apps would
   have to push their stores to it.
2. **App-driven rounds with a stateless server call** (chosen). Each app owns
   a `LiveSuggestions` session from `bruhs_models`, polls it every two
   seconds, and sends a bounded request to `POST
   /api/agent/meeting-suggestions`, which reuses the isolated, tool-free
   runner built for next-prompt suggestions.
3. **A separate rail** beside the meeting page. Rejected: during a meeting the
   user is usually elsewhere in the app, and a second surface would duplicate
   the dock. Suggestions live in the dock (web) and above the live pill
   (desktop).

## Decisions

- Trigger: about 160 characters of new speech with rounds at least 30
  seconds apart, or at least 40 new characters followed by a 10 second pause
  with rounds at least 15 seconds apart. One round at a time. A failed round
  waits 60 seconds.
- Window: the newest two minutes, merged by speaker, at most 40 lines and
  6,000 characters, trimmed from the oldest end.
- Context: the meeting (with attendee names) plus up to seven items with
  saliency at least 0.2.
- Reply: at most three thoughts, each at most 160 characters, kinds
  action_item, question, conflict, context, follow_up, with `for`, `against`,
  `score` and an optional `source` reference. Strict JSON, unknown fields
  rejected.
- Threshold: starts at 4 within 3 to 5 and moves by one after six or more
  outcomes (a quarter or fewer kept raises it, three quarters or more lowers
  it), then the window restarts.
- Default model: Claude Haiku. Claude and Codex models only.
- Failure copy is shared (`failure_message`) so both apps say the same thing.
- Hidden thinking is off for rounds (`MAX_THINKING_TOKENS=0`). Measured on
  2026-11-14 with Claude Haiku 4.5 through the CLI: about 37 seconds and
  4,600 output tokens with thinking, about 4 seconds and 214 tokens
  without. The reply already argues for and against each note in the open.
- Replies are parsed per thought: an overlong or malformed note is dropped
  on its own, reasons are clipped to 160 characters, and the top three by
  score are kept. One bad note once cost a whole round.
- Repeats are suppressed by the overlap coefficient of title words (at least
  0.7 of the shorter note's words), because the model rephrases notes it was
  told it already showed.
- The rubric tells the model that commitments and open questions are worth a
  4 even when the user heard them. The first rubric ("most notes deserve 2 or
  3") scored a clear commitment at 2, which made the default threshold behave
  like the passive "selective participant" the thesis found least liked.

## Verification

- Unit tests: `bruhs-models` (session state machine, triggers, retrieval,
  calibration, dedupe, budgets), `bruhs-core` (reply parsing, prompt
  contents, refusal before a provider starts, thinking flag),
  `bruhs-server` (route refusal), `bruhs-web` and `bruhs-desktop` (copy and
  option lists, failure mapping).
- Live web check: `scripts/verify-meeting-suggestions.playwright.mjs` records
  through the Record button against a stand-in capture daemon, streams live
  segments through the real server, runs real Haiku rounds, and checks Add
  task, Dismiss calibration, the settings level, an automatic round on a
  pause, the alert badge and Stop. It passed on 2026-11-14.
- Desktop: the `meeting-suggestions` smoke scene renders the panel above the
  live pill for a comparison screenshot.

## Left open

- Measured acceptance: suggestion precision, latency and token cost per
  meeting hour have not been measured on real meetings.
- Project-level overrides of the level and model.
- Saving kept suggestions with the meeting record.
