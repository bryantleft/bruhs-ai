# Suggested next prompt

**Suggest next prompt** is on by default on web and desktop. Turn it off under
**Settings → General**. After a
successful reply finishes, the composer offers a short predicted next message.
Press **Tab** to put it in the draft, then edit or send it. **Escape** dismisses
it. The hint is also clickable. Enter sends only the real draft.

Typing an exact prefix keeps the remaining ghost text. Moving the caret into
the draft, selecting text, composing with an IME, or opening a mention picker
prevents acceptance. Attachments, context chips, queued prompts and queue editing
suppress suggestions. Failed or stopped turns do not request one.

Each prediction uses one extra provider request. **Prompt suggestion model**
defaults to Claude Haiku 4.5 for Claude chats and GPT-6 Luna for Codex chats.
The main chat model is unchanged. An explicit suggestion-model choice wins.
Both local Claude and Codex logins are supported.
Changing a model or disabling predictions dismisses the current suggestion.
Saved `prompt_suggestions: false` stays off. Missing preferences follow the new
on-by-default behavior; an off choice is always written explicitly.

## Usage safeguards

Predictions reserve the last 20% of reported quota windows for user-directed
chat. Claude plan warnings and rejections from the main conversation's event
stream pause predictions until the reported reset or a healthy update for that
same quota window. Claude utilization reports also pause at 80%. If the CLI has
not supplied quota telemetry, its remaining allowance is unknown; token counts
are never used as a substitute.

Codex first reads `account/rateLimits/read` through a disposable local app-server
process using the CLI's own login. This is account metadata, not an inference
request. It creates no thread and reads no token file directly. Any active
window at 80% or above, an explicit limit rejection, malformed data, or unavailable
quota metadata skips the prediction. API-key-only accounts without plan quota
metadata therefore skip Codex predictions. No larger-model fallback or retry
runs when a prediction is unavailable.

## Request and lifecycle

The two composers share `bruhs_models::logic::prompt_suggestion`. A prediction is
keyed by thread, completed turn and model. Each turn is attempted once; empty
predictions, errors and Escape do not trigger retries. Results from older
requests cannot replace a newer prediction or revive a dismissed one. Predictions
are transient and are never saved as messages or draft text until accepted.

The native `bruhs_core::prompt_suggestion` runner executes in a disposable empty
directory, independently of the conversation session. Neither provider saves
the prediction session. Claude has no tools, settings sources or project plugins.
Codex is ephemeral and read-only, ignores
user configuration and rules, and disables shell execution, apps, plugins,
subagents, browser/computer use and web search. App credential variables are
removed. No project path, tool results, thinking or attachment contents are sent.

The browser calls `POST /api/agent/prompt-suggestion` through the companion
server. Browser requests abort on dismissal, thread changes, a new turn,
disablement and unmount; an already dispatched server request remains bounded
by the runner's deadline. Native cancellation also aborts the worker and drops
its provider stream. The SDK terminates the child when its consumer disappears.

Unlike T3's provider-native suggestions, these CLI wrappers do not expose a
cache-preserving prediction API. A separate context-only request avoids adding
synthetic prompts to the user's session. It does not reuse the turn's prompt
cache. No provider is launched for historical messages when opening a thread.

## Initial resource budgets

These are product limits, not measured latency claims:

- Context: the last six conversational messages, at most 2,000 characters each.
  Retain message tails so the answer's conclusion survives truncation.
- Output: one sentence, at most 240 characters, in a strictly validated JSON
  envelope. A model may return an empty prediction. Malformed output is rejected.
- HTTP body: 96 KiB, covering 12,000 characters even with six-byte JSON escaping
  plus the envelope. Provider reply parsing accepts at most 4 KiB.
- Work: one prediction child per process, no queued children and no automatic
  retries. A busy runner skips the optional prediction.
- Time: a provisional 20-second wall-clock ceiling including CLI startup. A
  prediction that takes longer is discarded. Debug logs report provider,
  elapsed time and success without logging conversation text. Measure these
  runs before changing the latency budget or claiming interactive speed.

Verification covers request validation, Unicode prefixes, selections, stale
results, dismissal, native Tab insertion without sending, and native undo.
