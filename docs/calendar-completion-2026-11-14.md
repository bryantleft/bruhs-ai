# Calendar completion checklist

Recorded 2026-11-14. Parent: [product completion goal](product-completion-goal-2026-11-13.md).
**Status: in progress.** This records the Calendar capability inventory,
numbered acceptance cases and the first audit's fixes. It does not certify
completion. Reuse the parent goal's shared gates. The Google Calendar boundary
stays read only: bruhs requests read-only scopes and never writes to the
provider.

Audit revision: branch `t3/audit-calendar-pillar`, based on `cdf35b42`. All
fixtures are synthetic. No account was connected, and no request reached
Google or any other remote service.

## Capability inventory

| Capability | Web baseline | Desktop baseline | Required outcome |
| --- | --- | --- | --- |
| Views and navigation | Month, Week, Day; Previous, Today, Next; `+ New`. Mode pills had no selected state for assistive technology, used persimmon for selection, and were 20 to 24px tall. | Month, Week, Day with the same header. | Equivalent views, keyboard reachable, selected state announced, 32px (28px compact secondary) and 44px touch targets. |
| Local events | Create, edit, delete in one dialog; all day; seven fruit colors; notes. Stored as UTC instants in IndexedDB `events`. | Same dialog. Stored as **floating wall-clock** `NaiveDateTime` strings in the desktop state file. | Durable create/edit/delete with recovery, the same time semantics on both surfaces. |
| Time grid editing | Drag to move, drag the bottom edge to resize, click a row to create. Positions used elapsed milliseconds since midnight. | Same gestures on wall-clock `NaiveDateTime`. | Correct positions and edits on 23- and 25-hour days. |
| Focus blocks and scheduling | Smart schedule panel: availability nudges, focus protection, scheduling links, place a task as a focus block, ask the agent. | Same panel; same model. | Correct slots across DST, durable task links in both directions. |
| Booking links | `#/book/<slug>` page with six candidate slots and local booking. | `AppView::Booking` renders the Calendar page; there is no booking surface. | Supported on both surfaces, or recorded as a web-only scope. |
| Remote-change conflicts | Remote changes to an event edited locally after the last sync wait as conflicts: keep local, accept remote, duplicate local. | Same detection and resolution. | Only real local edits create conflicts; every resolution leaves editable, honest records. |
| Google connection | Settings: connect (OAuth PKCE through bruhs-server), refresh, calendar toggles, disconnect, remove Google events. | Same through the native settings. | Supported flows with honest loading, offline, failure and success states. |
| Google sync | Server polls every 5 minutes (`singleEvents=true`, `showDeleted=true`, sync tokens, 410 recovery, replay on reconnect). Web stores events in IndexedDB. | Desktop holds streamed Google events in memory; the server replays the window on connect. | Read-only, complete, recoverable, with no duplicate records. |
| Recurrence | Google series arrive expanded into instances. Local events have **no recurrence**. Tasks have their own recurrence rule. | Same. | Recorded scope decision for local recurring events; correct provider instances. |
| Task links | Month cells show tasks due that day. Focus blocks record `time_block_event_ids` on the task. | Same. Clicking a due marker opens the task. | Open a linked task from the calendar and the block's source task from the event. |

## Numbered acceptance cases

Status keys follow the parent goal: **open**, **in progress**, **awaiting
external verification**, **verified** (for the named platform and path only).

| Case | Scenario and expected result | Status |
| --- | --- | --- |
| CAL-01 | Switch Month, Week and Day; navigate Previous, Today, Next. The selected view is announced; targets meet the size standard on 1280×800, 640×480 and 390×844 touch. | **Verified on web** (browser harness, all three cells). Desktop open. |
| CAL-02 | Create, edit and delete a local event, timed and all day. Reload restores it. | Partially verified: web reload/move persistence verified; create/delete through the dialog not driven by the new harness. Desktop has unit coverage only. Open. |
| CAL-03 | On the 2026-03-08 and 2026-11-01 Los Angeles DST days, a 10:00 event sits on the 10:00 row, clicking the 11:00 row prefills 11:00, and dragging to the 14:00 row saves 14:00 local. | **Verified on web** after the fix (failing before: rows were off by one hour). Desktop is wall-clock by construction; no GPUI regression run. |
| CAL-04 | Slot search (focus blocks, booking links) keeps working hours on both sides of a DST change inside its 30-day window. | **Verified in shared model tests** after the fix (failed before: the slot was 08:00 PST). Web and desktop callers now pass per-instant offsets. |
| CAL-05 | A Google all-day date (`2026-11-02`) shows on that date only in every zone; previously stored records migrate once. | **Verified on web** (harness, model tests). Desktop normalizes on ingest; no GPUI run. |
| CAL-06 | Google events are read only on every path: no drag, resize, edit, save or delete; the dialog offers the provider link instead. | **Verified on web**, including a forged drop. Desktop open: blocks show a grab cursor and drag affordance that silently do nothing, and clicking a Google block opens nothing. |
| CAL-07 | Sync only issues GET requests to the Calendar API with read-only scopes. | **Verified** with wiremock (`a_sync_only_reads_from_the_provider`) and scope source review. Live Google: awaiting external verification. |
| CAL-08 | A remote change to an untouched synced event applies directly; only an edit made after the last completed sync becomes a conflict. | **Verified in core tests** after the fix (failed before: every remote change after the second sync was deferred as a false conflict). |
| CAL-09 | A failed calendar sync stays visible as an error and does not advance the last-sync time. | **Verified in core and model tests** after the fix (failed before: SyncCompleted cleared the error, and SyncFailed advanced the sync time). Settings UI not driven. |
| CAL-10 | Resolving a conflict with Duplicate leaves an editable, deletable local copy without provider identity. | **Verified in model tests** after the fix. The defect was found by source review: the copy kept `source: google`, which the web treats as read only and undeletable. |
| CAL-11 | Reconnect and offline: events stay readable offline; the stream reconnects with backoff and replays the window; replayed frames do not duplicate. | Partially verified: harness confirms a replayed frame does not duplicate. Offline/reconnect UI and live network loss are open. |
| CAL-12 | Recurrence: provider series instances appear across the visible range; local recurrence scope is decided. | Open. Initial sync lists only `now − 1 day` to `now + 14 days` and then continues by sync token, so instances of an unchanged series beyond that window may never arrive until a full resync. Needs live verification and likely a sliding window. Local recurring events are not implemented. |
| CAL-13 | Time zones: a change of the machine zone keeps instants on web; desktop local events are floating wall times. | Open (parity decision). Web stores instants; desktop stores wall clock, so the same event moves on one surface and not the other after travel. |
| CAL-14 | Task links: a due marker opens its task; a focus block opens its source task. | Web due marker **verified by source and build only** after the fix (it only set a hidden selection before). Focus block back-link is open on both surfaces. |
| CAL-15 | Event dialog actions remain reachable at 640×480. | **Verified on web** after the fix (failing before: Save and Cancel were below the viewport with no scroll). |
| CAL-16 | Keyboard: time-grid events are focusable and open with Enter or Space; controls have accessible names. | **Verified on web** for Enter on a focused block (time-grid blocks were not focusable before). Desktop blocks are focusable by source review. |
| CAL-17 | Web and desktop parity with the same synthetic records at the same viewport. | Open: no same-size desktop screenshot pass was run. |
| CAL-18 | Performance of calendar navigation and dense days under the performance goal. | Open; reuse `calendar_costs` evidence when audited. |

## Defects found and fixed

Each fix has a test that failed first, except where noted.

| Defect | Fix | Evidence |
| --- | --- | --- |
| The sync engine stamped mapped Google events with the mapping time, which is later than the tick time `SyncCompleted` reports. From the second sync on, every remote change to an untouched event was deferred as a false "local edit" conflict and never applied. | `classify` takes the tick time for `created_at`/`updated_at` ([mapping.rs](../bruhs-core/src/google_calendar/mapping.rs)). | `synced_records_are_not_newer_than_the_sync_that_delivered_them` failed (`…047 > …044`), now passes. |
| A calendar whose listing failed emitted `SyncFailed`, then the tick emitted `SyncCompleted`, which cleared the error: a failed sync looked successful. | `run_tick` skips `SyncCompleted` when any calendar failed ([sync_engine.rs](../bruhs-core/src/google_calendar/sync_engine.rs)). | `a_failed_calendar_does_not_report_a_completed_sync` failed, now passes. |
| `SyncFailed` advanced `last_synced_at` on web and desktop, so the settings showed a fresh sync and conflict detection moved its boundary past changes never received. | One shared reducer, `GoogleCalendarSyncState::apply_status`, used by [web](../bruhs-web/src/calendar_sync_stream.rs) and [desktop](../bruhs-desktop/src/views/meetings_hub.rs). | `a_failed_sync_keeps_the_last_completed_sync_time`. |
| Google all-day dates arrive as UTC midnight; local all-day events are local midnight. In Los Angeles a 2026-11-02 holiday started on November 1 at 16:00 and spanned two cells; east of UTC it spilled into the next day. | `CalendarEvent::normalize_provider_all_day` on stream ingest (web, desktop) and a one-time saved migration of stored records at web hydration ([workflow_executor.rs](../bruhs-web/src/workflow_executor.rs)). | Model tests; harness checks storage and the month cell. |
| Web time grid positioned events, click-to-create and drag/resize by elapsed time since midnight: on DST days everything was one row off. | Shared wall-clock helpers in [`calendar::zone`](../bruhs-models/src/calendar/zone.rs) (JavaScript gap/overlap semantics), used by [timegrid_view.rs](../bruhs-web/src/components/timegrid_view.rs). | Zone unit tests; harness spring/fall checks. |
| Slot search used one UTC offset captured at the start of a 30-day window: after a DST change, working hours shifted by an hour (09:00 became 08:00 PST). | `first_available_slot_in_zone` with a per-instant offset; web booking/schedule and desktop scheduling use it. The fixed-offset method remains for callers that want it. | `slot_search_keeps_working_hours_across_daylight_saving_end` (asserts the old behavior too). |
| Web let Google events be dragged and resized: a local-only move looked like a provider change, unlike the read-only dialog and desktop. | Google blocks are not draggable, have no resize handle, and move/resize refuse read-only events even for a forged drop. | Harness `read-only provider` stage. |
| Duplicate during conflict resolution produced a copy that kept `source: google`, so the "local copy" was read only and undeletable on web. | The copy becomes a local event with no provider id, calendar or link. | `a_duplicated_provider_event_becomes_an_editable_local_event` (written before the fix, but first run after it during a resource hold, so no recorded red run). |
| Web calendar header: 20 to 24px targets, no announced view state, persimmon used for selection. | `bruhs-control` sizing (32px, 28px for mode pills, 44px touch), `aria-pressed`, a named view group, blueberry selection, labelled New event. | Harness target and state checks on all cells. |
| Event dialog actions were unreachable at 640×480. | Top-anchored dialog with a scrolling field area, as Inbox does. | Harness compact cell. |
| Time-grid blocks were mouse-only. | `role="button"`, `tabindex`, Enter/Space, labels that name read-only provider events. | Harness keyboard check. |
| Month due-task markers only set a hidden selection. | They open the task in Tasks and focus its title, as Search and Today do. | Source and wasm clippy; not driven by the harness (no synthetic task seeding yet). |
| Shared `ui::Button` used `min-h-8`/`min-w-8`/`-7`, which this Tailwind v3 build never generates: Default buttons rendered 29.5px tall. | Arbitrary 32px/28px values in [button.rs](../bruhs-web/src/ui/button.rs). **Cross-pillar:** every dialog using `Button` changes height slightly. | Harness dialog target checks. |

## Cross-pillar and open items

These need files outside this audit's scope or a product decision.

- **Today/day planning (not edited here):** desktop `planning_calendar_events`
  in `today_ui.rs` converts wall times with `Local.from_local_datetime(..).single()`,
  which drops events in the repeated hour of a fall-back day from Today and
  from slot search. Use `earliest()` or the shared zone helpers.
- **Other `min-h-8`/`min-w-8` users** (no rule generated):
  `ui/input.rs`, `today_page.rs`, `database_panel.rs`, `files_panel.rs`,
  `api_client_panel.rs`, `reactive_automations.rs`, `rich_response*`.
  Their owners should switch to arbitrary values.
- **Desktop read-only affordance and inspection:** Google time-grid blocks
  show a grab cursor and drag preview, and clicking one opens nothing
  (`open_event_edit` searches local events only). Needs a read-only flag on
  `DesktopCalendarEvent` and a read-only detail.
- **Desktop time semantics:** local events are floating wall times; web uses
  instants. Choose one model and migrate.
- **Accept remote** applies only start and end, although conflicts are
  detected on title and notes too; the conflict record does not carry the
  remote title.
- **Sync window:** see CAL-12. A periodic full resync or a sliding window is
  likely needed for long-running recurring series; confirm against live
  Google behavior before changing it.
- **Google dialog color and Save:** the read-only dialog shows "Persimmon"
  for an uncolored Google event, and its disabled Save keeps the filled
  persimmon look at 60% opacity, which still reads as available.
- **Desktop booking:** `#/book/<slug>` has no desktop surface.
- **Focus block back-link:** the event does not link to its source task on
  either surface.

## Evidence log

| Date | Cases | Work and verification | Remaining |
| --- | --- | --- | --- |
| 2026-11-14 | All | Inventory and cases recorded from source review of web, desktop, models, core and server. | Implementation and verification as listed. |
| 2026-11-14 | CAL-07 to CAL-10 | Red: `target/calendar-qa/core-red.log` (2 failing core tests). Green: see verification below. | Live provider acceptance. |
| 2026-11-14 | CAL-01, 03, 05, 06, 11, 15, 16 | Browser harness [`calendar_browser.mjs`](../bruhs-web/tests/calendar_browser.mjs) against a debug Trunk build, America/Los_Angeles, three cells. | Desktop GPUI checks, same-size parity screenshots. |

### Verification

All runs on macOS (Darwin 25.3), debug profile, branch `t3/audit-calendar-pillar`.

| Check | Command | Result |
| --- | --- | --- |
| Shared models | `cargo test -p bruhs-models --lib --locked -- calendar scheduling google_calendar` | 38 passed, 1 ignored (timing diagnostic). |
| Core sync | `cargo test -p bruhs-core --lib --locked google_calendar` | 62 passed. Before the fix: 60 passed, 2 failed (`target/calendar-qa/core-red.log`). |
| Lints | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo clippy -p bruhs-web --target wasm32-unknown-unknown --locked -- -D warnings` | Clean. |
| Calendar browser | `node bruhs-web/tests/calendar_browser.mjs` on a debug `trunk build` | 159 passed, 0 failed (53 per cell at 1280×800, 640×480, 390×844 touch); 12 screenshots reviewed in `target/calendar-qa/`. The first runs found the unreachable compact dialog and the 29.5px shared buttons, both fixed. |
| Inbox regression for the shared Button change | `BRUHS_INBOX_UI_DIST=target/calendar-ui-dist node bruhs-web/tests/inbox_browser.mjs` | 399 passed, 0 failed (`target/calendar-qa/inbox-harness.log`). Debug bundle, not the release capture. |
| Gate | `scripts/gate.sh --quick` | Partial. fmt, clippy (native, all targets and features) and clippy-wasm passed. The test step finished compiling but was stopped while running because free disk fell below the 20 GB limit set for this audit (space shared with another audit). The already-built binaries were then run with `cargo test --workspace --locked --no-fail-fast` (no compilation): **3,022 passed, 1 failed, 54 ignored** across 32 suites, including bruhs-desktop 800, bruhs-web native 431, bruhs-models 512 and bruhs-core 679 (`target/calendar-qa/workspace-test.log`). The failure is `delivery::gitlab::tests::gitlab_branch_stack_merges_bottom_up_with_sha_leases_and_retargeting`, a timeout (`Elapsed`) under full-suite load in code this branch does not touch; it passed 3 of 3 runs on its own. `web-assets` passed. `site-test` and `mobile` were not run (no `node_modules` in this worktree, and nothing was installed because of the disk limit); `bench-check` was not run. The full gate, the wasm build and `trunk build --release` remain to be run before merge. |

Not verified: the desktop app in a real window (no GPUI calendar regression
or screenshots), the release bundle, live Google OAuth/sync/reconnect,
physical touch devices, screen readers, and performance.

### How to reproduce

```sh
cargo test -p bruhs-models --lib --locked -- calendar scheduling google_calendar
cargo test -p bruhs-core --lib --locked google_calendar
cd bruhs-web && env NO_COLOR=true trunk build --dist ../target/calendar-ui-dist && cd ..
node bruhs-web/tests/calendar_browser.mjs
```

The harness uses isolated browser contexts, the real IndexedDB writer and a
fixed clock (`page.clock.setFixedTime`) on 2026-11-01 and 2026-03-08. It
seeds three synthetic events, streams synthetic Google frames from a local
fixture, refuses stale assets, and fails on any non-GET request or any
request outside loopback. It does not exercise the desktop app, a real
Google account, a physical touch device or screen readers.
