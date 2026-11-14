# Semantic action catalogue (incomplete)

The first reviewed group maps 86 registered commands to 45 user intents across web and desktop. Exact published-main and working-tree source profiles and dispatch comparisons are retained in JSON; matching spans do not prove identical downstream behavior. All 1,245 binding search references still require semantic/test/lifecycle review, and other entrypoint kinds remain outside this first group. The full goal remains active.

Opening a form and saving its record are separate actions. New document creates a record immediately; New task opens a form. Creating an integrated PTY differs from launching system Terminal. A route or panel is not ready merely because its state flag changed.

| Intent | Registered palette clients | Completion boundary to verify |
| --- | --- | --- |
| Open smart scheduling | desktop, web | Scheduling controls and their loaded/empty/error state are usable. |
| Open quick capture editor | desktop, web | A fresh capture editor is visible with no edit identity retained. |
| Request a day plan draft | desktop, web | The Today request is consumed and its draft/feedback is ready. |
| Open conversation import | desktop, web | Inbox import/create controls are visible and usable; saving/importing is separate. |
| Create and open a knowledge document | desktop, web | The new document is visible in its detail editor and its durable write result is observed. |
| Open web clip creation | desktop, web | Knowledge clip creation controls are usable; fetching/saving a clip is separate. |
| Open new meeting note editor | desktop, web | A fresh note editor is visible on Meetings; committing the note is separate. |
| Open model selector | desktop, web | The intended selector state and model/provider choices are usable. |
| Toggle diff panel | desktop, web | The diff surface reaches usable content/empty/error or closed state. |
| Toggle files panel | desktop, web | File controls reach ready/empty/error or closed state. |
| Toggle preview panel | desktop, web | Preview reaches usable content/empty/error or closes. |
| Toggle right panel | desktop, web | The requested right-panel surface/launcher is usable or closed. |
| Toggle tasks panel | desktop, web | Tasks panel reaches ready/empty/error or closed state. |
| Open active project in editor | web | Editor launch result/feedback is observed, or no-project feedback is visible. |
| Open project chooser | desktop, web | The chooser is visible and usable; opening the selected project is a separate action. |
| Stash prompt or open stash menu | desktop, web | A nonempty draft/chips are stashed and cleared, or the empty-draft stash chooser is usable. |
| Copy displayed pull request link | desktop, web | Clipboard contains the displayed pull URL, or the panel/no-pull notice is shown. |
| Copy displayed pull request number | desktop, web | Clipboard contains the displayed pull number, or the panel/no-pull notice is shown. |
| Open local server chooser | desktop | The local-server chooser is usable; launching/navigating a chosen server is separate. |
| Toggle sidebar | desktop, web | The applicable sidebar reaches the requested visible state. |
| Open task creation form | desktop, web | Task form is visible on the Tasks surface; submitting the task is a separate action. |
| Create integrated terminal session | web | The server-created PTY session is selected and usable, or creation feedback is shown. |
| Open system Terminal | desktop | The operating-system Terminal launch is acknowledged or an error is reported. |
| Toggle integrated terminal | desktop, web | The terminal panel is visible and reaches its session/empty/error state, or closes. |
| Continue active thread in T3 | desktop, web | The controlled bridge acknowledges the created thread, or missing-project/bridge-error feedback is shown. |
| Copy active thread link | desktop, web | Clipboard contains the client-specific thread address, or the no-active-thread outcome is observed. |
| Copy thread Markdown | desktop, web | Clipboard contains the bounded transcript and the expected notice appears. |
| Copy CLI resume command | desktop, web | Clipboard contains the provider/session/project command, or the correct missing-session/no-active-thread outcome is observed. |
| Open thread import | desktop, web | Import chooser is visible and usable; selection/import/commit are separate actions. |
| Create a thread or choose its project | desktop, web | A fresh thread is selected with a ready composer and durable result, or the required project chooser is usable. |
| Pin or unpin the active thread | desktop, web | Pinned state and ordering are reflected, with the writer result observed. |
| Settle or unsettle the active thread | desktop, web | Settled state, unread state and ordering are reflected, with the writer result observed. |
| Navigate to Automations | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Calendar | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Chat | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Inbox | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Knowledge | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Life | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Meetings | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to People | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Search | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Settings | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Tasks | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Terminal | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| Navigate to Today | desktop, web | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |

A missing palette client is not proof the app lacks another entrypoint for that intent. The JSON keeps exact source profiles, registry/dispatch locations, required variants and unresolved measurement status. No whole-action acceptance is established.

## Remaining work

1. Classify all binding search candidates, including tests/comments, lifecycle callbacks, pointer/context menus, field inputs and submit operations.
2. Review downstream loading, storage, network, completion/error/cancel/retry behavior for every registered intent and supported client.
3. Expand HTTP parameter/method/fallback, slash argument/provider/state, MCP operation and workflow/lifecycle branches into semantic actions.
4. Include user actions absent from command palette, settings changes, clipboard/OS integrations, background stream/reconnect and all supported client variants.
5. Measure complete actions against these boundaries; partial component/fixture results do not automatically accept a whole action.
