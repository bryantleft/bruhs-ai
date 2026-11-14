# Live meeting suggestions

While a meeting is recording, bruhs can offer private notes in the live
meeting dock: action items, open questions, conflicts with your tasks or
calendar, related tasks and events, and things to follow up. Nothing is shared
with other participants and nothing acts on its own. You choose what to keep.

## Use it

1. Start recording a meeting. On the web, select **Open** on the meeting dock.
   On the desktop, select **Suggestions** on the live pill.
2. Select **Suggest now** for notes on the last two minutes of the transcript.
3. On each suggestion, choose **Add task** (action items and follow ups),
   **Copy**, or **Dismiss**. An added task lands in your task list with the
   meeting's title in its description.

With an automatic level, rounds also run by themselves after new speech or a
pause. **Pause** stops them for the current meeting only.

## Settings

Settings, General, has two rows next to the meeting summary model.

**Meeting suggestions** sets how much initiative suggestions take:

| Level | What happens |
|---|---|
| Off | No suggestions and no requests. |
| When I ask (default) | A round runs only when you select Suggest now. |
| Automatic | Rounds run after about 160 characters of new speech (at most every 30 seconds) or after a 10 second pause. |
| Automatic with alerts | Automatic, and the closed dock shows a count of new suggestions on Open (the desktop pill shows it beside Suggestions). |

**Meeting suggestion model** picks the model. Automatic uses Claude Haiku.
Any Claude or Codex model works; other runtimes cannot run an isolated,
tool-free request.

Both apps store these preferences: the web as `bruhs.meetingSuggestions` in
local storage, the desktop as `meeting_suggestions` in `desktop-state.json`.

## How a round works

This follows the Inner Thoughts framework from Bruce Liu's dissertation
*Beyond Taking Turns in Human-AI Interaction* (UCLA, 2026). Each round has
five steps:

1. **Trigger.** New speech or a pause, or Suggest now.
2. **Retrieval.** The app keeps the meeting itself and up to seven of your
   records that the talk touches: open tasks, events from a day ago to a week
   ahead, and people. Saliency is word overlap with the transcript, weighted
   by kind, so it runs in the browser without embeddings. The model sees a
   short reference such as `t2`, never a record id, and attendee names but
   never their addresses.
3. **Thought formation.** One isolated model request, with no tools, no
   session, no project configuration and no app credentials, reads the
   newest two minutes (at most 6,000 characters) and forms up to three
   notes.
4. **Evaluation.** For each note the model first argues for raising it now,
   then against, then rates it 1 to 5. Arguing both sides first keeps the
   scores from running high.
5. **Participation.** A note appears only if its score reaches your
   threshold and it does not repeat one already shown or dismissed.

The model does not think privately before answering: its reasons are in the
reply itself, which keeps a round to a few seconds on Haiku.

The threshold starts at 4 and tunes itself from what you do. When at most a
quarter of your last six or more decisions kept a suggestion, the bar rises
by one (to at most 5). When at least three quarters were kept, it falls by one
(to at least 3). Add task and Copy count as keeping; Dismiss does not. Only
the first decision on a suggestion counts. Each new meeting also eases a
raised bar one step back toward 4, because at 5 too few suggestions appear
for feedback alone to lower it.

## Usage and limits

Each round is one extra model request. Only one round runs at a time, and it
has 45 seconds. Rounds follow the same rule as next-prompt suggestions: they
stop when the provider reports that a usage window is 80% used or limited, so
the rest stays for your own chats. A failed round waits a minute before the
automatic loop tries again, or 15 seconds when the server was busy with
another round (for example one started by the other app).

Suggestions live only for the current meeting session. They are not saved
with the meeting, and reloading the page clears them. Added tasks are saved
like any other task.

## Code

- Shared rules, budgets and the session state machine:
  `bruhs-models/src/meeting_suggestions.rs`
- The model request: `bruhs-core/src/meeting_suggestions.rs`, served at
  `POST /api/agent/meeting-suggestions`
- Web: `bruhs-web/src/meeting_suggestions.rs` and
  `bruhs-web/src/components/meeting_suggestions_panel.rs`
- Desktop: `bruhs-desktop/src/views/workspace/meetings_ui/suggestions.rs`
